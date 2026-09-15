//! Expected schedules and a source-order evaluator authored independently of RA.
#[path = "support/compiled.rs"]
mod compiled;
#[path = "support/rust.rs"]
mod rust;
use compiled::compile_ra;
use cq_compose::compile_cq_derivation;
use cq_compose::explain_cq_plan;
use cq_ir::{cq, relational_plan as ra, symbol_name};
use cq_plan::JoinPolicy;
use cq_plan::PlanningOptions;
use cq_plan::Rule as JoinRule;
use quote::{format_ident, quote};
use rust::run_rust;

const CASES: &[(&str, &str, &[&str])] = &[
    ("literal", "R(7,x), T(y,_), S(x,y)", &["R", "S", "T"]),
    (
        "literal_let",
        "let k:i32 = 7, R(k,x), T(y,_), S(x,y)",
        &["R", "S", "T"],
    ),
    (
        "dependent_let",
        "R(x,_), let k = f(x), T(y,_), S(k,y)",
        &["R", "S", "T"],
    ),
    (
        "guard_pair",
        "R(x,_), T(y,_), if guard(x,y), S(x,y)",
        &["R", "T", "S"],
    ),
    (
        "guard_after",
        "R(x,_), T(y,_), S(x,y), if guard(x,y)",
        &["R", "S", "T"],
    ),
    (
        "opaque_guard",
        "R(x,_), if allowed!(x), T(y,_), S(x,y)",
        &["R", "S", "T"],
    ),
    ("same_atom", "R(x,f(x)), T(y,_), S(x,y)", &["R", "S", "T"]),
    ("repeated", "R(x,x), T(y,_), S(x,y)", &["R", "S", "T"]),
    (
        "pure_guard",
        "R(x,_), T(y,_), S(x,y), if x > 0",
        &["R", "S", "T"],
    ),
    (
        "computed_argument",
        "R(x,_), T(y,_), S(f(x),y)",
        &["R", "T", "S"],
    ),
    (
        "destructuring",
        "R(x,_), let (k,other) = (f(x),*x), T(y,_), S(k,y), if other > 0",
        &["R", "S", "T"],
    ),
    (
        "two_guards",
        "R(x,_), if allowed!(x), T(y,_), S(x,y), if guard(x,y)",
        &["R", "S", "T"],
    ),
];

#[test]
fn parsed_optional_clauses_cannot_be_silently_dropped_by_any_planner() {
    for body in ["R(x,_), !T(x,_)", "R(x,_), agg total = sum(y) in T(x,y)"] {
        let source = source(body);
        for strategy in [
            JoinPolicy::Written,
            JoinPolicy::Connected,
            JoinPolicy::GreedyWidth,
            JoinPolicy::Auto,
        ] {
            let error = explain_cq_plan(
                &source,
                PlanningOptions {
                    strategy,
                    max_arity: None,
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains("extended CQ semantic contract"));
        }
    }
}

#[test]
fn replay_and_independent_contract_reject_invalid_expression_rules() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32);
        answer(x) :- R(x), let k:i32 = 7, if k > 0;
    };
    let (plan, mut report) = explain_cq_plan(
        &source,
        PlanningOptions {
            strategy: JoinPolicy::Written,
            max_arity: None,
        },
    )
    .unwrap();
    for replacement in [JoinRule::Guard { clause: 1 }, JoinRule::Bind { clause: 99 }] {
        let mut wrong = report.clone();
        wrong.derivation[1] = replacement;
        assert!(compile_cq_derivation(&source, &wrong).is_err());
        assert!(cq_compose::check_cq_plan(&source, &plan, &wrong).is_err());
    }
    report.derivation.swap(1, 2);
    let error = compile_cq_derivation(&source, &report).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("logical input before its producer")
    );
    let error = cq_compose::check_cq_plan(&source, &plan, &report).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("logical input before its producer")
    );
}

#[test]
fn every_clause_permutation_agrees_with_written_dependency_constraints() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32); relation S(b:i32); relation T(c:i32);
        answer(x) :- R(x), S(y), let z:i32 = *x + *y, T(k), if z > 0;
    };
    let (_, report) = explain_cq_plan(
        &source,
        PlanningOptions {
            strategy: JoinPolicy::Written,
            max_arity: None,
        },
    )
    .unwrap();
    // Authored from source semantics: R and S may swap, both precede the let;
    // T and the primitive guard may swap, both remain after that computation.
    let legal = [
        [0, 1, 2, 3, 4],
        [1, 0, 2, 3, 4],
        [0, 1, 2, 4, 3],
        [1, 0, 2, 4, 3],
    ];
    let rules = [
        JoinRule::Match { clause: 0 },
        JoinRule::Match { clause: 1 },
        JoinRule::Bind { clause: 2 },
        JoinRule::Match { clause: 3 },
        JoinRule::Guard { clause: 4 },
    ];
    let mut permutations = vec![Vec::new()];
    for _ in 0..5 {
        permutations = permutations
            .into_iter()
            .flat_map(|prefix| {
                (0..5)
                    .filter(|i| !prefix.contains(i))
                    .map(|i| {
                        let mut order = prefix.clone();
                        order.push(i);
                        order
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    assert_eq!(permutations.len(), 120);
    for order in permutations {
        let mut candidate = report.clone();
        candidate.derivation = order
            .iter()
            .map(|&i| rules[i].clone())
            .chain([JoinRule::Finish])
            .collect();
        let result = compile_cq_derivation(&source, &candidate);
        assert_eq!(
            result.is_ok(),
            legal.iter().any(|expected| expected.as_slice() == order),
            "{order:?}: {result:?}"
        );
    }
}

#[test]
fn atom_matching_a_let_value_cannot_be_its_producer() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32); answer(k) :- let k:i32 = 7, R(k);
    };
    let (target, mut report) = explain_cq_plan(
        &source,
        PlanningOptions {
            strategy: JoinPolicy::Written,
            max_arity: None,
        },
    )
    .unwrap();
    report.derivation.swap(0, 1);
    for error in [
        compile_cq_derivation(&source, &report).unwrap_err(),
        cq_compose::check_cq_plan(&source, &target, &report).unwrap_err(),
    ] {
        assert!(
            error
                .to_string()
                .contains("logical input before its producer"),
            "{error}"
        );
    }
}

fn source(body: &str) -> cq::Module {
    syn::parse_str(&format!("pub struct P; relation R(a:i32,b:i32); relation S(a:i32,b:i32); relation T(a:i32,b:i32); relation answer(value:i32); answer(head(x)) :- {body};")).unwrap()
}

fn order(plan: &ra::Module) -> Vec<String> {
    plan.plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Rename(r) => Some(symbol_name(&r.relation)),
            _ => None,
        })
        .collect()
}

#[test]
fn twelve_expression_patterns_have_expected_join_orders_and_compiled_results() {
    let mut programs = Vec::new();
    let mut checks = Vec::new();
    for (case, &(name, body, expected_order)) in CASES.iter().enumerate() {
        for policy in [
            JoinPolicy::Written,
            JoinPolicy::Connected,
            JoinPolicy::GreedyWidth,
            JoinPolicy::Auto,
        ] {
            let policy_name = format!("{policy:?}");
            let source = source(body);
            let (plan, report) = explain_cq_plan(
                &source,
                PlanningOptions {
                    strategy: policy,
                    max_arity: None,
                },
            )
            .unwrap();
            let expected = if policy == JoinPolicy::Written {
                vec!["R", "T", "S"]
            } else {
                expected_order.to_vec()
            };
            assert_eq!(order(&plan), expected, "{name} {policy:?}");
            assert_eq!(compile_cq_derivation(&source, &report).unwrap(), plan);
            if name == "pure_guard" && policy != JoinPolicy::Written {
                let filter = plan
                    .plan
                    .definitions
                    .iter()
                    .position(|d| matches!(d.operator, ra::Operator::Select(_)))
                    .unwrap();
                let second_atom = plan
                    .plan
                    .definitions
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| matches!(d.operator, ra::Operator::Rename(_)))
                    .nth(1)
                    .unwrap()
                    .0;
                assert!(
                    filter < second_atom,
                    "ready total comparison must precede the next join"
                );
            }
            let module = format_ident!("case_{}", programs.len());
            let rust = compile_ra(&plan).unwrap();
            programs.push(quote! { mod #module {
                use std::sync::atomic::{AtomicUsize,Ordering::Relaxed};
                pub static KEY:AtomicUsize=AtomicUsize::new(0);
                pub static GUARD:AtomicUsize=AtomicUsize::new(0);
                pub static HEAD:AtomicUsize=AtomicUsize::new(0);
                fn f(x:&i32)->i32 { KEY.fetch_add(1,Relaxed); x+1 }
                fn guard(x:&i32,y:&i32)->bool { GUARD.fetch_add(1,Relaxed); x+y>2 }
                fn head(x:&i32)->i32 { HEAD.fetch_add(1,Relaxed); *x }
                macro_rules! allowed { ($x:expr) => { guard($x,&0) }; }
                #rust
            }});
            checks.push(quote! {
                #module::KEY.store(0,Relaxed); #module::GUARD.store(0,Relaxed); #module::HEAD.store(0,Relaxed);
                let expected=reference(#case,&r,&s,&t);
                let (actual,stats)=#module::P::run_with_stats(r.clone(),s.clone(),t.clone());
                assert_eq!(actual,expected.0,"{} {:?}",#name,#policy_name);
                assert_eq!([#module::KEY.load(Relaxed),#module::GUARD.load(Relaxed),#module::HEAD.load(Relaxed)],expected.1,"{} evaluation multiplicities",#name);
                if #case == 0 && !r.is_empty() && !s.is_empty() && !t.is_empty() {
                    // Count actual enumerated join pairs, not a claimed report metric.
                    pair_counts.push((#policy_name,stats.join_pairs));
                }
            });
        }
    }
    // The reference follows source order on deduplicated inputs. No compiler
    // IR, planner helper, or RA interpreter contributes its expected results.
    let oracle = quote! {
        fn reference(case:usize,r:&[(i32,i32)],s:&[(i32,i32)],t:&[(i32,i32)]) -> (Vec<(i32,)>,[usize;3]) {
            let r:std::collections::BTreeSet<_>=r.iter().copied().collect();
            let s:std::collections::BTreeSet<_>=s.iter().copied().collect();
            let t:std::collections::BTreeSet<_>=t.iter().copied().collect();
            let mut out=std::collections::BTreeSet::new(); let mut calls=[0;3];
            for &(a,b) in &r {
                let x=if case<=1 { if a!=7 {continue;} b } else {a};
                if case==6 {calls[0]+=1; if b!=x+1 {continue;}}
                if case==7 && a!=b {continue;}
                let mut key=x;
                if case==2 || case==10 {calls[0]+=1; key=x+1;}
                if case==5 || case==11 {calls[1]+=1; if x<=2 {continue;}}
                for &(y,_) in &t {
                    if case==3 {calls[1]+=1; if x+y<=2 {continue;}}
                    if case==9 {calls[0]+=1; key=x+1;}
                    for &(left,right) in &s {
                        if left!=key || right!=y {continue;}
                        if case==4 || case==11 {calls[1]+=1; if x+y<=2 {continue;}}
                        if (case==8 || case==10) && x<=0 {continue;}
                        calls[2]+=1; out.insert((x,));
                    }
                }
            }
            (out.into_iter().collect(),calls)
        }
    };
    run_rust(quote! { #(#programs)* #oracle fn main() {
        use std::sync::atomic::Ordering::Relaxed;
        let datasets=vec![
            (vec![(7,1),(1,2),(2,3),(1,1),(7,2),(1,2),(3,4)],vec![(1,1),(1,2),(2,3),(3,2),(2,2)],vec![(1,9),(2,9),(2,8)]),
            (vec![],vec![(1,1)],vec![(1,2)]),
            (vec![(7,1),(1,2)],vec![],vec![(1,2),(2,3)]),
            (vec![(7,1),(1,2)],vec![(1,1)],vec![]),
            (vec![(7,1),(7,1),(1,2),(1,2)],vec![(1,1),(1,1),(2,1)],vec![(1,9),(1,9)]),
            (vec![(7,1),(1,2),(0,0),(-1,0)],vec![(9,9)],vec![(1,9),(2,8)]),
        ];
        for (dataset,(r,s,t)) in datasets.into_iter().enumerate() {
            let mut pair_counts=Vec::new(); #(#checks)*
            if dataset==0 { assert_eq!(pair_counts.len(),4); for (_,pairs) in &pair_counts[1..] { assert!(*pairs<pair_counts[0].1,"connected plans must enumerate fewer join pairs than the written Cartesian prefix"); } }
        }
    }});
}

#[test]
fn literal_normalization_and_connected_joins_match_handwritten_ra() {
    let source = source(CASES[0].1);
    let program = &source.program;
    let known: ra::Module = syn::parse_quote! { #program relational {
        r0=unit;
        r1=extend r0 with {__cq_0_0:i32} value ({let __cq_0_0:i32=7; (__cq_0_0,)});
        r2=rename R {a->__cq_0_0,b->x};
        r3=natural_join r1 with r2;
        r4=rename S {a->x,b->y};
        r5=natural_join r3 with r4;
        r6=rename T {a->y,b->__cq_1_1};
        r7=natural_join r5 with r6;
        r8=project r7 keep {r#__cq_0_0,r#__cq_1_1,r#x,r#y};
        output r8 as answer(head(x));
    }};
    let (actual, report) = explain_cq_plan(
        &source,
        PlanningOptions {
            strategy: JoinPolicy::Connected,
            max_arity: None,
        },
    )
    .unwrap();
    assert_eq!(actual, known);
    assert_eq!(
        report.derivation,
        vec![
            JoinRule::Constant {
                clause: 0,
                argument: 0
            },
            JoinRule::Match { clause: 0 },
            JoinRule::Match { clause: 2 },
            JoinRule::Match { clause: 1 },
            JoinRule::Finish
        ]
    );
}

#[test]
fn width_policy_counts_anonymous_columns_and_enforces_the_cap() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation Wide(a:i32,b:i32,c:i32,d:i32); relation Seed(a:i32);
        relation answer(value:i32);
        answer(f(x)) :- Wide(x,_,_,_), Seed(x);
    };
    for strategy in [JoinPolicy::GreedyWidth, JoinPolicy::Auto] {
        let (plan, report) = explain_cq_plan(
            &source,
            PlanningOptions {
                strategy,
                max_arity: Some(4),
            },
        )
        .unwrap();
        assert_eq!(order(&plan), vec!["Seed", "Wide"]);
        assert_eq!(report.peak_arity, 4);
        assert!(
            explain_cq_plan(
                &source,
                PlanningOptions {
                    strategy,
                    max_arity: Some(3)
                }
            )
            .is_err()
        );
    }
}
