use cq_lang_sequence::{
    Evidence, Guard, Len, OutputProtocol, Program, Term, check_rewrite, contract,
};
use cq_sequence_plan::rewrites::{
    AdaptiveCandidate, adaptive, adaptive_candidates, adaptive_choose, adaptive_filter,
    apply_adaptive, select_filter,
};
use quote::{ToTokens, quote};

fn program() -> Program {
    syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,tag:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,tag -> tag};
            selected = let point select r if (tag = 7);
            reduced = pipe semi eager [a] l,selected;
            filtered_join = pipe canon probe first [a] reduced,selected;
            raw_join = pipe canon probe first [a] l,selected;
            alternatives = pipe canon union filtered_join,raw_join;
            return alternatives as answer(a,tag);
        }
        structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r host result(selected) = Reader [tag];
            selected = Provider [a];
            filtered_join canon = Canon;
            raw_join canon = Canon;
            alternatives canon = Canon;
        }
        sequence {
            published = store map selected rows r;
            out = store capacity 1024 flatten lookup selected [a] rows l;
            output bounded;
        }
    }
}

fn alternatives() -> (Term, Term) {
    (
        syn::parse_quote!(flatten lookup selected [a] compact reduced lookup selected [a] rows l),
        syn::parse_quote!(flatten lookup selected [a] rows l),
    )
}

fn dynamic(source: &Program) -> syn::Result<Program> {
    let (filtered, raw) = alternatives();
    adaptive_choose(
        source,
        &syn::parse_quote!(out),
        &syn::parse_quote!(published),
        36,
        filtered,
        raw,
    )
}

#[test]
fn published_count_selects_real_optional_probing_and_preserves_publication() {
    let source = program();
    let target = dynamic(&source).unwrap();
    check_rewrite(&source, &target).unwrap();
    contract::check(&target).unwrap();
    assert_eq!(target.structures, source.structures);
    assert_eq!(target.sequence.output, Some(OutputProtocol::Bounded));
    assert!(matches!(
        target.sequence.definitions.last().unwrap().term,
        Term::Store {
            capacity: Some(1024),
            ..
        }
    ));
    let arms = target
        .sequence
        .definitions
        .iter()
        .find_map(|definition| match &definition.term {
            Term::Choose(arms) => Some(arms),
            _ => None,
        })
        .unwrap();
    assert_eq!(arms[0].0, Guard::Lt(Len::Input("published".into()), 36));
    assert_eq!(arms[1].0, Guard::Else);
    let (filtered, raw) = alternatives();
    assert_eq!(&arms[0].1, &filtered);
    assert_eq!(&arms[1].1, &raw);
    assert_ne!(filtered, raw);
    assert_eq!(
        syn::parse2::<Program>(target.to_token_stream()).unwrap(),
        target
    );
}

#[test]
fn selective_and_nonselective_evidence_select_checked_static_arms() {
    let source = program();
    for (ratio, expected) in [
        (quote!(1 / 10), true),
        (quote!(7 / 20), true),
        (quote!(99 / 100), false),
    ] {
        let evidence: Evidence =
            syn::parse2(quote!(evidence { selectivity selected = #ratio; })).unwrap();
        let (filtered, raw) = alternatives();
        let expected_term = if expected {
            filtered.clone()
        } else {
            raw.clone()
        };
        let target = adaptive_filter(
            &source,
            &syn::parse_quote!(out),
            &syn::parse_quote!(published),
            36,
            filtered,
            raw,
            &evidence,
            "selected",
        )
        .unwrap();
        assert_eq!(select_filter(&evidence, "selected"), Some(expected));
        check_rewrite(&source, &target).unwrap();
        let Term::Store { inner, .. } = &target.sequence.definitions.last().unwrap().term else {
            panic!("output publication must remain authored")
        };
        assert_eq!(inner.as_ref(), &expected_term);
        assert!(
            !target
                .sequence
                .definitions
                .iter()
                .any(|definition| matches!(definition.term, Term::Choose(_)))
        );
    }
}

#[test]
fn absent_or_unrelated_selectivity_generates_dynamic_choice() {
    let source = program();
    for evidence in [
        Evidence::default(),
        syn::parse_quote!(evidence { count selected = 100; }),
    ] {
        assert_eq!(select_filter(&evidence, "selected"), None);
        let (filtered, raw) = alternatives();
        let target = adaptive_filter(
            &source,
            &syn::parse_quote!(out),
            &syn::parse_quote!(published),
            36,
            filtered,
            raw,
            &evidence,
            "selected",
        )
        .unwrap();
        assert_eq!(target, dynamic(&source).unwrap());
    }
}

#[test]
fn evidence_cannot_justify_an_incorrect_discarded_arm() {
    let source = program();
    let evidence: Evidence = syn::parse_quote!(evidence { selectivity selected = 0 / 1; });
    let (filtered, _) = alternatives();
    assert!(
        adaptive_filter(
            &source,
            &syn::parse_quote!(out),
            &syn::parse_quote!(published),
            36,
            filtered,
            syn::parse_quote!(rows l),
            &evidence,
            "selected"
        )
        .is_err()
    );
}

#[test]
fn identical_alias_arms_and_unpublished_counts_are_rejected() {
    let source = program();
    let (_, raw) = alternatives();
    assert!(
        adaptive_choose(
            &source,
            &syn::parse_quote!(out),
            &syn::parse_quote!(published),
            36,
            raw.clone(),
            raw.clone()
        )
        .is_err()
    );
    let mut alias = source.clone();
    alias.sequence.definitions.insert(
        1,
        syn::parse_quote!(same = flatten lookup selected [a] rows l;),
    );
    assert!(
        adaptive_choose(
            &alias,
            &syn::parse_quote!(out),
            &syn::parse_quote!(published),
            36,
            raw,
            syn::parse_quote!(same)
        )
        .is_err()
    );
    let mut unmaterialized = source.clone();
    unmaterialized.sequence.definitions[0].term = syn::parse_quote!(compact selected rows r);
    assert!(dynamic(&unmaterialized).is_err());
    let (filtered, raw) = alternatives();
    assert!(
        adaptive_choose(
            &source,
            &syn::parse_quote!(out),
            &syn::parse_quote!(out),
            36,
            filtered.clone(),
            raw.clone()
        )
        .is_err()
    );
    assert!(
        adaptive_choose(
            &source,
            &syn::parse_quote!(out),
            &syn::parse_quote!(published),
            0,
            filtered,
            raw
        )
        .is_err()
    );
}

#[test]
fn names_are_deterministic_and_avoid_authored_collisions() {
    let mut source = program();
    source
        .sequence
        .definitions
        .insert(1, syn::parse_quote!(__cq_out_adaptive_0 = rows l;));
    let first = dynamic(&source).unwrap();
    let second = dynamic(&source).unwrap();
    assert_eq!(first, second);
    assert!(
        first
            .sequence
            .definitions
            .iter()
            .any(|definition| definition.name == "__cq_out_adaptive_1")
    );
}

#[test]
fn threshold_comparison_does_not_overflow_on_large_advice() {
    let evidence: Evidence = syn::parse_quote! {
        evidence { selectivity selected = 18446744073709551615 / 18446744073709551615; }
    };
    assert_eq!(select_filter(&evidence, "selected"), Some(false));
}

#[test]
fn opaque_predicate_publications_are_eligible_without_moving_the_predicate() {
    let source: Program = syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe rename R@0 {a -> a};
            guarded = pipe canon guard r if (*a > 0);
            return guarded as answer(a);
        }
        structures { r host scan = Reader []; guarded canon = Canon; }
        sequence {
            published = store map guarded rows r;
            out = store published;
        }
    };
    let target = adaptive_choose(
        &source,
        &syn::parse_quote!(out),
        &syn::parse_quote!(published),
        36,
        syn::parse_quote!(flatten tile 8 published),
        syn::parse_quote!(published),
    )
    .unwrap();
    assert_eq!(
        target.sequence.definitions[0],
        source.sequence.definitions[0]
    );
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn plain_unfiltered_store_is_not_a_selectivity_measurement() {
    let mut source = program();
    source
        .sequence
        .definitions
        .insert(1, syn::parse_quote!(unfiltered = store rows l;));
    let (filtered, raw) = alternatives();
    assert!(
        adaptive_choose(
            &source,
            &syn::parse_quote!(out),
            &syn::parse_quote!(unfiltered),
            36,
            filtered,
            raw
        )
        .is_err()
    );
}

#[test]
fn arm_materialization_stays_beneath_its_runtime_guard() {
    let source = program();
    let (filtered, raw) = alternatives();
    let filtered = Term::Store {
        capacity: None,
        inner: Box::new(filtered),
    };
    let target = adaptive_choose(
        &source,
        &syn::parse_quote!(out),
        &syn::parse_quote!(published),
        36,
        filtered.clone(),
        raw.clone(),
    )
    .unwrap();
    assert_eq!(
        target.sequence.definitions.len(),
        source.sequence.definitions.len() + 1
    );
    let Term::Choose(arms) = &target.sequence.definitions[1].term else {
        panic!("materialization must remain guarded, not lifted into an eager arm definition")
    };
    assert_eq!(arms[0].1, filtered);
    assert_eq!(arms[1].1, raw);
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn direct_choice_checks_each_arm_against_source_support() {
    let source = program();
    let (filtered, _) = alternatives();
    let unrelated: Term = syn::parse_quote!(rows l);
    for (yes, no) in [(filtered.clone(), unrelated.clone()), (unrelated, filtered)] {
        assert!(
            adaptive_choose(
                &source,
                &syn::parse_quote!(out),
                &syn::parse_quote!(published),
                36,
                yes,
                no,
            )
            .is_err()
        );
    }
}

fn automatic_source() -> Program {
    let structures = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,tag:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,tag -> tag};
            selected = let point select r if (tag = 7);
            __lookup_lookup = pipe semi eager [a] l,selected;
            joined = pipe probe first [a] __lookup_lookup,selected;
            result = pipe canon project joined keep {a};
            return result as answer(a);
        }
        structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r host result(selected) = Reader [tag];
            selected = Provider [a];
            result canon = Canon;
        }
    };
    let mut program = cq_sequence_lowering::compile(&structures).unwrap();
    program.sequence.optional_filters = vec![syn::parse_quote!(__lookup_lookup)];
    program
}

#[test]
fn optional_lookup_discovery_survives_alpha_renaming_and_output_advice() {
    fn rename(tokens: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        tokens
            .into_iter()
            .map(|token| match token {
                proc_macro2::TokenTree::Ident(name) if name == "__lookup_lookup" => {
                    proc_macro2::TokenTree::Ident(syn::Ident::new("filter_step", name.span()))
                }
                proc_macro2::TokenTree::Group(group) => proc_macro2::TokenTree::Group(
                    proc_macro2::Group::new(group.delimiter(), rename(group.stream())),
                ),
                token => token,
            })
            .collect()
    }
    let mut source: Program = syn::parse2(rename(automatic_source().to_token_stream())).unwrap();
    source.sequence.output = Some(cq_lang_sequence::OutputProtocol::Reserve);
    let candidates = adaptive_candidates(&source).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].filter, "filter_step");
    let target = apply_adaptive(&source, &candidates[0]).unwrap();
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn automatic_candidates_create_checked_real_branches_and_freeze_advice() {
    let source = automatic_source();
    apply_adaptive(
        &source,
        &syn::parse_quote!(__lookup_lookup, selected, 100000),
    )
    .unwrap();
    let candidates = adaptive_candidates(&source).unwrap();
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.filter, "__lookup_lookup");
    assert_eq!(candidate.published, "selected");
    assert_eq!(candidate.threshold, 100_000);
    assert_eq!(candidate.select, None);
    let target = apply_adaptive(&source, candidate).unwrap();
    check_rewrite(&source, &target).unwrap();
    assert!(
        target
            .sequence
            .definitions
            .iter()
            .any(|definition| matches!(definition.term, Term::Choose(_)))
    );
    assert_eq!(
        adaptive(&source, candidate.to_token_stream()).unwrap(),
        target
    );
    assert_eq!(
        syn::parse2::<AdaptiveCandidate>(candidate.to_token_stream()).unwrap(),
        *candidate
    );
    let result = target
        .structures
        .physical
        .program
        .definitions
        .iter()
        .find(|definition| definition.name == "result")
        .unwrap();
    assert!(matches!(
        result.operator,
        cq_lang_sequence::structures::physical::Operator::Project(_)
    ));
    assert_eq!(target.structures.physical.program.returned, "result");
    assert!(adaptive_candidates(&target).unwrap().is_empty());
}

#[test]
fn automatic_policy_uses_evidence_but_replay_does_not() {
    let mut source = automatic_source();
    source.evidence = syn::parse_quote!(evidence {
        count R = 100;
        selectivity selected = 99 / 100;
    });
    let candidates = adaptive_candidates(&source).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].threshold, 36);
    assert_eq!(candidates[0].select, Some(false));
    let candidate = &candidates[0];
    assert_eq!(
        syn::parse2::<AdaptiveCandidate>(candidate.to_token_stream()).unwrap(),
        *candidate
    );
    source.evidence = Evidence::default();
    let target = apply_adaptive(&source, candidate).unwrap();
    check_rewrite(&source, &target).unwrap();
    assert!(
        !target
            .sequence
            .definitions
            .iter()
            .any(|definition| matches!(definition.term, Term::Choose(_)))
    );
}

#[test]
fn authored_decomposition_is_not_rebuilt_by_adaptive_preparation() {
    let source = automatic_source();
    let candidate: AdaptiveCandidate = syn::parse_quote!(__lookup_lookup, selected, 100000);
    let authored = cq_sequence_plan::rewrites::store(&source, quote!(__lookup_lookup)).unwrap();
    assert!(adaptive_candidates(&authored).unwrap().is_empty());
    assert!(apply_adaptive(&authored, &candidate).is_err());
}

fn two_filter_source() -> Program {
    let structures = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,rtag:i32); relation S(a:i32,stag:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,rtag -> rtag};
            s = pipe rename S@2 {a -> a,stag -> stag};
            rf = let point select r if (rtag = 7);
            sf = let point select s if (stag = 9);
            __lookup_lookup_r = pipe semi eager [a] l,rf;
            __lookup_lookup_s = pipe semi eager [a] __lookup_lookup_r,sf;
            jr = pipe probe first [a] __lookup_lookup_s,rf;
            joined = pipe probe first [a] jr,sf;
            result = pipe canon project joined keep {a};
            return result as answer(a);
        }
        structures {
            l host scan = Reader [];
            r host scan = Reader [];
            s host scan = Reader [];
            r host result(rf) = Reader [rtag];
            s host result(sf) = Reader [stag];
            rf = Provider [a];
            sf = Provider [a];
            result canon = Canon;
        }
    };
    let mut program = cq_sequence_lowering::compile(&structures).unwrap();
    program.sequence.optional_filters = vec![
        syn::parse_quote!(__lookup_lookup_r),
        syn::parse_quote!(__lookup_lookup_s),
    ];
    program
}

#[test]
fn every_unestimated_filter_gets_a_guard_without_erasing_previous_choices() {
    let source = two_filter_source();
    let candidates = adaptive_candidates(&source).unwrap();
    assert_eq!(candidates.len(), 2);
    let first = apply_adaptive(&source, &candidates[0]).unwrap();
    let both = apply_adaptive(&first, &candidates[1]).unwrap();
    check_rewrite(&source, &both).unwrap();
    let guards: std::collections::BTreeSet<_> = both
        .sequence
        .definitions
        .iter()
        .filter_map(|definition| {
            let Term::Choose(arms) = &definition.term else {
                return None;
            };
            let Guard::Lt(Len::Input(name), _) = &arms[0].0 else {
                return None;
            };
            Some(name.as_str())
        })
        .collect();
    assert!(guards.contains("rf"));
    assert!(guards.contains("sf"));
    for prior in first
        .sequence
        .definitions
        .iter()
        .filter(|definition| matches!(definition.term, Term::Choose(_)))
    {
        assert_eq!(
            both.sequence
                .definitions
                .iter()
                .find(|definition| definition.name == prior.name),
            Some(prior)
        );
    }
    let replayed = candidates
        .iter()
        .try_fold(source.clone(), |program, candidate| {
            adaptive(&program, candidate.to_token_stream())
        })
        .unwrap();
    assert_eq!(both, replayed);
}

#[test]
fn static_filter_advice_composes_with_an_existing_dynamic_choice() {
    let source = two_filter_source();
    let mut candidates = adaptive_candidates(&source).unwrap();
    assert_eq!(candidates.len(), 2);
    candidates[1].select = Some(false);
    let first = apply_adaptive(&source, &candidates[0]).unwrap();
    let target = apply_adaptive(&first, &candidates[1]).unwrap();
    check_rewrite(&source, &target).unwrap();
    assert!(
        target
            .sequence
            .definitions
            .iter()
            .any(|definition| matches!(&definition.term,
        Term::Choose(arms) if arms[0].0 == Guard::Lt(Len::Input("rf".into()), 100_000)))
    );
    assert!(
        !target
            .sequence
            .definitions
            .iter()
            .any(|definition| matches!(&definition.term,
        Term::Choose(arms) if arms[0].0 == Guard::Lt(Len::Input("sf".into()), 100_000)))
    );
}
