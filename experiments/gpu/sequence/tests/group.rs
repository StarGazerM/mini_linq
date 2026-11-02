use cq_gpu_runtime::{Codec, DeviceColumns, Dictionary, Pool};
use cq_gpu_sequence_runtime::Result;
use cq_lang_sequence::{Program, check_grading};
use quote::ToTokens;

/// Accumulator providers of every emitted `fold` call.
fn fold_providers(generated: &syn::Expr) -> Vec<Option<syn::Type>> {
    struct Folds(Vec<Option<syn::Type>>);
    impl<'a> syn::visit::Visit<'a> for Folds {
        fn visit_expr_call(&mut self, call: &'a syn::ExprCall) {
            if let syn::Expr::Path(path) = &*call.func
                && path
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "fold")
            {
                self.0
                    .push(path.qself.as_ref().map(|qself| (*qself.ty).clone()));
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    let mut folds = Folds(Vec::new());
    syn::visit::Visit::visit_expr(&mut folds, generated);
    folds.0
}

#[test]
fn translates_grouped_canonical_publication() {
    let mut program: Program = syn::parse_quote! {
        physical {
            relation R(k:u32,id:u32,value:i64) = (data);
            r = let canon rename R@0 {k -> k,id -> id,value -> value};
            grouped = let group r by [k] { n:usize=count(); total:i64=sum(value); low:Option<i64> = min(value); high:Option<i64> = max(value); };
            return grouped as answer(k,n,total,low,high);
        }
        structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; grouped_rows = ::cq_gpu_sequence_runtime::group::GroupedRows<_> { reads [exact, range, membership], build typed }; group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed }; }
            r host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,i64)> [];
            r rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
            r canon = rows;
            grouped rows = @grouped_rows as ::cq_gpu_sequence_runtime::group::GroupedRows<_>;
            grouped group = @group_map as ::std::collections::HashMap<_, (usize, i64, Option<i64>, Option<i64>)>;
        }
        sequence {
            source=store rows r;
            feed=canon flatten tile 256 source;
            valuations=store feed;
            result=store group grouped valuations;
            output count;
        }
        graded gpu {source at host; feed at device.block; valuations at host; result at host;}
    };
    // Authored Sequence skips the `stage` pass: bind target Structures explicitly.
    program.structures = cq_gpu_sequence::bind_structures(&program.structures).unwrap();
    let program = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
    let analysis = check_grading(&program, &cq_gpu_sequence::machine()).unwrap();
    let expression = cq_gpu_sequence::translate(&program, &analysis).unwrap();
    let code = expression.to_token_stream().to_string();
    let segmented: syn::Type = syn::parse_quote!(::cq_gpu_sequence_runtime::group::SegmentedGroup);
    assert_eq!(analysis.structures.groups["grouped"].ty, segmented);
    assert_eq!(fold_providers(&expression), vec![Some(segmented)], "{code}");
    assert!(code.contains("canonicalize"));
    assert!(code.contains("valid [3usize]"));
}

#[test]
fn gpu_grouped_counts_sums_extrema_and_canonical_valuations() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    for raw in [
        vec![(1u32, 1u32, 5i64), (1, 1, 5), (1, 2, 5), (2, 1, -7)],
        vec![],
    ] {
        let data = DeviceColumns::from_rows(
            pool.clone(),
            &raw,
            &mut Dictionary::default(),
            &[Codec::For; 3],
        )?;
        let result = cq_gpu_sequence_test_macros::sequence! {
            physical {
                relation R(k:u32,id:u32,value:i64) = (&data);
                r = let canon rename R@0 {k -> k,id -> id,value -> value};
                grouped = let group r by [k] { n:usize=count(); total:i64=sum(value); low:Option<i64> = min(value); high:Option<i64> = max(value); };
                return grouped as answer(k,n,total,low,high);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; grouped_rows = ::cq_gpu_sequence_runtime::group::GroupedRows<_> { reads [exact, range, membership], build typed }; group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed }; }
                r host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,i64)> [];
                r rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
                r canon = rows;
                grouped rows = @grouped_rows as ::cq_gpu_sequence_runtime::group::GroupedRows<_>;
                grouped group = @group_map as ::std::collections::HashMap<_, (usize, i64, Option<i64>, Option<i64>)>;
            }
            sequence {
                source=store rows r;
                feed=canon flatten tile 256 source;
                valuations=store feed;
                result=store group grouped valuations;
                output count;
            }
            graded gpu {source at host; feed at device.block; valuations at host; result at host;}
        };
        // DECODE P: publish grouped nullable answers to the host correctness oracle.
        let actual: Vec<(u32, usize, i64, Option<i64>, Option<i64>)> = result.to_host()?;
        if raw.is_empty() {
            assert!(actual.is_empty());
        } else {
            assert_eq!(
                actual,
                vec![(1, 2, 10, Some(5), Some(5)), (2, 1, -7, Some(-7), Some(-7))]
            );
        }
    }
    Ok(())
}

#[test]
fn gpu_keyed_group_reads_resident_columns_and_filters_before_folding() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    for raw in [
        vec![(1u32, 1u32, 5i64), (1, 2, 5), (2, 1, -7)],
        vec![(2, 1, -7)],
        vec![],
    ] {
        let data = DeviceColumns::from_rows(
            pool.clone(),
            &raw,
            &mut Dictionary::default(),
            &[Codec::For; 3],
        )?;
        let result = cq_gpu_sequence_test_macros::sequence! {
            physical {
                relation R(k:u32,id:u32,value:i64) = (&data);
                r = pipe rename R@0 {k -> k,id -> id,value -> value};
                f = let canon select r if (value > 0);
                grouped = let group f by [k] {
                    n:usize=count(); total:i64=sum(value);
                    low:Option<i64> = min(value); high:Option<i64> = max(value);
                };
                return grouped as answer(k,n,total,low,high);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; grouped_rows = ::cq_gpu_sequence_runtime::group::GroupedRows<_> { reads [exact, range, membership], build typed }; group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed }; }
                r host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,i64)> [];
                f rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
                f canon = rows;
                grouped rows = @grouped_rows as ::cq_gpu_sequence_runtime::group::GroupedRows<_>;
                grouped group = @group_map as ::std::collections::HashMap<_, (usize, i64, Option<i64>, Option<i64>)>;
            }
            sequence {
                source=rows r;
                feed=compact f source;
                result=store group grouped feed;
                output count;
            }
            graded gpu {source at host; feed at host; result at host;}
            evidence { R(k,id) key; }
        };
        // DECODE P: publish filtered grouped answers to the host correctness oracle.
        let actual: Vec<(u32, usize, i64, Option<i64>, Option<i64>)> = result.to_host()?;
        assert_eq!(
            actual,
            if raw.iter().all(|row| row.2 <= 0) {
                vec![]
            } else {
                vec![(1, 2, 10, Some(5), Some(5))]
            }
        );
    }
    Ok(())
}

#[test]
fn whole_input_fold_combines_block_partials_including_empty_identity() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    for count in [0usize, 5, 100_003] {
        let raw: Vec<_> = (0..count).map(|n| (n as i64 - 7,)).collect();
        let expected = raw.iter().map(|row| row.0).sum::<i64>();
        let data = DeviceColumns::from_rows(
            pool.clone(),
            &raw,
            &mut Dictionary::default(),
            &[Codec::For],
        )?;
        let result = cq_gpu_sequence_test_macros::sequence! {
            physical {
                relation R(a:i64) = (&data);
                r = let canon rename R@0 {a -> a};
                return r as answer(a);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; }
                r host scan = @device_columns input { keys 0; columns [i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                r rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
                r canon = rows;
            }
            sequence {
                input = store rows r;
                result = store capacity 1 fold sum a input;
                output bounded;
            }
            graded gpu { input at host; result at device.block; }
        };
        // DECODE P: publish the final scalar fold answer to the host correctness oracle.
        let actual: Vec<(i64,)> = result.to_host()?;
        assert_eq!(actual, vec![(expected,)]);
    }
    Ok(())
}

#[test]
fn deferred_range_offsets_scan_large_domains_and_reject_overflow() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    for count in [0usize, 5, 100_003] {
        let mut lengths: Vec<u32> = (0..count).map(|i| (i % 17) as u32).collect();
        lengths.extend([u32::MAX, 123]);
        let offsets = cq_gpu_runtime::Buffer::from_host(pool.clone(), &lengths)?;
        let meta = cq_gpu_runtime::Buffer::<u32>::zeroed(pool.clone(), 4)?;
        let total = cq_gpu_sequence_runtime::offsets::scan(&pool, &offsets, count, &meta)?;
        let actual = offsets.to_host()?;
        let mut sum = 0;
        for i in 0..count {
            assert_eq!(actual[i], sum);
            sum += lengths[i];
        }
        assert_eq!(total, sum as usize);
        assert_eq!(actual[count], sum);
        assert_eq!(actual[count + 1], 123);
    }
    let offsets = cq_gpu_runtime::Buffer::from_host(pool.clone(), &[u32::MAX, 1, 0])?;
    let meta = cq_gpu_runtime::Buffer::<u32>::zeroed(pool.clone(), 4)?;
    assert!(cq_gpu_sequence_runtime::offsets::scan(&pool, &offsets, 2, &meta).is_err());
    Ok(())
}
