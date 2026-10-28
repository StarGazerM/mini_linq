use cq_lang_vo as valuation;
use cq_reduce::{Options, Reductions, check, compile};
use cq_testkit::{corpus as scenarios, databases, valuation_semantics as evaluator};
use valuation::cq;
#[test]
fn reductions_preserve_answers_and_correspond_to_options() {
    for case in scenarios::SCENARIOS
        .iter()
        .filter(|c| !c.name.ends_with("_reduced"))
    {
        let source: valuation::Program = syn::parse_str(case.vo).unwrap();
        {
            for semijoins in [Reductions::None, Reductions::Tree, Reductions::Hypergraph] {
                let options = Options { semijoins };
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
fn reduction_never_pushes_opaque_rust_steps() {
    let source: valuation::Program = syn::parse_str("struct Q; relation R(a:i32,b:i32); answer(a) :- R(a,b), if { b > 0 }; valuation { R@0(a,b); if { b > 0 }; }").unwrap();
    let options = Options {
        semijoins: Reductions::None,
    };
    let target = compile(&source, &options).unwrap();
    assert_eq!(target, source);
    check(&source, &target, &Options::default()).unwrap();
}
#[test]
fn opaque_and_multi_atom_guards_remain_steps() {
    for guard in ["opaque!(a)", "{ a < c }"] {
        let text = format!(
            "struct Q; relation R(a:i32,b:i32); relation S(b:i32,c:i32); answer(a,c) :- R(a,b), S(b,c), if {guard}; valuation {{ R@0(a,b); S@1(b,c); if {guard}; }}"
        );
        let source = syn::parse_str(&text).unwrap();
        let target = compile(
            &source,
            &Options {
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
fn all_guard_kinds_keep_their_schedule_around_tree_reductions() {
    let query = CHAIN.replace(
        "S(b, c);",
        "S(b, c), if { b > 0 }, if { a != c }, if { a > 0 };",
    );
    let source = schedule(
        &query,
        &format!("{ATOMS} if {{ b > 0 }}; if {{ a != c }}; if {{ a > 0 }};"),
    );
    for semijoins in [Reductions::None, Reductions::Tree] {
        let options = Options { semijoins };
        let target = compile(&source, &options).unwrap();
        let retained: Vec<_> = target
            .valuation
            .steps
            .iter()
            .filter(|s| !matches!(s, valuation::Step::Reduce(..)))
            .cloned()
            .collect();
        assert_eq!(retained, source.valuation.steps);
        check(&source, &target, &options).unwrap();
    }
}

#[test]
fn computed_and_opaque_guards_keep_their_schedule() {
    let source = schedule(
        "struct Q; relation L(a: i32);
        answer(a) :- L(a), let d: i32 = a + 1, if { d > 0 }, if matches!(a, 1 | 2);",
        "L@0(a); let d: i32 = a + 1; if { d > 0 }; if matches!(a, 1 | 2);",
    );
    let options = Options { ..TREE };
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

fn reject(
    source: &valuation::Program,
    target: &valuation::Program,
    options: &Options,
    names: &[&str],
) {
    let error = check(source, target, options).unwrap_err().to_string();
    for name in names {
        assert!(error.contains(name), "{error}");
    }
}

#[test]
fn correspondence_checks_domains_erasure_and_each_reduction_obligation() {
    let source = schedule(CHAIN, ATOMS);
    let good = compile(&source, &TREE).unwrap();
    reject(&good, &good, &TREE, &["clause #1", "already"]);
    let mut invalid = source.clone();
    invalid.valuation.steps[0] = syn::parse_quote!(L@9(a););
    reject(&invalid, &good, &TREE, &["clause #9"]);
    reject(&source, &invalid, &TREE, &["clause #9"]);
    let cycle = schedule(
        "struct Cycle; relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(a:i32,c:i32); answer(a,b,c) :- R(a,b), S(b,c), T(a,c);",
        "R@0(a,b); S@1(b,c); T@2(a,c);",
    );
    reject(&cycle, &cycle, &TREE, &["Cycle", "acyclic"]);
    let mut changed = good.clone();
    changed.cq.name = syn::parse_quote!(Other);
    reject(&source, &changed, &TREE, &["Other", "preserve"]);
    let mut reordered = source.clone();
    reordered.valuation.steps.swap(0, 1);
    valuation::contract::check(&reordered).unwrap();
    reject(
        &source,
        &reordered,
        &Options::default(),
        &["clause #1", "erasure"],
    );
    reject(
        &source,
        &good,
        &Options::default(),
        &["clause #1", "semijoins=None"],
    );
    for position in 0..4 {
        let mut missing = good.clone();
        missing.valuation.steps.remove(position);
        reject(&source, &missing, &TREE, &["clause #", "missing"]);
    }
    let mut wrong = good.clone();
    wrong.valuation.steps[0] = syn::parse_quote!(reduce R@1 with L@0(_););
    reject(&source, &wrong, &TREE, &["clause #1", "shared"]);
    let mut nonedge = good.clone();
    nonedge
        .valuation
        .steps
        .insert(0, syn::parse_quote!(reduce L@0 with S@2(_, _);));
    reject(
        &source,
        &nonedge,
        &TREE,
        &["clause #0", "not a join-tree edge"],
    );
    let mut duplicate = good.clone();
    duplicate
        .valuation
        .steps
        .insert(0, good.valuation.steps[0].clone());
    reject(&source, &duplicate, &TREE, &["clause #1", "duplicate"]);
    let mut phase = good.clone();
    phase.valuation.steps.swap(1, 2);
    reject(&source, &phase, &TREE, &["clause #2", "precede"]);
    let mut upward = good.clone();
    upward.valuation.steps.swap(0, 1);
    reject(&source, &upward, &TREE, &["clause #1", "upward"]);
    let mut downward = good.clone();
    downward.valuation.steps.swap(2, 3);
    reject(&source, &downward, &TREE, &["clause #0", "downward"]);
}

#[test]
fn rust_guard_reductions_are_not_in_the_language() {
    for expression in ["b > 0", "10 / b > 1", "opaque!(b)"] {
        assert!(
            syn::parse_str::<valuation::Step>(&format!("reduce R@1 if {expression};")).is_err()
        );
    }
}

#[test]
fn full_reducer_accepts_independent_sibling_orders() {
    let query = "struct Star; relation A(a:i32,x:i32); relation B(b:i32,y:i32); relation C(c:i32,z:i32); relation R(a:i32,b:i32,c:i32); answer(a,b,c) :- A(a,x), B(b,y), C(c,z), R(a,b,c);";
    let atoms = "A@0(a,x); B@1(b,y); C@2(c,z); R@3(a,b,c);";
    let source = schedule(query, atoms);
    let alternative = schedule(
        query,
        &format!(
            "reduce R@3 with A@0(a,_); reduce R@3 with B@1(b,_); reduce R@3 with C@2(c,_); reduce C@2 with R@3(_,_,c); reduce B@1 with R@3(_,b,_); reduce A@0 with R@3(a,_,_); {atoms}"
        ),
    );
    assert_ne!(compile(&source, &TREE).unwrap(), alternative);
    check(&source, &alternative, &TREE).unwrap();
}

const HYPERGRAPH: Options = Options {
    semijoins: Reductions::Hypergraph,
};

fn direct_reductions(steps: &[valuation::Step]) -> Vec<valuation::Step> {
    steps
        .iter()
        .filter(|step| matches!(step, valuation::Step::Reduce(..)))
        .cloned()
        .collect()
}

#[test]
fn cyclic_body_keeps_exact_block_cuts_and_reduces_acyclic_branches() {
    let source = schedule(
        "struct Cycle; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
         relation T(a:i32,c:i32); relation U(a:i32); relation V(a:i32);
         answer(a,b,c) :- R(a,b), S(b,c), T(a,c), {U(a),V(a)} | {U(a)};",
        "R@0(a,b); S@1(b,c); T@2(a,c);
         block @3(a) hoisted {U@3.0.0(a); V@3.0.1(a);} | {U@3.1.0(a);};",
    );
    assert!(!cq::hypergraph::Analysis::of(&source.cq).acyclic);
    let target = compile(&source, &HYPERGRAPH).unwrap();
    assert_eq!(
        direct_reductions(&target.valuation.steps),
        vec![
            syn::parse_quote!(reduce R@0 with block@3(a);),
            syn::parse_quote!(reduce T@2 with block@3(a);),
        ]
    );
    let valuation::Step::Block(block, _) = target.valuation.steps.last().unwrap() else {
        unreachable!()
    };
    assert_eq!(block.placement, valuation::blocks::Placement::Hoisted);
    // Acyclic does not by itself justify indexing unfiltered supports.
    assert!(direct_reductions(&block.branches[0].steps).is_empty());
    assert!(direct_reductions(&block.branches[1].steps).is_empty());
    check(&source, &target, &HYPERGRAPH).unwrap();
    assert!(compile(&source, &TREE).is_err());
    reject(&source, &source, &TREE, &["acyclic"]);
    for position in 0..2 {
        let mut missing = target.clone();
        missing.valuation.steps.remove(position);
        assert!(check(&source, &missing, &HYPERGRAPH).is_err());
    }
    let mut extra = target.clone();
    extra
        .valuation
        .steps
        .insert(0, syn::parse_quote!(reduce R@0 with S@1(b,_);));
    valuation::contract::check(&extra).unwrap();
    assert!(check(&source, &extra, &HYPERGRAPH).is_err());
}

#[test]
fn cyclic_branches_keep_block_cuts_and_plan_descendants_independently() {
    let source = schedule(
        "struct Branch; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
         relation T(a:i32,c:i32); relation U(a:i32); relation V(a:i32);
         answer(a) :- {R(a,b), S(b,c), T(a,c), {U(a),V(a)} | {U(a)}} | {U(a)};",
        "block @0(a) hoisted {
            R@0.0.0(a,b); S@0.0.1(b,c); T@0.0.2(a,c);
            block @0.0.3(a) hoisted {U@0.0.3.0.0(a); V@0.0.3.0.1(a);}
                | {U@0.0.3.1.0(a);};
         } | {U@0.1.0(a);};",
    );
    assert!(cq::hypergraph::Analysis::of(&source.cq).acyclic);
    let target = compile(&source, &HYPERGRAPH).unwrap();
    assert!(direct_reductions(&target.valuation.steps).is_empty());
    let valuation::Step::Block(block, _) = &target.valuation.steps[0] else {
        unreachable!()
    };
    assert_eq!(
        direct_reductions(&block.branches[0].steps),
        vec![
            syn::parse_quote!(reduce R@0.0.0 with block@0.0.3(a);),
            syn::parse_quote!(reduce T@0.0.2 with block@0.0.3(a);),
        ]
    );
    let valuation::Step::Block(nested, _) = block.branches[0].steps.last().unwrap() else {
        unreachable!()
    };
    assert!(direct_reductions(&nested.branches[0].steps).is_empty());
    check(&source, &target, &HYPERGRAPH).unwrap();
    for position in 0..2 {
        let mut missing = target.clone();
        let valuation::Step::Block(block, _) = &mut missing.valuation.steps[0] else {
            unreachable!()
        };
        block.branches[0].steps.remove(position);
        assert!(check(&source, &missing, &HYPERGRAPH).is_err());
    }
    let mut extra = target.clone();
    let valuation::Step::Block(block, _) = &mut extra.valuation.steps[0] else {
        unreachable!()
    };
    block.branches[0]
        .steps
        .insert(0, syn::parse_quote!(reduce R@0.0.0 with S@0.0.1(b,_);));
    valuation::contract::check(&extra).unwrap();
    assert!(check(&source, &extra, &HYPERGRAPH).is_err());
}

#[test]
fn acyclic_body_retains_tree_reductions_and_additional_block_cuts() {
    let source = schedule(
        "struct TreeBlock; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
         relation U(b:i32); relation V(b:i32);
         answer(a,c) :- R(a,b), S(b,c), {U(b)} | {V(b)};",
        "R@0(a,b); S@1(b,c); block @2(b) hoisted {U@2.0.0(b);} | {V@2.1.0(b);};",
    );
    assert!(cq::hypergraph::Analysis::of(&source.cq).acyclic);
    let target = compile(&source, &HYPERGRAPH).unwrap();
    check(&source, &target, &HYPERGRAPH).unwrap();
    assert!(
        direct_reductions(&target.valuation.steps).len()
            < direct_reductions(&compile(&source, &TREE).unwrap().valuation.steps).len() + 2
    );
    for cut in [
        syn::parse_quote!(reduce R@0 with block@2(b);),
        syn::parse_quote!(reduce S@1 with block@2(b);),
    ] {
        assert!(target.valuation.steps.contains(&cut));
        let mut missing = target.clone();
        let position = missing
            .valuation
            .steps
            .iter()
            .position(|step| step == &cut)
            .unwrap();
        missing.valuation.steps.remove(position);
        assert!(check(&source, &missing, &HYPERGRAPH).is_err());
    }
}

#[test]
fn cyclic_body_without_blocks_has_no_hypergraph_reductions() {
    let source = schedule(
        "struct Cycle; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
         relation T(a:i32,c:i32); answer(a,b,c) :- R(a,b),S(b,c),T(a,c);",
        "R@0(a,b); S@1(b,c); T@2(a,c);",
    );
    assert!(!cq::hypergraph::Analysis::of(&source.cq).acyclic);
    assert_eq!(compile(&source, &HYPERGRAPH).unwrap(), source);
    check(&source, &source, &HYPERGRAPH).unwrap();
    let mut extra = source.clone();
    extra
        .valuation
        .steps
        .insert(0, syn::parse_quote!(reduce R@0 with S@1(b,_);));
    valuation::contract::check(&extra).unwrap();
    assert!(check(&source, &extra, &HYPERGRAPH).is_err());
}

#[test]
fn filtered_reducer_omits_unfiltered_dangling_checks_without_assuming_keys() {
    let source = schedule(CHAIN, ATOMS);
    assert_eq!(compile(&source, &HYPERGRAPH).unwrap(), source);
    check(&source, &source, &HYPERGRAPH).unwrap();
    let full = compile(&source, &TREE).unwrap();
    assert!(
        check(&source, &full, &HYPERGRAPH)
            .unwrap_err()
            .to_string()
            .contains("no relational filter")
    );
    // The join still rejects dangling tuples; omission does not claim they exist.
    for seed in 0..16 {
        let database = databases::binary_database(&source.cq, seed);
        assert_eq!(
            evaluator::evaluate(&source, &database),
            evaluator::evaluate(&full, &database)
        );
    }
}

#[test]
fn filtered_supports_run_point_before_domain_and_propagate_to_unfiltered_parents() {
    let query = "struct Star; relation Domain(a:i32,x:i32); relation Point(b:i32,y:i32);
        relation Empty(c:i32,z:i32); relation Fact(a:i32,b:i32,c:i32);
        answer(a,b,c) :- Domain(a,x), if *x != 4, Point(b,y), if *y == 7, Empty(c,z), Fact(a,b,c);";
    let mut source = schedule(
        query,
        "Domain@0(a,x); Point@2(b,y); Empty@4(c,z); Fact@5(a,b,c);",
    );
    source.cq = cq::normal::normalize(&source.cq).unwrap();
    for clause in [1, 3] {
        let step = match &source.cq.query.body[clause] {
            cq::BodyItem::Constraint(c) => {
                valuation::Step::Constraint(c.clone(), Default::default())
            }
            cq::BodyItem::Equality(e) => valuation::Step::Equality(e.clone(), Default::default()),
            other => panic!("expected relational predicate: {other:?}"),
        };
        source.valuation.steps.insert(clause, step);
    }
    let target = compile(&source, &HYPERGRAPH).unwrap();
    check(&source, &target, &HYPERGRAPH).unwrap();
    let reductions = direct_reductions(&target.valuation.steps);
    let supports: Vec<_> = reductions
        .iter()
        .map(|step| {
            let valuation::Step::Reduce(r, _) = step else {
                unreachable!()
            };
            let valuation::Reduction::With(_, support) = &r.reduction;
            support.relation.to_string()
        })
        .collect();
    assert_eq!(supports[0], "Point");
    assert_eq!(supports[1], "Domain");
    assert!(!supports.contains(&"Empty".to_string()));
    let mut wrong = target.clone();
    wrong.valuation.steps.swap(0, 1);
    assert!(check(&source, &wrong, &HYPERGRAPH).is_err());
    let mut missing = target.clone();
    missing.valuation.steps.remove(0);
    assert!(check(&source, &missing, &HYPERGRAPH).is_err());
    for seed in 0..16 {
        let database = databases::binary_database(&source.cq, seed);
        assert_eq!(
            evaluator::evaluate(&source, &database),
            evaluator::evaluate(&target, &database)
        );
    }
}

#[test]
fn leaf_only_filter_never_flows_back_into_its_own_subtree() {
    let query = "struct Leaf; relation L(a:i32,x:i32); relation R(a:i32,b:i32);
        relation S(b:i32,c:i32); answer(a,c) :- L(a,x), if *x == 1, R(a,b), S(b,c);";
    let mut source = schedule(query, "L@0(a,x); R@2(a,b); S@3(b,c);");
    source.cq = cq::normal::normalize(&source.cq).unwrap();
    let step = match &source.cq.query.body[1] {
        cq::BodyItem::Constraint(c) => valuation::Step::Constraint(c.clone(), Default::default()),
        cq::BodyItem::Equality(e) => valuation::Step::Equality(e.clone(), Default::default()),
        other => panic!("expected relational predicate: {other:?}"),
    };
    source.valuation.steps.insert(1, step);
    let target = compile(&source, &HYPERGRAPH).unwrap();
    check(&source, &target, &HYPERGRAPH).unwrap();
    let reductions = direct_reductions(&target.valuation.steps);
    assert_eq!(
        reductions,
        vec![
            syn::parse_quote!(reduce R@2 with L@0(a,_);),
            syn::parse_quote!(reduce S@3 with R@2(_,b);),
        ]
    );
}

#[test]
fn range_side_streams_against_point_driving_component_without_reverse_index() {
    let query = "struct PointRange; relation P(k:i32,x:i32); relation R(k:i32,y:i32);
        answer(k) :- P(k,x), if *x == 1, R(k,y), if *y > 4;";
    let mut source = schedule(query, "P@0(k,x); R@2(k,y);");
    source.cq = cq::normal::normalize(&source.cq).unwrap();
    for clause in [1, 3] {
        let step = match &source.cq.query.body[clause] {
            cq::BodyItem::Constraint(c) => {
                valuation::Step::Constraint(c.clone(), Default::default())
            }
            cq::BodyItem::Equality(e) => valuation::Step::Equality(e.clone(), Default::default()),
            other => panic!("expected relational predicate: {other:?}"),
        };
        source.valuation.steps.insert(clause, step);
    }
    let target = compile(&source, &HYPERGRAPH).unwrap();
    check(&source, &target, &HYPERGRAPH).unwrap();
    assert_eq!(
        direct_reductions(&target.valuation.steps),
        vec![syn::parse_quote!(reduce R@2 with P@0(k,_);)]
    );
    let mut reverse = target.clone();
    reverse
        .valuation
        .steps
        .insert(1, syn::parse_quote!(reduce P@0 with R@2(k,_);));
    assert!(check(&source, &reverse, &HYPERGRAPH).is_err());
    for seed in 0..16 {
        let database = databases::binary_database(&source.cq, seed);
        assert_eq!(
            evaluator::evaluate(&source, &database),
            evaluator::evaluate(&target, &database)
        );
    }
}

#[test]
fn reductions_preserve_and_check_independent_branch_metadata() {
    let source: valuation::Program = syn::parse_quote! {
        struct Q; relation L(o:i32,p:i32,s:i32); relation S(s:i32,n:i32); relation N(n:i32);
        answer(o,p,s) :- L(o,p,s), S(s,n), N(n);
        valuation branches { branch [1,2] (s); } {
            L@0(o,p,s); S@1(s,n); N@2(n);
        }
    };
    for semijoins in [Reductions::None, Reductions::Tree, Reductions::Hypergraph] {
        let options = Options { semijoins };
        if semijoins == Reductions::Tree {
            assert!(compile(&source, &options).is_err());
            continue;
        }
        let target = compile(&source, &options).unwrap();
        check(&source, &target, &options).unwrap();
        assert_eq!(
            source.valuation.join_branches,
            target.valuation.join_branches
        );
        let mut wrong = target;
        wrong.valuation.join_branches = None;
        assert!(check(&source, &wrong, &options).is_err());
    }
}

#[test]
fn reductions_preserve_generators_as_opaque_non_targets() {
    use quote::ToTokens;
    let source: valuation::Program = syn::parse_str(
        "struct Q; relation R(a:i32); relation S(a:i32); relation answer(b:i32); answer(b) :- R(a), S(a), for b in 0..a; valuation { R@0(a); S@1(a); for b in 0..a; }"
    ).unwrap();
    for semijoins in [
        Reductions::None,
        Reductions::Tree,
        Reductions::Hypergraph,
        Reductions::Masks,
    ] {
        let options = Options { semijoins };
        let target = compile(&source, &options).unwrap();
        check(&source, &target, &options).unwrap();
        assert_eq!(
            syn::parse2::<valuation::Program>(target.to_token_stream()).unwrap(),
            target
        );
        assert!(matches!(
            target.valuation.steps.last(),
            Some(valuation::Step::For(..))
        ));
        for step in &target.valuation.steps {
            if let valuation::Step::Reduce(reduction, _) = step {
                assert_ne!(reduction.clause(), 2);
                let valuation::Reduction::With(_, support) = &reduction.reduction;
                assert_ne!(support.clause(), 2);
            }
        }
        let mut changed = target.clone();
        changed.valuation.steps.pop();
        assert!(check(&source, &changed, &options).is_err());
    }
}

#[test]
fn reductions_keep_literal_occurrences_without_literal_columns() {
    let source: valuation::Program = syn::parse_quote! {
        struct Q; relation R(a:i32,b:i32); relation S(a:i32); relation T(a:i32);
        answer(x) :- R(7,x), S(x), T(9);
        valuation { R@0(x); S@1(x); T@2(); }
    };
    for semijoins in [Reductions::None, Reductions::Tree, Reductions::Hypergraph] {
        let options = Options { semijoins };
        let target = compile(&source, &options).unwrap();
        check(&source, &target, &options).unwrap();
        assert_eq!(target.cq, source.cq);
        valuation::contract::check(&target).unwrap();
    }
}

#[test]
fn negation_is_preserved_but_never_used_as_reduction_support_or_target() {
    let source = schedule(
        "struct Negative; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
         relation K(a:i32); answer(x,z) :- R(x,y), S(y,z), !K(x);",
        "R@0(x,y); S@1(y,z); !K(x);",
    );
    for semijoins in [Reductions::None, Reductions::Tree, Reductions::Hypergraph] {
        let options = Options { semijoins };
        let target = compile(&source, &options).unwrap();
        check(&source, &target, &options).unwrap();
        assert_eq!(
            target
                .valuation
                .steps
                .iter()
                .filter(|s| matches!(s, valuation::Step::Negation(..)))
                .count(),
            1
        );
        for step in &target.valuation.steps {
            if let valuation::Step::Reduce(r, _) = step {
                let valuation::Reduction::With(_, support) = &r.reduction;
                assert_ne!(r.clause(), 2);
                assert_ne!(support.clause(), 2);
            }
        }
        for seed in 0..16 {
            let database = databases::binary_database(&source.cq, seed);
            assert_eq!(
                evaluator::evaluate(&source, &database),
                evaluator::evaluate(&target, &database)
            );
        }
    }
}

#[test]
fn masks_checks_edge_premises_and_child_versions_without_schedule_equality() {
    let source = schedule(
        "struct Masks; relation A(a:i32); relation B(a:i32,b:i32);
         relation C(b:i32,c:i32); relation D(c:i32,k:i32);
         answer(a) :- A(a), B(a,b), C(b,c), D(c,7);",
        "A@0(a); B@1(a,b); C@2(b,c); D@3(c);",
    );
    let options = Options {
        semijoins: Reductions::Masks,
    };
    let target = compile(&source, &options).unwrap();
    check(&source, &target, &options).unwrap();
    let mut late = target.clone();
    let outer = late.valuation.steps.remove(2);
    late.valuation.steps.insert(0, outer);
    valuation::contract::check(&late).unwrap();
    assert!(check(&source, &late, &options).is_err());
    for reductions in [
        "reduce B@1 with C@2(b,_);", // missing filtered leaf
        "reduce B@1 with C@2(b,_); reduce C@2 with D@3(c);", // stale child
        "reduce C@2 with D@3(c); reduce C@2 with D@3(c); reduce B@1 with C@2(b,_);",
        "reduce C@2 with D@3(c); reduce B@1 with C@2(b,_); reduce A@0 with B@1(a,_);",
        "reduce C@2 with D@3(c); reduce B@1 with C@2(b,_); reduce B@1 with A@0(a);",
        "reduce C@2 with D@3(c); reduce B@1 with C@2(b,_); reduce D@3 with B@1(_,_);",
    ] {
        let mut changed = source.clone();
        let prefix: valuation::Valuation =
            syn::parse_str(&format!("valuation {{ {reductions} }}")).unwrap();
        changed.valuation.steps.splice(0..0, prefix.steps);
        valuation::contract::check(&changed).unwrap();
        assert!(check(&source, &changed, &options).is_err(), "{reductions}");
    }
    let unfiltered = schedule(CHAIN, ATOMS);
    let mut unsupported = unfiltered.clone();
    unsupported
        .valuation
        .steps
        .insert(0, syn::parse_quote!(reduce R@1 with S@2(b,_);));
    valuation::contract::check(&unsupported).unwrap();
    assert!(check(&unfiltered, &unsupported, &options).is_err());
}

#[test]
fn masks_checks_sibling_priority_but_allows_independent_subtree_interleavings() {
    let source = schedule(
        "struct Masks; relation A(a:i32,b:i32); relation B(a:i32,c:i32);
         relation C(c:i32,k:i32); relation D(b:i32,d:i32); relation E(d:i32,k:i32);
         answer(a,b) :- A(a,b), B(a,c), C(c,7), D(b,d), E(d,9);",
        "A@0(a,b); B@1(a,c); C@2(c); D@3(b,d); E@4(d);",
    );
    let options = Options {
        semijoins: Reductions::Masks,
    };
    let mut target = compile(&source, &options).unwrap();
    assert!(matches!(
        target.valuation.steps[0],
        valuation::Step::Reduce(..)
    ));
    assert!(matches!(
        target.valuation.steps[1],
        valuation::Step::Reduce(..)
    ));
    target.valuation.steps.swap(0, 1);
    check(&source, &target, &options).unwrap();

    let source = schedule(
        "struct Siblings; relation A(a:i32); relation B(a:i32,b:i32,c:i32);
         relation P(b:i32,k:i32); relation Q(c:i32,k:i32);
         answer(a) :- A(a), B(a,b,c), P(b,7), Q(c,9);",
        "A@0(a); B@1(a,b,c); P@2(b); Q@3(c);",
    );
    let mut target = compile(&source, &options).unwrap();
    check(&source, &target, &options).unwrap();
    target.valuation.steps.swap(0, 1);
    valuation::contract::check(&target).unwrap();
    assert!(check(&source, &target, &options).is_err());
}

#[test]
fn correlated_blocks_are_not_independent_reduction_operands() {
    let source = schedule(
        "struct Correlated; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
         relation U(b:i32); relation V(b:i32);
         answer(a,c) :- R(a,b), S(b,c), {U(b)} | {V(b)};",
        "R@0(a,b); S@1(b,c);
         block @2(b) correlated {U@2.0.0(b);} | {V@2.1.0(b);};",
    );
    for semijoins in [Reductions::Tree, Reductions::Hypergraph, Reductions::Masks] {
        let options = Options { semijoins };
        let target = compile(&source, &options).unwrap();
        check(&source, &target, &options).unwrap();
        for seed in 0..8 {
            let database = databases::binary_database(&source.cq, seed);
            assert_eq!(
                evaluator::evaluate(&source, &database),
                evaluator::evaluate(&target, &database),
                "{semijoins:?} seed {seed}",
            );
        }
    }
}

#[test]
fn masks_never_uses_an_outer_block_as_cut_support() {
    let source = schedule(
        "struct OuterBlock; relation U(a:i32); relation V(a:i32);
         relation R(a:i32,b:i32); relation S(b:i32,k:i32);
         answer(a) :- {U(a)} | {V(a)}, R(a,b), S(b,7);",
        "block @0(a) hoisted {U@0.0.0(a);} | {V@0.1.0(a);};
         R@1(a,b); S@2(b);",
    );
    let options = Options {
        semijoins: Reductions::Masks,
    };
    let target = compile(&source, &options).unwrap();
    check(&source, &target, &options).unwrap();
    assert_eq!(
        direct_reductions(&target.valuation.steps),
        vec![syn::parse_quote!(reduce R@1 with S@2(b);)]
    );
    let mut changed = target;
    changed
        .valuation
        .steps
        .insert(0, syn::parse_quote!(reduce R@1 with block@0(a);));
    valuation::contract::check(&changed).unwrap();
    assert!(check(&source, &changed, &options).is_err());
}
