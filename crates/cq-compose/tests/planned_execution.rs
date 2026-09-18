//! Production acceptance: these tests must invoke the planner and Rust backend.
#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[path = "support/compiled.rs"]
mod compiled;
#[path = "planned_execution/derivations.rs"]
mod contracts;
#[allow(dead_code)]
#[path = "../example-support/planned_boundaries.rs"]
mod documented;
#[path = "support/materialized.rs"]
mod materialized;
#[path = "support/rust.rs"]
mod rust;
use materialized::assert_physical_widths;
#[path = "../../../test-support/relational/databases.rs"]
mod databases;
use compiled::compile_ra;
use databases::binary_database;
use rust::{check_rust, run_rust};
#[allow(dead_code)]
#[path = "../../../test-support/relational/join_trees.rs"]
mod join_trees;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;

use cq_compose::check_cq_plan;
use cq_compose::explain_cq_plan;
use cq_ir::{cq, relational_plan as ra};
use cq_query_lowering::JoinPolicy as Strategy;
use cq_query_lowering::PlanningOptions as Options;
use quote::{ToTokens, format_ident, quote};

#[test]
fn documented_complete_boundaries_match_the_typed_passes() {
    for (name, file) in documented::artifacts() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../doc/planned-example")
            .join(name);
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            prettyplease::unparse(&file),
            "{name}"
        );
    }
    let (_, rust) = documented::artifacts().pop().unwrap();
    run_rust(quote! { #rust fn main() {
        assert_eq!(PlannedExample::run([(1,2),(9,7),(1,2)],[(2,3),(7,-1)]),vec![(1,)]);
    }});
}

#[test]
fn all_classic_patterns_use_the_production_planner_and_compiled_backend() {
    let mut modules = Vec::new();
    let mut checks = Vec::new();
    for case in cases::CASES {
        let known: ra::Module = syn::parse_str(case.plan).unwrap();
        let mut source = cq::Module {
            program: known.program.clone(),
        };
        source.program.visibility = syn::parse_quote!(pub);
        let mut databases = vec![database(case)];
        databases.extend((0..64).map(|seed| binary_database(&source.program, seed)));
        assert_eq!(
            oracle::evaluate_cq(&source.program, &databases[0]),
            case.expected.iter().map(|row| row.to_vec()).collect()
        );
        let parameters: Vec<_> = (0..source.program.inputs().count())
            .map(|i| format_ident!("input_{i}"))
            .collect();
        let datasets: Vec<_> = databases
            .iter()
            .map(|database| {
                let inputs = source
                    .program
                    .inputs()
                    .map(|input| rows(&database[&cq_ir::symbol_name(&input.name)]));
                let expected = rows(&oracle::evaluate_cq(&source.program, database));
                quote! { (#(#inputs,)* #expected) }
            })
            .collect();
        let mut calls = Vec::new();
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
            check_cq_plan(&source, &ra, &planned).unwrap();
            assert_eq!(
                planned.join_tree.is_some(),
                strategy == Strategy::Auto && join_trees::has_join_tree(&source.program)
            );
            // Check logical results, including the frozen family, separately
            // from the executable output and its actual tuple layout.
            for database in &databases {
                let evaluation = oracle::evaluate_plan(source.program.inputs(), &ra.plan, database);
                assert_eq!(
                    evaluation.output,
                    oracle::evaluate_cq(&source.program, database)
                );
                if planned.join_tree.is_some() {
                    assert_full_reduction(&source, &ra, database, &evaluation);
                }
            }
            let module = format_ident!("{}_{}", case.name, modules.len());
            let program = &source.program.name;
            let rust = compile_ra(&ra).unwrap();
            assert_physical_widths(&ra, &rust, &databases[0]);
            let widths = ra::check_in(source.program.inputs(), &ra.plan)
                .unwrap()
                .into_iter()
                .map(|(name, info)| {
                    let width = info.heading.len();
                    quote! { (#name, #width) }
                })
                .collect::<Vec<_>>();
            modules.push(quote! { mod #module { #rust } });
            calls.push(quote! {
                let (actual, stats) = #module::#program::run_with_stats(#(#parameters.clone()),*);
                assert_eq!(actual, expected.iter().copied().collect::<Vec<_>>(), stringify!(#module));
                let widths: std::collections::BTreeMap<_, _> = [#(#widths),*].into_iter().collect();
                assert_eq!(stats.results.len(), widths.len());
                for (name, width, _) in stats.results { assert_eq!(width, widths[name]); }
            });
        }
        // Compile independently authored RA without invoking the CQ planner.
        let mut direct = known.clone();
        direct.program.visibility = syn::parse_quote!(pub);
        let rust = compile_ra(&direct).unwrap();
        let module = format_ident!("{}_known", case.name);
        let program = &source.program.name;
        modules.push(quote! { mod #module { #rust } });
        calls.push(quote! { assert_eq!(#module::#program::run(#(#parameters.clone()),*), expected.iter().copied().collect::<Vec<_>>()); });
        checks.push(quote! { for (#(#parameters,)* expected) in [#(#datasets),*] { #(#calls)* } });
    }
    run_rust(quote! { #(#modules)* fn main() { #(#checks)* } });
}

fn assert_full_reduction(
    source: &cq::Module,
    ra: &ra::Module,
    database: &oracle::Database,
    evaluation: &oracle::Evaluation,
) {
    use cq_ir::symbol_name;
    let mut latest = std::collections::BTreeMap::new();
    let mut owners = std::collections::BTreeMap::new();
    let mut occurrence = 0;
    // All these fixtures have positive occurrences and no predicates.
    // Semijoins end at freeze, before any assembly joins or projections.
    for definition in &ra.plan.definitions {
        let name = symbol_name(&definition.result);
        match &definition.operator {
            ra::Operator::Rename(_) => {
                owners.insert(name.clone(), occurrence);
                latest.insert(occurrence, name);
                occurrence += 1;
            }
            ra::Operator::Semijoin(join) => {
                let owner = owners[&symbol_name(&join.left)];
                owners.insert(name.clone(), owner);
                latest.insert(owner, name);
            }
            _ => break,
        }
    }
    let valuations = oracle::valuations(&source.program, database);
    for (i, heading) in source
        .program
        .query
        .body
        .iter()
        .filter_map(cq::BodyItem::atom)
        .map(|a| {
            a.variables()
                .map(cq_ir::symbol_name)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .collect::<Vec<_>>()
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            evaluation.definitions[&latest[&i]],
            valuations.project(heading),
            "frozen occurrence {i}"
        );
    }
}

#[test]
fn selections_are_pushed_down_and_predicate_only_columns_are_pruned() {
    let source: cq::Module = syn::parse_str(
        "pub struct Filtered;
        relation R(x: i32, y: i32); relation S(y: i32, z: i32);
        answer(x) :- R(x,y), S(y,z), if (x > 0), if (x < z);",
    )
    .unwrap();
    let (ra, planned) = explain_cq_plan(
        &source,
        Options {
            strategy: Strategy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    check_cq_plan(&source, &ra, &planned).unwrap();
    let rust = compile_ra(&ra).unwrap();
    run_rust(quote! {
        #rust
        fn main() {
            assert_eq!(Filtered::run(vec![(1,2),(4,2),(-1,2)], vec![(2,3)]),
                vec![(1,)]);
        }
    });
}

#[test]
fn all_comparisons_and_extreme_literals_match_independent_valuations() {
    let mut modules = Vec::new();
    let mut checks = Vec::new();
    for (i, predicate) in [
        "x == 0",
        "x != 0",
        "x < -1",
        "x <= 0",
        "x > 0",
        "x >= 1",
        "-2147483648 <= x",
        "x <= 2147483647",
        "0 == 1",
    ]
    .into_iter()
    .enumerate()
    {
        let source: cq::Module = syn::parse_str(&format!(
            "pub struct Compare; relation R(v:i32); answer(x) :- R(x), if ({predicate});"
        ))
        .unwrap();
        let database = std::collections::BTreeMap::from([(
            "R".into(),
            [-2, -1, 0, 1, 2, i32::MIN, i32::MAX]
                .into_iter()
                .map(|x| vec![x])
                .collect(),
        )]);
        let expected = rows(&oracle::evaluate_cq(&source.program, &database));
        let input = rows(&database["R"]);
        let ra = cq_compose::compile_cq_planned(&source).unwrap();
        let rust = compile_ra(&ra).unwrap();
        let m = format_ident!("compare{i}");
        modules.push(quote!(mod #m { #rust }));
        checks.push(quote!(assert_eq!(#m::Compare::run(#input),#expected);));
    }
    run_rust(quote!(#(#modules)* fn main() { #(#checks)* }));
}

#[test]
fn rust_rejects_non_i32_predicates_without_a_second_logical_type_system() {
    let source: cq::Module =
        syn::parse_str("pub struct Invalid; relation R(x:String); answer(x) :- R(x), if (x > 0);")
            .unwrap();
    cq::contract::check(&source).unwrap();
    let ra = cq_compose::compile_cq_planned(&source).unwrap();
    let rust = compile_ra(&ra).unwrap();
    check_rust(quote!(#rust fn main() {}), Some("mismatched types"));
}

#[test]
fn directly_quoted_ra_preserves_shared_operands_without_replanning() {
    let direct: ra::Module = syn::parse_str("pub struct Direct;
        relation R(a: i32, b: i32); relation S(b: i32, c: i32); relation T(b: i32, d: i32);
        answer(a,c,d) :- R(a,b), S(c,b), T(d,b);
        relational {
            r = rename R {a -> a, b -> b}; s = rename S {b -> b, c -> c}; t = rename T {b -> b, d -> d};
            rs = natural_join r with s; rt = natural_join r with t;
            result = natural_join rs with rt; output result as answer(a,c,d);
        }").unwrap();
    // Require explicit output projection: local checking must not invent one.
    assert!(ra::contract::check(&direct).is_err());
    let text = direct
        .to_token_stream()
        .to_string()
        .replace("output result", "p = project result keep {a,c,d}; output p");
    let direct: ra::Module = syn::parse_str(&text).unwrap();
    let rust = compile_ra(&direct).unwrap();
    run_rust(quote! { #rust fn main() {
        assert_eq!(Direct::run(vec![(1,2)],vec![(2,3)],vec![(2,4)]), vec![(1,3,4)]);
    }});
}

fn database(case: &cases::Case) -> oracle::Database {
    case.inputs
        .iter()
        .map(|(name, rows)| {
            (
                name.to_string(),
                rows.iter().map(|row| row.to_vec()).collect(),
            )
        })
        .collect()
}

fn rows(rows: &oracle::Rows) -> proc_macro2::TokenStream {
    let tuples = rows.iter().map(|row| quote! { (#(#row,)*) });
    quote! { vec![#(#tuples),*] }
}
