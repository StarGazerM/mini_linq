//! IX → Stream: the kernel's output equals independently authored Stream
//! programs, and denotes the RA oracle on every shared relational case.
#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;
#[allow(dead_code)]
#[path = "../../../test-support/relational/stream_semantics.rs"]
mod stream_oracle;

use cq_ir::{index_requirements as ix, relational_plan as ra, stream};
use cq_stream_lowering::{Policy, check, compile};
use quote::{ToTokens, quote};
use stream::Realization;

fn parse_policy(tokens: proc_macro2::TokenStream) -> Policy {
    syn::parse2(tokens).unwrap()
}

fn chain_ix() -> ix::Module {
    let source: ra::Module = syn::parse_quote! {
        struct Chain;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(a, b), S(b, c);
        relational {
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            p = project j1 keep {a, c};
            output p as answer(a, c);
        }
    };
    cq_index_policy::compile(&source).unwrap()
}

#[test]
fn seek_policy_realizes_the_authored_hash_chain() {
    let source = chain_ix();
    let policy = parse_policy(quote! {
        policy { L provides { iter }; R provides { iter }; S provides { iter }; }
    });
    let target = compile(&source, &policy).unwrap();
    let expected: stream::Module = syn::parse_quote! {
        #source
        stream {
            open L provides { iter };
            open R provides { iter };
            open S provides { iter };
            l = alias L;
            r = let provides { seek @r[0] } for (a, b,) in scan R { emit (a, b,) };
            s = let provides { seek @s[0] } for (b, c,) in scan S { emit (b, c,) };
            j0 = pipe for (a,) in scan l { for (b,) in seek r[0](a,) { emit (a, b,) } };
            j1 = pipe for (a, b,) in scan j0 { for (c,) in seek s[0](b,) { emit (a, b, c,) } };
            p = pipe support { for (a, b, c,) in scan j1 { emit (a, c,) } };
            return p;
        }
    };
    assert_eq!(
        target.to_token_stream().to_string(),
        expected.to_token_stream().to_string()
    );
    assert!(check(&source, &target, &policy));
    assert!(!check(
        &source,
        &expected,
        &parse_policy(quote! {
            policy { L provides { iter }; R provides { iter }; S provides { iter }; join = merge; }
        })
    ));
}

#[test]
fn merge_policy_materializes_both_operands_ordered_and_co_iterates() {
    let source = chain_ix();
    let policy = parse_policy(quote! {
        policy { L provides { iter }; R provides { iter }; S provides { iter }; join = merge; }
    });
    let target = compile(&source, &policy).unwrap();
    let text = target.program.to_token_stream().to_string();
    assert!(
        text.contains("l = let provides { ordered @ l [0] }"),
        "{text}"
    );
    assert!(
        text.contains("j0 = let provides { ordered @ j0 [1] }"),
        "{text}"
    );
    assert!(
        text.contains("co (() in order l [0] , (b ,) in order r [0]) on (a ,)"),
        "{text}"
    );
    assert!(
        text.contains("co ((a ,) in order j0 [1] , (c ,) in order s [0]) on (b ,)"),
        "{text}"
    );
}

#[test]
fn policy_must_configure_exactly_the_declared_hosts() {
    let source = chain_ix();
    let missing = parse_policy(quote! { policy { L provides { iter }; R provides { iter }; } });
    assert!(
        compile(&source, &missing)
            .unwrap_err()
            .to_string()
            .contains("`S`")
    );
    let extra = parse_policy(quote! {
        policy { L provides { iter }; R provides { iter }; S provides { iter }; T provides { iter }; }
    });
    assert!(
        compile(&source, &extra)
            .unwrap_err()
            .to_string()
            .contains("undeclared")
    );
}

#[test]
fn prefer_lazy_marks_only_semijoins_whose_consumer_retests_the_reducer() {
    let source: ra::Module = syn::parse_quote! {
        struct Reduced;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, b, c) :- R(a, b), S(b, c);
        relational {
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            r1 = semijoin r with s;
            j = natural_join r1 with s;
            output j as answer(a, b, c);
        }
    };
    let source = cq_index_policy::compile(&source).unwrap();
    let policy = parse_policy(quote! {
        policy { R provides { iter }; S provides { iter }; semijoin = prefer_lazy; }
    });
    let target = compile(&source, &policy).unwrap();
    let Realization::Stream(streamed) = &target.program.definitions[2].realization else {
        panic!("semijoin is streamed")
    };
    assert!(streamed.lazy.is_some());
    // The same plan whose join no longer reads `s` cannot be lazy.
    let eager = parse_policy(quote! { policy { R provides { iter }; S provides { iter }; } });
    let eager = compile(&source, &eager).unwrap();
    let Realization::Stream(streamed) = &eager.program.definitions[2].realization else {
        panic!("semijoin is streamed")
    };
    assert!(streamed.lazy.is_none());
}

#[test]
fn every_shared_case_denotes_the_relational_oracle_under_both_join_rules() {
    for case in cases::CASES {
        let ra_module: ra::Module = syn::parse_str(case.plan).unwrap();
        let database: oracle::Database = case
            .inputs
            .iter()
            .map(|(name, rows)| {
                (
                    (*name).to_owned(),
                    rows.iter().map(|r| r.to_vec()).collect(),
                )
            })
            .collect();
        let expected = oracle::evaluate_ra(&ra_module, &database).output;
        let source = cq_index_policy::compile(&ra_module).unwrap();
        let hosts = ra_module.program.inputs().map(|d| {
            let name = &d.name;
            quote! { #name provides { iter }; }
        });
        let hosts: proc_macro2::TokenStream = hosts.collect();
        for join in [quote!(), quote!(join = merge;)] {
            let policy = parse_policy(quote! { policy { #hosts #join } });
            let target = compile(&source, &policy).unwrap_or_else(|e| panic!("{}: {e}", case.name));
            assert!(check(&source, &target, &policy), "{}", case.name);
            let actual = stream_oracle::evaluate(&target, &database).output;
            assert_eq!(actual, expected, "{} under {join}", case.name);
        }
    }
}

#[test]
fn renames_served_by_the_host_become_aliases_and_still_denote_the_oracle() {
    let source = chain_ix();
    let policy = parse_policy(quote! {
        policy { L provides { iter }; R provides { seek @R[0] }; S provides { seek @S[0] }; }
    });
    let target = compile(&source, &policy).unwrap();
    let text = target.program.to_token_stream().to_string();
    assert!(text.contains("r = alias R ;"), "{text}");
    assert!(text.contains("s = alias S ;"), "{text}");
    assert!(text.contains("l = alias L ;"), "{text}");
    let database: oracle::Database = [
        ("L".to_owned(), [vec![1], vec![2]].into_iter().collect()),
        (
            "R".to_owned(),
            [vec![1, 10], vec![2, 20], vec![2, 21]]
                .into_iter()
                .collect(),
        ),
        (
            "S".to_owned(),
            [vec![10, 100], vec![21, 210]].into_iter().collect(),
        ),
    ]
    .into_iter()
    .collect();
    let ra_module = &source.relational_plan;
    let expected = oracle::evaluate_ra(ra_module, &database).output;
    assert_eq!(stream_oracle::evaluate(&target, &database).output, expected);
}
