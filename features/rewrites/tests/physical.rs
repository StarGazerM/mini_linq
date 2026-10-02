use cq_lang_physical::{self as ir, Choice, Mode, Operator};
use cq_rewrite_passes::{canon_prefixes, lazy_semijoins, merge_joins, smallest_intersections};
use cq_testkit::{PassCase, SCENARIOS, Scenario, semantics};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

type Rewrite = fn(&ir::Program, TokenStream) -> syn::Result<ir::Program>;

fn scenario(name: &str) -> &'static Scenario {
    SCENARIOS.iter().find(|s| s.name == name).unwrap()
}

fn altered_head(p: &ir::Program) -> ir::Program {
    let mut p = p.clone();
    p.program.head.args = p.program.head.args.into_iter().rev().collect();
    p
}

fn unknown_key(p: &ir::Program) -> ir::Program {
    let mut p = p.clone();
    let key = p
        .program
        .definitions
        .iter_mut()
        .find_map(|d| match &mut d.operator {
            Operator::Probe(_, k) | Operator::Merge(_, k) | Operator::Semi(_, k) => Some(k),
            _ => None,
        })
        .unwrap();
    let unbound: syn::Ident = syn::parse_quote!(unbound);
    key.columns = [unbound].into_iter().collect();
    p
}

fn conform(source: ir::Program, expected: &str, rewrite: Rewrite, case_name: &str) {
    let case = scenario(case_name).case().unwrap();
    PassCase {
        source,
        expected_next: Some(syn::parse_str::<ir::Program>(expected).unwrap()),
        compile: Box::new(move |p| rewrite(p, TokenStream::new())),
        check: Box::new(ir::check_rewrite),
        mutations: vec![Box::new(altered_head), Box::new(unknown_key)],
        answers: Some(Box::new(|p, db| {
            semantics::evaluate_ra(&cq_physical_lowering::erase(p), db).output
        })),
    }
    .run(Some(case))
    .unwrap();
}

#[test]
fn merge_output_is_pinned_and_named_selection_is_exact() {
    let source = scenario("intersection").parse_physical().unwrap();
    let expected = include_str!("fixtures/merge.physical");
    conform(source.clone(), expected, merge_joins, "intersection");
    for args in [quote!(all), quote!(r2), quote!(r2,), quote!(r#r2)] {
        assert_eq!(
            merge_joins(&source, args).unwrap(),
            syn::parse_str::<ir::Program>(expected).unwrap()
        );
    }
    for args in [
        quote!(missing),
        quote!(r0),
        quote!(r2, r2),
        quote!(all, r2),
        quote!(r2 => X),
    ] {
        assert!(merge_joins(&source, args).is_err());
    }
    let source = scenario("fine_triangle").parse_physical().unwrap();
    assert_eq!(merge_joins(&source, quote!(all)).unwrap(), source);
    assert!(merge_joins(&source, quote!(r2)).is_err());
}

#[test]
fn merge_preserves_retention_and_allows_same_key_stream_chains() {
    let source: ir::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32); relation S(a:i32);
        l = let rename L {a -> x}; r = let rename R {a -> x};
        s = let rename S {a -> x};
        j = pipe probe first [x] l,r;
        k = pipe canon probe first [x] j,s; return k as answer(x);
    });
    let expected: ir::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32); relation S(a:i32);
        l = let rename L {a -> x}; r = let rename R {a -> x};
        s = let rename S {a -> x};
        j = pipe merge [x] l,r;
        k = pipe canon merge [x] j,s; return k as answer(x);
    });
    assert_eq!(merge_joins(&source, quote!(all)).unwrap(), expected);
    let mut only_j = source.clone();
    only_j.program.definitions[3] = expected.program.definitions[3].clone();
    assert_eq!(merge_joins(&source, quote!(j)).unwrap(), only_j);
    assert!(merge_joins(&source, quote!(k)).is_err());
    let selected = cq_structure_selection::compile(&expected).unwrap();
    cq_testkit::structures::contract::check(&selected).unwrap();
}

#[test]
fn canon_output_is_pinned_and_existing_boundaries_are_preserved() {
    let source = scenario("fine_triangle").parse_physical().unwrap();
    let expected = include_str!("fixtures/canon.physical");
    conform(source.clone(), expected, canon_prefixes, "fine_triangle");
    assert_eq!(
        canon_prefixes(&source, quote!(r1)).unwrap(),
        syn::parse_str::<ir::Program>(expected).unwrap()
    );
    assert!(canon_prefixes(&source, quote!(r7)).is_err());
    let target = canon_prefixes(&source, quote!()).unwrap();
    assert_eq!(canon_prefixes(&target, quote!()).unwrap(), target);
    assert_eq!(target.program.definitions[7], source.program.definitions[7]);
}

#[test]
fn canon_avoids_projection_that_preserves_an_inferred_key() {
    let source: ir::Program = syn::parse_quote!(physical {
        relation R(a:i32); r = pipe canon rename R {a -> x};
        b = pipe bind r with y:i32 = (*x + 1);
        p = pipe project b keep {x}; return p as answer(x);
    });
    assert_eq!(canon_prefixes(&source, quote!()).unwrap(), source);
    assert_eq!(canon_prefixes(&source, quote!(p)).unwrap(), source);
    cq_structure_selection::compile(&source).unwrap();
}

#[test]
fn canon_does_not_require_equality_on_unobserved_float_columns() {
    let source: ir::Program = syn::parse_quote!(physical {
        relation R(a:i32,b:f64,c:i32);
        r = pipe rename R {a -> x,b -> y,c -> z};
        p = pipe project r keep {x,y};
        q = pipe canon project p keep {x}; return q as answer(x);
    });
    assert_eq!(canon_prefixes(&source, quote!(all)).unwrap(), source);
    assert!(canon_prefixes(&source, quote!(p)).is_err());
}

#[test]
fn smallest_output_is_pinned_and_nonadjacent_corpus_is_unchanged() {
    let source: ir::Program = syn::parse_str(include_str!("fixtures/adjacent.physical")).unwrap();
    conform(
        source.clone(),
        include_str!("fixtures/smallest.physical"),
        smallest_intersections,
        "fine_triangle",
    );
    let expected = smallest_intersections(&source, quote!(r4)).unwrap();
    assert_eq!(
        expected,
        smallest_intersections(&source, quote!(all)).unwrap()
    );
    let source = scenario("fine_triangle").parse_physical().unwrap();
    assert_eq!(smallest_intersections(&source, quote!()).unwrap(), source);
    assert!(smallest_intersections(&source, quote!(r4)).is_err());
}

#[test]
fn smallest_rejects_mutations_of_its_local_premises() {
    let source: ir::Program = syn::parse_str(include_str!("fixtures/adjacent.physical")).unwrap();
    let target = smallest_intersections(&source, quote!()).unwrap();
    for mutate in [0, 1, 2] {
        let mut invalid = target.clone();
        match mutate {
            0 => invalid.program.definitions[5].canon = Some(Default::default()),
            1 => invalid.program.definitions[5].mode = Mode::Let(Default::default()),
            _ => {
                invalid.program.definitions.swap(2, 6);
            }
        }
        assert!(ir::check_rewrite(&source, &invalid).is_err());
    }
    let structures = cq_structure_selection::compile(&target).unwrap();
    cq_testkit::structures::contract::check(&structures).unwrap();
}

#[test]
fn lazy_output_is_pinned_and_retained_membership_stays_eager() {
    let source = scenario("ya_composite_reduced").parse_physical().unwrap();
    conform(
        source.clone(),
        include_str!("fixtures/lazy.physical"),
        lazy_semijoins,
        "ya_composite_reduced",
    );
    assert!(lazy_semijoins(&source, quote!(r4)).is_err());
    let target = lazy_semijoins(&source, quote!(r2)).unwrap();
    assert_eq!(target, lazy_semijoins(&source, quote!(all)).unwrap());
    cq_structure_selection::compile(&target).unwrap();
}

#[test]
fn lazy_shared_keys_change_atomically_and_payload_conflicts_are_rejected() {
    let source: ir::Program = syn::parse_quote!(physical {
        relation R(a:i32); relation S(a:i32);
        r = pipe rename R {a -> x}; s = pipe rename S {a -> x};
        a = pipe semi eager [x] r,s; b = pipe canon semi eager [x] a,s;
        return b as answer(x);
    });
    assert!(lazy_semijoins(&source, quote!(a)).is_err());
    let target = lazy_semijoins(&source, quote!(a, b)).unwrap();
    assert_eq!(target, lazy_semijoins(&source, quote!()).unwrap());
    let mut invalid = target.clone();
    if let Operator::Semi(_, k) = &mut invalid.program.definitions[3].operator {
        k.choice = Some(Choice::Eager(Default::default()));
    }
    assert!(ir::check_rewrite(&source, &invalid).is_err());
    let mut conflicting = source.clone();
    conflicting.program.definitions[3] = syn::parse_quote!(b = pipe canon probe first [x] a,s;);
    assert_eq!(lazy_semijoins(&conflicting, quote!()).unwrap(), conflicting);
    assert!(lazy_semijoins(&conflicting, quote!(a)).is_err());
}

#[test]
fn lazy_shared_keys_are_sets_even_when_written_in_different_orders() {
    let source: ir::Program = syn::parse_quote!(physical {
        relation R(a:i32,b:i32); relation S(a:i32,b:i32);
        r = pipe rename R {a -> x,b -> y}; s = pipe rename S {a -> x,b -> y};
        a = pipe semi eager [x,y] r,s; b = pipe canon semi eager [y,x] a,s;
        return b as answer(x,y);
    });
    assert!(lazy_semijoins(&source, quote!(a)).is_err());
    let target = lazy_semijoins(&source, quote!(a, b)).unwrap();
    cq_structure_selection::compile(&target).unwrap();
    let mut payload = source;
    payload.program.definitions[3] = syn::parse_quote!(b = pipe canon probe first [y,x] a,s;);
    assert_eq!(lazy_semijoins(&payload, quote!(all)).unwrap(), payload);
}

#[test]
fn all_corpus_programs_preserve_erasure_contract_and_set_answers() {
    for scenario in SCENARIOS {
        let source = scenario.parse_physical().unwrap();
        for rewrite in [
            merge_joins,
            canon_prefixes,
            smallest_intersections,
            lazy_semijoins,
        ] {
            let target =
                rewrite(&source, quote!()).unwrap_or_else(|e| panic!("{}: {e}", scenario.name));
            ir::check_rewrite(&source, &target).unwrap();
            cq_structure_selection::compile(&target).unwrap();
            assert_eq!(
                cq_physical_lowering::erase(&source),
                cq_physical_lowering::erase(&target)
            );
            let case = scenario.case().unwrap();
            assert_eq!(
                semantics::evaluate_ra(&cq_physical_lowering::erase(&target), &case.database())
                    .output,
                case.expected_rows(),
                "{}",
                scenario.name
            );
            assert_eq!(rewrite(&target, quote!()).unwrap(), target);
            assert_eq!(
                syn::parse2::<ir::Program>(target.to_token_stream()).unwrap(),
                target
            );
        }
    }
}
