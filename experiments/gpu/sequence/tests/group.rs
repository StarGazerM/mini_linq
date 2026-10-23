use cq_gpu_runtime::{Codec, DeviceColumns, Dictionary, Pool};
use cq_gpu_sequence_runtime::Result;
use cq_lang_sequence::{Program, check_grading};
use quote::ToTokens;

#[test]
fn translates_grouped_canonical_publication() {
    let program: Program = syn::parse_quote! {
        physical {
            relation R(k:u32,id:u32,value:i64) = (data);
            r = pipe canon rename R@0 {k -> k,id -> id,value -> value};
            grouped = let group r by [k] { n:usize=count(); total:i64=sum(value); low:Option<i64> = min(value); high:Option<i64> = max(value); };
            return grouped as answer(k,n,total,low,high);
        }
        structures {
            r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,i64)> [];
            r canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            grouped rows = ::cq_gpu_sequence_runtime::group::GroupedRows<_>;
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
    let analysis = check_grading(&program, &cq_gpu_sequence::machine()).unwrap();
    let code = cq_gpu_sequence::translate(&program, &analysis)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(code.contains("group :: fold"));
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
                r = pipe canon rename R@0 {k -> k,id -> id,value -> value};
                grouped = let group r by [k] { n:usize=count(); total:i64=sum(value); low:Option<i64> = min(value); high:Option<i64> = max(value); };
                return grouped as answer(k,n,total,low,high);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32,i64)> [];
                r canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                grouped rows = ::cq_gpu_sequence_runtime::group::GroupedRows<_>;
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
