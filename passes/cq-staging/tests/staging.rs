use cq_lang_structures as structures;
use quote::quote;

fn scan() -> (cq_lang_structures::physical::Program, structures::Program) {
    let physical = syn::parse_quote! {
        physical {
            relation R(x:i32);
            r = pipe rename R {x -> x};
            s = pipe select r if (x >= 0);
            b = let canon bind s with y: i32 = (*x + 1);
            return b as answer(x,y);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    (physical, structures)
}

#[test]
fn defaults_leave_plain_scans_scalar_and_batch_stops_before_bind() {
    let (physical, source) = scan();
    let default = cq_staging::compile(&physical, &source).unwrap();
    assert_eq!(default, source);
    cq_staging::check(&physical, &source, &default).unwrap();
    let target = cq_staging::compile_with(&physical, &source, quote!(batch(64))).unwrap();
    let facts = structures::contract::check(&physical, &target).unwrap();
    assert_eq!(
        facts.stages.into_iter().collect::<Vec<_>>(),
        vec![("s".into(), 64)]
    );
    cq_staging::check(&physical, &source, &target).unwrap();
}

#[test]
fn default_records_only_the_eligible_built_probe_outer() {
    let physical = syn::parse_quote! {
        physical {
            relation L(k:i32); relation R(k:i32,v:i32);
            l = pipe rename L {k -> k};
            r = let rename R {k -> k,v -> v};
            j = let canon probe first [k] l,r;
            return j as answer(k,v);
        }
    };
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = cq_staging::compile(&physical, &source).unwrap();
    assert_eq!(
        structures::contract::check(&physical, &target).unwrap().stages["l"],
        128
    );
    cq_staging::check(&physical, &source, &target).unwrap();
    assert!(cq_staging::check(&physical, &source, &source).is_err());
}

#[test]
fn invalid_arguments_and_nonmaximal_recording_are_rejected() {
    let (physical, source) = scan();
    for arguments in [
        quote!(batch(0)),
        quote!(batch(-1)),
        quote!(batch(2, 3)),
        quote!(other),
    ] {
        assert!(cq_staging::compile_with(&physical, &source, arguments).is_err());
    }
    let mut wrong = source.clone();
    wrong
        .structures
        .entries
        .push(syn::parse_quote!(r stage = batch 64;));
    assert!(cq_staging::check(&physical, &source, &wrong).is_err());
}

#[test]
fn existing_boundaries_and_other_decisions_are_preserved() {
    let (physical, mut source) = scan();
    source
        .structures
        .entries
        .push(syn::parse_quote!(r stage = batch 8;));
    let target = cq_staging::compile_with(&physical, &source, quote!(batch(64))).unwrap();
    cq_staging::check(&physical, &source, &target).unwrap();
    assert_eq!(structures::contract::check(&physical, &target).unwrap().stages["r"], 8);
}

#[test]
fn retained_rows_start_a_new_total_segment() {
    let physical = syn::parse_quote! {
        physical {
            relation R(x:i32);
            r = let rename R {x -> x};
            s = pipe select r if (x >= 0);
            b = let canon bind s with y:i32 = (*x + 1);
            return b as answer(x,y);
        }
    };
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = cq_staging::compile_with(&physical, &source, quote!(batch(7))).unwrap();
    let facts = structures::contract::check(&physical, &target).unwrap();
    assert_eq!(
        facts.stages.into_iter().collect::<Vec<_>>(),
        vec![("s".into(), 7)]
    );
    cq_staging::check(&physical, &source, &target).unwrap();
}
