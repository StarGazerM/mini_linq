use cq_gpu_sequence::machine;
use cq_lang_sequence::{Program, check_grading};

#[test]
fn fold_at_noncollaborative_device_is_rejected() {
    let program: Program = syn::parse_quote! {
        physical {
            relation R(a:i32) = (&data.r);
            r = let canon rename R {a -> a};
            return r as answer(a);
        }
        structures { providers { reader = Reader { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            r host scan = @reader as Reader [];
            r rows = @sort as ::cq_storage::dedup::Sort;
            r canon = rows;
        }
        sequence {
            input = store rows r;
            count = fold count input;
        }
        graded gpu { input at host; count at device; }
    };
    let error = check_grading(&program, &machine()).unwrap_err().to_string();
    assert!(error.contains("BE-Collective"), "{error}");
}

#[test]
fn rust_below_host_requires_identity_representation() {
    let mut program: Program = syn::parse_quote! {
        physical {
            relation R(a:i32) = (&data.r);
            r = pipe rename R {a -> a};
            bound = let canon bind r with n:i32 = (opaque(*a));
            return bound as answer(a,n);
        }
        structures { providers { reader = Reader { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            r host scan = @reader as Reader [];
            bound rows = @sort as ::cq_storage::dedup::Sort;
            bound canon = rows;
        }
        sequence { result = store map bound rows r; }
        graded gpu { result at device; }
    };
    let error = check_grading(&program, &machine()).unwrap_err().to_string();
    assert!(error.contains("BE-RustRepresentation"), "{error}");
    program.representation.encoded.insert("a".into());
    let error = check_grading(&program, &machine()).unwrap_err().to_string();
    assert!(error.contains("BE-RustRepresentation"), "{error}");
    program.representation.encoded.clear();
    program.representation.identity.insert("a".into());
    assert!(
        check_grading(&program, &machine()).is_ok(),
        "arbitrary total Rust is legal on identity device columns"
    );
    program.representation.identity.clear();
    program.graded = Some(syn::parse_quote!(graded gpu { result at host; }));
    assert!(
        check_grading(&program, &machine()).is_ok(),
        "opaque Rust remains legal at host"
    );
    let unknown = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
    assert!(!unknown.representation.identity.contains("a"));
    assert!(
        unknown.representation.identity.contains("n"),
        "the authored result annotation records identity without admitting unknown inputs"
    );
    assert!(unknown.representation.encoded.is_empty());
    program.structures.structures.entries[0] = syn::parse_quote!(
        r host scan = @radix_relation input { keys 1; columns [[0] i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,), (), u64, u64, ()> [];
    );
    program
        .structures
        .structures
        .declare(&cq_gpu_sequence::providers::declarations())
        .unwrap();
    let native = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
    assert!(native.representation.identity.contains("a"));
    assert!(!native.representation.encoded.contains("a"));
    program.structures.structures.entries[0] = syn::parse_quote!(
        r host scan = @radix_relation input { keys 1; columns [[0] i32 => ::gpu_datalog_runtime::Symbol<i32, TestCodec> codec [0] TestCodec] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<
            (::gpu_datalog_runtime::Symbol<i32, TestCodec>,), (), u64, u64, ()
        > [];
    );
    let encoded = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
    assert!(encoded.representation.encoded.contains("a"));
    assert!(!encoded.representation.identity.contains("a"));
}

#[test]
fn scalar_fold_host_code_parses() {
    use quote::{ToTokens, quote};
    for operation in [
        quote!(fold count input),
        quote!(fold sum a tile 1024 items 4 input),
    ] {
        let program: Program = syn::parse2(quote! {
            physical {
                relation R(a:i64) = (&data.r);
                r = let canon rename R {a -> a};
                return r as answer(a);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
                r host scan = @device_columns input { keys 0; columns [i64 => i64] } as ::cq_gpu_runtime::DeviceColumns [];
                r rows = @sort as ::cq_storage::dedup::Sort;
                r canon = rows;
            }
            sequence {
                input = store rows r;
                result = store capacity 1024 #operation;
                output bounded;
            }
            graded gpu { input at host; result at device.block; }
        })
        .unwrap();
        let analysis = check_grading(&program, &machine()).unwrap();
        let generated = cq_gpu_sequence::translate(&program, &analysis)
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(generated.contains("__cq_fold_accumulator_"));
        assert!(generated.contains("__fold_result"));
        assert!(!generated.contains("__row"));
    }
}

#[test]
fn host_choice_of_expanded_rows_preserves_upstream_range_translation() {
    let program: Program = syn::parse_quote! {
        physical {
            relation R(a:u32) = (&data.r);
            relation P(a:u32,b:u32) = (&data.p);
            relation Q(b:u32,c:u32) = (&data.q);
            r = pipe rename R {a -> a};
            p = let rename P {a -> a,b -> b};
            q = let rename Q {b -> b,c -> c};
            joined = pipe probe first [a] r,p;
            expanded = pipe probe first [b] joined,q;
            answer = let canon project expanded keep {a,b,c};
            return answer as output(a,b,c);
        }
        structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
            r host scan = @device_columns input { keys 0; columns [u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,)> [];
            p host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32)> [];
            q host scan = @device_columns input { keys 0; columns [u32 => u32, u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,u32)> [];
            p = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];
            q = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [b];
            answer rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
            answer canon = rows;
        }
        sequence {
            input = store rows r;
            first = store flatten lookup p [a] flatten tile 256 input;
            second_feed = flatten lookup q [b] flatten tile 256 first;
            second = store second_feed;
            chosen = store choose [len second < 100 => second, else => second];
            feed = canon map answer flatten tile 256 chosen;
            result = store feed;
            output count;
        }
        graded gpu {
            input at host;
            first at device.block;
            second_feed at device.block;
            second at host;
            chosen at host;
            feed at device.block;
            result at device;
        }
    };
    let program = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
    for column in ["a", "b", "c"] {
        assert!(program.representation.identity.contains(column));
    }
    let analysis = check_grading(&program, &machine()).unwrap();
    cq_gpu_sequence::translate(&program, &analysis).unwrap();
}
