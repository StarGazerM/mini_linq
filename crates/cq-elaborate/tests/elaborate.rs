//! VO → RA: written-order relation-at-a-time plans elaborate to the RA the
//! earlier planners emitted, fine plans to semijoin/projection shapes that
//! keep the query's kernel, and `check` holds on every output.
#[allow(dead_code)]
#[path = "../../../test-support/relational/databases.rs"]
mod databases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;

use cq_elaborate::{check, compile};
use cq_ir::cq::{self, normal::Kernel};
use cq_ir::{relational_plan as ra, valuation};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn cq(tokens: TokenStream) -> cq::Module {
    syn::parse2(tokens).unwrap()
}

fn vo(source: &cq::Module, steps: TokenStream) -> valuation::Module {
    let program = &source.program;
    syn::parse2(quote! { #program valuation { #steps } }).unwrap()
}

/// One full subatom per atom, guards and bindings where written.
fn written_order(source: &cq::Module) -> valuation::Module {
    let steps = source
        .program
        .query
        .body
        .iter()
        .enumerate()
        .map(|(clause, item)| match item {
            cq::BodyItem::Positive { atom } => {
                let relation = &atom.relation;
                let clause = proc_macro2::Literal::usize_unsuffixed(clause);
                let columns = atom.variables();
                quote!(#relation @ #clause (#(#columns),*);)
            }
            cq::BodyItem::Filter(filter) => quote!(#filter;),
            cq::BodyItem::Let(binding) => quote!(#binding;),
            _ => unreachable!("normal form"),
        });
    vo(source, quote!(#(#steps)*))
}

fn text(target: &ra::Module) -> String {
    target.plan.to_token_stream().to_string()
}

fn assert_plan(target: &ra::Module, plan: TokenStream) {
    let program = &target.program;
    let expected: ra::Module = syn::parse2(quote! { #program relational { #plan } }).unwrap();
    assert_eq!(text(target), text(&expected));
}

fn chain() -> cq::Module {
    cq(quote! {
        struct Chain;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(a, b), S(b, c);
    })
}

fn triangle() -> cq::Module {
    cq(quote! {
        struct Triangle;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        triangle(a, b, c) :- R(a, b), S(b, c), T(a, c);
    })
}

#[test]
fn coarse_written_order_plans_elaborate_to_the_earlier_planners_ra() {
    for (source, steps) in [
        (chain(), quote! { L@0(a); R@1(a, b); S@2(b, c); }),
        (triangle(), quote! { R@0(a, b); S@1(b, c); T@2(a, c); }),
    ] {
        let plan = vo(&source, steps);
        let target = compile(&plan).unwrap();
        assert!(check(&plan, &target));
        let clause = cq_query_lowering::clause_prefix::compile(&source).unwrap();
        assert_eq!(text(&target), text(&clause));
        assert_eq!(target, clause);
        assert!(check(&plan, &clause));
        let written = cq_query_lowering::written_order::compile(&source).unwrap();
        assert_eq!(text(&target).replace("r#", ""), text(&written));
        assert!(check(&plan, &written));
    }
    let target = compile(&vo(&chain(), quote! { L@0(a); R@1(a, b); S@2(b, c); })).unwrap();
    assert_plan(
        &target,
        quote! {
            r0 = rename L { a -> a };
            r1 = rename R { a -> a, b -> b };
            r2 = natural_join r0 with r1;
            r3 = rename S { b -> b, c -> c };
            r4 = natural_join r2 with r3;
            r5 = project r4 keep { r#a, r#c };
            output r5 as answer(a, c);
        },
    );
}

#[test]
fn a_normalized_query_in_written_order_matches_the_clause_planner_on_the_original() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, y) :- R(x, 5), S(x, x), if x > 0, let y: i32 = x + 1;
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let plan = written_order(&normal);
    let target = compile(&plan).unwrap();
    assert!(check(&plan, &target));
    assert_plan(
        &target,
        quote! {
            r0 = unit;
            r1 = extend r0 with { __cq_0_1: i32 } value ({ let __cq_0_1: i32 = 5; (__cq_0_1,) });
            r2 = rename R { a -> x, b -> __cq_0_1 };
            r3 = natural_join r1 with r2;
            r4 = rename S { b -> x, c -> __cq_1_1 };
            r5 = natural_join r3 with r4;
            r6 = select r5 if (__cq_1_1 == x);
            r7 = select r6 if (x > 0);
            r8 = extend r7 with { y: i32 } value ({ let y: i32 = x + 1; (y,) });
            r9 = project r8 keep { r#x, r#y };
            output r9 as answer(x, y);
        },
    );
    let clause = cq_query_lowering::clause_prefix::compile(&source).unwrap();
    assert_eq!(text(&target), text(&clause));
}

#[test]
fn a_computed_head_retains_the_whole_heading() {
    let source = cq(quote! {
        struct Sum;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation answer(value: i32);
        answer(a + c) :- R(a, b), S(b, c);
    });
    let plan = written_order(&source);
    let target = compile(&plan).unwrap();
    assert!(check(&plan, &target));
    assert_plan(
        &target,
        quote! {
            r0 = rename R { a -> a, b -> b };
            r1 = rename S { b -> b, c -> c };
            r2 = natural_join r0 with r1;
            r3 = project r2 keep { r#a, r#b, r#c };
            output r3 as answer(a + c);
        },
    );
    let clause = cq_query_lowering::clause_prefix::compile(&source).unwrap();
    assert_eq!(text(&target), text(&clause));
}

fn guarded_triangle() -> cq::Module {
    cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(a, b, c) :- R(a, b), S(b, c), T(a, c), if a > 0, let d: i32 = a + c;
    })
}

#[test]
fn the_fine_triangle_plan_semijoins_checks_and_keeps_the_kernel() {
    let source = guarded_triangle();
    let plan = vo(
        &source,
        quote! {
            R@0(a, _); T@2(a, _); if a > 0;
            R@0(a, b); S@1(b, _);
            S@1(b, c); T@2(a, c); let d: i32 = a + c;
        },
    );
    let target = compile(&plan).unwrap();
    ra::contract::check(&target).unwrap();
    assert_eq!(ra::kernel(&target), Kernel::of(&source));
    assert!(check(&plan, &target));
    assert_plan(
        &target,
        quote! {
            r0 = rename R { a -> a, b -> b };
            r1 = project r0 keep { a };
            r2 = rename T { a -> a, c -> c };
            r3 = project r2 keep { a };
            r4 = semijoin r1 with r3;
            r5 = select r4 if (a > 0);
            r6 = rename R { a -> a, b -> b };
            r7 = natural_join r5 with r6;
            r8 = rename S { b -> b, c -> c };
            r9 = project r8 keep { b };
            r10 = semijoin r7 with r9;
            r11 = rename S { b -> b, c -> c };
            r12 = natural_join r10 with r11;
            r13 = rename T { a -> a, c -> c };
            r14 = natural_join r12 with r13;
            r15 = extend r14 with { d: i32 } value ({ let d: i32 = a + c; (d,) });
            r16 = project r15 keep { r#a, r#b, r#c };
            output r16 as answer(a, b, c);
        },
    );
    // The check is about what is computed: the coarse plan's erasure is the
    // same query, and an RA that drops the guard is not.
    let coarse = written_order(&source);
    assert!(check(&coarse, &target));
    assert!(check(&plan, &compile(&coarse).unwrap()));
    let program = &source.program;
    let unguarded: ra::Module = syn::parse2(quote! {
        #program
        relational {
            r0 = rename R { a -> a, b -> b };
            r1 = rename S { b -> b, c -> c };
            r2 = natural_join r0 with r1;
            r3 = rename T { a -> a, c -> c };
            r4 = natural_join r2 with r3;
            r5 = extend r4 with { d: i32 } value ({ let d: i32 = a + c; (d,) });
            r6 = project r5 keep { a, b, c };
            output r6 as answer(a, b, c);
        }
    })
    .unwrap();
    assert!(ra::contract::well_formed(&unguarded));
    assert!(!check(&plan, &unguarded));
}

#[test]
fn fine_plans_compute_the_query_on_binary_databases() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(a, c) :- R(a, b), S(b, c), T(a, c), if a != c;
    });
    let plans = [
        quote! { R@0(a, _); T@2(a, _); R@0(a, b); S@1(b, _); S@1(b, c); if a != c; T@2(a, c); },
        quote! { S@1(b, c); R@0(a, b); if a != c; T@2(a, c); },
        quote! { T@2(a, c); if a != c; R@0(a, _); R@0(a, b); S@1(b, c); },
    ];
    for steps in plans {
        let plan = vo(&source, steps);
        let target = compile(&plan).unwrap();
        assert!(check(&plan, &target));
        for seed in 0..64 {
            let database = databases::binary_database(&source.program, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                oracle::evaluate_cq(&source.program, &database),
                "seed {seed}: {}",
                text(&target)
            );
        }
    }
}
