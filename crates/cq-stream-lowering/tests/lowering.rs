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

use cq_ir::policy::Policy;
use cq_ir::{index_requirements as ix, relational_plan as ra, stream, structures};
use cq_stream_lowering::{check, compile};
use quote::{ToTokens, quote};
use stream::Realization;

fn parse_policy(tokens: proc_macro2::TokenStream) -> Policy {
    syn::parse2(tokens).unwrap()
}

fn select(source: &ix::Module, policy: &Policy) -> structures::Module {
    cq_structure_selection::compile(source, policy).unwrap()
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
    let source = select(&source, &policy);
    let target = compile(&source, &policy).unwrap();
    let expected: stream::Module = syn::parse_quote! {
        #source
        stream {
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
    let merge = parse_policy(quote! {
        policy { L provides { iter }; R provides { iter }; S provides { iter }; join = merge; }
    });
    assert!(!check(
        &select(&source.index_requirements, &merge),
        &expected,
        &merge
    ));
}

#[test]
fn merge_policy_materializes_both_operands_ordered_and_co_iterates() {
    let source = chain_ix();
    let policy = parse_policy(quote! {
        policy { L provides { iter }; R provides { iter }; S provides { iter }; join = merge; }
    });
    let target = compile(&select(&source, &policy), &policy).unwrap();
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
        cq_structure_selection::compile(&source, &missing)
            .unwrap_err()
            .to_string()
            .contains("`S`")
    );
    let extra = parse_policy(quote! {
        policy { L provides { iter }; R provides { iter }; S provides { iter }; T provides { iter }; }
    });
    assert!(
        cq_structure_selection::compile(&source, &extra)
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
    let target = compile(&select(&source, &policy), &policy).unwrap();
    let Realization::Stream(streamed) = &target.program.definitions[2].realization else {
        panic!("semijoin is streamed")
    };
    assert!(streamed.lazy.is_some());
    // The same plan whose join no longer reads `s` cannot be lazy.
    let eager = parse_policy(quote! { policy { R provides { iter }; S provides { iter }; } });
    let eager = compile(&select(&source, &eager), &eager).unwrap();
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
            let structures = select(&source, &policy);
            let target =
                compile(&structures, &policy).unwrap_or_else(|e| panic!("{}: {e}", case.name));
            assert!(check(&structures, &target, &policy), "{}", case.name);
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
    let target = compile(&select(&source, &policy), &policy).unwrap();
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

#[test]
fn comparisons_on_an_ordered_host_column_become_bounded_scans_with_residual_guards() {
    let plan: ra::Module = syn::parse_quote! {
        struct Windowed;
        relation R(a: i32, b: i32);
        answer(a, b) :- R(a, b), if a >= 2, if a < 9, if b != 5;
        relational {
            r = rename R {a -> a, b -> b};
            s = select r if (a >= 2 && 9 > a && b != 5);
            t = select s if (b >= 0);
            output t as answer(a, b);
        }
    };
    let source = cq_index_policy::compile(&plan).unwrap();
    let ordered = parse_policy(quote! { policy { R provides { iter, ordered @R[0] }; } });
    let target = compile(&select(&source, &ordered), &ordered).unwrap();
    let text = target.program.to_token_stream().to_string();
    assert!(text.contains("s = pipe for (a , b ,) in scan r within [0] ((2i32) , (8i32)) { if (b != 5) { emit (a , b ,) } }"), "{text}");
    // A select whose operand is not a host rename keeps its guard.
    assert!(
        text.contains("t = pipe for (a , b ,) in scan s { if (b >= 0) { emit (a , b ,) } }"),
        "{text}"
    );
    let unordered = parse_policy(quote! { policy { R provides { iter }; } });
    let plain = compile(&select(&source, &unordered), &unordered).unwrap();
    assert!(
        plain
            .program
            .to_token_stream()
            .to_string()
            .contains("scan r { if (a >= 2 && 9 > a && b != 5)")
    );

    // Both denote the RA oracle on a database around the window edges.
    let rows: std::collections::BTreeSet<Vec<i32>> =
        [(1, 0), (2, 5), (2, 6), (8, 1), (9, 1), (3, 5)]
            .map(|(a, b)| vec![a, b])
            .into_iter()
            .collect();
    let database: oracle::Database = [("R".to_owned(), rows)].into_iter().collect();
    let expected = oracle::evaluate_ra(&plan, &database).output;
    assert_eq!(expected, [vec![2, 6], vec![8, 1]].into_iter().collect());
    assert_eq!(stream_oracle::evaluate(&target, &database).output, expected);
    assert_eq!(stream_oracle::evaluate(&plain, &database).output, expected);
}
