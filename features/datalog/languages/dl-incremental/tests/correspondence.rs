use cq_lang_datalog_incremental::source::BodyItem;
use cq_lang_datalog_incremental::{Program, check_rewrite, contract};
use quote::{ToTokens, quote};
type Mutation = Box<dyn Fn(&mut Program)>;
const SEMINAIVE: &str = include_str!("fixtures/incremental.datalog");
fn scheduled() -> Program {
    syn::parse_str(SEMINAIVE).unwrap()
}
#[test]
fn delta_declarations_are_fresh_complete_unique_and_idb_only() {
    let original = scheduled();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| {
            p.deltas.clear();
        }),
        Box::new(|p| {
            p.deltas.pop();
        }),
        Box::new(|p| {
            p.deltas.push(p.deltas[0].clone());
        }),
        Box::new(|p| {
            p.deltas[1].name = p.deltas[0].name.clone();
        }),
        Box::new(|p| {
            p.deltas[0].name = syn::parse_quote!(Edge);
        }),
        Box::new(|p| {
            p.deltas[0].name = syn::parse_quote!(edges);
        }),
        Box::new(|p| {
            p.deltas[0].name = syn::parse_quote!(x);
        }),
        Box::new(|p| {
            p.deltas[0].relation = syn::parse_quote!(Unknown);
        }),
        Box::new(|p| {
            p.deltas[0].relation = syn::parse_quote!(Edge);
        }),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut program = original.clone();
        mutate(&mut program);
        assert!(contract::check(&program).is_err(), "mutation {index}");
        assert!(
            check_rewrite(&original, &program).is_err(),
            "mutation {index}"
        );
        assert!(
            check_rewrite(&program, &original).is_err(),
            "mutation {index}"
        );
    }
}
#[test]
fn variants_require_complete_coverage_and_erasure() {
    let original = scheduled();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| {
            p.variants.pop();
        }),
        Box::new(|p| {
            p.variants[0].rule = 100;
        }),
        Box::new(|p| {
            p.variants[1].delta = None;
        }),
        Box::new(|p| {
            p.variants[0].delta = Some(0);
        }),
        Box::new(|p| {
            p.variants[1].delta = Some(100);
        }),
        Box::new(|p| {
            p.variants[0].query.head.args[0] = syn::parse_quote!(*y);
        }),
        Box::new(|p| {
            p.variants[1].query = syn::parse_quote!(Reach(*x,*z) :- Reach(x,y), Reach(y,z););
        }),
        Box::new(|p| {
            p.variants[1].query =
                syn::parse_quote!(Reach(*x,*z) :- ReachChanges(x,y), ReachChanges(y,z););
        }),
        Box::new(|p| {
            p.variants[3].query = syn::parse_quote!(Result(*x,*y) :- Reach(x,y););
        }),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut program = original.clone();
        mutate(&mut program);
        assert!(contract::check(&program).is_err(), "mutation {index}");
        assert!(
            check_rewrite(&original, &program).is_err(),
            "mutation {index}"
        );
        assert!(
            check_rewrite(&program, &original).is_err(),
            "mutation {index}"
        );
    }
}

#[test]
fn malformed_variants_fail_standalone_cq_validation_before_erasure() {
    for (query, expected) in [
        (
            quote!(Reach(*x,*z) :- ReachChanges(x), Reach(y,z);),
            "arity",
        ),
        (quote!(Reach(*x) :- ReachChanges(x,y), Reach(y,z);), "arity"),
        (
            quote!(Reach(*missing,*z) :- ReachChanges(x,y), Reach(y,z);),
            "range restricted",
        ),
        (
            quote!(Reach(*x,*z) :- Missing(x,y), Reach(y,z);),
            "not declared",
        ),
        (
            quote!(Reach(*x,*z) :- if *x > 0, ReachChanges(x,y), Reach(y,z);),
            "before",
        ),
        (
            quote!(Reach(*x,*z) :- ReachChanges(x,y), !Blocked(z,_), Reach(y,z);),
            "before",
        ),
        (
            quote!(Reach(*x,*z) :- ReachChanges(x,y), Reach(y,z), !Blocked(x);),
            "arity",
        ),
    ] {
        let mut program = scheduled();
        program.variants[1].query = syn::parse2(query).unwrap();
        let error = contract::check(&program).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
        assert!(
            !error.contains("erasure"),
            "standalone check must run first: {error}"
        );
    }
}

#[test]
fn rewrites_admit_fresh_delta_names_and_decision_order() {
    let original = scheduled();
    let mut renamed = scheduled();
    renamed.deltas[0].name = syn::parse_quote!(Alternative);
    for variant in &mut renamed.variants {
        if let Some(position) = variant.delta {
            let BodyItem::Positive { atom } = &mut variant.query.body[position] else {
                unreachable!()
            };
            atom.relation = syn::parse_quote!(Alternative);
        }
    }
    contract::check(&renamed).unwrap();
    check_rewrite(&original, &renamed).unwrap();
    let mut reordered = original.clone();
    reordered.variants.reverse();
    contract::check(&reordered).unwrap();
    reordered.deltas.reverse();
    check_rewrite(&original, &reordered).unwrap();
    let mut changed = original.clone();
    changed.strata.source.observer = Some(syn::parse_quote!(observer));
    assert!(check_rewrite(&original, &changed).is_err());
}

#[test]
fn negative_validation_projection_does_not_extend_core_cq() {
    let program = scheduled();
    contract::check(&program).unwrap();
    let declarations = &program.strata.source.declarations;
    let query = &program.strata.source.rules[2];
    let cq: cq_lang_cq::Program =
        syn::parse2(quote!(struct Check; #(#declarations)* #query)).unwrap();
    assert!(
        cq_lang_cq::contract::check(&cq)
            .unwrap_err()
            .to_string()
            .contains("extended CQ")
    );
    assert!(matches!(query.body[1], BodyItem::Negation(_)));
}

#[test]
fn analysis_supplies_declared_graph_schedule_and_delta_facts() {
    let a = contract::check(&scheduled()).unwrap();
    assert_eq!(a.graph.names, ["Edge", "Blocked", "Reach", "Result"]);
    assert_eq!(a.positions["Reach"], 2);
    assert_eq!(a.components, vec![vec![2], vec![3]]);
    assert_eq!(a.component, vec![None, None, Some(0), Some(1)]);
    assert_eq!(a.changing_positions, vec![vec![], vec![0, 1], vec![]]);
    assert_eq!(a.deltas["Reach"], "ReachChanges");
    assert!(a.dependencies.contains(&(3, 1, true)));
    assert!(a.graph.reach[3][0]);
}
#[test]
fn authored_incremental_fixtures_roundtrip() {
    for text in [
        SEMINAIVE,
        include_str!("fixtures/written_order.datalog"),
        include_str!("fixtures/delta_probes.datalog"),
    ] {
        let p: Program = syn::parse_str(text).unwrap();
        contract::check(&p).unwrap();
        assert_eq!(p, syn::parse2::<Program>(p.to_token_stream()).unwrap());
        check_rewrite(&p, &p).unwrap();
    }
}

#[test]
fn covering_variants_may_repeat_but_every_copy_must_erase() {
    let original = scheduled();
    let mut redundant = original.clone();
    redundant.variants.extend(original.variants.iter().cloned());
    redundant.variants.reverse();
    contract::check(&redundant).unwrap();
    check_rewrite(&original, &redundant).unwrap();
    check_rewrite(&redundant, &original).unwrap();

    redundant.variants.last_mut().unwrap().query.head.args[0] = syn::parse_quote!(*y);
    assert!(contract::check(&redundant).is_err());
    assert!(check_rewrite(&original, &redundant).is_err());
}

#[test]
fn coverage_is_per_changing_position_not_just_per_rule() {
    let original = scheduled();
    for missing in [0, 1] {
        let mut incomplete = original.clone();
        incomplete
            .variants
            .retain(|v| v.rule != 1 || v.delta != Some(missing));
        // Duplicating the other position cannot fill the missing position.
        let other = incomplete
            .variants
            .iter()
            .find(|v| v.rule == 1)
            .unwrap()
            .clone();
        incomplete.variants.push(other);
        let error = contract::check(&incomplete).unwrap_err().to_string();
        assert!(
            error.contains("missing changing-position variant"),
            "{error}"
        );
    }
    let mut missing_seed = original.clone();
    missing_seed.variants.retain(|v| v.rule != 0);
    let error = contract::check(&missing_seed).unwrap_err().to_string();
    assert!(error.contains("missing seed variant"), "{error}");
}

#[test]
fn coarser_strata_require_delta_coverage_for_newly_changing_reads() {
    let original = scheduled();
    let mut coarser = original.clone();
    coarser.strata.components = vec![vec![syn::parse_quote!(Reach), syn::parse_quote!(Result)]];
    assert!(contract::check(&coarser).is_err());
    coarser.variants[3].delta = Some(0);
    coarser.variants[3].query =
        syn::parse_quote!(Result(*x,*y) :- ReachChanges(x,y), !Blocked(x,_););
    let analysis = contract::check(&coarser).unwrap();
    assert_eq!(
        analysis.changing_positions,
        vec![vec![], vec![0, 1], vec![0]]
    );
    // Admissible incremental programs over different strata are not L→L rewrites.
    assert!(check_rewrite(&original, &coarser).is_err());

    let mut changed_rule = original.clone();
    changed_rule.strata.source.rules[0].head.args[0] = syn::parse_quote!(*y);
    changed_rule.variants[0].query.head.args[0] = syn::parse_quote!(*y);
    contract::check(&changed_rule).unwrap();
    assert!(check_rewrite(&original, &changed_rule).is_err());
}

#[test]
fn only_seminaive_covering_is_a_supported_scheme() {
    for scheme in ["naive", "seminaive_exact", "unknown"] {
        assert!(
            syn::parse_str::<Program>(
                &SEMINAIVE.replace("scheme seminaive_covering;", &format!("scheme {scheme};"))
            )
            .is_err()
        );
    }
}

#[test]
fn negative_idb_reads_are_frozen_below_delta_rounds() {
    let original: Program = syn::parse_quote! {
        struct Frozen;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output B;
        A(*x) :- E(x);
        B(*x) :- E(x), !A(x);
        B(*x) :- B(x), !A(x);
        strata { component [A]; component [B]; }
        incremental {
            scheme seminaive_covering;
            delta A = AChanges;
            delta B = BChanges;
            variant 0 seed { A(*x) :- E(x); }
            variant 1 seed { B(*x) :- E(x), !A(x); }
            variant 2 delta 0 { B(*x) :- BChanges(x), !A(x); }
        }
    };
    contract::check(&original).unwrap();
    let mut changed = original.clone();
    changed.variants[2].query = syn::parse_quote!(B(*x) :- BChanges(x), !AChanges(x););
    assert!(contract::check(&changed).is_err());
    changed = original.clone();
    changed.variants[2].delta = Some(1);
    assert!(contract::check(&changed).is_err());
    changed = original.clone();
    changed.strata.components = vec![vec![syn::parse_quote!(A), syn::parse_quote!(B)]];
    assert!(contract::check(&changed).is_err());
}
