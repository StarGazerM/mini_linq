//! The composed route retains IndexRequirements and rejects unsupported demands.

use std::collections::BTreeMap;

use cq_compiler::{
    PhysicalOptions, compile_composed, compile_deferred_indexes, compile_materialized,
    compile_result_indexes,
};
use cq_ir::{index_requirements as indexes, relational_plan as ra};

fn source() -> ra::Module {
    syn::parse_quote! {
        struct Deferred;
        relation R(k:i64,x:i64);
        relation S(k:i64,y:i64);
        answer(x,y) :- R(k,x), S(k,y);
        relational {
            r = rename R {k -> k,x -> x};
            s = rename S {k -> k,y -> y};
            reduced = semijoin r with s;
            joined = natural_join reduced with s;
            result = project joined keep {x,y};
            output result as answer(x,y);
        }
    }
}

fn options() -> PhysicalOptions {
    PhysicalOptions {
        storage: syn::parse_quote!(cq_storage),
        access: syn::parse_quote!(application::collect),
        inputs: BTreeMap::from([
            ("R".into(), syn::parse_quote!(application::left_rows)),
            ("S".into(), syn::parse_quote!(application::right_rows)),
        ]),
        materialize: syn::parse_quote!(application::output_rows),
        results: BTreeMap::new(),
        semijoin: syn::parse_quote!(application::reduce),
        semijoins: BTreeMap::new(),
        keys: syn::parse_quote!(application::Keys),
        key_sets: BTreeMap::new(),
        join: syn::parse_quote!(application::join),
        joins: BTreeMap::new(),
    }
}

#[test]
fn deferred_selection_preserves_all_definitions_and_current_versions() {
    let source = source();
    let indexed = compile_deferred_indexes(&source).unwrap();
    assert_eq!(indexed.relational_plan, source);
    assert!(indexed.indexes.is_empty());
    indexes::contract::check(&indexed).unwrap();
    compile_composed(&indexed, &options()).unwrap();
}

#[test]
fn composed_realization_rejects_every_valid_written_requirement() {
    let source = source();
    // These cover a required result key, a valid extra result key and an unused
    // base-relation key. None becomes an optional hint on the composed route.
    for requirement in [
        syn::parse_quote!(@s[0]),
        syn::parse_quote!(@joined[1]),
        syn::parse_quote!(R[1]),
    ] {
        let mut indexed = compile_deferred_indexes(&source).unwrap();
        indexed.indexes.push(requirement);
        indexes::contract::check(&indexed).unwrap();
        let error = compile_composed(&indexed, &options()).unwrap_err();
        assert!(error.to_string().contains("only an empty index overlay"));
    }
}

#[test]
fn canonical_materialization_still_requires_its_exact_result_indexes() {
    let source = source();
    let deferred = compile_deferred_indexes(&source).unwrap();
    let error = compile_materialized(&deferred).unwrap_err();
    assert!(error.to_string().contains("missing required index"));

    let canonical = compile_result_indexes(&source).unwrap();
    assert!(!canonical.indexes.is_empty());
    compile_materialized(&canonical).unwrap();
}

#[test]
fn composed_realization_checks_index_well_formedness_before_its_domain() {
    let mut indexed = compile_deferred_indexes(&source()).unwrap();
    indexed.indexes.push(syn::parse_quote!(@missing[0]));
    let error = compile_composed(&indexed, &options()).unwrap_err();
    assert!(error.to_string().contains("undeclared relation"));
}
