//! Written, prefix-ranked and elimination orders, coarse and fine: every plan
//! passes the VO contract and erases to its query; prefix rankings place the
//! clauses in the pinned sequences (those of the clause-prefix planner they
//! replace).
use cq_lang_cq as cq;
use cq_lang_cq::BodyItem;
use cq_lang_vo as valuation;
use cq_order::{Heuristic, Options, Ranking, Split};
use quote::{ToTokens, quote};

fn cq(tokens: proc_macro2::TokenStream) -> cq::Program {
    syn::parse2(tokens).unwrap()
}

fn normal(tokens: proc_macro2::TokenStream) -> cq::Program {
    cq::normal::normalize(&cq(tokens)).unwrap()
}

fn triangle() -> cq::Program {
    cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(a, b, c) :- R(a, b), S(b, c), T(a, c);
    })
}

fn chain() -> cq::Program {
    cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(c: i32, d: i32);
        answer(x, w) :- R(x, y), S(y, z), T(z, w);
    })
}

/// Compile and check the schedule against its source.
fn plan(source: &cq::Program, heuristic: Heuristic, split: Split) -> valuation::Program {
    let options = Options { heuristic, split };
    let plan = cq_order::compile(source, &options).unwrap();
    valuation::contract::check(&plan).unwrap();
    assert!(cq_order::check(source, &plan, &options).is_ok());
    plan
}

/// The plan with the given steps over the source program.
fn expected(source: &cq::Program, steps: proc_macro2::TokenStream) -> valuation::Program {
    let program = &source;
    syn::parse2(quote! { #program valuation { #steps } }).unwrap()
}

/// The clause each coarse step draws on.
fn sequence(plan: &valuation::Program) -> Vec<usize> {
    let body = &plan.cq.query.body;
    let nth = |n: usize, kind: fn(&BodyItem) -> bool| {
        body.iter()
            .enumerate()
            .filter(|(_, item)| kind(item))
            .nth(n)
            .map(|(clause, _)| clause)
            .unwrap()
    };
    let (mut guards, mut lets, mut equalities) = (0, 0, 0);
    plan.valuation
        .steps
        .iter()
        .map(|step| match step {
            valuation::Step::Atom(subatom, _) => subatom.clause(),
            valuation::Step::Equality(..) => {
                equalities += 1;
                nth(equalities - 1, |item| matches!(item, BodyItem::Equality(_)))
            }
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
                R@0(x, __cq_0_1);
                __cq_0_1 = 5;
                S@2(__cq_1_0, z);
                R@3(__cq_2_0, __cq_2_1);
                x = __cq_2_0;
                x = __cq_2_1;
                if z > 1;
                let w: i32 = x + z;
            }
        )
    );
}

#[test]
fn written_fine_splits_each_atom_in_its_reference_position() {
    let triangle = triangle();
    assert_eq!(
        plan(&triangle, Heuristic::Written, Split::Fine),
        expected(
            &triangle,
            quote! { R@0(a, _); R@0(a, b); S@1(b, c); T@2(a, c); }
        )
    );
    // Guards and bindings follow every complete written relational predecessor.
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
                R@0(a, _); R@0(a, b);
                S@1(b, c); T@2(a, c); if a > 0; let d: i32 = a + c;
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
        // Connected ranking binds z from T before S. Both guards retain their
        // written order and wait until all three atoms have been checked.
        (
            cq(quote! {
                struct Q;
                relation R(a: i32, b: i32);
                relation S(a: i32, b: i32);
                relation T(a: i32, b: i32);
                answer(x) :- R(x, y), S(z, w), T(y, z), if x != z, if w > 1;
            }),
            &[0, 1, 2, 3, 4],
            &[0, 2, 1, 3, 4],
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
            &[0, 1, 2, 3],
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
                source.query.to_token_stream()
            );
        }
    }
}

#[test]
fn literals_are_relational_steps_after_their_atoms() {
    let normal = normal(quote! {
        struct Q; relation R(a:i32,b:i32); relation S(a:i32,b:i32); relation T(a:i32,b:i32);
        answer(x,z) :- R(x,7), S(7,z), T(z,x);
    });
    assert!(
        !normal
            .query
            .body
            .iter()
            .any(|item| matches!(item, BodyItem::Let(_)))
    );
    assert_eq!(
        normal
            .query
            .body
            .iter()
            .filter(|item| matches!(item, BodyItem::Equality(_)))
            .count(),
        4
    );
    for heuristic in [
        Heuristic::Written,
        Heuristic::Prefix(Ranking::Connected),
        Heuristic::Elimination,
    ] {
        for split in [Split::Coarse, Split::Fine] {
            plan(&normal, heuristic, split);
        }
    }
}

#[test]
fn elimination_orders_atoms_by_their_earliest_variable() {
    for source in [chain(), triangle()] {
        let order = cq::hypergraph::Analysis::of(&source).elimination_order;
        let position = |v: &String| order.iter().position(|o| o == v).unwrap();
        let coarse = plan(&source, Heuristic::Elimination, Split::Coarse);
        let earliest: Vec<usize> = coarse
            .valuation
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
        // Fine enumerates in reverse elimination order to connect shared keys.
        let fine = plan(&source, Heuristic::Elimination, Split::Fine);
        assert_eq!(
            atom_bound(&fine),
            order.into_iter().rev().collect::<Vec<_>>()
        );
    }
}

/// The variables the plan's subatoms introduce, in step order.
fn atom_bound(plan: &valuation::Program) -> Vec<String> {
    let analysis = valuation::contract::check(plan).unwrap();
    analysis
        .introduces
        .iter()
        .zip(&plan.valuation.steps)
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
    let atom_order: Vec<String> = order.iter().rev().filter(|v| *v != "k").cloned().collect();
    assert_eq!(atom_bound(&fine), atom_order);
    // No subatom lists `k` before the binding supplies it.
    let binding = fine
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, valuation::Step::Let(..)))
        .unwrap();
    let first_check = fine
        .valuation
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
    assert!(error.contains("R"), "{error}");
    let early = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        answer(x) :- if y > 1, R(x, y);
    });
    let error = cq_order::compile(&early, &options).unwrap_err().to_string();
    assert!(error.contains("y"), "{error}");
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
                for step in &plan.valuation.steps {
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

#[allow(dead_code)]
#[path = "../../../test-support/relational/cq_semantics.rs"]
mod cq_oracle;
#[allow(dead_code)]
#[path = "../../../test-support/relational/databases.rs"]
mod databases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/valuation_semantics.rs"]
mod evaluator;
#[path = "../../../test-support/relational/scenarios.rs"]
mod scenarios;
#[allow(dead_code)]
#[path = "../../../test-support/relational/values.rs"]
mod values;
#[test]
fn shared_scenarios_preserve_answers_and_reject_changed_schedules() {
    for case in scenarios::SCENARIOS {
        let source = cq::normal::normalize(&syn::parse_str(case.cq).unwrap()).unwrap();
        for split in [Split::Coarse, Split::Fine] {
            let options = Options {
                split,
                ..Default::default()
            };
            let target = cq_order::compile(&source, &options).unwrap();
            cq_order::check(&source, &target, &options).unwrap();
            let fixture_split = if case.name == "fine_triangle" {
                Split::Fine
            } else {
                Split::Coarse
            };
            if !case.name.ends_with("_reduced") && split == fixture_split {
                assert_eq!(
                    target,
                    syn::parse_str::<valuation::Program>(case.vo).unwrap(),
                    "{}",
                    case.name
                );
            }
            for seed in 0..4 {
                let database = databases::binary_database(&source, seed);
                assert_eq!(
                    evaluator::evaluate(&target, &database),
                    cq_oracle::evaluate_cq(&source, &database),
                    "{}",
                    case.name
                );
            }
            let mut changed = target;
            changed.valuation.steps.pop();
            assert!(cq_order::check(&source, &changed, &options).is_err());
        }
    }
}

#[test]
fn correspondence_diagnoses_each_obligation_and_accepts_free_rankings() {
    let source = chain();
    let coarse = Options::default();
    let good = expected(&source, quote! { R@0(x,y); S@1(y,z); T@2(z,w); });
    let reject = |s: &cq::Program, t: &valuation::Program, options: &Options, names: &[&str]| {
        let error = cq_order::check(s, t, options).unwrap_err().to_string();
        for name in names {
            assert!(error.contains(name), "{error}");
        }
    };
    let mut changed = source.clone();
    changed.query.body[0] = syn::parse_quote!(R(x, x));
    reject(&changed, &good, &coarse, &["R", "normal form"]);
    let malformed = expected(&source, quote! { R@9(x,y); S@1(y,z); T@2(z,w); });
    reject(&source, &malformed, &coarse, &["clause #9"]);
    let mut changed = good.clone();
    changed.cq.name = syn::parse_quote!(Changed);
    reject(&source, &changed, &coarse, &["Changed", "preserve"]);
    let partial = expected(&source, quote! { R@0(x,_); R@0(x,y); S@1(y,z); T@2(z,w); });
    valuation::contract::check(&partial).unwrap();
    reject(&source, &partial, &coarse, &["clause #0", "Coarse"]);
    let duplicated = expected(&source, quote! { R@0(x,y); R@0(x,y); S@1(y,z); T@2(z,w); });
    reject(&source, &duplicated, &coarse, &["clause #0", "Coarse"]);
    reject(
        &source,
        &good,
        &Options {
            split: Split::Fine,
            ..coarse
        },
        &["clause #0", "Fine"],
    );
    let reduced = expected(
        &source,
        quote! { reduce R@0 with S@1(y,_); R@0(x,y); S@1(y,z); T@2(z,w); },
    );
    reject(&source, &reduced, &coarse, &["clause #0", "reductions"]);
    let reverse = expected(&source, quote! { T@2(z,w); S@1(y,z); R@0(x,y); });
    valuation::contract::check(&reverse).unwrap();
    reject(&source, &reverse, &coarse, &["clause #2", "Written"]);
    let fine = Options {
        split: Split::Fine,
        ..coarse
    };
    let fine_reverse = expected(
        &source,
        quote! { T@2(_,w); T@2(z,w); S@1(_,z); S@1(y,z); R@0(_,y); R@0(x,y); },
    );
    valuation::contract::check(&fine_reverse).unwrap();
    reject(&source, &fine_reverse, &fine, &["clause #2", "Written"]);
    let alternative = expected(&source, quote! { S@1(y,z); R@0(x,y); T@2(z,w); });
    for heuristic in [
        Heuristic::Prefix(Ranking::Connected),
        Heuristic::Prefix(Ranking::GreedyWidth),
        Heuristic::Prefix(Ranking::Auto),
        Heuristic::Elimination,
    ] {
        let options = Options {
            heuristic,
            ..coarse
        };
        assert_ne!(cq_order::compile(&source, &options).unwrap(), alternative);
        cq_order::check(&source, &alternative, &options).unwrap();
    }
}

#[test]
fn partial_rust_steps_keep_cross_kind_order_and_all_written_relations() {
    for source in [
        normal(quote! {
            struct Q; relation R(a:i32); relation S(a:i32);
            answer(x,y) :- R(x), S(w), if *x != 0 && *w > 1, let y:i32 = 10 / *x;
        }),
        normal(quote! {
            struct Q; relation R(a:i32); relation S(a:i32,b:i32);
            answer(x,y) :- R(x), if *x != 0, S(10 / *x,y);
        }),
    ] {
        for heuristic in [
            Heuristic::Written,
            Heuristic::Prefix(Ranking::Connected),
            Heuristic::Prefix(Ranking::GreedyWidth),
            Heuristic::Prefix(Ranking::Auto),
            Heuristic::Elimination,
        ] {
            for split in [Split::Coarse, Split::Fine] {
                let planned = plan(&source, heuristic, split);
                let guard = planned
                    .valuation
                    .steps
                    .iter()
                    .position(|s| matches!(s, valuation::Step::Guard(..)))
                    .unwrap();
                let binding = planned
                    .valuation
                    .steps
                    .iter()
                    .position(|s| matches!(s, valuation::Step::Let(..)))
                    .unwrap();
                assert!(guard < binding, "{heuristic:?}/{split:?}");
                let mut changed = planned.clone();
                changed.valuation.steps.swap(guard, binding);
                assert!(valuation::contract::check(&changed).is_err());
            }
        }
    }
}

#[test]
fn before_condition_rejects_a_relational_step_moved_after_rust() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation S(a:i32);
        answer(x,y) :- R(x), S(w), if *x != 0, let y:i32 = 10 / *x;
    });
    let mut target = plan(&source, Heuristic::Written, Split::Coarse);
    target.valuation.steps.swap(1, 2);
    let error = valuation::contract::check(&target).unwrap_err().to_string();
    assert!(error.contains("Before"), "{error}");
}

#[test]
fn a_partial_subatom_does_not_count_as_a_written_relational_predecessor() {
    let source = cq(quote! {
        struct Q; relation R(a:i32,b:i32);
        answer(x) :- R(x,y), if host();
    });
    let target = expected(&source, quote! { R@0(x,_); if host(); R@0(x,y); });
    let error = valuation::contract::check(&target).unwrap_err().to_string();
    assert!(error.contains("Before"), "{error}");
}

#[test]
fn generic_join_checks_all_atom_aliases_before_the_next_variable_class() {
    let source = cq::normal::normalize(&triangle()).unwrap();
    let target = plan(&source, Heuristic::Prefix(Ranking::Connected), Split::Fine);
    let atoms: Vec<_> = target
        .valuation
        .steps
        .iter()
        .filter_map(|step| match step {
            valuation::Step::Atom(atom, _) => Some((
                atom.clause(),
                atom.variables()
                    .map(|(_, v)| v.to_string())
                    .collect::<Vec<_>>(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(atoms[0], (0, vec!["a".into()]));
    assert_eq!(atoms[1], (3, vec!["__cq_2_0".into()]));
    assert_eq!(atoms[2], (0, vec!["a".into(), "b".into()]));
    assert_eq!(atoms[3], (1, vec!["__cq_1_0".into()]));
}

#[test]
fn elimination_fine_binds_identity_aliases_through_the_first_ranked_atom() {
    let source = normal(quote! {
        struct Q;
        relation R(b:i32,a:i32); relation S(c:i32); relation T(b:i32,c:i32,k:i32);
        answer(k) :- R(b,a),S(c),T(b,c,k),if observe(b,c);
    });
    let target = plan(&source, Heuristic::Elimination, Split::Fine);
    let first = target.valuation.steps.first().unwrap();
    let valuation::Step::Atom(atom, _) = first else {
        panic!("a shared variable must be introduced by an atom")
    };
    assert_eq!(atom.relation, "T");
    assert_eq!(atom.variables().count(), 1);
    let guard = target
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, valuation::Step::Guard(..)))
        .unwrap();
    assert_eq!(guard, target.valuation.steps.len() - 1);
}

#[test]
fn partial_atoms_can_introduce_free_variables_before_the_full_before_check() {
    let source = cq(quote! {
        struct Q; relation R(x:Key,z:i32);
        answer(y) :- R(x,z), let y:bool=x.flag;
    });
    let changed = expected(&source, quote! { R@0(x,_); R@0(x,z); let y:bool=x.flag; });
    valuation::contract::check(&changed).unwrap();
    let incomplete = expected(&source, quote! { R@0(x,_); let y:bool=x.flag; R@0(x,z); });
    assert!(
        valuation::contract::check(&incomplete)
            .unwrap_err()
            .to_string()
            .contains("Before condition"),
        "projection must still check the complete written relational predecessor"
    );
    for heuristic in [
        Heuristic::Written,
        Heuristic::Prefix(Ranking::Connected),
        Heuristic::Prefix(Ranking::GreedyWidth),
        Heuristic::Elimination,
    ] {
        let target = plan(&source, heuristic, Split::Fine);
        let first = target
            .valuation
            .steps
            .iter()
            .find_map(|step| match step {
                valuation::Step::Atom(atom, _) if atom.variables().any(|(_, v)| v == "x") => {
                    Some(atom)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(first.variables().count(), 1);
    }
}

#[test]
fn shared_free_variables_can_be_supplied_by_any_admissible_atom() {
    let source = cq(quote! {
        struct Q; relation R(x:Key,z:i32); relation S(x:Key);
        answer(y) :- R(x,z),S(x),let y:bool=x.flag;
    });
    let changed = expected(&source, quote! { S@1(x); R@0(x,z); let y:bool=x.flag; });
    valuation::contract::check(&changed).unwrap();
    plan(
        &source,
        Heuristic::Prefix(Ranking::GreedyWidth),
        Split::Coarse,
    );
}
