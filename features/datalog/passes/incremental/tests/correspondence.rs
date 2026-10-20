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
        A(*x) :- E(x), if *x == 1;
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
