use cq_lang_sequence::check_grading;
include!("support/stack.rs");
use quote::ToTokens;

#[test]
fn triangle_retains_i64_payloads_and_its_recorded_plan() {
    let Stack { physical, structures, program } = syn::parse_quote! {
        physical {
            relation E(src:i64,dst:i64) = (edges);
            l = pipe rename E@0 {src -> a,dst -> b};
            r = let rename E@1 {src -> b,dst -> c};
            s = let rename E@2 {src -> a,dst -> c};
            candidate = pipe probe first [b] l,r;
            checked = let canon probe first [a,c] candidate,s;
            return checked as answer(a,b,c);
        }
        structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
            l host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            s host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [b];
            s = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a][c];
            r rows = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider;
            s rows = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider;
            checked rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
            checked canon = rows;
        }
        sequence {
            Edges = store rows l;
            ByB = store rows r;
            ByA = store rows s;
            result = store canon flatten map checked flatten lookup s [a,c]
                flatten lookup r [b] tile 1024 items 4 Edges;
            output count;
        }
        graded gpu { Edges at host; ByB at host; ByA at host; result at device; }
    };
    let program = cq_gpu_sequence_core::runs::record_representation(&physical, &structures, &program).unwrap();
    let analysis = check_grading(&physical, &structures, &program, &cq_gpu_sequence::machine()).unwrap();
    assert_eq!(analysis.dispatches.len(), 1);
    let translated = cq_gpu_sequence::translate(&physical, &structures, &program, &analysis).unwrap();
    let generated = translated.to_token_stream().to_string();
    assert!(generated.contains("PayloadKind :: I64"), "{generated}");
    assert!(
        generated.contains("device :: decode ("),
        "i64 range payloads must use the full-width decoder: {generated}"
    );
    assert!(
        generated.contains("< i64 as :: gpu_datalog_runtime :: Scalar > :: COLUMN"),
        "payload width must come from the recorded scalar provider: {generated}"
    );
    assert_eq!(analysis.results["result"], "checked");

    // An authored smallest probe is not permission to emit a first-probe
    // fallback: a non-radix side (here a Range provider) has no exact
    // candidate reads, so GPU-Smallest rejects the program.
    let mut smallest_physical = physical.clone();
    let mut smallest_structures = structures.clone();
    smallest_physical.program.definitions[3].operator =
        syn::parse_quote!(probe smallest [b] l,r);
    let provider = smallest_structures.structures
        .entries
        .iter_mut()
        .find(|entry| {
            entry.name == "r"
                && matches!(
                    entry.structure,
                    cq_lang_sequence::structures::Structure::Built { .. }
                )
        })
        .unwrap();
    *provider =
        syn::parse_quote!(r = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [b][c];);
    smallest_structures.structures
        .declare(&cq_gpu_sequence::providers::declarations())
        .unwrap();
    let analysis = check_grading(&smallest_physical, &smallest_structures, &program, &cq_gpu_sequence::machine()).unwrap();
    let error = cq_gpu_sequence::translate(&smallest_physical, &smallest_structures, &program, &analysis).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("T-Gpu: probe smallest is realized only over two-level radix layouts")
    );
}

#[test]
fn two_key_membership_preserves_full_width_last_key() -> cq_gpu_sequence_runtime::Result<()> {
    use cq_gpu_runtime::{Codec, DeviceColumns, Dictionary, Pool};
    let pool = Pool::new(0, 0, 0)?;
    let raw = vec![
        (0i64, 7i64),
        (0, (1i64 << 32) + 7),
        (0, -(1i64 << 32) + 7),
        (0, 8),
        (1, 7),
    ];
    let edges = DeviceColumns::from_rows(
        pool.clone(),
        &raw,
        &mut Dictionary::default(),
        &[Codec::For, Codec::For],
    )?;
    let stored = DeviceColumns::from_rows(
        pool.clone(),
        &[(0i64, 7i64)],
        &mut Dictionary::default(),
        &[Codec::For, Codec::For],
    )?;
    let output = cq_gpu_sequence_test_macros::sequence! {
        physical {
            relation R(a:i64,c:i64) = (&edges);
            relation S(a:i64,c:i64) = (&stored);
            l = pipe rename R@0 {a -> a,c -> c};
            s = let rename S@0 {a -> a,c -> c};
            checked = let canon semi eager [a,c] l,s;
            return checked as answer(a,c);
        }
        structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
            l host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            s host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            s = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a][c];
            s rows = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider;
            checked rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
            checked canon = rows;
        }
        sequence {
            Edges = store rows l;
            ByA = store rows s;
            result = store canon compact checked lookup s [a,c]
                flatten tile 1024 items 4 Edges;
            output count;
        }
        graded gpu { Edges at host; ByA at host; result at device; }
    };
    let mut actual: Vec<(i64, i64)> = output.to_host()?;
    actual.sort_unstable();
    assert_eq!(actual, vec![(0, 7)]);
    Ok(())
}
