//! Inspect parsed Rust syntax, including clauses requiring external expansion.
//! Parsing alone does not make negation or aggregation executable core CQ.
#![recursion_limit = "512"]

use cq_lang_cq as cq;
use cq_lang_cq::BodyItem;

fn report(position: &str, expression: &syn::Expr) {
    let free = cq::rust::free_variables(expression);
    // Illustrative candidates only; the student's CQ analysis supplies the
    // actual environment at each clause and accounts for aggregate-local names.
    let environment = ["x", "limit", "config", "amount"].map(String::from).into();
    println!(
        "{position}: names={:?}, opaque={}, logical inputs={:?}",
        free.names,
        free.opaque,
        free.dependencies(&environment)
    );
}

fn source() -> cq::Program {
    syn::parse_quote! {
        struct Inspect; relation R(value:i32);
        answer(x) :- R(transform(x)),
            if { let threshold = limit; x > threshold },
            let label: String = format!("{x}"),
            !R(offset(x)),
            agg total = (factory(config))(amount) in R(amount);
    }
}

fn main() {
    let source = source();
    for clause in &source.query.body {
        match clause {
            BodyItem::Positive { atom } => {
                for expression in &atom.args {
                    report("atom", expression);
                }
            }
            BodyItem::Constraint(c) => {
                println!(
                    "relational constraint inputs: {:?}; parameters: {:?}",
                    c.variables().collect::<Vec<_>>(),
                    c.parameters().collect::<Vec<_>>()
                );
            }
            BodyItem::Equality(e) => println!(
                "relational equality inputs: {:?}",
                e.variables().collect::<Vec<_>>()
            ),
            BodyItem::Filter(filter) => report("if", &filter.condition),
            BodyItem::Let(binding) => {
                report("let initializer", &binding.expression);
                println!("let outputs: {:?}", cq::rust::bindings(&binding.pattern));
            }
            BodyItem::Negation(negation) => {
                for expression in &negation.atom.args {
                    report("negated atom", expression);
                }
            }
            BodyItem::Extension { name, .. } => println!("unexpanded {name}!"),
            BodyItem::Aggregate(aggregate) => {
                if let cq::Aggregator::Expression { expression, .. } = &aggregate.aggregator {
                    report("aggregator", expression);
                }
                for expression in &aggregate.atom.args {
                    report("aggregate input", expression);
                }
                println!(
                    "aggregate outputs: {:?}",
                    cq::rust::bindings(&aggregate.pattern)
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_testkit::{Rows, harness};
    use std::collections::BTreeSet;

    fn names(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn original_surface_retains_every_demo_and_is_not_a_contracted_core_query() {
        let source = source();
        assert_eq!(source.query.body.len(), 5);
        let BodyItem::Positive { atom } = &source.query.body[0] else {
            panic!("computed atom demonstration");
        };
        assert_eq!(
            cq::rust::free_variables(&atom.args[0]).names,
            names(&["transform", "x"])
        );
        let BodyItem::Filter(filter) = &source.query.body[1] else {
            panic!("scoped block demonstration");
        };
        let free = cq::rust::free_variables(&filter.condition);
        assert_eq!(free.names, names(&["limit", "x"]));
        assert!(!free.opaque);
        let BodyItem::Let(binding) = &source.query.body[2] else {
            panic!("macro and binding demonstration");
        };
        assert!(cq::rust::free_variables(&binding.expression).opaque);
        assert_eq!(
            cq::rust::bindings(&binding.pattern)
                .iter()
                .map(cq::symbol_name)
                .collect::<Vec<_>>(),
            ["label"]
        );
        let BodyItem::Negation(negation) = &source.query.body[3] else {
            panic!("negated expression demonstration");
        };
        assert_eq!(
            cq::rust::free_variables(&negation.atom.args[0]).names,
            names(&["offset", "x"])
        );
        let BodyItem::Aggregate(aggregate) = &source.query.body[4] else {
            panic!("aggregate demonstration");
        };
        let cq::Aggregator::Expression { expression, .. } = &aggregate.aggregator else {
            panic!("computed aggregator");
        };
        assert_eq!(
            cq::rust::free_variables(expression).names,
            names(&["factory", "config"])
        );
        assert_eq!(
            aggregate
                .arguments
                .iter()
                .map(cq::symbol_name)
                .collect::<Vec<_>>(),
            ["amount"]
        );
        assert_eq!(
            cq::rust::free_variables(&aggregate.atom.args[0]).names,
            names(&["amount"])
        );
        assert_eq!(
            cq::rust::bindings(&aggregate.pattern)
                .iter()
                .map(cq::symbol_name)
                .collect::<Vec<_>>(),
            ["total"]
        );
        assert!(cq::contract::check(&source).is_err());
    }

    #[test]
    fn lexical_scope_and_macro_opacity_keep_dependencies_conservative() {
        let expression: syn::Expr = syn::parse_quote! {
            { let threshold = limit; (|limit| *x > limit)(threshold) }
        };
        let free = cq::rust::free_variables(&expression);
        assert_eq!(free.names, names(&["limit", "x"]));
        assert!(!free.opaque);
        let environment = names(&["x", "y", "limit", "config"]);
        assert_eq!(free.dependencies(&environment), names(&["x", "limit"]));
        let hidden: syn::Expr = syn::parse_quote!(format!("{x}"));
        let hidden = cq::rust::free_variables(&hidden);
        assert!(hidden.opaque);
        assert_eq!(hidden.dependencies(&environment), environment);
        let pattern: syn::Pat = syn::parse_quote!((ref x, (y, r#label)));
        assert_eq!(
            cq::rust::bindings(&pattern)
                .iter()
                .map(cq::symbol_name)
                .collect::<Vec<_>>(),
            ["x", "y", "label"]
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn inspected_scopes_and_macros_execute_with_written_order_refinement() {
        fn partial(x: &i32) -> bool {
            assert_ne!(*x, 9);
            *x > 0
        }
        for R in [vec![], vec![(0,), (1,), (1,), (2,)], vec![(9,)]] {
            let limit = 0;
            harness::check_refinement(
                || {
                    let mut answer = Rows::new();
                    for &(x,) in &R {
                        let passes = {
                            let threshold = limit;
                            let above = |limit| x > limit;
                            above(threshold)
                        };
                        if passes {
                            let label = format!("{x}");
                            if partial(&x) && !label.is_empty() {
                                answer.insert(vec![x]);
                            }
                        }
                    }
                    answer
                },
                || {
                    mini_linq::compile! {
                        pipeline {
                            plan = mini_linq::passes::plan_written;
                            ix = mini_linq::passes::index_demands;
                            lower = mini_linq::passes::lower;
                            select = mini_linq::passes::select;
                            print = mini_linq::passes::print_pull;
                        }
                        struct Scoped;
                        relation R(value: i32);
                        answer(x) :- R(x),
                            if {
                                let threshold = limit;
                                let above = |limit| *x > limit;
                                above(threshold)
                            },
                            let label: String = format!("{x}"),
                            if partial(x) && !label.is_empty();
                    }
                    .map(|(x,)| vec![*x])
                    .collect::<Rows>()
                },
            )
            .unwrap();
        }
    }
}
