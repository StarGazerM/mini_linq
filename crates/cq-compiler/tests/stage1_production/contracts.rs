use super::*;
use cq_compiler::JoinRule as Rule;
use std::collections::BTreeSet;

fn star() -> cq::Module {
    syn::parse_str("pub struct Star; relation R(a:i32,k:i32); relation S(k:i32,b:i32); relation T(k:i32,c:i32);
        answer(a) :- R(a,k), S(k,b), T(k,c), if (b > 0);").unwrap()
}

#[test]
fn missing_or_repeated_administrative_rules_are_rejected() {
    let source = star();
    let (ra, planned) = explain_cq_plan(
        &source,
        Options {
            strategy: Strategy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    // Every rule has a real obligation, including no-output phase transitions.
    for i in 0..planned.derivation.len() {
        let mut missing = planned.clone();
        missing.derivation.remove(i);
        assert!(
            check_cq_plan(&source, &ra, &missing).is_err(),
            "omitted {:?}",
            planned.derivation[i]
        );
        let mut repeated = planned.clone();
        repeated.derivation.insert(i, planned.derivation[i].clone());
        assert!(
            check_cq_plan(&source, &ra, &repeated).is_err(),
            "repeated {:?}",
            planned.derivation[i]
        );
    }
    let mut wrong = planned.clone();
    let combine = wrong
        .derivation
        .iter_mut()
        .find(|r| matches!(r, Rule::Combine { .. }))
        .unwrap();
    if let Rule::Combine { left, right } = combine {
        *right = *left;
    }
    assert!(check_cq_plan(&source, &ra, &wrong).is_err());
    let mut wrong = planned.clone();
    wrong.peak_arity = 0;
    assert!(check_cq_plan(&source, &ra, &wrong).is_err());
    let mut wrong = planned.clone();
    wrong.analysis.pop();
    assert!(check_cq_plan(&source, &ra, &wrong).is_err());
}

#[test]
fn reduction_requires_phase_dependencies_and_latest_versions() {
    let source = star();
    let (ra, planned) = explain_cq_plan(
        &source,
        Options {
            strategy: Strategy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    let mut early = planned.clone();
    let freeze = early
        .derivation
        .iter()
        .position(|r| *r == Rule::Freeze)
        .unwrap();
    early.derivation.remove(freeze);
    let begin = early
        .derivation
        .iter()
        .position(|r| *r == Rule::Begin)
        .unwrap();
    early.derivation.insert(begin + 1, Rule::Freeze);
    assert!(check_cq_plan(&source, &ra, &early).is_err());
    let mut wrong = planned.clone();
    let up = wrong
        .derivation
        .iter()
        .position(|r| matches!(r, Rule::Up { .. }))
        .unwrap();
    let down = wrong
        .derivation
        .iter()
        .position(|r| matches!(r, Rule::Down { .. }))
        .unwrap();
    wrong.derivation.swap(up, down);
    assert!(check_cq_plan(&source, &ra, &wrong).is_err());
    let mut stale = ra.clone();
    let mut earlier = std::collections::BTreeMap::<String, syn::Ident>::new();
    let mut changed = false;
    for d in &mut stale.plan.definitions {
        if let ra::Operator::Semijoin(join) = &mut d.operator {
            if let Some(old) = earlier.get(&cq_ir::symbol_name(&join.right)) {
                join.right = old.clone();
                changed = true;
                break;
            }
            earlier.insert(cq_ir::symbol_name(&d.result), join.left.clone());
        }
    }
    assert!(changed);
    assert!(check_cq_plan(&source, &stale, &planned).is_err());
}

#[test]
fn a_different_policy_can_reorder_independent_reductions() {
    let source: cq::Module=syn::parse_str("struct Branch; relation R(a:i32,b:i32,c:i32); relation S(a:i32,x:i32); relation T(b:i32,y:i32); relation U(c:i32,z:i32);
        answer(a) :- R(a,b,c), S(a,x), T(b,y), U(c,z);").unwrap();
    let (_ra, planned) = explain_cq_plan(
        &source,
        Options {
            strategy: Strategy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    let tree = planned.join_tree.as_ref().unwrap();
    let down: Vec<_> = planned
        .derivation
        .iter()
        .enumerate()
        .filter_map(|(i, r)| match r {
            Rule::Down { child } => Some((i, *child)),
            _ => None,
        })
        .collect();
    let siblings = down
        .iter()
        .enumerate()
        .find_map(|(i, a)| {
            down.iter()
                .skip(i + 1)
                .find(|b| tree.parent[a.1] == tree.parent[b.1])
                .map(|b| (a.0, b.0))
        })
        .unwrap();
    let mut other = planned.clone();
    other.derivation.swap(siblings.0, siblings.1);
    let alternate = cq_compiler::compile_cq_derivation(&source, &other).unwrap();
    check_cq_plan(&source, &alternate, &other).unwrap();
    for seed in 0..64 {
        let db = binary_database(&source.program, seed);
        assert_eq!(
            oracle::evaluate_plan(source.program.inputs(), &alternate.plan, &db).output,
            oracle::evaluate_cq(&source.program, &db)
        );
    }
}

#[test]
fn predicates_must_be_consumed_before_their_last_columns_are_pruned() {
    let source: cq::Module = syn::parse_str(
        "struct Predicate; relation R(a:i32,b:i32); relation S(c:i32,d:i32);
        answer(a) :- R(a,b), S(c,d), if (b < c);",
    )
    .unwrap();
    for strategy in [
        Strategy::Written,
        Strategy::Connected,
        Strategy::GreedyWidth,
        Strategy::Auto,
    ] {
        let (ra, planned) = explain_cq_plan(
            &source,
            Options {
                strategy,
                max_arity: None,
            },
        )
        .unwrap();
        let info = ra::check_in(source.program.inputs(), &ra.plan).unwrap();
        let select = ra
            .plan
            .definitions
            .iter()
            .find_map(|d| match &d.operator {
                ra::Operator::Select(s) => Some((d, s)),
                _ => None,
            })
            .unwrap();
        let before = &info[&cq_ir::symbol_name(&select.1.input)].heading;
        assert!(before.contains("b") && before.contains("c"));
        assert_eq!(
            info[&cq_ir::symbol_name(&ra.plan.output.input)].heading,
            BTreeSet::from(["a".into()])
        );
        for seed in 0..64 {
            let db = binary_database(&source.program, seed);
            assert_eq!(
                oracle::evaluate_plan(source.program.inputs(), &ra.plan, &db).output,
                oracle::evaluate_cq(&source.program, &db)
            );
        }
        let mut wrong = planned.clone();
        let index = wrong
            .derivation
            .iter()
            .position(|r| matches!(r, Rule::Select { .. }))
            .unwrap();
        wrong.derivation.remove(index);
        assert!(check_cq_plan(&source, &ra, &wrong).is_err());
    }
}

#[test]
fn arity_bound_applies_to_every_emitted_result_and_greedy_reduces_width() {
    // Written joins the disconnected ends before the connecting occurrence.
    let source: cq::Module = syn::parse_str(
        "struct Width; relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
        answer(a,d) :- R(a,b), T(c,d), S(b,c);",
    )
    .unwrap();
    let (ra, written) = explain_cq_plan(
        &source,
        Options {
            strategy: Strategy::Written,
            max_arity: None,
        },
    )
    .unwrap();
    let (_, greedy) = explain_cq_plan(
        &source,
        Options {
            strategy: Strategy::GreedyWidth,
            max_arity: None,
        },
    )
    .unwrap();
    assert_eq!(written.peak_arity, 4);
    assert_eq!(greedy.peak_arity, 3);
    assert!(
        explain_cq_plan(
            &source,
            Options {
                strategy: Strategy::Written,
                max_arity: Some(3)
            }
        )
        .is_err()
    );
    assert!(
        explain_cq_plan(
            &source,
            Options {
                strategy: Strategy::GreedyWidth,
                max_arity: Some(3)
            }
        )
        .is_ok()
    );
    assert!(
        explain_cq_plan(
            &source,
            Options {
                strategy: Strategy::Auto,
                max_arity: Some(1)
            }
        )
        .is_err()
    );
    let mut tampered = written;
    tampered.options.max_arity = Some(3);
    assert!(check_cq_plan(&source, &ra, &tampered).is_err());
}

#[test]
fn semijoin_backend_uses_composite_existence_keys_without_pair_enumeration() {
    let direct: ra::Module = syn::parse_str(
        "pub struct Semi;
        relation R(a:i32,b:i32,p:i32); relation S(a:i32,b:i32,q:i32);
        answer(p) :- R(a,b,p), S(a,b,q);
        relational { r=rename R {a->a,b->b,p->p}; s=rename S {a->a,b->b,q->q};
        f=semijoin r with s; p=project f keep {p}; output p as answer(p); }",
    )
    .unwrap();
    let rust = compile_ra(&direct).unwrap();
    run_rust(quote! {#rust fn main() {
        let (rows,stats)=Semi::run_with_stats((0..100).map(|p|(1,2,p)),(0..1000).map(|q|(1,2,q)));
        assert_eq!(rows.len(),100); assert_eq!(stats.semijoin_probes,100); assert_eq!(stats.join_pairs,0);
        assert!(Semi::run([(1,2,0)],[(1,9,0),(8,2,1)]).is_empty());
        assert!(Semi::run([(1,2,0)],[]).is_empty());
    }});
}

#[test]
fn rust_expression_syntax_reaches_both_logical_routes() {
    let source = star();
    assert!(cq_compiler::compile_cq(&source).is_ok());
    for predicate in [
        "f(x) > 0",
        "x + 1 > 0",
        "unknown > 0",
        "x > 2147483648",
        "x > 0u32",
    ] {
        let text = format!("struct P; relation R(x:i32); answer(x) :- R(x), if ({predicate});");
        let source = syn::parse_str::<cq::Module>(&text).unwrap();
        cq_compiler::compile_cq_planned(&source).unwrap();
        cq_compiler::compile_cq(&source).unwrap();
    }
    // Name resolution, literal typing and trait selection belong to Rust.
    // This checks parser/lowering preservation, not that these host fragments
    // typecheck without declarations in their enclosing Rust module.
}
