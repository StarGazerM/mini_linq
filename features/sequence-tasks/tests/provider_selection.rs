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
            joined = compact joined lookup r[a] l;
            output = store joined;
        }
    }
}

#[test]
fn host_only_provider_survives_an_unavailable_shareable_rewrite() {
    let source = projected_host_reader();
    let mut catalog = cq_sequence_tasks::catalog();
    catalog.providers.retain_mut(|token| {
        token.levels.retain(|level| level == &["host"]);
        !token.levels.is_empty()
    });
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

#[test]
fn positional_spine_preserves_raw_host_membership_at_host_when_retention_is_illegal() {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32); relation S(a:i32);
            r = pipe rename R@0 {a -> a,b -> b};
            s = let rename S@1 {a -> a};
            selected = let semi eager [a] r,s;
            expanded = pipe probe first [a] s,selected;
            member = pipe probe first [a,b] expanded,r;
            answer = pipe canon project member keep {a,b};
            return answer as answer(a,b);
        }
    };
    let structures = cq_structure_selection::compile_with(
        &physical,
        quote::quote!(positional(R, S)),
    ).unwrap();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    // Keep the authored reader publication separate from join-order advice.
    let source = cq_sequence_plan::rewrites::store(&source, quote::quote!(member)).unwrap();
    let before = cq_lang_sequence::contract::check(&source).unwrap();
    assert!(before.structures.reads.contains_key("selected"));
    assert!(before.structures.membership.contains(&("r".into(), vec!["a".into(), "b".into()])));
    let catalog = cq_sequence_tasks::catalog();
    let target = cq_sequence_tasks::select_providers(&source, &catalog).unwrap();
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert!(facts.structures.host_read("r"));
    assert!(facts.structures.reads.contains_key("selected"));
    cq_lang_sequence::check_rewrite(&source, &target).unwrap();
    let planned =
        cq_sequence_plan::decompose(&target, &cq_sequence_tasks::machine(), &catalog).unwrap();
    let facts =
        cq_lang_sequence::check_grading(&planned.program, &cq_sequence_tasks::machine()).unwrap();
    assert_eq!(facts.levels["member"], ["host"]);
    assert!(matches!(
        facts.publications.get("s"),
        Some(cq_lang_sequence::Publication::BorrowHost { result, .. }) if result == "s"
    ));
    let expression = cq_sequence_tasks::translate(&planned.program, &facts).unwrap();
    let rendered = quote::ToTokens::to_token_stream(&expression).to_string();
    let keyset = rendered.find("let __cq_s_keys_a =").expect("borrowed source keyset");
    let read = rendered.find("ExactMembership :: contains (__cq_s_keys_a")
        .expect("partition producer membership read");
    assert!(keyset < read, "publish keysets before dependent partition producers");
    assert_eq!(rendered.matches("let __cq_s_keys_a =").count(), 1);
    assert!(!rendered.contains("let __cq_s_rows"), "borrowed rows need no tuple buffer");
    // Each backend selects its own valid catalog; positional markers need
    // the caller's exact cq-load dependency in the generated-Rust runtime.
    let runtime = cq_testkit::rustc::Runtime::with_dependencies(vec![
        cq_testkit::rustc::linked_dependency::<cq_storage::sharded::FullKey>(
            "cq_storage", "cq_storage::sharded::FullKey"),
        cq_testkit::rustc::linked_dependency::<cq_load::ColumnAt<0>>(
            "cq_load", "cq_load::ColumnAt<0>"),
    ]);
    for setup in [
        quote::quote! { let R: Vec<(i32,i32)> = vec![]; let S: Vec<(i32,)> = vec![]; },
        quote::quote! { let R = vec![(1i32,2i32),(1,2),(2,3)]; let S = vec![(1i32,),(1,)]; },
    ] {
        cq_sequence_conformance::assert_conformant_with_runtime(
            &source, &[Target::Sequential, Target::Tasks], &setup, &runtime,
            &cq_lang_sequence::Catalog::default(),
        );
    }
}
