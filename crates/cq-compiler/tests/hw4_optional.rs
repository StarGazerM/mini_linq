#![cfg(feature = "hw4")]

use cq_compiler::{compile_cq, compile_pull, compile_relational_plan};
use cq_ir::{cq, index_requirements, iterator_pipeline, relational_plan, rust_access_plan};
use quote::quote;

#[test]
fn ascent_body_items_derive_only_the_correlated_index_columns() {
    assert_indexes(
        quote! {
        pub struct SpendProgram;
        relation Person(c0: i32);
        relation Blocked(c0: i32, c1: i32);
        relation Purchase(c0: i32, c1: i32, c2: i32);
        spend(person, total) :-
            Person(person),
            !Blocked(person, _),
            agg total = sum(amount) in Purchase(person, _, amount).
        },
        &[("Blocked", &[0]), ("Purchase", &[0])],
    );
}

#[test]
fn empty_and_full_keys_keep_their_existing_index_requirement_meaning() {
    assert_indexes(
        quote! {
            struct FullKeyNegation;
            relation Person(c0: i32);
            relation Blocked(c0: i32);
            allowed(person) :- Person(person), !Blocked(person).
        },
        &[("Blocked", &[0])],
    );

    assert_indexes(
        quote! {
            struct EmptyKeys;
            relation Blocker(c0: i32);
            relation Values(c0: i32, c1: i32);
            global(total) :-
                !Blocker(_),
                agg total = sum(value) in Values(_, value).
        },
        &[],
    );
}

#[test]
fn leading_global_negation_uses_unit_before_filter_and_join() {
    let source: rust_access_plan::Plan = syn::parse2(quote! {
        global(total) => ((total,)) :-
            !Blocker(_) => if (!index0.iter().next().is_some()),
            agg total = sum(value) in Values(_, value) =>
                for total in (aggregate_total()).
    })
    .expect("OPTIONAL HW4: parse negation and aggregation in RustAccessPlan");
    rust_access_plan::contract::check(&source)
        .expect("OPTIONAL HW4: accept a safe global negation followed by aggregation");

    let actual = compile_pull(&source);
    let expected: iterator_pipeline::Pipeline = syn::parse2(quote! {
        iter0 = unit yield ();
        iter1 = filter iter0 as () if (!index0.iter().next().is_some());
        iter2 = join iter1 as () with total in (aggregate_total()) yield (total,);
        iter3 = project iter2 as (total,) yield ((total,));
        iter4 = distinct iter3;
        return iter4.
    })
    .unwrap();

    assert_eq!(actual, expected);
}

fn assert_indexes(source: proc_macro2::TokenStream, expected: &[(&str, &[usize])]) {
    let source: cq::Module = syn::parse2(source)
        .expect("OPTIONAL HW4: add nominal negation and aggregate body-item syntax");
    cq::contract::check(&source)
        .expect("OPTIONAL HW4: implement the extended left-to-right CQ contract");
    let logical = compile_cq(&source)
        .expect("OPTIONAL HW4: lower extended CQ body items to named relational operators");
    relational_plan::contract::check(&logical)
        .expect("OPTIONAL HW4: make the extended RelationalPlan locally well formed");
    let target = compile_relational_plan(&logical)
        .expect("OPTIONAL HW4: derive negation and aggregate correlation indexes");
    index_requirements::contract::check(&target).unwrap();
    let signatures = target
        .indexes
        .iter()
        .map(|index| {
            (
                cq_ir::symbol_name(&index.relation),
                index
                    .key_columns
                    .iter()
                    .map(|column| column.index as usize)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();

    let expected = expected
        .iter()
        .map(|(relation, columns)| ((*relation).to_owned(), columns.to_vec()))
        .collect::<Vec<_>>();
    assert_eq!(signatures, expected);
}
