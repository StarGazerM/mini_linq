use cq_lang_vo as valuation;
use cq_reduce::{Options, Pushdown, Reductions, check, compile};
use valuation::cq;
#[allow(dead_code)]
#[path = "../../../test-support/relational/databases.rs"]
mod databases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/valuation_semantics.rs"]
mod evaluator;
#[path = "../../../test-support/relational/scenarios.rs"]
mod scenarios;
#[allow(dead_code)]
#[path = "../../../test-support/relational/values.rs"]
mod values;
#[test]
fn reductions_preserve_answers_and_correspond_to_options() {
    for case in scenarios::SCENARIOS
        .iter()
        .filter(|c| !c.name.ends_with("_reduced"))
    {
        let source: valuation::Program = syn::parse_str(case.vo).unwrap();
        for guards in [Pushdown::None, Pushdown::Atoms] {
            for semijoins in [Reductions::None, Reductions::Tree] {
                let options = Options { guards, semijoins };
                if semijoins == Reductions::Tree
                    && !cq::hypergraph::Analysis::of(&source.cq).acyclic
                {
                    assert!(compile(&source, &options).is_err());
                    continue;
                }
                let target = compile(&source, &options).unwrap();
                check(&source, &target, &options).unwrap();
                for seed in 0..8 {
                    let database = databases::binary_database(&source.cq, seed);
                    assert_eq!(
                        evaluator::evaluate(&source, &database),
                        evaluator::evaluate(&target, &database),
                        "{} seed {seed}",
                        case.name
                    );
                }
                let mut changed = target.clone();
                changed.valuation.steps.pop();
                assert!(check(&source, &changed, &options).is_err());
            }
        }
    }
}
#[test]
fn pushed_guards_precede_reads_and_reduction_is_not_applied_twice() {
    let source: valuation::Program = syn::parse_str("struct Q; relation R(a:i32,b:i32); answer(a) :- R(a,b), if b > 0; valuation { R@0(a,b); if b > 0; }").unwrap();
    let options = Options {
        guards: Pushdown::Atoms,
        semijoins: Reductions::None,
    };
    let target = compile(&source, &options).unwrap();
    assert!(matches!(
        target.valuation.steps[0],
        valuation::Step::Reduce(..)
    ));
    assert!(matches!(
        target.valuation.steps[1],
        valuation::Step::Atom(..)
    ));
    assert!(compile(&target, &options).is_err());
    assert!(check(&source, &target, &Options::default()).is_err());
}
#[test]
fn opaque_and_multi_atom_guards_remain_steps() {
    for guard in ["opaque!(a)", "a < c"] {
        let text = format!(
            "struct Q; relation R(a:i32,b:i32); relation S(b:i32,c:i32); answer(a,c) :- R(a,b), S(b,c), if {guard}; valuation {{ R@0(a,b); S@1(b,c); if {guard}; }}"
        );
        let source = syn::parse_str(&text).unwrap();
        let target = compile(
            &source,
            &Options {
                guards: Pushdown::Atoms,
                semijoins: Reductions::None,
            },
        )
        .unwrap();
        assert!(matches!(
            target.valuation.steps.last(),
            Some(valuation::Step::Guard(..))
        ));
    }
}

fn schedule(query: &str, steps: &str) -> valuation::Program {
    syn::parse_str(&format!("{query} valuation {{ {steps} }}")).unwrap()
}

const CHAIN: &str = "struct Chain;
    relation L(a: i32);
    relation R(a: i32, b: i32);
    relation S(b: i32, c: i32);
    answer(a, c) :- L(a), R(a, b), S(b, c);";
const ATOMS: &str = "L@0(a); R@1(a, b); S@2(b, c);";
const TREE: Options = Options {
    guards: Pushdown::None,
    semijoins: Reductions::Tree,
};

#[test]
fn tree_reduction_pins_both_sweeps_and_rejects_missing_or_foreign_reducers() {
    let source = schedule(CHAIN, ATOMS);
    let expected = schedule(
        CHAIN,
        "
        reduce R@1 with L@0(a);
        reduce S@2 with R@1(_, b);
        reduce R@1 with S@2(b, _);
        reduce L@0 with R@1(a, _);
        L@0(a); R@1(a, b); S@2(b, c);",
    );
    assert_eq!(compile(&source, &TREE).unwrap(), expected);
    assert_eq!(compile(&source, &Options::default()).unwrap(), source);
    assert!(check(&source, &source, &TREE).is_err());
    let mut foreign = expected;
    foreign
        .valuation
        .steps
        .insert(0, syn::parse_quote!(reduce L@0 with R@1(a, _);));
    valuation::contract::check(&foreign).unwrap();
    assert!(check(&source, &foreign, &TREE).is_err());
}

#[test]
fn guard_pushdown_precedes_tree_reduction_and_keeps_multi_atom_guards_last() {
    let query = CHAIN.replace("S(b, c);", "S(b, c), if b > 0, if a != c, if a > 0;");
    let source = schedule(&query, &format!("{ATOMS} if b > 0; if a != c; if a > 0;"));
    for (semijoins, steps) in [
        (
            Reductions::None,
            "
            reduce L@0 if a > 0; L@0(a);
            reduce R@1 if b > 0; R@1(a, b);
            S@2(b, c); if a != c;",
        ),
        (
            Reductions::Tree,
            "
            reduce R@1 if b > 0;
            reduce L@0 if a > 0;
            reduce R@1 with L@0(a);
            reduce S@2 with R@1(_, b);
            reduce R@1 with S@2(b, _);
            reduce L@0 with R@1(a, _);
            L@0(a); R@1(a, b); S@2(b, c); if a != c;",
        ),
    ] {
        let options = Options {
            guards: Pushdown::Atoms,
            semijoins,
        };
        assert_eq!(compile(&source, &options).unwrap(), schedule(&query, steps));
    }
}

#[test]
fn computed_and_opaque_guards_keep_their_schedule() {
    let source = schedule(
        "struct Q; relation L(a: i32);
        answer(a) :- L(a), let d: i32 = a + 1, if d > 0, if matches!(a, 1 | 2);",
        "L@0(a); let d: i32 = a + 1; if d > 0; if matches!(a, 1 | 2);",
    );
    let options = Options {
        guards: Pushdown::Atoms,
        ..TREE
    };
    assert_eq!(compile(&source, &options).unwrap(), source);
}

#[test]
fn reduced_scenario_fixtures_pin_the_tree_schedule() {
    for name in ["ya_composite", "ya_deep", "ya_delivery", "ya_disconnected"] {
        let source = scenarios::SCENARIOS
            .iter()
            .find(|c| c.name == name)
            .unwrap();
        let reduced = scenarios::SCENARIOS
            .iter()
            .find(|c| c.name == format!("{name}_reduced"))
            .unwrap();
        assert_eq!(
            compile(&syn::parse_str(source.vo).unwrap(), &TREE).unwrap(),
            syn::parse_str::<valuation::Program>(reduced.vo).unwrap(),
            "{name}"
        );
    }
}
