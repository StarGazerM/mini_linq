#![cfg(feature = "hw4")]

use cq_ir::{cq, relational_plan, rust_access_plan};
use quote::{ToTokens, quote};

fn spend_query() -> proc_macro2::TokenStream {
    quote! {
        pub struct SpendProgram;
        relation Person(c0: i32);
        relation Blocked(c0: i32, c1: i32);
        relation Purchase(c0: i32, c1: i32, c2: i32);
        spend(person, total) :-
            Person(person),
            !Blocked(person, _),
            agg total = sum(amount) in Purchase(person, _, amount).
    }
}

#[test]
fn ascent_style_body_items_parse_round_trip_and_are_well_formed() {
    let first: cq::Module = syn::parse2(spend_query())
        .expect("OPTIONAL HW4: add nominal negation and aggregate body-item syntax");
    cq::contract::check(&first)
        .expect("OPTIONAL HW4: implement the extended left-to-right CQ contract");

    let second: cq::Module = syn::parse2(first.to_token_stream()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn unsafe_negation_and_aggregation_are_rejected() {
    let unsafe_negation: cq::Module = syn::parse2(quote! {
        struct UnsafeNegation;
        relation Person(c0: i32);
        relation Blocked(c0: i32, c1: i32);
        answer(person) :- !Blocked(person, _), Person(person).
    })
    .expect("OPTIONAL HW4: negation syntax should parse before its contract is checked");
    assert!(cq::contract::check(&unsafe_negation).is_err());

    let unsafe_correlation: cq::Module = syn::parse2(quote! {
        struct UnsafeAggregate;
        relation Person(c0: i32);
        relation Purchase(c0: i32, c1: i32, c2: i32);
        answer(person, total) :-
            Person(person),
            agg total = sum(amount) in Purchase(other, _, amount).
    })
    .expect("OPTIONAL HW4: aggregate syntax should parse before its contract is checked");
    assert!(cq::contract::check(&unsafe_correlation).is_err());

    let shadowed_result: cq::Module = syn::parse2(quote! {
        struct ShadowedAggregate;
        relation Person(c0: i32);
        relation Purchase(c0: i32, c1: i32, c2: i32);
        answer(person) :-
            Person(person),
            agg person = sum(amount) in Purchase(person, _, amount).
    })
    .expect("OPTIONAL HW4: aggregate syntax should parse before its contract is checked");
    assert!(cq::contract::check(&shadowed_result).is_err());

    let invalid_aggregates = [
        quote! {
            struct MissingArgument;
            relation Person(c0: i32);
            relation Purchase(c0: i32, c1: i32, c2: i32);
            answer(person, total) :-
                Person(person),
                agg total = sum() in Purchase(person, _, amount).
        },
        quote! {
            struct UnsupportedAggregator;
            relation Person(c0: i32);
            relation Purchase(c0: i32, c1: i32, c2: i32);
            answer(person, total) :-
                Person(person),
                agg total = count(amount) in Purchase(person, _, amount).
        },
        quote! {
            struct QualifiedSumPath;
            relation Person(c0: i32);
            relation Purchase(c0: i32, c1: i32, c2: i32);
            answer(person, total) :-
                Person(person),
                agg total = some::sum(amount) in Purchase(person, _, amount).
        },
        quote! {
            struct MissingValueTerm;
            relation Person(c0: i32);
            relation Purchase(c0: i32, c1: i32, c2: i32);
            answer(person, total) :-
                Person(person),
                agg total = sum(amount) in Purchase(person, _, _).
        },
        quote! {
            struct RepeatedValueTerm;
            relation Person(c0: i32);
            relation Purchase(c0: i32, c1: i32, c2: i32);
            answer(person, total) :-
                Person(person),
                agg total = sum(amount) in Purchase(person, amount, amount).
        },
        quote! {
            struct AlreadyBoundValue;
            relation Person(c0: i32);
            relation Amount(c0: i32);
            relation Purchase(c0: i32, c1: i32, c2: i32);
            answer(person, total) :-
                Person(person),
                Amount(amount),
                agg total = sum(amount) in Purchase(person, _, amount).
        },
        quote! {
            struct SameResultAndValue;
            relation Person(c0: i32);
            relation Purchase(c0: i32, c1: i32, c2: i32);
            answer(person, amount) :-
                Person(person),
                agg amount = sum(amount) in Purchase(person, _, amount).
        },
        quote! {
            struct EscapingLocalValue;
            relation Person(c0: i32);
            relation Purchase(c0: i32, c1: i32, c2: i32);
            answer(person, amount) :-
                Person(person),
                agg total = sum(amount) in Purchase(person, _, amount).
        },
    ];

    for tokens in invalid_aggregates {
        let module: cq::Module = syn::parse2(tokens)
            .expect("HW4 aggregate syntax should parse before its contract is checked");
        assert!(cq::contract::check(&module).is_err());
    }
}

#[test]
fn extended_relational_plans_use_renames_inferred_correlation_and_one_global_unit() {
    let plans = [
        quote! {
            pub struct SpendProgram;
            relation Person(c0: i32);
            relation Blocked(c0: i32, c1: i32);
            relation Purchase(c0: i32, c1: i32, c2: i32);
            spend(person, total) :-
                Person(person),
                !Blocked(person, _),
                agg total = sum(amount) in Purchase(person, _, amount).
            relational {
                r0 = rename Person {c0 -> person};
                r1 = rename Blocked {c0 -> person, c1 -> blocked_anon0};
                r2 = antisemijoin r0 with r1;
                r3 = rename Purchase {
                    c0 -> person,
                    c1 -> purchase_anon0,
                    c2 -> amount,
                };
                r4 = aggregate_apply r2 with r3 value amount using sum into total;
                r5 = project r4 keep {person, total};
                output r5 as spend(person, total).
            }
        },
        quote! {
            pub struct GlobalSummaryProgram;
            relation Blocked(c0: i32);
            relation Purchase(c0: i32, c1: i32);
            summary(total) :-
                !Blocked(_),
                agg total = sum(amount) in Purchase(_, amount).
            relational {
                r0 = unit;
                r1 = rename Blocked {c0 -> blocked_anon0};
                r2 = antisemijoin r0 with r1;
                r3 = rename Purchase {c0 -> purchase_anon0, c1 -> amount};
                r4 = aggregate_apply r2 with r3 value amount using sum into total;
                r5 = project r4 keep {total};
                output r5 as summary(total).
            }
        },
    ];

    for tokens in plans {
        let first: relational_plan::Module = syn::parse2(tokens)
            .expect("OPTIONAL HW4: add AntiSemijoin and AggregateApply to RelationalPlan");
        relational_plan::contract::check(&first)
            .expect("OPTIONAL HW4: check inferred correlations, headings, and global Unit");
        let second: relational_plan::Module = syn::parse2(first.to_token_stream()).unwrap();
        assert_eq!(first, second);
    }
}

#[test]
fn extended_body_items_keep_their_occurrence_labels_until_pull_lowering() {
    let plan: rust_access_plan::Plan = syn::parse2(quote! {
        spend(person, total) => ((person.clone(), total,)) :-
            Person(person) => for (person,) in (rows0.iter()),
            !Blocked(person, _) => if (!index0.contains_key(&(person.clone(),))),
            agg total = sum(amount) in Purchase(person, _, amount) =>
                for total in (aggregate_totals(*person)).
    })
    .expect("OPTIONAL HW4: RustAccessPlan must parse the extended BodyItem variants");

    rust_access_plan::contract::check(&plan)
        .expect("OPTIONAL HW4: annotate negation with If and aggregation with For");
}
