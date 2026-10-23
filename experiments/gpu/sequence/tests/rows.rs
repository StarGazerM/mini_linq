use cq_lang_sequence::{Program, check_grading};
use quote::{ToTokens, quote};

fn filtered_probe(condition: proc_macro2::TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation Driver(a:i32, value:i32, bound:i32) = (&data.driver);
            relation Provider(a:i32, payload:i32) = (&data.provider);
            driver = pipe rename Driver@0 {a -> a, value -> value, bound -> bound};
            provider = let rename Provider@1 {a -> a, payload -> payload};
            filtered = pipe select driver if (#condition);
            present = pipe semi eager [a] filtered, provider;
            projected = pipe project present keep {a, value};
            joined = pipe probe first [a] projected, provider;
            result = pipe canon project joined keep {a, value, payload};
            return result as answer(a, value, payload);
        }
        structures {
            driver host scan = ::cq_gpu_runtime::DeviceColumns [];
            provider host scan = ::cq_gpu_runtime::DeviceColumns [];
            provider = ::cq_gpu_sequence_runtime::RangeProvider [a];
            result canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
        }
        sequence {
            input = store rows driver;
            index = store rows provider;
            filtered_rows = compact filtered flatten tile 256 input;
            present_rows = compact present lookup provider [a] filtered_rows;
            projected_rows = map projected present_rows;
            joined_rows = flatten lookup provider [a] projected_rows;
            output_rows = store canon map result joined_rows;
            output count;
        }
        graded gpu {
            input at host;
            index at host;
            filtered_rows at host.device.block;
            present_rows at host.device.block;
            projected_rows at host.device.block;
            joined_rows at host.device.block;
            output_rows at host.device;
        }
    })
    .unwrap()
}

fn translate(program: &Program) -> String {
    let analysis = check_grading(program, &cq_gpu_sequence::machine()).unwrap();
    cq_gpu_sequence::translate(program, &analysis)
        .unwrap()
        .to_token_stream()
        .to_string()
}

#[test]
fn compact_and_project_preserve_probe_row_provenance() {
    // The same provider is consumed first for membership and then for payload.
    // Its aggregate Exact capability must not turn Compact into a payload read.
    let generated = translate(&filtered_probe(quote!(value > 0)));
    assert!(generated.contains("__begin < __end"));
    assert!(generated.contains("__child_position"));
    assert!(!generated.contains("__row"));
}

#[test]
fn row_valued_bound_is_evaluated_per_lane_before_range_lookup() {
    let generated = translate(&filtered_probe(quote!(value > bound)));
    assert!(generated.contains("let value = &"));
    assert!(generated.contains("let bound = &"));
    assert!(generated.contains("* value > * bound"));
    assert!(generated.contains("device :: range"));
    assert!(!generated.contains("RangeSelection"));
    assert!(!generated.contains("__row"));
}

#[test]
fn published_row_bound_filter_is_not_rebuilt_as_a_host_interval() {
    let mut program = filtered_probe(quote!(value > bound));
    for definition in &mut program.sequence.definitions {
        if definition.name == "filtered_rows" {
            let inner = &definition.term;
            definition.term = syn::parse2(quote!(store #inner)).unwrap();
        }
        if definition.name == "present_rows" {
            definition.term = syn::parse_quote!(
                compact present lookup provider [a] flatten tile 256 filtered_rows
            );
        }
    }
    for grade in &mut program.graded.as_mut().unwrap().grades {
        if grade.name == "filtered_rows" {
            grade.level.pop();
        }
    }
    let generated = translate(&program);
    assert!(generated.contains("* value > * bound"));
    assert!(!generated.contains("RangeSelection"));
    assert!(!generated.contains("__row"));
}
