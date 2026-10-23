//! Written, prefix-ranked and elimination orders, coarse and fine: every plan
//! passes the VO contract and erases to its query; prefix rankings place the
//! clauses in the pinned sequences (those of the clause-prefix planner they
//! replace).
use cq_lang_cq as cq;
use cq_lang_cq::BodyItem;
use cq_lang_vo as valuation;
use cq_order::{Heuristic, Options, Ranking, Split};
use cq_testkit::{
    corpus as scenarios, cq_semantics as cq_oracle, databases, valuation_semantics as evaluator,
};
use quote::{ToTokens, quote};

fn cq(tokens: proc_macro2::TokenStream) -> cq::Program {
    syn::parse2(tokens).unwrap()
}

fn normal(tokens: proc_macro2::TokenStream) -> cq::Program {
    cq::normal::normalize(&cq(tokens)).unwrap()
}

#[test]
fn explicit_order_rejects_dependency_conflicts_instead_of_ignoring_advice() {
    let source = normal(quote! {
        struct Q;
        relation R(a:i32);
        relation S(a:i32);
        answer(x) :- R(x), let y:i32 = *x + 1, S(y);
    });
    let error = cq_order::compile_explicit(&source, &[syn::parse_quote!(S), syn::parse_quote!(R)])
        .unwrap_err();
    assert!(error.to_string().contains("order advice conflicts"));
}

#[test]
fn explicit_order_preserves_source_and_records_authored_traversal() {
    let source = normal(quote! {
        struct Q;
        relation R(a:i32,b:i32);
        relation S(b:i32,c:i32);
        relation T(c:i32);
        answer(a,c) :- R(a,b), S(b,c), T(c);
    });
    let order = [
        syn::parse_quote!(T),
        syn::parse_quote!(S),
        syn::parse_quote!(R),
    ];
    let planned = cq_order::compile_explicit(&source, &order).unwrap();
    assert_eq!(planned.cq, source);
    let atoms: Vec<_> = planned
        .valuation
        .steps
        .iter()
        .filter_map(|step| {
            if let valuation::Step::Atom(atom, _) = step {
                Some(atom.relation.to_string())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(atoms, ["T", "S", "R"]);
    assert!(
        cq_order::compile_explicit(&source, &[syn::parse_quote!(T), syn::parse_quote!(T)]).is_err()
    );
    assert!(cq_order::compile_explicit(&source, &[syn::parse_quote!(Missing)]).is_err());
}

#[test]
fn explicit_order_does_not_move_opaque_predicates_before_their_domain() {
    let source = normal(quote! {
        struct Q;
        relation Part(k:i32,name:String);
        relation Keys(k:i32);
        answer(k) :- Part(k,name), if name.contains("green"), Keys(k);
    });
    let planned =
        cq_order::compile_explicit(&source, &[syn::parse_quote!(Part), syn::parse_quote!(Keys)])
            .unwrap();
    let part = planned
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, valuation::Step::Atom(atom, _) if atom.relation == "Part"))
        .unwrap();
    let guard = planned
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, valuation::Step::Guard(..)))
        .unwrap();
    assert!(part < guard);
}

#[test]
fn occurrence_qualified_order_uses_normal_body_positions() {
    let source = normal(quote! {
        struct Q;
        relation R(a:i32,b:i32);
        relation S(a:i32);
        answer(a,b,c) :- R(a,b), if b > 0, R(a,c), S(a);
    });
    let order = [
        syn::parse_quote!(S),
        syn::parse_quote!(R@2),
        syn::parse_quote!(R@0),
    ];
    let planned = cq_order::compile_explicit(&source, &order).unwrap();
    assert_eq!(planned.cq, source);
    let atoms: Vec<_> = planned
        .valuation
        .steps
        .iter()
        .filter_map(|step| {
            if let valuation::Step::Atom(atom, _) = step {
                Some(atom.clause())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(atoms, [3, 2, 0]);
    for (order, message) in [
        (vec![syn::parse_quote!(R)], "needs one top-level occurrence"),
        (
            vec![syn::parse_quote!(R@1)],
            "does not name that top-level positive atom",
        ),
        (
            vec![syn::parse_quote!(R@3)],
            "does not name that top-level positive atom",
        ),
        (
            vec![syn::parse_quote!(R@99)],
            "does not name that top-level positive atom",
        ),
        (
            vec![syn::parse_quote!(R@0), syn::parse_quote!(R@0)],
            "duplicate atom occurrence",
        ),
        (
            vec![syn::parse_quote!(S), syn::parse_quote!(S@3)],
            "duplicate atom occurrence",
        ),
    ] {
        assert!(
            cq_order::compile_explicit(&source, &order)
                .unwrap_err()
                .to_string()
                .contains(message)
        );
    }
}

#[test]
fn explicit_block_order_preserves_branches_and_checks_dependencies() {
    let source = normal(quote! {
        struct Q;
        relation R(a:i32,b:i32);
        relation S(a:i32);
        answer(x) :- S(x), { R(x,1) } | { R(x,2) };
    });
    let selector: cq_order::OrderAtom = syn::parse_quote!(block@1);
    assert_eq!(selector.relation, "block");
    assert_eq!(selector.occurrence, Some(1));
    let planned =
        cq_order::compile_explicit(&source, &[selector.clone(), syn::parse_quote!(S)]).unwrap();
    assert_eq!(planned.cq, source);
    assert!(matches!(&planned.valuation.steps[0],
        valuation::Step::Block(block, _) if block.source.clause() == 1));
    for order in [
        vec![syn::parse_quote!(block@0)],
        vec![syn::parse_quote!(block@99)],
        vec![syn::parse_quote!(R@1)],
        vec![selector.clone(), selector],
    ] {
        assert!(cq_order::compile_explicit(&source, &order).is_err());
    }
    assert!(syn::parse_str::<cq_order::OrderAtom>("block@").is_err());
    assert!(syn::parse_str::<cq_order::OrderAtom>("block@-1").is_err());
    let opaque = normal(quote! {
        struct Q;
        relation R(a:i32);
        relation S(a:i32);
        answer(x,y) :- R(x), { let y:i32 = *x + 1, S(y) }
            | { let y:i32 = *x + 2, S(y) };
    });
    let error =
        cq_order::compile_explicit(&opaque, &[syn::parse_quote!(block@1), syn::parse_quote!(R)])
            .unwrap_err();
    assert!(error.to_string().contains("order advice conflicts"));
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
    let (mut guards, mut lets, mut equalities, mut constraints) = (0, 0, 0, 0);
    let mut generators = 0;
    let mut negations = 0;
    plan.valuation
        .steps
        .iter()
        .map(|step| match step {
            valuation::Step::Atom(subatom, _) => subatom.clause(),
            valuation::Step::Equality(..) => {
                equalities += 1;
                nth(equalities - 1, |item| matches!(item, BodyItem::Equality(_)))
            }
            valuation::Step::Constraint(..) => {
                constraints += 1;
                nth(constraints - 1, |item| {
                    matches!(item, BodyItem::Constraint(_))
                })
            }
            valuation::Step::Negation(..) => {
                negations += 1;
                nth(negations - 1, |item| matches!(item, BodyItem::Negation(_)))
            }
            valuation::Step::Guard(..) => {
                guards += 1;
                nth(guards - 1, |item| matches!(item, BodyItem::Filter(_)))
            }
            valuation::Step::Let(..) => {
                lets += 1;
                nth(lets - 1, |item| matches!(item, BodyItem::Let(_)))
            }
            valuation::Step::For(..) => {
                generators += 1;
                nth(generators - 1, |item| matches!(item, BodyItem::For(_)))
            }
            valuation::Step::Block(b, _) => b.source.clause(),
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
        answer(x, z) :- R(x, 5), S(_, z), R(x, x), if { z > 1 }, let w: i32 = x + z;
    });
    assert_eq!(
        plan(&messy, Heuristic::Written, Split::Coarse),
        expected(
            &messy,
            quote! {
                R@0(x);
                S@1(__cq_1_0, z);
                R@2(x, __cq_2_1);
                x = __cq_2_1;
                if { z > 1 };
                let w: i32 = x + z;
            }
        )
    );
}

#[test]
fn optimizing_orders_place_constraints_at_the_first_available_binding() {
    let source: cq::Program = syn::parse_quote! {
        struct Early; relation R(a:i32); relation S(b:i32);
        answer(x,y) :- R(x), if opaque(x), if * x < cutoff, S(y);
    };
    for heuristic in [
        Heuristic::Prefix(Ranking::Connected),
        Heuristic::Elimination,
    ] {
        for split in [Split::Coarse, Split::Fine] {
            let options = Options { heuristic, split };
            let plan = cq_order::compile(&source, &options).unwrap();
            cq_order::check(&source, &plan, &options).unwrap();
            let binder = plan
                .valuation
                .steps
                .iter()
                .position(|s| matches!(s, valuation::Step::Atom(a, _) if a.clause() == 0))
                .unwrap();
            let constraint = plan
                .valuation
                .steps
                .iter()
                .position(|s| matches!(s, valuation::Step::Constraint(..)))
                .unwrap();
            let guard = plan
                .valuation
                .steps
                .iter()
                .position(|s| matches!(s, valuation::Step::Guard(..)))
                .unwrap();
            assert_eq!(constraint, binder + 1, "{heuristic:?}/{split:?}");
            assert_eq!(guard, constraint + 1, "{heuristic:?}/{split:?}");
        }
    }
}

#[test]
fn across_atom_sharing_is_a_join_and_only_same_atom_repetition_gets_identity() {
    let source: cq::Program = syn::parse_quote! {
        struct Shared; relation Customer(id:i32,segment:&str);
        relation Orders(id:i32,customer:i32,date:i32);
        answer(o,c,d) :- Customer(c,s), if *s == "BUILDING", Orders(o,c,d);
    };
    let nf = cq::normal::normalize(&source).unwrap();
    assert_eq!(nf, source);
    let vo = cq_order::compile(&nf, &Options::default()).unwrap();
    cq_order::check(&nf, &vo, &Options::default()).unwrap();
    let text = vo.valuation.to_token_stream().to_string().replace(' ', "");
    assert!(text.contains("Orders@2(o,c,d);"), "{text}");
    assert!(
        !text.contains("__cq_")
            && !vo
                .valuation
                .steps
                .iter()
                .any(|s| matches!(s, valuation::Step::Equality(e, _) if matches!(e.right,cq::EqualityValue::Variable(_))))
    );
    let mut dropped = vo.clone();
    let valuation::Step::Atom(a, _) = &mut dropped.valuation.steps[2] else {
        panic!()
    };
    a.columns[1] = valuation::Column::Dropped(Default::default());
    assert!(
        valuation::contract::check(&dropped)
            .unwrap_err()
            .to_string()
            .contains("bound")
    );
    let mut renamed = vo.clone();
    let valuation::Step::Atom(a, _) = &mut renamed.valuation.steps[2] else {
        panic!()
    };
    a.columns[1] = valuation::Column::Variable(syn::parse_quote!(__cq_wrong));
    assert!(cq_order::check(&nf, &renamed, &Options::default()).is_err());

    let repeated: cq::Program =
        syn::parse_quote! { struct Repeat; relation R(a:i32,b:i32); answer(x) :- R(x,x); };
    let nf = cq::normal::normalize(&repeated).unwrap();
    assert_eq!(nf.query.body.len(), 2);
    let vo = cq_order::compile(&nf, &Options::default()).unwrap();
    let text = vo.valuation.to_token_stream().to_string().replace(' ', "");
    assert_eq!(text, "valuation{R@0(x,__cq_0_1);x=__cq_0_1;}");
    assert_eq!(cq::normal::normalize(&nf).unwrap(), nf);
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
        answer(a, b, c) :- R(a, b), S(b, c), T(a, c), if { a > 0 }, let d: i32 = a + c;
    });
    assert_eq!(
        plan(&guarded, Heuristic::Written, Split::Fine),
        expected(
            &guarded,
            quote! {
                R@0(a, _); R@0(a, b);
                S@1(b, c); T@2(a, c); if { a > 0 }; let d: i32 = a + c;
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
                answer(x) :- R(x, y), S(z, w), T(y, z), if { x != z }, if { w > 1 };
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
                answer(x, z) :- R(x, y), S(y, z), T(x, z), if { y > x };
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
fn literals_stay_in_atoms_without_relational_steps() {
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
        0
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
fn fine_order_places_literal_only_occurrences_before_dependent_bindings() {
    let normal = normal(quote! {
        struct Q; relation R(a:i32,b:i32); relation answer(value:i32);
        answer(2) :- R(7,8);
    });
    for heuristic in [
        Heuristic::Written,
        Heuristic::Prefix(Ranking::Connected),
        Heuristic::Prefix(Ranking::GreedyWidth),
        Heuristic::Elimination,
    ] {
        plan(&normal, heuristic, Split::Fine);
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
        answer(x, y) :- R(x), let k: i32 = x + 1, S(k, y), if { y > k };
    });
    assert_eq!(
        plan(&source, Heuristic::Elimination, Split::Coarse),
        expected(
            &source,
            quote! { R@0(x); let k: i32 = x + 1; S@2(k, y); if { y > k }; }
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
            answer(w, y) :- R(w, x), S(x, y), T(y, z), U(z, w), let k: i32 = w * 2, if { k > y };
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
            answer(x,y) :- R(x), S(w), if { *x != 0 && *w > 1 }, let y:i32 = 10 / *x;
        }),
        normal(quote! {
            struct Q; relation R(a:i32); relation S(a:i32,b:i32);
            answer(x,y) :- R(x), if { *x != 0 }, S(10 / *x,y);
        }),
    ] {
        for heuristic in [
            Heuristic::Written,
            Heuristic::Prefix(Ranking::Connected),
            Heuristic::Prefix(Ranking::GreedyWidth),
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
        answer(x,y) :- R(x), S(w), if { *x != 0 }, let y:i32 = 10 / *x;
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
    assert_eq!(atoms[1], (2, vec!["a".into()]));
    assert_eq!(atoms[2], (0, vec!["a".into(), "b".into()]));
    assert_eq!(atoms[3], (1, vec!["b".into()]));
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

#[test]
fn filtered_order_starts_with_point_then_grows_connected_before_disconnected_ranges() {
    let source = normal(quote! {
        struct Q; relation Customer(c:i32,n:i32); relation Orders(o:i32,c:i32,d:i32);
        relation Lineitem(o:i32,s:i32); relation Supplier(s:i32,n:i32);
        relation Nation(n:i32,r:i32); relation Region(r:i32,name:i32);
        answer(c,o,s) :- Customer(c,n), Orders(o,c,d), if *d >= 10,
            Lineitem(o,s), Supplier(s,n), Nation(n,r), Region(r,name), if *name == 1;
    });
    let target = plan(&source, Heuristic::Filtered, Split::Coarse);
    let atoms: Vec<_> = target
        .valuation
        .steps
        .iter()
        .filter_map(|step| match step {
            valuation::Step::Atom(a, _) => Some(a.relation.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        atoms,
        [
            "Region", "Nation", "Customer", "Orders", "Lineitem", "Supplier"
        ]
    );
    let written = plan(&source, Heuristic::Written, Split::Coarse);
    assert!(
        cq_order::check(
            &source,
            &written,
            &Options {
                heuristic: Heuristic::Filtered,
                split: Split::Coarse
            }
        )
        .is_err()
    );
}

#[test]
fn filtered_order_preserves_opaque_before_and_rust_chain() {
    let source = normal(quote! {
        struct Q; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        answer(a,c) :- R(a,b), if opaque(a), S(b,c), if *c == 1;
    });
    let target = plan(&source, Heuristic::Filtered, Split::Coarse);
    valuation::contract::check(&target).unwrap();
    let guard = target
        .valuation
        .steps
        .iter()
        .position(|s| matches!(s, valuation::Step::Guard(..)))
        .unwrap();
    let before = target
        .valuation
        .steps
        .iter()
        .position(|s| matches!(s, valuation::Step::Atom(a, _) if a.relation == "R"))
        .unwrap();
    assert!(before < guard);
}

#[test]
fn filtered_order_records_independent_dimension_branches() {
    let source = normal(quote! {
        struct Q;
        relation L(o:i32,p:i32,s:i32);
        relation PS(p:i32,s:i32);
        relation P(p:i32,n:i32);
        relation S(s:i32,n:i32);
        relation O(o:i32);
        relation N(n:i32);
        answer(o,p,s) :- L(o,p,s), PS(p,s), P(p,label), if *label == 7,
            S(s,n), O(o), N(n);
    });
    let target = plan(&source, Heuristic::Filtered, Split::Coarse);
    let groups = target.valuation.join_branches.as_ref().unwrap();
    assert_eq!(groups.branches.len(), 1);
    let relations: Vec<_> = groups.branches[0]
        .clauses()
        .into_iter()
        .map(|c| {
            let BodyItem::Positive { atom } = &source.query.body[c] else {
                panic!()
            };
            atom.relation.to_string()
        })
        .collect();
    assert_eq!(relations, ["PS", "S", "N"]);
    let reparsed: valuation::Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(reparsed, target);
    let mut wrong = target.clone();
    wrong.valuation.join_branches.as_mut().unwrap().branches[0]
        .columns
        .clear();
    assert!(valuation::contract::check(&wrong).is_err());
    let mut omitted = target.clone();
    omitted.valuation.join_branches = None;
    assert!(
        cq_order::check(
            &source,
            &omitted,
            &Options {
                heuristic: Heuristic::Filtered,
                split: Split::Coarse
            }
        )
        .is_err()
    );
    assert!(valuation::check_rewrite(&target, &omitted).is_err());
}

#[test]
fn dimension_branches_do_not_cross_opaque_rust() {
    let source = normal(quote! {
        struct Q; relation L(o:i32,p:i32,s:i32); relation P(p:i32);
        relation S(s:i32,n:i32); relation N(n:i32); relation O(o:i32);
        answer(o,p,s) :- L(o,p,s), P(p), S(s,n), if { 10 / *n > 0 }, N(n), O(o);
    });
    let target = plan(&source, Heuristic::Filtered, Split::Coarse);
    assert!(target.valuation.join_branches.is_none());
}

#[test]
fn dimension_ears_keep_a_customer_supplier_cycle_on_the_spine() {
    let source = normal(quote! {
        struct Q; relation L(o:i32,p:i32,s:i32); relation O(o:i32,c:i32);
        relation PS(p:i32,s:i32); relation S(s:i32,n:i32); relation P(p:i32);
        relation C(c:i32,n:i32); relation N(n:i32);
        answer(o,p,s,n) :- L(o,p,s), O(o,c), PS(p,s), S(s,n), P(p), C(c,n), N(n);
    });
    let target = plan(&source, Heuristic::Filtered, Split::Coarse);
    let groups = target.valuation.join_branches.as_ref().unwrap();
    let relations: Vec<Vec<_>> = groups
        .branches
        .iter()
        .map(|b| {
            b.clauses()
                .into_iter()
                .map(|c| {
                    let BodyItem::Positive { atom } = &source.query.body[c] else {
                        panic!()
                    };
                    atom.relation.to_string()
                })
                .collect()
        })
        .collect();
    assert_eq!(relations, [vec!["PS", "P"], vec!["S", "N"]]);
}

#[test]
fn filtered_dimension_branch_precedes_a_weaker_fact_neighbor() {
    let source = normal(quote! {
        struct Q; relation L(o:i32,p:i32,s:i32); relation O(o:i32,d:i32);
        relation P(p:i32,b:i32); relation S(s:i32,n:i32); relation N(n:i32,k:i32);
        answer(o,p,s) :- L(o,p,s), O(o,d), if *d > 5,
            P(p,b), if *b == 7, S(s,n), N(n,k), if *k == 1;
    });
    let target = plan(&source, Heuristic::Filtered, Split::Coarse);
    let atom_order: Vec<_> = target
        .valuation
        .steps
        .iter()
        .filter_map(|step| {
            if let valuation::Step::Atom(a, _) = step {
                Some(a.relation.to_string())
            } else {
                None
            }
        })
        .collect();
    assert!(atom_order.iter().position(|r| r == "S") < atom_order.iter().position(|r| r == "O"));
}

#[test]
fn generator_planners_preserve_chain_dependencies_and_before() {
    let source = normal(quote! {
        struct Q; relation R(a: i32); relation S(b: i32); relation answer(b: i32);
        answer(b) :- R(a), for b in 0..a, S(b), if { b > 0 };
    });
    for heuristic in [
        Heuristic::Written,
        Heuristic::Filtered,
        Heuristic::Prefix(Ranking::Connected),
        Heuristic::Prefix(Ranking::GreedyWidth),
        Heuristic::Elimination,
    ] {
        for split in [Split::Coarse, Split::Fine] {
            let planned = plan(&source, heuristic, split);
            assert_eq!(
                syn::parse2::<valuation::Program>(planned.to_token_stream()).unwrap(),
                planned
            );
            let generator = planned
                .valuation
                .steps
                .iter()
                .position(|s| matches!(s, valuation::Step::For(..)))
                .unwrap();
            assert!(generator > 0);
            let mut changed = planned.clone();
            changed.valuation.steps.swap(0, generator);
            assert!(cq_order::check(&source, &changed, &Options { heuristic, split }).is_err());
        }
    }
}

#[test]
fn filtered_priority_uses_literal_occurrences_without_extra_steps() {
    let source = normal(quote! {
        struct Q; relation R(a:i32); relation S(a:i32,b:i32);
        answer(x) :- R(x), S(x,7);
    });
    for split in [Split::Coarse, Split::Fine] {
        let target = plan(&source, Heuristic::Filtered, split);
        assert_eq!(target.cq, source);
        assert_eq!(target.valuation.steps.len(), 2);
        assert!(matches!(&target.valuation.steps[0],
            valuation::Step::Atom(atom, _) if atom.clause() == 1 && atom.columns.len() == 1));
    }
}

#[test]
fn ready_negation_moves_before_opaque_rust_but_never_before_its_binder() {
    let source = cq(quote! {
        struct Negative; relation R(a:i32); relation K(a:i32);
        answer(x) :- R(x), if opaque(x), !K(x), let y = x;
    });
    for heuristic in [
        Heuristic::Filtered,
        Heuristic::Prefix(Ranking::Connected),
        Heuristic::Prefix(Ranking::GreedyWidth),
        Heuristic::Elimination,
    ] {
        for split in [Split::Coarse, Split::Fine] {
            let planned = plan(&source, heuristic, split);
            assert_eq!(sequence(&planned), [0, 2, 1, 3], "{heuristic:?}/{split:?}");
        }
    }
    for split in [Split::Coarse, Split::Fine] {
        let written = plan(&source, Heuristic::Written, split);
        assert_eq!(sequence(&written), [0, 1, 2, 3]);
    }
}

#[test]
fn negation_retains_earlier_rust_binders_and_all_wildcards_require_none() {
    let source = cq(quote! {
        struct Negative; relation R(a:i32); relation K(a:i32,b:i32);
        answer(x) :- R(x), let y = *x + 1, !K(y,_), if opaque(x), !K(_,_);
    });
    for split in [Split::Coarse, Split::Fine] {
        let planned = plan(&source, Heuristic::Prefix(Ranking::Connected), split);
        assert!(matches!(
            planned.valuation.steps[0],
            valuation::Step::Negation(..)
        ));
        let binding = planned
            .valuation
            .steps
            .iter()
            .position(|s| matches!(s, valuation::Step::Let(..)))
            .unwrap();
        let dependent = planned.valuation.steps.iter().position(|s|
            matches!(s, valuation::Step::Negation(n, _) if n.atom.variables().any(|v| v == "y"))).unwrap();
        assert!(dependent > binding);
        let guard = planned
            .valuation
            .steps
            .iter()
            .position(|s| matches!(s, valuation::Step::Guard(..)))
            .unwrap();
        assert!(guard > dependent);
    }
}

#[test]
fn ordering_negation_preserves_reference_answers_for_constants_repeats_and_wildcards() {
    for args in ["x,x", "x,1", "x,_", "_,_", "1,_"] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct Negative; relation R(a:i32); relation K(a:i32,b:i32);
             answer(x) :- R(x), !K({args});"
        ))
        .unwrap();
        for split in [Split::Coarse, Split::Fine] {
            let planned = plan(&source, Heuristic::Prefix(Ranking::Connected), split);
            for seed in 0..16 {
                let database = databases::binary_database(&source, seed);
                assert_eq!(
                    evaluator::evaluate(&planned, &database),
                    cq_oracle::evaluate_cq(&source, &database),
                    "{args}/{split:?}/{seed}"
                );
            }
        }
    }
}
