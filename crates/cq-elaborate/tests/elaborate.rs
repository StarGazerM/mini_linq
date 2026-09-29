//! VO → RA: coarse plans in written order elaborate to relation-at-a-time
//! RA (rename, join, select, extend, project in step order), fine plans to
//! semijoin/projection shapes that keep the query's kernel, reductions to
//! selections and semijoins on the clause's relation, and `check` holds on
//! every output.
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
fn coarse(source: &cq::Module) -> valuation::Module {
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
fn coarse_plans_elaborate_relation_at_a_time() {
    let plan = vo(&chain(), quote! { L@0(a); R@1(a, b); S@2(b, c); });
    let target = compile(&plan).unwrap();
    assert!(check(&plan, &target));
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
    let plan = vo(&triangle(), quote! { R@0(a, b); S@1(b, c); T@2(a, c); });
    let target = compile(&plan).unwrap();
    assert!(check(&plan, &target));
    assert_plan(
        &target,
        quote! {
            r0 = rename R { a -> a, b -> b };
            r1 = rename S { b -> b, c -> c };
            r2 = natural_join r0 with r1;
            r3 = rename T { a -> a, c -> c };
            r4 = natural_join r2 with r3;
            r5 = project r4 keep { r#a, r#b, r#c };
            output r5 as triangle(a, b, c);
        },
    );
}

#[test]
fn a_normalized_query_binds_its_constants_before_their_atoms() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, y) :- R(x, 5), S(x, x), if x > 0, let y: i32 = x + 1;
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let plan = coarse(&normal);
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
    let plan = coarse(&source);
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
            r6 = natural_join r5 with r0;
            r7 = rename S { b -> b, c -> c };
            r8 = project r7 keep { b };
            r9 = semijoin r6 with r8;
            r10 = natural_join r9 with r7;
            r11 = natural_join r10 with r2;
            r12 = extend r11 with { d: i32 } value ({ let d: i32 = a + c; (d,) });
            r13 = project r12 keep { r#a, r#b, r#c };
            output r13 as answer(a, b, c);
        },
    );
    // The check is about what is computed: the coarse plan's erasure is the
    // same query, and an RA that drops the guard is not.
    let coarse = coarse(&source);
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

#[test]
fn reductions_restrict_the_clause_relation_before_its_first_join() {
    // The Yannakakis passes of the chain: up from the leaf L through R to
    // the root S, then down again; every semijoin is on the clauses'
    // current relations, and the joins draw on the reduced ones.
    let source = chain();
    let plan = vo(
        &source,
        quote! {
            reduce R@1 with L@0(a); reduce S@2 with R@1(_, b);
            reduce R@1 with S@2(b, _); reduce L@0 with R@1(a, _);
            L@0(a); R@1(a, b); S@2(b, c);
        },
    );
    let target = compile(&plan).unwrap();
    assert!(check(&plan, &target));
    assert_eq!(ra::kernel(&target), Kernel::of(&source));
    assert_plan(
        &target,
        quote! {
            r0 = rename R { a -> a, b -> b };
            r1 = rename L { a -> a };
            r2 = semijoin r0 with r1;
            r3 = rename S { b -> b, c -> c };
            r4 = semijoin r3 with r2;
            r5 = semijoin r2 with r4;
            r6 = semijoin r1 with r5;
            r7 = natural_join r6 with r5;
            r8 = natural_join r7 with r4;
            r9 = project r8 keep { r#a, r#c };
            output r9 as answer(a, c);
        },
    );
    // A pushed guard selects on the clause's relation and is placed by that;
    // a reduced clause's fine subatoms draw on the reduced relation.
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(a, b, c) :- R(a, b), S(b, c), T(a, c), if a > 0;
    });
    let plan = vo(
        &source,
        quote! {
            reduce R@0 if a > 0; reduce R@0 with T@2(a, _);
            R@0(a, _); R@0(a, b); S@1(b, c); T@2(a, c);
        },
    );
    let target = compile(&plan).unwrap();
    assert!(check(&plan, &target));
    assert_eq!(ra::kernel(&target), Kernel::of(&source));
    assert_plan(
        &target,
        quote! {
            r0 = rename R { a -> a, b -> b };
            r1 = select r0 if (a > 0);
            r2 = rename T { a -> a, c -> c };
            r3 = semijoin r1 with r2;
            r4 = project r3 keep { a };
            r5 = natural_join r4 with r3;
            r6 = rename S { b -> b, c -> c };
            r7 = natural_join r5 with r6;
            r8 = natural_join r7 with r2;
            r9 = project r8 keep { r#a, r#b, r#c };
            output r9 as answer(a, b, c);
        },
    );
    for seed in 0..64 {
        let database = databases::binary_database(&source.program, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            oracle::evaluate_cq(&source.program, &database),
            "seed {seed}"
        );
    }
}
