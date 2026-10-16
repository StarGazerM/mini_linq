use cq_lang_structures as structures;
use quote::quote;

fn scan() -> structures::Program {
    cq_structure_selection::compile(&syn::parse_quote! {
        physical {
            relation R(x:i32);
            r = pipe rename R@0 {x -> x};
            s = pipe select r if (x >= 0);
            b = pipe canon bind s with y: i32 = (*x + 1);
            return b as answer(x,y);
        }
    })
    .unwrap()
}

#[test]
fn defaults_leave_plain_scans_scalar_and_batch_stops_before_bind() {
    let source = scan();
    let default = cq_staging::compile(&source).unwrap();
    assert_eq!(default, source);
    cq_staging::check(&source, &default).unwrap();
    let target = cq_staging::compile_with(&source, quote!(batch(64))).unwrap();
    let facts = structures::contract::check(&target).unwrap();
    assert_eq!(
        facts.stages.into_iter().collect::<Vec<_>>(),
        vec![("s".into(), 64)]
    );
    cq_staging::check(&source, &target).unwrap();
}

#[test]
fn default_records_only_the_eligible_built_probe_outer() {
    let source = cq_structure_selection::compile(&syn::parse_quote! {
        physical {
            relation L(k:i32); relation R(k:i32,v:i32);
            l = pipe rename L@0 {k -> k};
            r = let rename R@1 {k -> k,v -> v};
            j = pipe canon probe first [k] l,r;
            return j as answer(k,v);
        }
    })
    .unwrap();
    let target = cq_staging::compile(&source).unwrap();
    assert_eq!(
        structures::contract::check(&target).unwrap().stages["l"],
        128
    );
    cq_staging::check(&source, &target).unwrap();
    assert!(cq_staging::check(&source, &source).is_err());
}

#[test]
fn invalid_arguments_and_nonmaximal_recording_are_rejected() {
    let source = scan();
    for arguments in [
        quote!(batch(0)),
        quote!(batch(-1)),
        quote!(batch(2, 3)),
        quote!(other),
    ] {
        assert!(cq_staging::compile_with(&source, arguments).is_err());
    }
    let mut wrong = source.clone();
    wrong
        .structures
        .entries
        .push(syn::parse_quote!(r stage = batch 64;));
    assert!(cq_staging::check(&source, &wrong).is_err());
}

#[test]
fn existing_boundaries_and_other_decisions_are_preserved() {
    let mut source = scan();
    source
        .structures
        .entries
        .push(syn::parse_quote!(r stage = batch 8;));
    let target = cq_staging::compile_with(&source, quote!(batch(64))).unwrap();
    cq_staging::check(&source, &target).unwrap();
    assert_eq!(structures::contract::check(&target).unwrap().stages["r"], 8);
}

#[test]
fn retained_rows_start_a_new_total_segment() {
    let physical = syn::parse_quote! {
        physical {
            relation R(x:i32);
            r = let rename R@0 {x -> x};
            s = pipe select r if (x >= 0);
            b = pipe canon bind s with y:i32 = (*x + 1);
            return b as answer(x,y);
        }
    };
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = cq_staging::compile_with(&source, quote!(batch(7))).unwrap();
    let facts = structures::contract::check(&target).unwrap();
    assert_eq!(
        facts.stages.into_iter().collect::<Vec<_>>(),
        vec![("s".into(), 7)]
    );
    cq_staging::check(&source, &target).unwrap();
}
