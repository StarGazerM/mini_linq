use cq_datalog_schedule::{seminaive, stratify};
use cq_lang_datalog::{BodyItem, Program, contract};
use quote::ToTokens;

fn source() -> Program {
    syn::parse_quote! {
        struct Paths;
        relation Edge(x:i32,y:i32) = (edges);
        relation Reach(x:i32,y:i32);
        relation Other(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y);
        Other(*x,*y) :- Reach(x,y);
        Reach(*x,*z) :- Other(x,y), Reach(y,z);
    }
}

#[test]
fn both_passes_preserve_complete_round_trippable_programs() {
    let source = source();
    let strata = stratify::compile(&source).unwrap();
    let variants = seminaive::compile(&strata).unwrap();
    stratify::check(&source, &strata).unwrap();
    seminaive::check(&strata, &variants).unwrap();
    assert!(source.negation_keys.is_none());
    assert!(strata.negation_keys.is_none());
    assert_eq!(
        variants.negation_keys.as_ref().unwrap(),
        "hash_set_borrowed"
    );
    for program in [&source, &strata, &variants] {
        contract::check(program).unwrap();
        let restored: Program = syn::parse2(program.to_token_stream()).unwrap();
        assert_eq!(*program, restored);
        contract::check(&restored).unwrap();
    }
    assert_eq!(strata.strata.as_ref().unwrap().len(), 1);
    assert_eq!(variants.variants.as_ref().unwrap().len(), 4);
    assert_eq!(variants.deltas.len(), 2);
}

#[test]
fn authored_alternatives_are_checked_without_replaying_compilers() {
    let source: Program = syn::parse_quote! {
        struct Independent;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output A;
        A(*x) :- E(x);
        B(*x) :- E(x);
    };
    let target: Program = syn::parse_quote! {
        struct Independent;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output A;
        A(*x) :- E(x);
        B(*x) :- E(x);
        strata { component [B]; component [A]; }
    };
    stratify::check(&source, &target).unwrap();
    let delta: Program = syn::parse_quote! {
        struct Independent;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output A;
        A(*x) :- E(x);
        B(*x) :- E(x);
        strata { component [B]; component [A]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta B = FreshB;
            delta A = FreshA;
            layout E [0] key 0;
            variant 1 seed { B(*x) :- E(x); }
            variant 0 seed { A(*x) :- E(x); }
        }
    };
    seminaive::check(&target, &delta).unwrap();
}

#[test]
fn stages_reject_wrong_input_and_target_shapes() {
    let source = source();
    let strata = stratify::compile(&source).unwrap();
    let delta = seminaive::compile(&strata).unwrap();
    assert!(stratify::compile(&strata).is_err());
    assert!(stratify::compile(&delta).is_err());
    assert!(seminaive::compile(&source).is_err());
    assert!(seminaive::compile(&delta).is_err());
    assert!(stratify::check(&source, &source).is_err());
    assert!(stratify::check(&source, &delta).is_err());
    assert!(stratify::check(&strata, &strata).is_err());
    assert!(seminaive::check(&source, &delta).is_err());
    assert!(seminaive::check(&strata, &strata).is_err());
    assert!(seminaive::check(&delta, &delta).is_err());
}

#[test]
fn checks_reject_source_edits_and_missing_decisions() {
    let source = source();
    let strata = stratify::compile(&source).unwrap();
    let delta = seminaive::compile(&strata).unwrap();
    let mut changed = strata.clone();
    changed.outputs = vec![syn::parse_quote!(Other)];
    contract::check(&changed).unwrap();
    assert!(stratify::check(&source, &changed).is_err());
    let mut changed = delta.clone();
    changed.outputs = vec![syn::parse_quote!(Other)];
    contract::check(&changed).unwrap();
    assert!(seminaive::check(&strata, &changed).is_err());
    let mut changed = delta.clone();
    changed.deltas.pop();
    assert!(seminaive::check(&strata, &changed).is_err());
    let mut changed = delta;
    changed.variants.as_mut().unwrap().pop();
    assert!(seminaive::check(&strata, &changed).is_err());
}

#[test]
fn delta_names_avoid_all_source_identifiers_but_are_not_canonical() {
    let source: Program = syn::parse_quote! {
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
    let variant = &mut delta.variants.as_mut().unwrap()[1];
    let BodyItem::Positive { atom } = &mut variant.query.body[0] else {
        unreachable!()
    };
    atom.relation = syn::parse_quote!(AuthoredDelta);
    seminaive::check(&strata, &delta).unwrap();
}

#[test]
fn negative_dependencies_force_completed_lower_components() {
    let source: Program = syn::parse_quote! {
        struct Stratified;
        relation E(x:i32) = (edges); relation A(x:i32); relation B(x:i32);
        output B;
        B(*x) :- E(x), !A(x);
        A(*x) :- E(x), if *x == 1;
    };
    let strata = stratify::compile(&source).unwrap();
    let expected: Vec<Vec<syn::Ident>> =
        vec![vec![syn::parse_quote!(A)], vec![syn::parse_quote!(B)]];
    assert_eq!(strata.strata.as_ref().unwrap(), &expected);
    let mut reversed = strata.clone();
    reversed.strata.as_mut().unwrap().reverse();
    assert!(stratify::check(&source, &reversed).is_err());
    let delta = seminaive::compile(&strata).unwrap();
    seminaive::check(&strata, &delta).unwrap();
    assert!(
        delta
            .variants
            .as_ref()
            .unwrap()
            .iter()
            .all(|v| v.delta.is_none())
    );
}

#[test]
fn multioutput_and_constant_negation_survive_both_schedules() {
    let source: Program = syn::parse_quote! {
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
    for program in [&source, &strata, &delta] {
        assert_eq!(program.outputs, source.outputs);
        let restored: Program = syn::parse2(program.to_token_stream()).unwrap();
        assert_eq!(program, &restored);
        contract::check(&restored).unwrap();
    }
    let mut reordered = strata.clone();
    reordered.outputs.reverse();
    contract::check(&reordered).unwrap();
    assert!(stratify::check(&source, &reordered).is_err());
    let mut reordered = delta.clone();
    reordered.outputs.reverse();
    assert!(seminaive::check(&strata, &reordered).is_err());
    let mut changed = delta.clone();
    let BodyItem::Negation(negative) = &mut changed.variants.as_mut().unwrap()[0].query.body[1]
    else {
        unreachable!()
    };
    negative.atom.args[0] = syn::parse_quote!("static");
    assert!(contract::check(&changed).is_err());
}

#[test]
fn edb_outputs_preserve_empty_and_mixed_schedules() {
    for text in [
        "struct P; relation E(x:i32) = (e); output E;",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output E, A; A(*x) :- E(x);",
    ] {
        let source: Program = syn::parse_str(text).unwrap();
        let strata = stratify::compile(&source).unwrap();
        let delta = seminaive::compile(&strata).unwrap();
        stratify::check(&source, &strata).unwrap();
        seminaive::check(&strata, &delta).unwrap();
        assert_eq!(delta.outputs, source.outputs);
        assert_eq!(delta.deltas.len(), source.rules.len());
        assert_eq!(
            delta,
            syn::parse2::<Program>(delta.to_token_stream()).unwrap()
        );
    }
}

#[test]
fn if_let_is_lowered_before_seminaive_variants_and_keeps_written_positions() {
    let source: Program = syn::parse_quote! {
        struct Matched;
        relation Seed(o:Option<i32>) = (seeds);
        relation Link(x:i32,y:i32) = (links);
        relation Reach(x:i32);
        output Reach;
        Reach(x) :- Seed(o), if let Some(x) = *o;
        Reach(*z) :- Reach(x), if let Some(y) = x.checked_add(1), Link(y,z);
    };
    let strata = stratify::compile(&source).unwrap();
    let variants = seminaive::compile(&strata).unwrap();
    stratify::check(&source, &strata).unwrap();
    seminaive::check(&strata, &variants).unwrap();
    for program in [&strata, &variants] {
        for rule in &program.rules {
            assert!(matches!(rule.body[1], BodyItem::For(_)));
        }
        assert_eq!(*program, syn::parse2(program.to_token_stream()).unwrap());
    }
    let variants = variants.variants.unwrap();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].delta, None);
    assert_eq!(variants[1].delta, Some(0));
    assert!(matches!(variants[1].query.body[1], BodyItem::For(_)));
    assert!(
        matches!(&variants[1].query.body[2], BodyItem::Positive { atom }
        if atom.relation == "Link")
    );
}
