use cq_datalog_incremental as seminaive;
use cq_datalog_stratify as stratify;
use cq_lang_datalog_incremental::{self as incremental, source, strata};
use quote::{ToTokens, quote};
use source::BodyItem;
fn source() -> source::Program {
    syn::parse_quote! {
        struct Paths;
        relation Edge(x:i32,y:i32) = (edges);
        relation Reach(x:i32,y:i32); relation Other(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y);
        Other(*x,*y) :- Reach(x,y);
        Reach(*x,*z) :- Other(x,y), Reach(y,z);
    }
}
fn roundtrip<L: cq_stage::Language + std::fmt::Debug + PartialEq>(p: &L) {
    p.check().unwrap();
    assert_eq!(p, &syn::parse2::<L>(p.to_token_stream()).unwrap());
}
#[test]
fn both_passes_preserve_complete_round_trippable_programs() {
    let source = source();
    let strata = stratify::compile(&source).unwrap();
    let variants = seminaive::compile(&strata).unwrap();
    stratify::check(&source, &strata).unwrap();
    seminaive::check(&strata, &variants).unwrap();
    roundtrip(&source);
    roundtrip(&strata);
    roundtrip(&variants);
    assert_eq!(strata.components.len(), 1);
    assert_eq!(variants.variants.len(), 4);
    assert_eq!(variants.deltas.len(), 2);
}

#[test]
fn delta_first_records_and_checks_source_occurrence_permutations() {
    let strata = stratify::compile(&source()).unwrap();
    let planned =
        seminaive::compile_with_driver(&strata, seminaive::DriverPolicy::DeltaFirst).unwrap();
    roundtrip(&planned);
    let moved = planned
        .variants
        .iter()
        .find(|variant| variant.rule == 2 && variant.delta == Some(1))
        .unwrap();
    assert_eq!(moved.order, [1, 0]);
    let BodyItem::Positive { atom } = &moved.query.body[0] else {
        panic!("delta must drive the variant")
    };
    assert_eq!(
        atom.relation,
        planned
            .deltas
            .iter()
            .find(|delta| delta.relation == "Reach")
            .unwrap()
            .name
    );
    let mut invalid = planned.clone();
    invalid
        .variants
        .iter_mut()
        .find(|variant| variant.rule == 2 && variant.delta == Some(1))
        .unwrap()
        .order = vec![0, 1];
    assert!(seminaive::check(&strata, &invalid).is_err());
}

#[test]
fn delta_first_connects_the_remaining_prefix_and_checks_the_full_permutation() {
    let source: source::Program = syn::parse_quote! {
        struct Connected;
        relation E(x:i32,y:i32) = (edges);
        relation Methods(m:i32) = (methods);
        relation Link(m:i32,x:i32,y:i32) = (links);
        relation R(x:i32,y:i32);
        output R;
        R(*x,*y) :- E(x,y);
        R(*x,*z) :- Methods(m), Link(m,x,y), R(y,z);
    };
    let strata = stratify::compile(&source).unwrap();
    let planned =
        seminaive::compile_with_driver(&strata, seminaive::DriverPolicy::DeltaFirst).unwrap();
    assert_eq!(planned.variants[1].order, [2, 1, 0]);
    roundtrip(&planned);
    for order in [vec![2, 2, 0], vec![2, 1], vec![3, 1, 0]] {
        let mut invalid = planned.clone();
        invalid.variants[1].order = order;
        assert!(seminaive::check(&strata, &invalid).is_err());
    }
    // A different valid certificate is accepted; checking does not rerun ranking.
    let mut alternative = planned.clone();
    alternative.variants[1].order = vec![2, 0, 1];
    let body = &mut alternative.variants[1].query.body;
    let saved = body[1].clone();
    body[1] = body[2].clone();
    body[2] = saved;
    seminaive::check(&strata, &alternative).unwrap();
}

#[test]
fn delta_first_does_not_cross_rust_or_negation() {
    let source: source::Program = syn::parse_quote! {
        struct Barriers;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Edge(x,y), where x != y, Reach(y,z);
        Reach(*x,*z) :- Edge(x,y), !Blocked(x), Reach(y,z);
    };
    let strata = stratify::compile(&source).unwrap();
    let written = seminaive::compile(&strata).unwrap();
    let planned =
        seminaive::compile_with_driver(&strata, seminaive::DriverPolicy::DeltaFirst).unwrap();
    assert_eq!(planned, written);
    roundtrip(&planned);
}

fn plans(tokens: proc_macro2::TokenStream) -> Vec<seminaive::RulePlan> {
    use syn::parse::{Parse, Parser};
    let parser = |input: syn::parse::ParseStream<'_>| {
        let mut plans = Vec::new();
        while !input.is_empty() {
            let content;
            syn::parenthesized!(content in input);
            let rule = content.parse()?;
            content.parse::<syn::Token![,]>()?;
            let delta = if content.peek(syn::Token![@]) {
                content.parse::<syn::Token![@]>()?;
                let delta = content.parse()?;
                content.parse::<syn::Token![,]>()?;
                Some(delta)
            } else {
                None
            };
            let order = content.parse_terminated(seminaive::OrderAtom::parse, syn::Token![,])?;
            plans.push(seminaive::RulePlan {
                rule,
                delta,
                order: order.into_iter().collect(),
                smallest: Vec::new(),
            });
        }
        Ok(plans)
    };
    parser.parse2(tokens).unwrap()
}

fn labelled() -> source::Program {
    syn::parse_quote! {
        struct Heaps;
        relation Alloc(h:i32,t:i32) = (alloc);
        relation Component(a:i32,c:i32) = (component);
        relation Super(c:i32,t:i32) = (supers);
        relation Edge(x:i32,y:i32) = (edges);
        relation Compat(h:i32,b:i32); relation Reach(x:i32,y:i32);
        output Compat, Reach;
        Helper: Compat(*h,*b) :- Alloc(h,t), Alloc(b,a), Component(a,c), Super(c,t), where a != 0;
        Reach(*x,*y) :- Edge(x,y);
        Step: Reach(*x,*z) :- Edge(x,y), Reach(y,z), Edge(z,x);
    }
}

#[test]
fn labels_round_trip_and_must_be_unique() {
    let program = labelled();
    roundtrip(&program);
    assert_eq!(program.labels.iter().flatten().count(), 2);
    let duplicate = program
        .to_token_stream()
        .to_string()
        .replace("Step", "Helper");
    let duplicate: source::Program = syn::parse_str(&duplicate).unwrap();
    assert!(cq_stage::Language::check(&duplicate).is_err());
}

#[test]
fn plan_advice_orders_the_named_version_with_cq_selectors() {
    let strata = stratify::compile(&labelled()).unwrap();
    let advice = plans(quote!(
        (Helper, Super, Component, Alloc@0)
        (Step, @Reach, Reach, Edge@2)
    ));
    let planned =
        seminaive::compile_with_advice(&strata, seminaive::DriverPolicy::DeltaFirst, &advice)
            .unwrap();
    roundtrip(&planned);
    seminaive::check(&strata, &planned).unwrap();
    // Named atoms first in advice order, the unnamed Alloc@1 next; the guard stays put.
    assert_eq!(
        planned.variants.iter().find(|v| v.rule == 0).unwrap().order,
        [3, 2, 0, 1, 4]
    );
    // Advice replaces delta_first's choice for the selected version only.
    assert_eq!(
        planned.variants.iter().find(|v| v.rule == 2).unwrap().order,
        [1, 2, 0]
    );
    let unadvised =
        seminaive::compile_with_driver(&strata, seminaive::DriverPolicy::DeltaFirst).unwrap();
    assert_eq!(
        unadvised
            .variants
            .iter()
            .find(|v| v.rule == 2)
            .unwrap()
            .order,
        [1, 0, 2]
    );
    for invalid in [
        quote!((Missing, Alloc @ 0)),
        quote!((Helper, Alloc)),
        quote!((Helper, Super, Super)),
        quote!((Helper, Alloc @ 1, Alloc @ 1)),
        quote!((Step, @Edge@0, Edge@0)),
        quote!((Helper, @Super, Super)),
        quote!((Step, @Reach@1(y), Reach@1)),
        quote!((Helper, Super)(Helper, Component)),
    ] {
        assert!(
            seminaive::compile_with_advice(
                &strata,
                seminaive::DriverPolicy::DeltaFirst,
                &plans(invalid)
            )
            .is_err()
        );
    }
}
#[test]
fn stages_reject_wrong_input_and_target_shapes() {
    let s = source();
    let t = stratify::compile(&s).unwrap();
    let u = seminaive::compile(&t).unwrap();
    assert!(syn::parse2::<source::Program>(t.to_token_stream()).is_err());
    assert!(syn::parse2::<source::Program>(u.to_token_stream()).is_err());
    assert!(syn::parse2::<strata::Program>(s.to_token_stream()).is_err());
    assert!(syn::parse2::<strata::Program>(u.to_token_stream()).is_err());
    assert!(syn::parse2::<incremental::Program>(s.to_token_stream()).is_err());
    assert!(syn::parse2::<incremental::Program>(t.to_token_stream()).is_err());
    let bad = u
        .to_token_stream()
        .to_string()
        .replace("seminaive_covering", "naive");
    assert!(syn::parse_str::<incremental::Program>(&bad).is_err());
}
#[test]
fn authored_alternatives_are_checked_without_replaying_compilers() {
    let s: source::Program = syn::parse_quote! {
        struct Independent; relation E(x:i32) = (e); relation A(x:i32); relation B(x:i32);
        output A; A(*x) :- E(x); B(*x) :- E(x);
    };
    let t: strata::Program =
        syn::parse2(quote!(#s strata {component [B]; component [A];})).unwrap();
    stratify::check(&s, &t).unwrap();
    let u: incremental::Program = syn::parse2(quote!(#t incremental {
        scheme seminaive_covering; delta B = FreshB; delta A = FreshA;
        variant 1 seed {B(*x) :- E(x);} variant 0 seed {A(*x) :- E(x);}
    }))
    .unwrap();
    seminaive::check(&t, &u).unwrap();
}
#[test]
fn checks_reject_source_edits_and_missing_decisions() {
    let s = source();
    let t = stratify::compile(&s).unwrap();
    let u = seminaive::compile(&t).unwrap();
    let mut changed = t.clone();
    changed.source.outputs = vec![syn::parse_quote!(Other)];
    strata::contract::check(&changed).unwrap();
    assert!(stratify::check(&s, &changed).is_err());
    let mut changed = u.clone();
    changed.strata.source.outputs = vec![syn::parse_quote!(Other)];
    incremental::contract::check(&changed).unwrap();
    assert!(seminaive::check(&t, &changed).is_err());
    let mut changed = u.clone();
    changed.deltas.pop();
    assert!(seminaive::check(&t, &changed).is_err());
    let mut changed = u;
    changed.variants.pop();
    assert!(seminaive::check(&t, &changed).is_err());
}
#[test]
fn delta_names_avoid_all_source_identifiers_but_are_not_canonical() {
    let source: source::Program = syn::parse_quote! {
        observe = (__cq_datalog_delta_0);
        struct Names;
        relation E(x:i32) = (__cq_datalog_delta_1);
        relation A(x:i32);
        output A;
        A(*x) :- E(x);
        A(*x) :- A(x);
    };
    let strata = stratify::compile(&source).unwrap();
    let mut delta = seminaive::compile(&strata).unwrap();
    assert_ne!(delta.deltas[0].name, "__cq_datalog_delta_0");
    assert_ne!(delta.deltas[0].name, "__cq_datalog_delta_1");
    delta.deltas[0].name = syn::parse_quote!(AuthoredDelta);
    let variant = &mut delta.variants[1];
    let BodyItem::Positive { atom } = &mut variant.query.body[0] else {
        unreachable!()
    };
    atom.relation = syn::parse_quote!(AuthoredDelta);
    seminaive::check(&strata, &delta).unwrap();
}

#[test]
fn negative_dependencies_force_completed_lower_components() {
    let source: source::Program = syn::parse_quote! {
        struct Stratified;
        relation E(x:i32) = (edges); relation A(x:i32); relation B(x:i32);
        output B;
        B(*x) :- E(x), !A(x);
        A(*x) :- E(x), where x == 1;
    };
    let strata = stratify::compile(&source).unwrap();
    let expected: Vec<Vec<syn::Ident>> =
        vec![vec![syn::parse_quote!(A)], vec![syn::parse_quote!(B)]];
    assert_eq!(&strata.components, &expected);
    let mut reversed = strata.clone();
    reversed.components.reverse();
    assert!(stratify::check(&source, &reversed).is_err());
    let delta = seminaive::compile(&strata).unwrap();
    seminaive::check(&strata, &delta).unwrap();
    assert!(delta.variants.iter().all(|v| v.delta.is_none()));
}

#[test]
fn multioutput_and_constant_negation_survive_both_schedules() {
    let source: source::Program = syn::parse_quote! {
        struct Constants;
        relation E(x:i32) = (edges);
        relation Modifier(kind:String,x:i32) = (modifiers);
        relation A(x:i32); relation B(x:i32);
        output B, A;
        A(*x) :- E(x), !Modifier("abstract",x);
        B(*x + 1) :- A(x), !Modifier("static",_);
    };
    let strata = stratify::compile(&source).unwrap();
    let delta = seminaive::compile(&strata).unwrap();
    roundtrip(&source);
    roundtrip(&strata);
    roundtrip(&delta);
    assert_eq!(strata.source.outputs, source.outputs);
    assert_eq!(delta.strata.source.outputs, source.outputs);
    let mut reordered = strata.clone();
    reordered.source.outputs.reverse();
    strata::contract::check(&reordered).unwrap();
    assert!(stratify::check(&source, &reordered).is_err());
    let mut reordered = delta.clone();
    reordered.strata.source.outputs.reverse();
    assert!(seminaive::check(&strata, &reordered).is_err());
    let mut changed = delta.clone();
    let BodyItem::Negation(negative) = &mut changed.variants[0].query.body[1] else {
        unreachable!()
    };
    negative.atom.args[0] = syn::parse_quote!("static");
    assert!(incremental::contract::check(&changed).is_err());
}

#[test]
fn edb_outputs_preserve_empty_and_mixed_schedules() {
    for text in [
        "struct P; relation E(x:i32) = (e); output E;",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output E, A; A(*x) :- E(x);",
    ] {
        let source: source::Program = syn::parse_str(text).unwrap();
        let strata = stratify::compile(&source).unwrap();
        let delta = seminaive::compile(&strata).unwrap();
        stratify::check(&source, &strata).unwrap();
        seminaive::check(&strata, &delta).unwrap();
        assert_eq!(delta.strata.source.outputs, source.outputs);
        assert_eq!(delta.deltas.len(), source.rules.len());
        assert_eq!(
            delta,
            syn::parse2::<incremental::Program>(delta.to_token_stream()).unwrap()
        );
    }
}

#[test]
fn if_let_is_lowered_before_seminaive_variants_and_keeps_written_positions() {
    let source: source::Program = syn::parse_quote! {
        struct Matched;
        relation Seed(o:Option<i32>) = (seeds);
        relation Link(x:i32,y:i32) = (links);
        relation Reach(x:i32);
        output Reach;
        Reach(x) :- Seed(o), if let Some(x) = *o;
        Reach(*z) :- Reach(x), if let Some(y) = x.checked_add(1), Link(y,z);
    };
    let source: source::Program =
        syn::parse2(cq_datalog_surface_if_let::compile(&source.to_token_stream()).unwrap())
            .unwrap();
    let strata = stratify::compile(&source).unwrap();
    let variants = seminaive::compile(&strata).unwrap();
    stratify::check(&source, &strata).unwrap();
    seminaive::check(&strata, &variants).unwrap();
    for rule in &strata.source.rules {
        assert!(matches!(rule.body[1], BodyItem::For(_)));
    }
    roundtrip(&strata);
    roundtrip(&variants);
    let variants = variants.variants;
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].delta, None);
    assert_eq!(variants[1].delta, Some(0));
    assert!(matches!(variants[1].query.body[1], BodyItem::For(_)));
    assert!(
        matches!(&variants[1].query.body[2], BodyItem::Positive { atom }
        if atom.relation == "Link")
    );
}

#[test]
fn authored_coarser_cover_with_redundancy_is_admitted_independently() {
    let source: strata::Program = syn::parse_quote! {
        struct Coarser;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output B;
        A(*x) :- E(x);
        B(*x) :- A(x);
        strata { component [B,A]; }
    };
    let target: incremental::Program = syn::parse2(quote! {
        #source
        incremental {
            scheme seminaive_covering;
            delta B = ChangesB;
            delta A = ChangesA;
            variant 1 delta 0 { B(*x) :- ChangesA(x); }
            variant 0 seed { A(*x) :- E(x); }
            variant 1 delta 0 { B(*x) :- ChangesA(x); }
            variant 0 seed { A(*x) :- E(x); }
        }
    })
    .unwrap();
    seminaive::check(&source, &target).unwrap();
    let default = seminaive::compile(&source).unwrap();
    assert_eq!(default.variants.len(), 2);
    assert_eq!(default.variants[0].delta, None);
    assert_eq!(default.variants[1].delta, Some(0));
}
