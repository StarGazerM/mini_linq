//! Authored schedules, independent expected CQ, and boundary mutations.
use cq_datalog_fork::{Fork, check, compile, fork};
use cq_lang_cq::BodyItem;
use cq_lang_datalog::Program;
use quote::{ToTokens, quote};

fn authored() -> Program {
    syn::parse_quote! {
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32,y:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y), !Blocked(y,_);
        Reach(*x,*z) :- Reach(x,y), Edge(y,z), !Blocked(z,_);
        strata { component [Reach]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta Reach = Recent;
            layout Edge [0,1] key 0;
            layout Edge [0,1] key 1;
            variant 0 seed { Reach(*x,*y) :- Edge(x,y), !Blocked(y,_); }
            variant 1 delta 0 { Reach(*x,*z) :- Recent(x,y), Edge(y,z), !Blocked(z,_); }
        }
    }
}

#[test]
fn authored_holes_preserve_contracts_and_select_borrowed_hosts() {
    let source = authored();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let context = &target.context;
    let stratum = &context.strata[0];
    assert_eq!(stratum.keys.len(), 1);
    assert_eq!(stratum.keys[0].columns, [0]);
    assert_eq!(target.holes.len(), 3);
    assert_eq!(context.layouts.len(), 2);
    for hole in &target.holes {
        cq_lang_cq::contract::check(hole).unwrap();
        let parsed: cq_lang_cq::Program = syn::parse2(hole.to_token_stream()).unwrap();
        assert_eq!(&parsed, hole);
        assert!(
            !hole
                .query
                .body
                .iter()
                .any(|item| matches!(item, BodyItem::Negation(_)))
        );
    }
    let hole = &target.holes[stratum.steps[0].hole];
    let name = &hole.name;
    let edge = &context.relations[0].all;
    let blocked = &context.relations[1].all;
    let reach = &context.relations[2].all;
    let delta = context.relations[2].delta_host.as_ref().unwrap();
    let scan = &context.layouts[0].name;
    let scan_host = &context.layouts[0].host;
    let lookup = &context.layouts[1].name;
    let lookup_host = &context.layouts[1].host;
    let keys = &stratum.keys[0].name;
    let expected: cq_lang_cq::Program = syn::parse2(quote! {
        struct #name;
        relation Edge(x:i32,y:i32) = (#edge);
        relation Blocked(x:i32,y:i32) = (#blocked);
        relation Reach(x:i32,y:i32) = (&#reach);
        relation Recent(x:i32,y:i32) = (&#delta);
        relation #scan(x:i32,y:i32) = (&#scan_host);
        relation #lookup(x:i32,y:i32) = (&#lookup_host);
        relation #name(x:i32,y:i32);
        #name(*x,*z) :- Recent(x,y), #lookup(y,z), #[total] if !#keys.contains(&(z,));
    })
    .unwrap();
    assert_eq!(hole, &expected);
}

fn rejected(change: impl FnOnce(&mut Fork)) {
    let source = authored();
    let mut target = compile(&source).unwrap();
    change(&mut target);
    assert!(check(&source, &target).is_err());
}

#[test]
fn checker_rejects_schedule_and_layout_mutations() {
    rejected(|t| t.context.outputs = vec![0]);
    rejected(|t| t.context.observer = Some(syn::parse_quote!(observer)));
    rejected(|t| t.context.relations[2].delta_host = None);
    rejected(|t| t.context.relations[2].delta_host = Some(syn::parse_quote!(Recent)));
    rejected(|t| t.context.relations[0].all = t.context.relations[1].all.clone());
    rejected(|t| t.context.strata[0].members.clear());
    rejected(|t| t.context.strata[0].steps[0].head = 0);
    rejected(|t| t.context.strata[0].steps[0].variant = 0);
    rejected(|t| t.context.strata[0].steps.clear());
    rejected(|t| {
        t.holes.pop();
    });
    rejected(|t| t.holes.push(t.holes[0].clone()));
    rejected(|t| t.context.layouts.clear());
    rejected(|t| t.context.layouts[0].key = 2);
    rejected(|t| t.context.layouts[1].columns.swap(0, 1));
    rejected(|t| t.context.layouts[0].relation = usize::MAX);
    rejected(|t| t.context.layouts[0].host = t.context.relations[0].all.clone());
    rejected(|t| t.context.layouts[0].name = syn::parse_quote!(Edge));
    rejected(|t| {
        let hole = t.context.strata[0].steps[0].hole;
        if let BodyItem::Positive { atom } = &mut t.holes[hole].query.body[1] {
            atom.relation = t.context.layouts[0].name.clone();
        }
    });
}

#[test]
fn checker_rejects_negation_mutations() {
    rejected(|t| t.context.strata[0].keys.clear());
    rejected(|t| t.context.strata[0].keys[0].columns = vec![1]);
    rejected(|t| t.context.strata[0].keys[0].columns = vec![0, 0]);
    rejected(|t| t.context.strata[0].keys[0].relation = usize::MAX);
    rejected(|t| t.context.strata[0].keys[0].hole = usize::MAX);
    rejected(|t| {
        let hole = t.context.strata[0].steps[0].hole;
        t.holes[hole].query.body[2] = syn::parse_quote!(#[total] if true);
    });
    rejected(|t| {
        let hole = t.context.strata[0].keys[0].hole;
        t.holes[hole].query.head.args.clear();
    });
}

#[test]
fn callback_is_checked_once_per_hole_and_errors_propagate() {
    let source = authored();
    let mut visited = Vec::new();
    let expanded = fork(&source, |hole| {
        cq_lang_cq::contract::check(&hole)?;
        visited.push(hole.clone());
        let name = hole.name;
        Ok(quote!(callback!(#name)))
    })
    .unwrap();
    assert_eq!(visited, compile(&source).unwrap().holes);
    assert_eq!(expanded.holes.len(), 3);
    let mut calls = 0;
    assert!(
        fork(&source, |hole| {
            calls += 1;
            Err(syn::Error::new_spanned(hole, "callback rejected hole"))
        })
        .is_err()
    );
    assert_eq!(calls, 1);
}

#[test]
fn reverse_layout_permutes_schema_and_arguments_together() {
    let source: Program = syn::parse_quote! {
        struct Reverse;
        relation E(x:i32,y:String) = (edges);
        relation R(x:i32,y:String);
        output R;
        R(*x,(*y).clone()) :- E(x,y);
        R(*z,(*y).clone()) :- R(x,y), E(z,y);
        strata { component [R]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta R = D;
            layout E [0,1] key 0;
            layout E [1,0] key 1;
            variant 0 seed { R(*x,(*y).clone()) :- E(x,y); }
            variant 1 delta 0 { R(*z,(*y).clone()) :- D(x,y), E(z,y); }
        }
    };
    let target = compile(&source).unwrap();
    let layout = &target.context.layouts[1];
    let hole = &target.holes[target.context.strata[0].steps[0].hole];
    let declaration = hole
        .declarations
        .iter()
        .find(|d| d.name == layout.name)
        .unwrap();
    assert_eq!(declaration.columns[0].ty, syn::parse_quote!(String));
    let name = &layout.name;
    let expected: BodyItem = syn::parse_quote!(#name(y,z));
    assert_eq!(hole.query.body[1], expected);
}

fn constant_schedule() -> Program {
    syn::parse_quote! {
        struct Constants;
        relation E(x:i32) = (edges);
        relation Modifier(kind:String,x:i32) = (modifiers);
        relation A(x:i32);
        relation B(x:i32);
        output B, A;
        A(*x) :- E(x), !Modifier("abstract",x), !Modifier("abstract",x), !Modifier("static",x);
        A(*x) :- A(x), !Modifier("abstract",x);
        B(*x) :- A(x), !Modifier("absent",_), !Modifier("abstract",7);
        strata { component [A]; component [B]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta A = DA;
            delta B = DB;
            layout E [0] key 0;
            layout A [0] key 0;
            variant 0 seed { A(*x) :- E(x), !Modifier("abstract",x), !Modifier("abstract",x), !Modifier("static",x); }
            variant 1 delta 0 { A(*x) :- DA(x), !Modifier("abstract",x); }
            variant 2 seed { B(*x) :- A(x), !Modifier("absent",_), !Modifier("abstract",7); }
        }
    }
}

#[test]
fn filtered_key_sets_are_shared_per_stratum_not_per_probe() {
    let source = constant_schedule();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(target.context.outputs, [3, 2]);
    let first = &target.context.strata[0];
    assert_eq!(first.keys.len(), 2);
    assert_eq!(first.keys[0].columns, [1]);
    assert_eq!(
        first.keys[0].filters,
        vec![(0, syn::parse_quote!("abstract"))]
    );
    assert_eq!(
        first.keys[1].filters,
        vec![(0, syn::parse_quote!("static"))]
    );
    let second = &target.context.strata[1];
    assert_eq!(second.keys.len(), 2);
    assert!(second.keys.iter().all(|key| key.columns.is_empty()));
    assert_eq!(
        second.keys[1].filters,
        vec![
            (0, syn::parse_quote!("abstract")),
            (1, syn::parse_quote!(7))
        ]
    );
    for hole in &target.holes {
        cq_lang_cq::contract::check(hole).unwrap();
        assert_eq!(
            hole,
            &syn::parse2::<cq_lang_cq::Program>(hole.to_token_stream()).unwrap()
        );
    }
}

#[test]
fn checker_rejects_constant_selection_and_output_order_mutations() {
    let source = constant_schedule();
    let original = compile(&source).unwrap();
    let mut changed = original.clone();
    changed.context.outputs.reverse();
    assert!(check(&source, &changed).is_err());
    let mut changed = original.clone();
    changed.context.strata[0].keys[0].filters[0].1 = syn::parse_quote!("static");
    assert!(check(&source, &changed).is_err());
    let mut changed = original.clone();
    changed.context.strata[0].keys[0].filters.clear();
    assert!(check(&source, &changed).is_err());
    let mut changed = original.clone();
    changed.context.strata[0].keys[0].filters[0].0 = 1;
    assert!(check(&source, &changed).is_err());
    let mut changed = original.clone();
    let hole = changed.context.strata[0].keys[0].hole;
    let BodyItem::Positive { atom } = &mut changed.holes[hole].query.body[0] else {
        unreachable!()
    };
    atom.args[0] = syn::parse_quote!("static");
    assert!(check(&source, &changed).is_err());
    let mut changed = original;
    let duplicate = changed.context.strata[0].keys[0].clone();
    changed.context.strata[0].keys.push(duplicate);
    assert!(check(&source, &changed).is_err());
}

#[test]
fn edb_outputs_do_not_need_delta_relations_or_rule_holes() {
    let source: Program = syn::parse_quote! {
        struct Facts;
        relation E(x:String) = (e);
        relation F(x:i32) = (f);
        output F, E;
        strata {}
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
        }
    };
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(target.context.outputs, [1, 0]);
    assert!(target.context.strata.is_empty());
    assert!(target.holes.is_empty());
    assert!(target.context.relations.iter().all(|r| r.delta.is_none()));
    let mut changed = target;
    changed.context.outputs.reverse();
    assert!(check(&source, &changed).is_err());
}

#[test]
fn owned_negative_constants_are_post_scan_filters_not_owned_prefixes() {
    let source: Program = syn::parse_quote! {
        struct OwnedConstant;
        relation D(x:String) = (domain);
        relation M(kind:String,x:String) = (modifiers);
        relation A(x:String);
        output A;
        A((*x).clone()) :- D(x), !M(String::from("static"),x);
        strata { component [A]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta A = DA;
            layout D [0] key 0;
            variant 0 seed { A((*x).clone()) :- D(x), !M(String::from("static"),x); }
        }
    };
    let target = compile(&source).unwrap();
    let key = &target.context.strata[0].keys[0];
    assert_eq!(key.columns, [1]);
    let hole = &target.holes[key.hole];
    assert_eq!(hole.query.body.len(), 2);
    let BodyItem::Positive { atom } = &hole.query.body[0] else {
        unreachable!()
    };
    assert_eq!(atom.variables().count(), 2);
    let variable = &atom.args[0];
    let expected: BodyItem = syn::parse_quote!(if *#variable == (String::from("static")));
    assert_eq!(hole.query.body[1], expected);
    assert!(
        !hole
            .query
            .body
            .iter()
            .any(|item| matches!(item, BodyItem::Let(_)))
    );
    check(&source, &target).unwrap();
    let hole_index = key.hole;
    let mut changed = target;
    changed.holes[hole_index].query.body[1] = syn::parse_quote!(if true);
    assert!(check(&source, &changed).is_err());
}

#[test]
fn literal_key_filter_correspondence_uses_complete_cq_recognition() {
    let source = constant_schedule();
    let target = compile(&source).unwrap();
    let key = &target.context.strata[0].keys[0];
    let hole = &target.holes[key.hole];
    let BodyItem::Positive { atom } = &hole.query.body[0] else {
        unreachable!()
    };
    let variable = &atom.args[0];
    let raw: BodyItem = syn::parse_quote!(if *#variable == ("abstract"));
    assert!(matches!(raw, BodyItem::Filter(_)));
    assert!(matches!(
        hole.query.body[1],
        BodyItem::Equality(_) | BodyItem::Constraint(_)
    ));
    assert_ne!(hole.query.body[1], raw);
    check(&source, &target).unwrap();
    let restored: cq_lang_cq::Program = syn::parse2(hole.to_token_stream()).unwrap();
    assert_eq!(&restored, hole);

    // A different literal remains a well-formed, recognized CQ predicate,
    // but must not pass correspondence with the recorded source selection.
    let name = &hole.name;
    let declarations = &hole.declarations;
    let head = &hole.query.head;
    let changed_hole: cq_lang_cq::Program = syn::parse2(quote! {
        struct #name;
        #(#declarations)*
        #head :- #atom, if *#variable == ("changed");
    })
    .unwrap();
    cq_lang_cq::contract::check(&changed_hole).unwrap();
    let hole_index = key.hole;
    let mut changed = target;
    changed.holes[hole_index] = changed_hole;
    assert!(check(&source, &changed).is_err());
}
