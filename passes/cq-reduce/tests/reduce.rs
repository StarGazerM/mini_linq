//! Reductions: the Yannakakis reducer of an acyclic plan is pinned and, once
//! elaborated, computes the query on every binary database; guards over one
//! atom are pushed onto it; cyclic queries are refused; `None` is the
//! identity and `check` holds on every output.
#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/databases.rs"]
mod databases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;

use cq_lang_cq as cq;
use cq_lang_cq::normal::Kernel;
use cq_lang_ra as ra;
use cq_lang_vo as valuation;
use cq_reduce::{Options, Pushdown, Reductions, check, compile};
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

fn steps(plan: &valuation::Module) -> String {
    plan.steps
        .iter()
        .map(|step| step.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn ra_text(target: &ra::Module) -> String {
    target.plan.to_token_stream().to_string()
}

fn assert_ra(target: &ra::Module, plan: TokenStream) {
    let program = &target.program;
    let expected: ra::Module = syn::parse2(quote! { #program relational { #plan } }).unwrap();
    assert_eq!(ra_text(target), ra_text(&expected));
}

const TREE: Options = Options {
    guards: Pushdown::Atoms,
    semijoins: Reductions::Tree,
};

/// Compile under `options`; the result checks against its source.
fn reduced(source: &valuation::Module, options: Options) -> valuation::Module {
    let target = compile(source, &options).unwrap();
    assert!(check(source, &target, &options), "{}", steps(&target));
    target
}

/// The RA of a reduced plan: it satisfies the RA contract, keeps the query's
/// kernel and computes the query on every binary database.
fn computes_the_query(source: &cq::Module, plan: &valuation::Module) -> ra::Module {
    let target = cq_elaborate::compile(plan).unwrap();
    ra::contract::check_module(&target).unwrap();
    assert_eq!(ra::kernel(&target), Kernel::of(source));
    for seed in 0..64 {
        let database = databases::binary_database(&source.program, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            oracle::evaluate_cq(&source.program, &database),
            "seed {seed}: {}",
            ra_text(&target)
        );
    }
    target
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
fn the_chain_carries_the_yannakakis_passes_before_its_first_atom() {
    let source = chain();
    let plan = reduced(&coarse(&source), TREE);
    // Up from the leaf L through R to the root S, then down again.
    assert_eq!(
        plan,
        vo(
            &source,
            quote! {
                reduce R@1 with L@0(a); reduce S@2 with R@1(_, b);
                reduce R@1 with S@2(b, _); reduce L@0 with R@1(a, _);
                L@0(a); R@1(a, b); S@2(b, c);
            }
        )
    );
    assert_eq!(valuation::kernel(&plan), Kernel::of(&source));
    let target = computes_the_query(&source, &plan);
    // The Y-UP/Y-DOWN semijoins on the clauses' relations as they are first
    // mentioned; the joins then follow the plan's order, and the head is
    // projected once at the end.
    assert_ra(
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
}

#[test]
fn a_guard_over_one_atom_is_pushed_onto_the_earliest_such_atom() {
    let source = cq(quote! {
        struct Guarded;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(a, b), S(b, c), if b > 0, if a != c, if a > 0;
    });
    let atoms = Options {
        guards: Pushdown::Atoms,
        semijoins: Reductions::None,
    };
    let plan = reduced(&coarse(&source), atoms);
    // `b > 0` reduces R, the first atom holding b, before R's step; `a > 0`
    // reduces L; `a != c` spans two atoms and stays where it was.
    assert_eq!(
        plan,
        vo(
            &source,
            quote! {
                reduce L@0 if a > 0; L@0(a);
                reduce R@1 if b > 0; R@1(a, b);
                S@2(b, c); if a != c;
            }
        )
    );
    computes_the_query(&source, &plan);
    // Under the tree, the pushed guards precede the reducer, in query order.
    let plan = reduced(&coarse(&source), TREE);
    assert_eq!(
        plan,
        vo(
            &source,
            quote! {
                reduce R@1 if b > 0; reduce L@0 if a > 0;
                reduce R@1 with L@0(a); reduce S@2 with R@1(_, b);
                reduce R@1 with S@2(b, _); reduce L@0 with R@1(a, _);
                L@0(a); R@1(a, b); S@2(b, c); if a != c;
            }
        )
    );
    computes_the_query(&source, &plan);
}

#[test]
fn guards_over_bindings_or_opaque_rust_stay_steps() {
    let source = cq(quote! {
        struct Bound;
        relation L(a: i32);
        answer(a) :- L(a), let d: i32 = a + 1, if d > 0, if matches!(a, 1 | 2);
    });
    let plan = reduced(&coarse(&source), TREE);
    assert_eq!(
        plan,
        vo(
            &source,
            quote! { L@0(a); let d: i32 = a + 1; if d > 0; if matches!(a, 1 | 2); }
        )
    );
}

#[test]
fn a_cyclic_query_has_no_reducer() {
    let error = compile(&coarse(&triangle()), &TREE)
        .expect_err("cyclic")
        .to_string();
    assert_eq!(error, "Yannakakis reduction requires an acyclic query");
    // Guards alone need no tree.
    let atoms = Options {
        guards: Pushdown::Atoms,
        semijoins: Reductions::None,
    };
    reduced(&coarse(&triangle()), atoms);
}

#[test]
fn no_reductions_is_the_identity_and_check_pins_the_options() {
    for source in [chain(), triangle()] {
        let plan = coarse(&source);
        assert_eq!(reduced(&plan, Options::default()), plan);
    }
    let plan = coarse(&chain());
    let tree = reduced(&plan, TREE);
    assert!(!check(&plan, &tree, &Options::default()));
    assert!(!check(&plan, &plan, &TREE));
    // A reduced plan is not reduced again.
    assert!(compile(&tree, &TREE).is_err());
    // A reduction the pass would not emit fails the correspondence even
    // though it is a well-formed plan of the same query.
    let foreign = vo(
        &chain(),
        quote! {
            reduce L@0 with R@1(a, _); reduce R@1 with L@0(a); reduce S@2 with R@1(_, b);
            reduce R@1 with S@2(b, _); reduce L@0 with R@1(a, _);
            L@0(a); R@1(a, b); S@2(b, c);
        },
    );
    assert!(valuation::contract::well_formed_module(&foreign));
    assert!(!check(&plan, &foreign, &TREE));
}

/// Every shared case: the reduced written-order plan (the full reducer when
/// the query is acyclic), elaborated, computes the authored answers and the
/// query on binary databases.
#[test]
fn the_shared_cases_compute_their_authored_answers() {
    let mut reduced_cases = 0;
    for case in cases::CASES {
        let authored: ra::Module = syn::parse_str(case.plan).unwrap();
        let source = cq::Module {
            program: authored.program,
        };
        let normal = cq::normal::normalize(&source).unwrap();
        let acyclic = cq::hypergraph::Analysis::of(&normal).acyclic;
        let options = Options {
            guards: Pushdown::Atoms,
            semijoins: if acyclic {
                Reductions::Tree
            } else {
                Reductions::None
            },
        };
        let plan = reduced(&coarse(&normal), options);
        reduced_cases += usize::from(acyclic);
        let target = cq_elaborate::compile(&plan).unwrap();
        assert_eq!(ra::kernel(&target), Kernel::of(&normal), "{}", case.name);
        let database: oracle::Database = case
            .inputs
            .iter()
            .map(|(name, rows)| {
                (
                    (*name).to_owned(),
                    rows.iter().map(|row| row.to_vec()).collect(),
                )
            })
            .collect();
        let expected: oracle::Rows = case.expected.iter().map(|row| row.to_vec()).collect();
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            expected,
            "{}: {}",
            case.name,
            ra_text(&target)
        );
        for seed in [1, 3, 5, 7] {
            let database = databases::binary_database(&source.program, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                oracle::evaluate_cq(&source.program, &database),
                "{} seed {seed}",
                case.name
            );
        }
    }
    assert!(reduced_cases >= 4, "{reduced_cases} acyclic cases");
}

#[allow(unused_imports, dead_code)]
mod cq_reduce {
    pub use ::cq_reduce::*;
    pub fn compile(
        source: &cq_lang_vo::Module,
        options: &::cq_reduce::Options,
    ) -> syn::Result<cq_lang_vo::Module> {
        Ok(cq_lang_vo::Module::new(
            source.program.clone(),
            ::cq_reduce::compile(source, options)?,
        ))
    }
    pub fn check(
        source: &cq_lang_vo::Module,
        target: &cq_lang_vo::Module,
        options: &::cq_reduce::Options,
    ) -> bool {
        target.program == source.program && ::cq_reduce::check(source, &target.section, options)
    }
}

#[allow(unused_imports, dead_code)]
mod cq_elaborate {
    pub use ::cq_elaborate::*;
    pub fn compile(source: &cq_lang_vo::Module) -> syn::Result<cq_lang_ra::Module> {
        Ok(cq_lang_ra::Module::new(
            source.program.clone(),
            ::cq_elaborate::compile(source)?,
        ))
    }
    pub fn check(source: &cq_lang_vo::Module, target: &cq_lang_ra::Module) -> bool {
        target.program == source.program && ::cq_elaborate::check(source, &target.section)
    }
}
