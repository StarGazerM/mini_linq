//! RA prime correspondence: independent relational tableaux between opaque
//! chain nodes, with source-prefix containment and equal final sets.
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

use cq_elaborate::check;
use cq_lang_ra as ra;
use cq_lang_vo as valuation;
use cq_lang_vo::cq;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn compile(source: &valuation::Program) -> syn::Result<ra::Program> {
    let target = cq_elaborate::compile(source)?;
    check(source, &target)?;
    Ok(target)
}

fn cq(tokens: TokenStream) -> cq::Program {
    syn::parse2(tokens).unwrap()
}
fn vo(source: &cq::Program, steps: TokenStream) -> valuation::Program {
    syn::parse2(quote! { #source valuation { #steps } }).unwrap()
}
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
            cq::BodyItem::Equality(equality) => quote!(#equality;),
            _ => unreachable!("normal form"),
        });
    vo(source, quote!(#(#steps)*))
}
fn assert_plan(target: &ra::Program, plan: TokenStream) {
    let declarations = &target.plan.declarations;
    let expected: ra::Program =
        syn::parse2(quote! { relational { #(#declarations)* #plan } }).unwrap();
    assert_eq!(target, &expected);
}
fn chain() -> cq::Program {
    cq(quote! {
        struct Chain; relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(a, b), S(b, c);
    })
}

#[test]
fn generated_results_do_not_shadow_declared_relations() {
    let source = cq(quote! {
        struct Collision; relation r0(x: i32); relation r1(x: i32); relation r2(x: i32);
        r2(x) :- r0(x);
    });
    let target = compile(&coarse(&source)).unwrap();
    assert_plan(
        &target,
        quote! {
            r3 = rename r0 { x -> x }; r4 = project r3 keep { x }; output r4 as r2(x);
        },
    );
}

#[test]
fn coarse_plans_elaborate_relation_at_a_time() {
    let target = compile(&coarse(&chain())).unwrap();
    assert_plan(
        &target,
        quote! {
            r0 = rename L { a -> a }; r1 = rename R { a -> a, b -> b };
            r2 = natural_join r0 with r1; r3 = rename S { b -> b, c -> c };
            r4 = natural_join r2 with r3; r5 = project r4 keep { a, c };
            output r5 as answer(a, c);
        },
    );
}

#[test]
fn literals_are_relational_reads_and_repeats_are_relational_equalities() {
    let source = cq(quote! {
        struct Q; relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        answer(x, y) :- R(x, 5), S(x, x), if *x > 0, let y: i32 = *x + 1;
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let target = compile(&coarse(&normal)).unwrap();
    let selections: Vec<_> = target
        .plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Select(select) => Some(select),
            _ => None,
        })
        .collect();
    assert!(
        selections
            .iter()
            .any(|s| s.condition.constant_keys().is_some())
    );
    assert!(selections.iter().any(|s| {
        s.condition
            .equalities
            .iter()
            .any(|e| matches!(e.right, ra::EqualityValue::Variable(_)))
    }));
    assert!(
        !target
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Unit(_)))
    );
    assert!(
        target
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Bind(_)))
    );
    assert!(
        target
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Guard(_)))
    );
}

#[test]
fn a_computed_head_is_a_final_bind_with_variable_output() {
    let source = cq(quote! {
        struct Sum; relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        relation answer(value: i32); answer(*a + *c) :- R(a, b), S(b, c);
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let target = compile(&coarse(&normal)).unwrap();
    assert_eq!(target.plan.output.head.variables().count(), 1);
    let final_bind = target
        .plan
        .definitions
        .iter()
        .rposition(|d| matches!(d.operator, ra::Operator::Bind(_)))
        .unwrap();
    assert_eq!(final_bind + 2, target.plan.definitions.len());
}

#[test]
fn normalized_cross_atom_equalities_become_shared_join_headings() {
    let normal = cq::normal::normalize(&chain()).unwrap();
    let target = compile(&coarse(&normal)).unwrap();
    let analysis = ra::contract::check(&target).unwrap();
    for definition in &target.plan.definitions {
        if let ra::Operator::NaturalJoin(join) = &definition.operator {
            assert!(
                !analysis[&join.left.to_string()]
                    .heading
                    .is_disjoint(&analysis[&join.right.to_string()].heading)
            );
        }
    }
}

#[test]
fn fine_plans_keep_existential_projection_and_semijoin_meaning() {
    let source = cq(quote! {
        struct Q; relation R(a: i32, b: i32); relation S(b: i32, c: i32); relation T(a: i32, c: i32);
        answer(a, c) :- R(a, b), S(b, c), T(a, c);
    });
    let plans = [
        quote! { R@0(a, _); T@2(a, _); R@0(a, b); S@1(b, _); S@1(b, c); T@2(a, c); },
        quote! { S@1(b, c); R@0(a, b); T@2(a, c); },
        quote! { T@2(a, c); R@0(a, _); R@0(a, b); S@1(b, c); },
    ];
    for steps in plans {
        let target = compile(&vo(&source, steps)).unwrap();
        for seed in 0..64 {
            let database = databases::binary_database(&source, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                cq_oracle::evaluate_cq(&source, &database),
                "seed {seed}: {}",
                target.to_token_stream()
            );
        }
    }
}

#[test]
fn semijoin_reductions_restrict_clause_relations() {
    let source = chain();
    let target = compile(&vo(
        &source,
        quote! {
            reduce R@1 with L@0(a); reduce S@2 with R@1(_, b);
            reduce R@1 with S@2(b, _); reduce L@0 with R@1(a, _);
            L@0(a); R@1(a, b); S@2(b, c);
        },
    ))
    .unwrap();
    for seed in 0..64 {
        let database = databases::binary_database(&source, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            cq_oracle::evaluate_cq(&source, &database)
        );
    }
}

#[test]
fn relational_hoisting_is_accepted_but_moving_a_relation_later_is_rejected() {
    let source = cq(quote! {
        struct Q; relation R(a: i32); relation S(b: i32);
        answer(x, w) :- R(x), if 10 / *x > 0, S(w);
    });
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> x}; s = rename S {b -> w};
        j = natural_join r with s; g = guard j if (10 / *x > 0);
        output g as answer(x,w);
    }};
    check(&coarse(&source), &target).unwrap();
    let later_source = cq(quote! {
        struct Q; relation R(a: i32); relation S(b: i32);
        answer(x, w) :- R(x), S(w), if 10 / *x > 0;
    });
    let late: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> x}; g = guard r if (10 / *x > 0);
        s = rename S {b -> w}; j = natural_join g with s;
        output j as answer(x,w);
    }};
    ra::contract::check(&late).unwrap();
    assert!(
        check(&coarse(&later_source), &late)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
}

#[test]
fn opaque_chain_reorder_and_payload_mutations_are_rejected() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); answer(x,y) :- R(x), if *x != 0, let y:i32 = 10 / *x;
    });
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        b = bind r with y:i32 = (10 / *x); g = guard b if (*x != 0);
        output g as answer(x,y);
    }};
    ra::contract::check(&target).unwrap();
    assert!(
        check(&coarse(&source), &target)
            .unwrap_err()
            .to_string()
            .contains("Rust chain")
    );
    let erased = cq_elaborate::erase(&target, &source);
    assert!(matches!(erased.query.body[1], cq::BodyItem::Let(_)));
    assert!(matches!(erased.query.body[2], cq::BodyItem::Filter(_)));
    let mut changed = compile(&coarse(&source)).unwrap();
    let node = changed
        .plan
        .definitions
        .iter_mut()
        .find(|d| matches!(d.operator, ra::Operator::Guard(_)))
        .unwrap();
    let ra::Operator::Guard(guard) = &mut node.operator else {
        unreachable!()
    };
    guard.condition = syn::parse_quote!(*x != 1);
    assert!(check(&coarse(&source), &changed).is_err());
}

#[test]
fn guard_and_division_chain_keeps_the_complete_written_prefix() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation S(b:i32);
        answer(x,y) :- R(x), S(w), if *x != 0 && *w > 1, let y:i32 = 10 / *x;
    });
    let target = compile(&coarse(&source)).unwrap();
    let chain: Vec<_> = target
        .plan
        .definitions
        .iter()
        .filter(|d| matches!(d.operator, ra::Operator::Guard(_) | ra::Operator::Bind(_)))
        .collect();
    assert!(matches!(chain[0].operator, ra::Operator::Guard(_)));
    let ra::Operator::Bind(bind) = &chain[1].operator else {
        unreachable!()
    };
    assert_eq!(bind.input, chain[0].result);
}

#[test]
fn projection_cannot_discard_a_join_correlation() {
    let source = cq(quote! {
        struct Q; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        answer(x,z) :- R(x,y), S(y,z);
    });
    let broken: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        r = rename R {a -> x,b -> y}; p = project r keep {x};
        s = rename S {b -> y,c -> z}; j = natural_join p with s;
        o = project j keep {x,z}; output o as answer(x,z);
    }};
    ra::contract::check(&broken).unwrap();
    assert!(
        check(&coarse(&source), &broken)
            .unwrap_err()
            .to_string()
            .contains("tableau")
    );
}

#[test]
fn source_before_equality_cannot_move_after_rust() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); answer(x) :- R(x), x = 1, if 10 / *x > 0;
    });
    let broken: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        g = guard r if (10 / *x > 0); s = select g if (x = 1);
        output s as answer(x);
    }};
    ra::contract::check(&broken).unwrap();
    assert!(
        check(&coarse(&source), &broken)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
}

#[test]
fn differing_literal_spellings_are_not_assumed_unequal() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); answer(x) :- R(x), x = 1, x = 0x1;
    });
    let target = compile(&coarse(&source)).unwrap();
    let mut changed = target.clone();
    let ra::Operator::Select(select) = &mut changed.plan.definitions[1].operator else {
        panic!()
    };
    select.condition.equalities[1].right = ra::EqualityValue::Literal(syn::parse_quote!(2));
    assert!(check(&coarse(&source), &changed).is_err());
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
        assert!(check(&source, &changed).is_err());
        let mut changed = target;
        changed.plan.declarations[0].columns[0].ty = syn::parse_quote!(u32);
        assert!(check(&source, &changed).is_err());
    }
}

#[test]
fn executable_written_reference_does_not_hoist_a_later_literal() {
    // R={(0,0,6)} makes the written bind panic before the literal rejects 6.
    // The optimized target may suppress that panic; the oracle must preserve it.
    let surface = cq(quote! {
        struct Q; relation R(x:i32, value:i32, tag:i32);
        answer(x) :- R(x, 10 / *x, 5);
    });
    let normal = cq::normal::normalize(&surface).unwrap();
    let source = coarse(&normal);
    let written = cq_elaborate::compile_written(&source).unwrap();
    check(&source, &written).unwrap();
    let bind = written
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, ra::Operator::Bind(_)))
        .unwrap();
    let literal = written.plan.definitions.iter().position(|d| matches!(&d.operator, ra::Operator::Select(s) if s.condition.constant_keys().is_some())).unwrap();
    assert!(bind < literal);
    let optimized = compile(&source).unwrap();
    let bind = optimized
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, ra::Operator::Bind(_)))
        .unwrap();
    let literal = optimized.plan.definitions.iter().position(|d| matches!(&d.operator, ra::Operator::Select(s) if s.condition.constant_keys().is_some())).unwrap();
    assert!(literal < bind);
}

#[test]
fn written_reference_rejects_a_reordered_valuation() {
    let source = cq(quote! {
        struct Q; relation R(x:i32); relation S(y:i32);
        answer(x,y) :- R(x), S(y);
    });
    let reordered = vo(&source, quote! { S@1(y); R@0(x); });
    valuation::contract::check(&reordered).unwrap();
    assert!(
        cq_elaborate::compile_written(&reordered)
            .unwrap_err()
            .to_string()
            .contains("written reference")
    );
}

#[test]
fn correspondence_preserves_the_supplied_vo_before_constraints() {
    let source = cq(quote! {
        struct Q; relation R(x:i32); relation S(w:i32);
        answer(x,w) :- R(x), if 10 / *x > 1, S(w);
    });
    let written = coarse(&source);
    let hoisted = vo(&source, quote! { R@0(x); S@2(w); if 10 / *x > 1; });
    valuation::contract::check(&hoisted).unwrap();
    let later = cq_elaborate::compile_written(&written).unwrap();
    // R={0}, S={} makes the supplied hoisted VO nonpanicking, while this
    // otherwise source-CQ-correct target would panic by delaying S again.
    check(&written, &later).unwrap();
    assert!(
        check(&hoisted, &later)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
    check(&written, &compile(&hoisted).unwrap()).unwrap();
}

#[test]
fn leibniz_equality_class_can_use_either_generator_binding() {
    // Key is admissible: equality includes every field Rust can read. Retain
    // the expression's free-variable name, then solve its relational aliases.
    let surface = cq(quote! {
        struct Q; relation R(key:Key, marker:i32); relation S(key:Key);
        answer(y) :- R(x,z), S(x), let y:bool = x.flag;
    });
    let source = cq::normal::normalize(&surface).unwrap();
    let source = coarse(&source);
    let target = compile(&source).unwrap();
    let first = &target.plan.definitions[0];
    let ra::Operator::Rename(rename) = &first.operator else {
        panic!()
    };
    assert_eq!(rename.relation, "R");
    let explicit = target.plan.definitions.iter().any(|d| matches!(&d.operator, ra::Operator::Select(select) if select.condition.equalities.iter().any(|e| matches!(e.right, ra::EqualityValue::Variable(_)))));
    assert!(!explicit, "the shared join heading decides identity");
    let replacement: ra::Program = syn::parse_quote! { relational {
        relation R(key:Key, marker:i32); relation S(key:Key);
        s = rename S {key -> x}; r = rename R {key -> x, marker -> z};
        j = natural_join s with r; b = bind j with y:bool = (x.flag);
        p = project b keep {y}; output p as answer(y);
    }};
    ra::contract::check(&replacement).unwrap();
    check(&source, &replacement).unwrap();
}

#[test]
fn projection_before_rust_can_discard_variables_outside_its_free_variables() {
    let source = cq(quote! {
        struct Q; relation R(key:Key, marker:i32);
        answer(y) :- R(x,z), let y:bool = x.flag;
    });
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(key:Key, marker:i32);
        r = rename R {key -> x, marker -> z}; p = project r keep {x};
        b = bind p with y:bool = (x.flag); out = project b keep {y};
        output out as answer(y);
    }};
    ra::contract::check(&target).unwrap();
    check(&coarse(&source), &target).unwrap();
}
