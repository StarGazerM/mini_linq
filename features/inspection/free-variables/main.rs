//! Inspect parsed Rust syntax, including clauses requiring external expansion.
//! Parsing alone does not make negation or aggregation executable core CQ.
//! Nested union/fallback blocks preserve interfaces, locals and occurrence paths.
//! Inspection walks all arms; execution chooses Set union or ordered first
//! nonempty fallback. Pure, total Rust follows its free-variable binders only.
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

fn inspect(body: &cq::CommaList<BodyItem>, mut bound: std::collections::BTreeSet<String>) {
    for clause in body {
        match clause {
            BodyItem::Block(block) => {
                let facts = block.analyze(&bound);
                println!("block {:?}: {:?}", block.kind, facts);
                for branch in &block.branches {
                    inspect(&branch.body, bound.clone());
                }
                bound.extend(facts.interface.into_keys());
            }
            BodyItem::Positive { atom } => {
                for expression in &atom.args {
                    report("atom", expression);
                }
                bound.extend(atom.variables().map(cq::symbol_name));
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
                bound.extend(
                    cq::rust::bindings(&binding.pattern)
                        .iter()
                        .map(cq::symbol_name),
                );
            }
            BodyItem::For(generator) => {
                report("for iterator", &generator.expression);
                println!("for outputs: {:?}", cq::rust::bindings(&generator.pattern));
                bound.extend(
                    cq::rust::bindings(&generator.pattern)
                        .iter()
                        .map(cq::symbol_name),
                );
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

fn main() {
    inspect(&source().query.body, Default::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_testkit::Rows;
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
    #[allow(non_snake_case)]
    fn inspected_scopes_and_macros_preserve_total_answers_after_their_binders() {
        fn positive(x: &i32) -> bool {
            *x > 0
        }
        let condition: syn::Expr = syn::parse_quote!(positive(x) && !label.is_empty());
        assert_eq!(
            cq::rust::free_variables(&condition).dependencies(&names(&["x", "label"])),
            names(&["x", "label"]),
        );
        for R in [vec![], vec![(0,), (1,), (1,), (2,)], vec![(9,)]] {
            let limit = 0;
            let mut expected = Rows::new();
            for &(x,) in &R {
                let passes = {
                    let threshold = limit;
                    let above = |limit| x > limit;
                    above(threshold)
                };
                if passes {
                    let label = format!("{x}");
                    if positive(&x) && !label.is_empty() {
                        expected.insert(vec![x]);
                    }
                }
            }
            // The final guard needs x from R and label from the macro let;
            // the independent scoped guard adds no written-order dependency.
            let actual = mini_linq::compile! {
                pipeline {
                    plan = mini_linq::passes::plan;
                    ix = mini_linq::passes::index_demands;
                    lower = mini_linq::passes::lower;
                    select = mini_linq::passes::select;
                    print = mini_linq::passes::translate_pull;
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
                    if positive(x) && !label.is_empty();
            }
            .map(|(x,)| vec![*x])
            .collect::<Rows>();
            assert_eq!(actual, expected);
        }
    }
}
