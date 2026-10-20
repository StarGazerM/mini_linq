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
    }
}
#[test]
fn variants_require_exact_coverage_and_erasure() {
    let original = scheduled();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| {
            p.variants.pop();
        }),
        Box::new(|p| {
            let v = p.variants[0].clone();
            p.variants.push(v);
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
fn rewrites_preserve_every_recorded_decision() {
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
    assert!(check_rewrite(&original, &renamed).is_err());
    let mut reordered = original.clone();
    reordered.variants.reverse();
    contract::check(&reordered).unwrap();
    assert!(check_rewrite(&original, &reordered).is_err());
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
