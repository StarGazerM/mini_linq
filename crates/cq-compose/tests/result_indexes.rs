//! Named-result index requirements and their materialized realization.
#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "support/compiled.rs"]
mod compiled;
#[path = "../../../test-support/relational/databases.rs"]
mod databases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;
#[path = "support/rust.rs"]
mod rust;
use compiled::compile_indexed;
use cq_ir::{cq, relational_plan as ra};
use cq_ir::{index_requirements as ir, symbol_name};
use cq_plan::JoinPolicy as Strategy;
use databases::binary_database;
use quote::{ToTokens, quote};
use rust::run_rust;

fn query() -> cq::Module {
    syn::parse_str(
        "pub struct Indexed; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        answer(a,c) :- R(a,b), S(b,c);",
    )
    .unwrap()
}

#[test]
fn indexes_preserve_algebra_and_name_complete_keys_of_actual_result_versions() {
    for case in cases::CASES {
        let known: ra::Module = syn::parse_str(case.plan).unwrap();
        let source = cq::Module {
            program: known.program,
        };
        let planned = cq_compose::compile_cq_planned(&source).unwrap();
        let indexed = cq_index::result::compile(&planned).unwrap();
        assert_eq!(indexed.relational_plan, planned);
        let interpreted = oracle::evaluate_ra(&planned, &binary_database(&source.program, 0));
        let mut seen = std::collections::BTreeSet::new();
        let mut expected = Vec::new();
        for d in &planned.plan.definitions {
            let (l, r) = match &d.operator {
                ra::Operator::NaturalJoin(j) => (&j.left, &j.right),
                ra::Operator::Semijoin(j) => (&j.left, &j.right),
                _ => continue,
            };
            let lh = &interpreted.definitions[&symbol_name(l)].heading;
            let rh = &interpreted.definitions[&symbol_name(r)].heading;
            let key: Vec<_> = rh
                .iter()
                .enumerate()
                .filter_map(|(i, x)| lh.contains(x).then_some(i))
                .collect();
            if !key.is_empty() && seen.insert((symbol_name(r), key.clone())) {
                expected.push((symbol_name(r), key));
            }
        }
        let actual = indexed
            .indexes
            .iter()
            .map(|k| {
                assert!(k.result_token.is_some());
                (
                    symbol_name(&k.relation),
                    k.key_columns
                        .iter()
                        .map(|c| c.index as usize)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        let roundtrip: ir::Module = syn::parse2(indexed.to_token_stream()).unwrap();
        assert_eq!(indexed, roundtrip);
    }
}

#[test]
fn missing_wrong_version_and_incomplete_keys_are_controlled_errors() {
    let source = query();
    let ra = cq_compose::compile_cq_planned(&source).unwrap();
    let indexed = cq_index::result::compile(&ra).unwrap();
    for i in 0..indexed.indexes.len() {
        let mut missing = indexed.clone();
        missing.indexes = missing
            .indexes
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, v)| v.clone())
            .collect();
        ir::contract::check(&missing).unwrap();
        assert!(
            cq_realize::hash_materialized::compile_materialized(
                &missing,
                &syn::parse_quote!(::mini_linq::pull_set)
            )
            .is_err()
        );
    }
    let mut wrong = indexed.clone();
    let old = wrong
        .indexes
        .first()
        .unwrap()
        .key_columns
        .first()
        .unwrap()
        .index;
    let changed = syn::Index::from(((old + 1) % 2) as usize);
    wrong.indexes.first_mut().unwrap().key_columns = std::iter::once(changed).collect();
    ir::contract::check(&wrong).unwrap();
    assert!(
        cq_realize::hash_materialized::compile_materialized(
            &wrong,
            &syn::parse_quote!(::mini_linq::pull_set)
        )
        .is_err()
    );
    let mut wrong = indexed.clone();
    wrong.indexes.first_mut().unwrap().relation = ra.plan.output.input.clone();
    ir::contract::check(&wrong).unwrap();
    assert!(
        cq_realize::hash_materialized::compile_materialized(
            &wrong,
            &syn::parse_quote!(::mini_linq::pull_set)
        )
        .is_err()
    );
    let mut duplicate = indexed.clone();
    duplicate
        .indexes
        .push(indexed.indexes.first().unwrap().clone());
    assert!(ir::contract::check(&duplicate).is_err());
    let mut out_of_range = indexed.clone();
    out_of_range.indexes.first_mut().unwrap().key_columns = syn::parse_quote!(9);
    assert!(ir::contract::check(&out_of_range).is_err());
}

#[test]
fn base_and_result_namespaces_are_distinct_and_input_route_rejects_result_indexes() {
    let source = query();
    let canonical = cq_compose::compile_cq(&source).unwrap();
    let indexed = cq_index::result::compile(&canonical).unwrap();
    ir::contract::check(&indexed).unwrap();
    assert!(
        cq_realize::hash_inputs::compile_input_indexed(
            &indexed,
            &syn::parse_quote!(::mini_linq::pull)
        )
        .is_err()
    );
    let mut base = indexed.clone();
    let k = base.indexes.first_mut().unwrap();
    k.result_token = None;
    k.relation = syn::parse_quote!(S);
    ir::contract::check(&base).unwrap();
    assert!(
        cq_realize::hash_materialized::compile_materialized(
            &base,
            &syn::parse_quote!(::mini_linq::pull_set)
        )
        .is_err()
    );

    let collision: ir::Module = syn::parse_quote! {
        pub struct Collision;
        relation r(a:i32,b:i32);
        answer(a) :- r(a,b);
        relational {
            r=rename r {a->b,b->a};
            out=project r keep {a};
            output out as answer(a);
        }
        indexes {r[0]; @r[0];}
    };
    let rust = compile_indexed(&collision).unwrap();
    run_rust(quote! {#rust fn main() {
        let (rows,stats)=Collision::run_with_stats([(1,20),(2,30)]);
        assert_eq!(rows,vec![(20,),(30,)]);
        assert_eq!(stats.index_rows,4);
    }});
}

#[test]
fn valid_extra_requirements_are_built_without_changing_the_plan() {
    let source = query();
    let ra = cq_compose::compile_cq_with_policy(&source, Strategy::Written).unwrap();
    let mut indexed = cq_index::result::compile(&ra).unwrap();
    indexed.indexes.push(syn::parse_quote!(R[0]));
    indexed.indexes.push(syn::parse_quote!(@r0[1]));
    ir::contract::check(&indexed).unwrap();
    let staged = cq_realize::hash_materialized::compile_materialized(
        &indexed,
        &syn::parse_quote!(::mini_linq::pull_set),
    )
    .unwrap();
    // The written extras are real concrete builders, not ignored annotations.
    let text = prettyplease::unparse(&staged);
    assert!(text.contains("let index1"));
    assert!(text.contains("let index2"));
    assert_eq!(indexed.relational_plan, ra);
    let rust = compile_indexed(&indexed).unwrap();
    run_rust(quote! {#rust fn main() {
        let (rows,stats)=Indexed::run_with_stats([(1,10),(2,20)],[(10,3),(20,4)]);
        assert_eq!(rows,vec![(1,3),(2,4)]);
        // The required right-result index and both written extras each visit
        // exactly two normalized rows, including the otherwise unused keys.
        assert_eq!(stats.index_rows,6);
    }});
}
