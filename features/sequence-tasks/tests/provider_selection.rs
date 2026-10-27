use cq_lang_sequence::Program;
use cq_sequence_conformance::{Target, assert_conformant};

fn projected_host_reader() -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            consumer canonical;
            l = pipe rename L@0 {a -> a};
            raw = pipe rename R@1 {a -> a,b -> b};
            r = let project raw keep {a};
            joined = pipe probe first [a] l,r;
            return joined as answer(a);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    syn::parse_quote! {
        #structures
        sequence {
            l = rows l;
            raw = rows raw;
            r = store map r raw;
            joined = flatten lookup r[a] l;
            output = store joined;
        }
    }
}

#[test]
fn host_only_provider_survives_an_unavailable_shareable_rewrite() {
    let source = projected_host_reader();
    let mut catalog = cq_sequence_tasks::catalog();
    catalog
        .providers
        .retain(|token| token.levels == vec![vec!["host".to_owned()]]);
    let target = cq_sequence_tasks::select_providers(&source, &catalog).unwrap();
    cq_sequence_plan::plan(&target, &cq_sequence_tasks::machine(), &catalog).unwrap();
    assert_eq!(source.structures.physical, target.structures.physical);
    let original: Vec<_> = source
        .structures
        .structures
        .entries
        .iter()
        .filter(|entry| entry.name == "r")
        .collect();
    let selected: Vec<_> = target
        .structures
        .structures
        .entries
        .iter()
        .filter(|entry| entry.name == "r")
        .collect();
    assert_eq!(original, selected);
    assert_conformant(&target, &[Target::Sequential, Target::Tasks]);
}

#[test]
fn inapplicable_retention_still_rejects_an_unadmitted_native_reader() {
    let source = projected_host_reader();
    let mut catalog = cq_sequence_tasks::catalog();
    catalog.providers.clear();
    assert!(cq_sequence_tasks::select_providers(&source, &catalog).is_err());
}
