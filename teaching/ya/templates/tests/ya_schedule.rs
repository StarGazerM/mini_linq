//! This suite invokes logical planning only; physical lowering is not needed.
use cq_compiler::{JoinPolicy, JoinRule, PlanningOptions, explain_cq_graph};
use cq_ir::{cq, relational_plan as ra};
use std::collections::BTreeSet;

#[test]
fn reductions_respect_tree_dependencies_and_exactly_once_obligations() {
    let cases = [
        "struct One; relation R(a:i32); answer(a) :- R(a);",
        "struct Chain; relation E(a:i32,b:i32); answer(a,f) :- E(a,b),E(b,c),E(c,d),E(d,e),E(e,f);",
        "struct Star; relation E(a:i32,b:i32); answer(a,b,c,d) :- E(a,b),E(a,c),E(a,d);",
        "struct Branches; relation E(a:i32,b:i32); answer(a,c,e,g) :- E(a,b),E(b,c),E(a,d),E(d,e),E(a,f),E(f,g);",
        "struct Composite; relation R(a:i32,b:i32,c:i32); relation S(a:i32,b:i32,d:i32); answer(c,d) :- R(a,b,c),S(a,b,d);",
        "struct Disconnected; relation E(a:i32,b:i32); answer(a,d) :- E(a,b),E(c,d);",
        "struct Repeated; relation E(a:i32,b:i32); answer(a,b) :- E(a,b),E(a,b);",
    ];
    for text in cases {
        let source: cq::Module = syn::parse_str(text).unwrap();
        let (target, report) = explain_cq_graph(
            &source,
            PlanningOptions {
                strategy: JoinPolicy::Auto,
                max_arity: None,
            },
        )
        .expect("YA scheduling exercise must emit complete legal reductions");
        let tree = report.join_tree.expect("every fixture is acyclic");
        let nodes = tree.parent.len();
        let mut up = BTreeSet::new();
        let mut down = BTreeSet::new();
        let mut frozen = false;
        for rule in report.derivation {
            match rule {
                JoinRule::Up { child } => {
                    assert!(!frozen && down.is_empty());
                    assert!(tree.parent[child].is_some());
                    assert!(
                        tree.children(child)
                            .all(|descendant| up.contains(&descendant))
                    );
                    assert!(up.insert(child), "duplicate upward edge");
                }
                JoinRule::Down { child } => {
                    assert!(!frozen && up.len() + 1 == nodes);
                    let parent = tree.parent[child].expect("root has no incoming edge");
                    assert!(tree.parent[parent].is_none() || down.contains(&parent));
                    assert!(down.insert(child), "duplicate downward edge");
                }
                JoinRule::Freeze => {
                    assert!(!frozen && up.len() + 1 == nodes && down.len() + 1 == nodes);
                    frozen = true;
                }
                JoinRule::Combine { .. } => assert!(frozen),
                _ => {}
            }
        }
        assert!(frozen);
        assert_eq!(up, down);
        assert_eq!(
            target
                .plan
                .definitions
                .iter()
                .filter(|d| matches!(d.operator, ra::Operator::Semijoin(_)))
                .count(),
            2 * (nodes - 1)
        );
    }
}

#[test]
fn selected_graph_stage_rejects_expression_scheduling() {
    let source: cq::Module = syn::parse_quote! {
        struct Ordered;
        relation R(a:i32);
        answer(x) :- R(x), let y:i32 = *x + 1, if y > 0;
    };
    assert!(
        explain_cq_graph(
            &source,
            PlanningOptions {
                strategy: JoinPolicy::Auto,
                max_arity: None,
            }
        )
        .is_err()
    );
}
