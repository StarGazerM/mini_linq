//! Inspect the exact CQ macro inputs executed by joins.rs, without a second query copy.
use cq_compiler::{JoinPolicy, JoinRule, PlanningOptions, compile_cq_derivation, explain_cq_plan};
use cq_ir::{cq, relational_plan as ra, symbol_name};

pub fn run() {
    use JoinRule::{Bind, Constant, Finish, Guard, Match};
    let file: syn::File = syn::parse_str(include_str!("joins.rs")).unwrap();
    let mut checked = 0;
    for item in file.items {
        let syn::Item::Macro(item) = item else {
            continue;
        };
        if !item.mac.path.is_ident("query") {
            continue;
        }
        let source: cq::Module = syn::parse2(item.mac.tokens).unwrap();
        let name = symbol_name(&source.program.name);
        // Clause numbers are zero-based positions in the written CQ body.
        let expected = match name.as_str() {
            "Constant" => vec![
                Constant {
                    clause: 0,
                    argument: 0,
                },
                Match { clause: 0 },
                Match { clause: 2 },
                Match { clause: 1 },
                Finish,
            ],
            "ConstantLet" => vec![
                Bind { clause: 0 },
                Match { clause: 1 },
                Match { clause: 3 },
                Match { clause: 2 },
                Finish,
            ],
            "DependentLet" => vec![
                Match { clause: 0 },
                Bind { clause: 1 },
                Match { clause: 3 },
                Match { clause: 2 },
                Finish,
            ],
            "GuardBoundary" => vec![
                Match { clause: 0 },
                Match { clause: 1 },
                Guard { clause: 2 },
                Match { clause: 3 },
                Finish,
            ],
            "ArgumentBoundary" => vec![
                Match { clause: 0 },
                Match { clause: 1 },
                Match { clause: 2 },
                Finish,
            ],
            "PureGuard" => vec![
                Match { clause: 0 },
                Guard { clause: 3 },
                Match { clause: 2 },
                Match { clause: 1 },
                Finish,
            ],
            _ => panic!("missing expected schedule for {name}"),
        };
        let (plan, report) = explain_cq_plan(
            &source,
            PlanningOptions {
                strategy: JoinPolicy::Auto,
                max_arity: None,
            },
        )
        .unwrap();
        assert_eq!(report.derivation, expected, "{name}");
        assert_eq!(compile_cq_derivation(&source, &report).unwrap(), plan);
        let order: Vec<_> = plan
            .plan
            .definitions
            .iter()
            .filter_map(|d| match &d.operator {
                ra::Operator::Rename(r) => Some(symbol_name(&r.relation)),
                _ => None,
            })
            .collect();
        let expected_order = if matches!(name.as_str(), "GuardBoundary" | "ArgumentBoundary") {
            ["R", "T", "S"]
        } else {
            ["R", "S", "T"]
        };
        assert_eq!(order, expected_order, "{name}");
        println!(
            "  {name}: RA inputs {order:?}; rules {:?}; peak column arity {}",
            report.derivation, report.peak_arity
        );
        checked += 1;
    }
    assert_eq!(checked, 6);
}
