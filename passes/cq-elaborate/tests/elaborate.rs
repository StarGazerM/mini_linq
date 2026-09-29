//! VO → RA: coarse plans in written order elaborate to relation-at-a-time
//! RA (rename, join, select, extend, project in step order), fine plans to
//! semijoin/projection shapes that keep the query's kernel, reductions to
//! selections and semijoins on the clause's relation, and `check` holds on
//! every output.
#[allow(dead_code)]
#[path = "../../../test-support/relational/cq_semantics.rs"]
mod cq_oracle;
#[allow(dead_code)]
#[path = "../../../test-support/relational/databases.rs"]
mod databases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;
#[allow(dead_code)]
#[path = "../../../test-support/relational/values.rs"]
mod values;

use cq_elaborate::{check, compile};
use cq_lang_ra as ra;
use cq_lang_vo as valuation;
use cq_lang_vo::cq;
use cq_lang_vo::cq::normal::Kernel;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn cq(tokens: TokenStream) -> cq::Program {
    syn::parse2(tokens).unwrap()
}

fn vo(source: &cq::Program, steps: TokenStream) -> valuation::Program {
    let program = &source;
    syn::parse2(quote! { #program valuation { #steps } }).unwrap()
}

/// One full subatom per atom, guards and bindings where written.
fn coarse(source: &cq::Program) -> valuation::Program {
    let steps = source
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

fn text(target: &ra::Program) -> String {
    target.plan.to_token_stream().to_string()
}

fn assert_plan(target: &ra::Program, plan: TokenStream) {
    let declarations = &target.plan.declarations;
    let expected: ra::Program =
        syn::parse2(quote! { relational { #(#declarations)* #plan } }).unwrap();
    assert_eq!(target, &expected);
}

fn chain() -> cq::Program {
    cq(quote! {
        struct Chain;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(a, b), S(b, c);
    })
}

fn triangle() -> cq::Program {
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
    assert!(check(&plan, &target).is_ok());
    assert_plan(
        &target,
        quote! {
            r0 = rename L { a -> a };
            r1 = rename R { a -> a, b -> b };
            r2 = natural_join r0 with r1;
            r3 = rename S { b -> b, c -> c };
            r4 = natural_join r2 with r3;
            r5 = project r4 keep { a, c };
            output r5 as answer(a, c);
        },
    );
    let plan = vo(&triangle(), quote! { R@0(a, b); S@1(b, c); T@2(a, c); });
    let target = compile(&plan).unwrap();
    assert!(check(&plan, &target).is_ok());
    assert_plan(
        &target,
        quote! {
            r0 = rename R { a -> a, b -> b };
            r1 = rename S { b -> b, c -> c };
            r2 = natural_join r0 with r1;
            r3 = rename T { a -> a, c -> c };
            r4 = natural_join r2 with r3;
            r5 = project r4 keep { a, b, c };
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
    assert!(check(&plan, &target).is_ok());
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
            r9 = project r8 keep { x, y };
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
    assert!(check(&plan, &target).is_ok());
    assert_plan(
        &target,
        quote! {
            r0 = rename R { a -> a, b -> b };
            r1 = rename S { b -> b, c -> c };
            r2 = natural_join r0 with r1;
            r3 = project r2 keep { a, b, c };
            output r3 as answer(a + c);
        },
    );
}

fn guarded_triangle() -> cq::Program {
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
    assert_eq!(cq_elaborate::kernel(&target, &source), Kernel::of(&source));
    assert!(check(&plan, &target).is_ok());
    assert_plan(
        &target,
        quote! {
            r0 = rename R { a -> a, b -> b };
            r1 = project r0 keep { a };
            r2 = rename T { a -> a, c -> c };
            r3 = semijoin r1 with r2;
            r4 = select r3 if (a > 0);
            r5 = natural_join r4 with r0;
            r6 = rename S { b -> b, c -> c };
            r7 = semijoin r5 with r6;
            r8 = natural_join r7 with r6;
            r9 = natural_join r8 with r2;
            r10 = extend r9 with { d: i32 } value ({ let d: i32 = a + c; (d,) });
            r11 = project r10 keep { a, b, c };
            output r11 as answer(a, b, c);
        },
    );
    // The check is about what is computed: the coarse plan's erasure is the
    // same query, and an RA that drops the guard is not.
    let coarse = coarse(&source);
    assert!(check(&coarse, &target).is_ok());
    assert!(check(&plan, &compile(&coarse).unwrap()).is_ok());
    let declarations = &source.declarations;
    let unguarded: ra::Program = syn::parse2(quote! {
        relational { #(#declarations)*
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
    assert!(ra::contract::check(&unguarded).is_ok());
    assert!(check(&plan, &unguarded).is_err());
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
        assert!(check(&plan, &target).is_ok());
        for seed in 0..64 {
            let database = databases::binary_database(&source, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                cq_oracle::evaluate_cq(&source, &database),
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
    assert!(check(&plan, &target).is_ok());
    assert_eq!(cq_elaborate::kernel(&target, &source), Kernel::of(&source));
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
            r9 = project r8 keep { a, c };
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
    assert!(check(&plan, &target).is_ok());
    assert_eq!(cq_elaborate::kernel(&target, &source), Kernel::of(&source));
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
            r9 = project r8 keep { a, b, c };
            output r9 as answer(a, b, c);
        },
    );
    for seed in 0..64 {
        let database = databases::binary_database(&source, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            cq_oracle::evaluate_cq(&source, &database),
            "seed {seed}"
        );
    }
}

#[path = "../../../test-support/relational/scenarios.rs"]
mod scenarios;
#[test]
fn shared_scenarios_and_mutated_output_correspondence() {
    for case in scenarios::SCENARIOS {
        let source: valuation::Program = syn::parse_str(case.vo).unwrap();
        let target = compile(&source).unwrap();
        assert_eq!(
            target,
            syn::parse_str::<ra::Program>(case.ra).unwrap(),
            "{}",
            case.name
        );
        check(&source, &target).unwrap();
        for seed in 0..4 {
            let database = databases::binary_database(&source.cq, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                cq_oracle::evaluate_cq(&source.cq, &database),
                "{}",
                case.name
            );
        }
        let mut changed = target.clone();
        changed.plan.output.head.relation = syn::parse_quote!(other);
        ra::contract::check(&changed).unwrap();
        assert!(check(&source, &changed).is_err());
        let mut changed = target;
        changed.plan.declarations[0].columns[0].ty = syn::parse_quote!(u32);
        assert!(check(&source, &changed).is_err());
    }
}

#[test]
fn erasure_restores_query_order_collapses_renames_and_handles_non_block_extends() {
    let source: cq::Program = syn::parse_quote! {
        struct P;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, k) :- R(x, y), S(y, z), R(z, x), if x != z, let k: i32 = x + z;
    };
    let plan: ra::Program = syn::parse_quote! { relational {
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        last = rename R {a -> z, b -> x};
        first = rename R {a -> x, b -> y};
        only_x = project first keep {x};
        seed = natural_join last with only_x;
        again = rename R {a -> x, b -> y};
        both = natural_join seed with again;
        mid = rename S {b -> y, c -> z};
        all = natural_join both with mid;
        kept = select all if (x != z);
        wide = extend kept with {k: i32} value ((x + z,));
        done = project wide keep {x, k};
        output done as answer(x, k);
    }};
    ra::contract::check(&plan).unwrap();
    let mut expected = source.clone();
    expected.query = syn::parse_quote! {
        answer(x, k) :- R(x, y), S(y, z), R(z, x), if x != z, let (k,) = (x + z,);
    };
    // syn prints the required comma for a singleton tuple pattern even when
    // its constructed Punctuated has no trailing separator. Compare the
    // consumer-visible, reparsed AST with the authored query.
    let erased = cq_elaborate::erase(&plan, &source);
    assert_eq!(
        syn::parse2::<cq::Program>(erased.to_token_stream()).unwrap(),
        expected
    );
    assert_eq!(cq_elaborate::kernel(&plan, &source), Kernel::of(&expected));
    let mut fewer = plan;
    fewer.plan.definitions.retain(|d| d.result != "kept");
    fewer.plan.definitions[8].operator =
        syn::parse_quote!(extend all with {k: i32} value ((x + z,)));
    ra::contract::check(&fewer).unwrap();
    assert!(cq_elaborate::kernel(&fewer, &source).guards.is_empty());
}

#[test]
fn independent_kernel_checks_obligations_and_accepts_result_names() {
    let source: valuation::Program = syn::parse_quote! {
        struct Q; relation R(a:i32); answer(a) :- R(a), if a > 0;
        valuation { R@0(a); if a > 0; }
    };
    let alternative: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32);
        input_rows = rename R {a -> a};
        filtered = select input_rows if (a > 0);
        answer_rows = project filtered keep {a};
        output answer_rows as answer(a);
    }};
    check(&source, &alternative).unwrap();
    assert_ne!(compile(&source).unwrap(), alternative);
    let reject = |s: &valuation::Program, t: &ra::Program, names: &[&str]| {
        let error = check(s, t).unwrap_err().to_string();
        for name in names {
            assert!(error.contains(name), "{error}");
        }
    };
    let mut invalid = source.clone();
    invalid.valuation.steps[0] = syn::parse_quote!(R@9(a););
    reject(&invalid, &alternative, &["clause #9"]);
    let mut invalid = alternative.clone();
    invalid.plan.output.input = syn::parse_quote!(missing);
    reject(&source, &invalid, &["missing"]);
    let mut changed = alternative.clone();
    changed.plan.declarations[0].columns[0].ty = syn::parse_quote!(u32);
    reject(&source, &changed, &["answer_rows", "declarations"]);
    let mut changed = alternative.clone();
    changed.plan.output.head.relation = syn::parse_quote!(other);
    reject(&source, &changed, &["answer_rows", "head"]);
    let mut changed = alternative.clone();
    changed.plan.definitions[1].operator = syn::parse_quote!(select input_rows if (a > 1));
    ra::contract::check(&changed).unwrap();
    reject(&source, &changed, &["answer_rows", "kernel"]);
}
