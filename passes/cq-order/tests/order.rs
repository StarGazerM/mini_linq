//! Written, prefix-ranked and elimination orders, coarse and fine: every plan
//! passes the VO contract and erases to its query; prefix rankings place the
//! clauses in the pinned sequences (those of the clause-prefix planner they
//! replace).
use cq_lang_cq as cq;
use cq_lang_cq::{BodyItem, normal::Kernel};
use cq_lang_vo as valuation;
use cq_order::{Heuristic, Options, Ranking, Split};
use quote::{ToTokens, quote};

fn cq(tokens: proc_macro2::TokenStream) -> cq::Module {
    syn::parse2(tokens).unwrap()
}

fn normal(tokens: proc_macro2::TokenStream) -> cq::Module {
    cq::normal::normalize(&cq(tokens)).unwrap()
}

fn triangle() -> cq::Module {
    cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(a, b, c) :- R(a, b), S(b, c), T(a, c);
    })
}

fn chain() -> cq::Module {
    cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(c: i32, d: i32);
        answer(x, w) :- R(x, y), S(y, z), T(z, w);
    })
}

/// Compile, and require the plan to check and to erase to the source.
fn plan(source: &cq::Module, heuristic: Heuristic, split: Split) -> valuation::Module {
    let options = Options { heuristic, split };
    let plan = cq_order::compile(source, &options).unwrap();
    valuation::contract::check_module(&plan).unwrap();
    assert!(cq_order::check(source, &plan, &options));
    assert_eq!(valuation::kernel(&plan), Kernel::of(source));
    plan
}

/// The plan with the given steps over the source program.
fn expected(source: &cq::Module, steps: proc_macro2::TokenStream) -> valuation::Module {
    let program = &source.program;
    syn::parse2(quote! { #program valuation { #steps } }).unwrap()
}

/// The clause each coarse step draws on.
fn sequence(plan: &valuation::Module) -> Vec<usize> {
    let body = &plan.program.query.body;
    let nth = |n: usize, kind: fn(&BodyItem) -> bool| {
        body.iter()
            .enumerate()
            .filter(|(_, item)| kind(item))
            .nth(n)
            .map(|(clause, _)| clause)
            .unwrap()
    };
    let (mut guards, mut lets) = (0, 0);
    plan.steps
        .iter()
        .map(|step| match step {
            valuation::Step::Atom(subatom, _) => subatom.clause(),
            valuation::Step::Guard(..) => {
                guards += 1;
                nth(guards - 1, |item| matches!(item, BodyItem::Filter(_)))
            }
            valuation::Step::Let(..) => {
                lets += 1;
                nth(lets - 1, |item| matches!(item, BodyItem::Let(_)))
            }
            valuation::Step::Reduce(..) => unreachable!("order emits no reductions"),
        })
        .collect()
}

#[test]
fn written_coarse_follows_the_normal_body() {
    let chain = chain();
    assert_eq!(
        plan(&chain, Heuristic::Written, Split::Coarse),
        expected(&chain, quote! { R@0(x, y); S@1(y, z); T@2(z, w); })
    );
    let messy = normal(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, z) :- R(x, 5), S(_, z), R(x, x), if z > 1, let w: i32 = x + z;
    });
    assert_eq!(
        plan(&messy, Heuristic::Written, Split::Coarse),
        expected(
            &messy,
            quote! {
                let __cq_0_1: i32 = 5;
                R@1(x, __cq_0_1);
                S@2(__cq_1_0, z);
                R@3(x, __cq_2_1);
                if __cq_2_1 == x;
                if z > 1;
                let w: i32 = x + z;
            }
        )
    );
}

#[test]
fn fine_split_of_the_triangle_is_generic_join() {
    let triangle = triangle();
    assert_eq!(
        plan(&triangle, Heuristic::Written, Split::Fine),
        expected(
            &triangle,
            quote! { R@0(a, _); T@2(a, _); R@0(a, b); S@1(b, _); S@1(b, c); T@2(a, c); }
        )
    );
    // Guards and bindings follow the step that binds their last variable.
    let guarded = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(a, b, c) :- R(a, b), S(b, c), T(a, c), if a > 0, let d: i32 = a + c;
    });
    assert_eq!(
        plan(&guarded, Heuristic::Written, Split::Fine),
        expected(
            &guarded,
            quote! {
                R@0(a, _); if a > 0; T@2(a, _);
                R@0(a, b); S@1(b, _);
                S@1(b, c); let d: i32 = a + c; T@2(a, c);
            }
        )
    );
}

#[test]
fn a_bound_key_is_checked_by_every_atom_holding_it() {
    let keyed = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, y) :- let k: i32 = 5, R(x, k), S(k, y);
    });
    let prefix = Heuristic::Prefix(Ranking::Connected);
    assert_eq!(
        plan(&keyed, prefix, Split::Coarse),
        expected(&keyed, quote! { let k: i32 = 5; R@1(x, k); S@2(k, y); })
    );
    assert_eq!(
        plan(&keyed, prefix, Split::Fine),
        expected(
            &keyed,
            quote! { let k: i32 = 5; R@1(_, k); S@2(k, _); R@1(x, k); S@2(k, y); }
        )
    );
}

#[test]
fn prefix_rankings_place_clauses_in_the_pinned_sequences() {
    // Each case pins the clause sequence under `Written`, then under the
    // three prefix rankings (`Connected`, `GreedyWidth`, `Auto`).
    let cases = [
        (chain(), [0, 1, 2].as_slice(), [0, 1, 2].as_slice()),
        (triangle(), &[0, 1, 2], &[0, 1, 2]),
        // Guards become ready in query order: VO keeps guards in that order,
        // so `x != z` is placed as soon as T binds z and `w > 1` after S.
        (
            cq(quote! {
                struct Q;
                relation R(a: i32, b: i32);
                relation S(a: i32, b: i32);
                relation T(a: i32, b: i32);
                answer(x) :- R(x, y), S(z, w), T(y, z), if x != z, if w > 1;
            }),
            &[0, 1, 2, 3, 4],
            &[0, 2, 3, 1, 4],
        ),
        (
            cq(quote! {
                struct Q;
                relation R(a: i32, b: i32);
                relation S(a: i32, b: i32);
                relation T(a: i32, b: i32);
                answer(x) :- let k: i32 = 5, R(x, k), S(x, y), T(y, z);
            }),
            &[0, 1, 2, 3],
            &[0, 1, 2, 3],
        ),
        (
            cq(quote! {
                struct Q;
                relation R(a: i32, b: i32);
                relation S(a: i32, b: i32);
                relation T(a: i32, b: i32);
                answer(x, z) :- R(x, y), S(y, z), T(x, z), if y > x;
            }),
            &[0, 1, 2, 3],
            &[0, 3, 1, 2],
        ),
    ];
    for (source, written, prefix) in &cases {
        for (heuristic, expected) in [
            (Heuristic::Written, *written),
            (Heuristic::Prefix(Ranking::Connected), *prefix),
            (Heuristic::Prefix(Ranking::GreedyWidth), *prefix),
            (Heuristic::Prefix(Ranking::Auto), *prefix),
        ] {
            assert_eq!(
                sequence(&plan(source, heuristic, Split::Coarse)),
                expected,
                "{heuristic:?} on {}",
                source.program.query.to_token_stream()
            );
        }
    }
}

#[test]
fn prefix_ranking_counts_normalized_constants_as_the_old_literal_keys() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(a: i32, b: i32);
        relation T(a: i32, b: i32);
        answer(x, z) :- R(x, 7), S(7, z), T(z, x);
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let constants = normal
        .program
        .query
        .body
        .iter()
        .take_while(|item| matches!(item, BodyItem::Let(_)))
        .count();
    assert_eq!(constants, 2);
    for heuristic in [
        Heuristic::Prefix(Ranking::Connected),
        Heuristic::Prefix(Ranking::GreedyWidth),
    ] {
        let sequence = sequence(&plan(&normal, heuristic, Split::Coarse));
        // The constants' bindings come first, as C-CONSTANT did.
        assert_eq!(sequence[..constants], [0, 1]);
        let clauses: Vec<usize> = sequence[constants..]
            .iter()
            .map(|c| c - constants)
            .collect();
        assert_eq!(clauses, [0, 1, 2], "{heuristic:?}");
    }
}

#[test]
fn elimination_orders_atoms_by_their_earliest_variable() {
    for source in [chain(), triangle()] {
        let order = cq::hypergraph::Analysis::of(&source).elimination_order;
        let position = |v: &String| order.iter().position(|o| o == v).unwrap();
        let coarse = plan(&source, Heuristic::Elimination, Split::Coarse);
        let earliest: Vec<usize> = coarse
            .steps
            .iter()
            .filter_map(|step| match step {
                valuation::Step::Atom(subatom, _) => Some(
                    subatom
                        .variables()
                        .map(|(_, v)| position(&v.to_string()))
                        .min()
                        .unwrap(),
                ),
                _ => None,
            })
            .collect();
        assert!(earliest.is_sorted(), "{earliest:?} for {order:?}");
        // Fine binds the atom variables in exactly the elimination order.
        let fine = plan(&source, Heuristic::Elimination, Split::Fine);
        assert_eq!(atom_bound(&fine), order);
    }
}

/// The variables the plan's subatoms introduce, in step order.
fn atom_bound(plan: &valuation::Module) -> Vec<String> {
    let analysis = valuation::contract::check_module(plan).unwrap();
    analysis
        .introduces
        .iter()
        .zip(&plan.steps)
        .filter(|(_, step)| matches!(step, valuation::Step::Atom(..)))
        .flat_map(|(introduced, _)| introduced.clone())
        .collect()
}

#[test]
fn elimination_defers_an_atom_to_the_binding_that_supplies_its_column() {
    // Whatever the elimination order says, `S(k, y)` waits for `k`, which
    // waits for `R(x)`.
    let source = cq(quote! {
        struct Q;
        relation R(a: i32);
        relation S(b: i32, c: i32);
        answer(x, y) :- R(x), let k: i32 = x + 1, S(k, y), if y > k;
    });
    assert_eq!(
        plan(&source, Heuristic::Elimination, Split::Coarse),
        expected(
            &source,
            quote! { R@0(x); let k: i32 = x + 1; S@2(k, y); if y > k; }
        )
    );
    let fine = plan(&source, Heuristic::Elimination, Split::Fine);
    let order = cq::hypergraph::Analysis::of(&source).elimination_order;
    let atom_order: Vec<String> = order.iter().filter(|v| *v != "k").cloned().collect();
    assert_eq!(atom_bound(&fine), atom_order);
    // No subatom lists `k` before the binding supplies it.
    let binding = fine
        .steps
        .iter()
        .position(|step| matches!(step, valuation::Step::Let(..)))
        .unwrap();
    let first_check = fine
        .steps
        .iter()
        .position(|step| match step {
            valuation::Step::Atom(subatom, _) => subatom.variables().any(|(_, v)| v == "k"),
            _ => false,
        })
        .unwrap();
    assert!(binding < first_check, "{}", fine.to_token_stream());
}

#[test]
fn sources_outside_normal_form_or_using_variables_early_are_rejected() {
    let options = Options::default();
    let repeated = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        answer(x) :- R(x, x);
    });
    let error = cq_order::compile(&repeated, &options)
        .unwrap_err()
        .to_string();
    assert!(error.contains("normal form"), "{error}");
    let early = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        answer(x) :- if y > 1, R(x, y);
    });
    let error = cq_order::compile(&early, &options).unwrap_err().to_string();
    assert!(error.contains("before it is bound"), "{error}");
}

#[test]
fn every_heuristic_and_split_ends_each_clause_fully_covered() {
    let cases = [
        normal(quote! {
            struct Q;
            relation R(a: i32, b: i32);
            answer(x, z) :- R(x, y), R(y, z), R(z, x), if x.abs() > 1, let s: i32 = x + y + z;
        }),
        normal(quote! {
            struct Q;
            relation R(a: i32, b: i32);
            relation S(b: i32, c: String);
            answer(x, n) :- R(1, x), S(x, n), R(x, _), if n.len() > 2, let m: usize = n.len(), R(m as i32, _);
        }),
        normal(quote! {
            struct Q;
            relation R(a: i32, b: i32);
            relation S(b: i32, c: i32);
            relation T(c: i32, d: i32);
            relation U(d: i32, a: i32);
            answer(w, y) :- R(w, x), S(x, y), T(y, z), U(z, w), let k: i32 = w * 2, if k > y;
        }),
    ];
    let heuristics = [
        Heuristic::Written,
        Heuristic::Prefix(Ranking::Connected),
        Heuristic::Prefix(Ranking::GreedyWidth),
        Heuristic::Elimination,
    ];
    for source in &cases {
        for heuristic in heuristics {
            for split in [Split::Coarse, Split::Fine] {
                let plan = plan(source, heuristic, split);
                let mut last: std::collections::BTreeMap<usize, &valuation::Subatom> =
                    Default::default();
                for step in &plan.steps {
                    if let valuation::Step::Atom(subatom, _) = step {
                        last.insert(subatom.clause(), subatom);
                    }
                }
                for (clause, subatom) in last {
                    assert!(
                        subatom
                            .columns
                            .iter()
                            .all(|c| matches!(c, valuation::Column::Variable(_))),
                        "{heuristic:?}/{split:?} leaves clause #{clause} at {}",
                        subatom.to_token_stream()
                    );
                }
            }
        }
    }
}

#[allow(unused_imports, dead_code)]
mod cq_order {
    pub use ::cq_order::*;
    pub fn compile(
        source: &cq_lang_cq::Module,
        options: &::cq_order::Options,
    ) -> syn::Result<cq_lang_vo::Module> {
        Ok(cq_lang_vo::Module::new(
            source.program.clone(),
            ::cq_order::reference::compile(source, options)?,
        ))
    }
    pub fn check(
        source: &cq_lang_cq::Module,
        target: &cq_lang_vo::Module,
        options: &::cq_order::Options,
    ) -> bool {
        target.program == source.program
            && ::cq_order::reference::check(source, &target.section, options)
    }
}
