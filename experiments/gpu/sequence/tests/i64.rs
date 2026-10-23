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
            candidate = pipe probe smallest [b] l,r;
            checked = pipe canon probe first [a,c] candidate,s;
            return checked as answer(a,b,c);
        }
        structures {
            l host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            s host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r = ::cq_gpu_sequence_runtime::RangeProvider [b][c];
            s = ::cq_gpu_sequence_runtime::RangeProvider [a][c];
            r rows = ::cq_gpu_sequence_runtime::RangeProvider;
            s rows = ::cq_gpu_sequence_runtime::RangeProvider;
            checked canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
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
    let analysis = check_grading(&program, &cq_gpu_sequence::machine()).unwrap();
    assert_eq!(analysis.dispatches.len(), 1);
    let translated = cq_gpu_sequence::translate(&program, &analysis).unwrap();
    let generated = translated.to_token_stream().to_string();
    assert!(generated.contains("PayloadKind :: I64"), "{generated}");
    assert!(
        generated.contains("device :: decode ("),
        "i64 range payloads must use the full-width decoder: {generated}"
    );
    assert!(!generated.contains("PayloadKind :: I32"));
    assert!(!generated.contains("PayloadKind :: U32"));
    assert_eq!(analysis.results["result"], "checked");
}
