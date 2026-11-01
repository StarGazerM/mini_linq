use cq_lang_sequence::{Program, check_grading};
use quote::ToTokens;

#[test]
fn triangle_retains_i64_payloads_and_its_recorded_plan() {
    let program: Program = syn::parse_quote! {
        physical {
            relation E(src:i64,dst:i64) = (edges);
            l = pipe rename E@0 {src -> a,dst -> b};
            r = let rename E@1 {src -> b,dst -> c};
            s = let rename E@2 {src -> a,dst -> c};
            candidate = pipe probe first [b] l,r;
            checked = let canon probe first [a,c] candidate,s;
            return checked as answer(a,b,c);
        }
        structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build macro, abi columns, bounds resident_columns }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; }
            l host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            s host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [b];
            s = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a][c];
            r rows = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider;
            s rows = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider;
            checked rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
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
    let program = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
    let analysis = check_grading(&program, &cq_gpu_sequence::machine()).unwrap();
    assert_eq!(analysis.dispatches.len(), 1);
    let translated = cq_gpu_sequence::translate(&program, &analysis).unwrap();
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
    // fallback. Keep the unsupported target policy visible beside width coverage.
    let mut smallest = program.clone();
    smallest.structures.physical.program.definitions[3].operator =
        syn::parse_quote!(probe smallest [b] l,r);
    let provider = smallest
        .structures
        .structures
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
    *provider = syn::parse_quote!(r = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [b][c];);
    smallest
        .structures
        .structures
        .declare(&cq_gpu_sequence::providers::declarations());
    let analysis = check_grading(&smallest, &cq_gpu_sequence::machine()).unwrap();
    let error = cq_gpu_sequence::translate(&smallest, &analysis).unwrap_err();
    assert!(error.to_string().contains("probe smallest is not realized"));
}
