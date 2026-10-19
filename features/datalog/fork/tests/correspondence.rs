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

#[test]
fn later_delta_reuses_var_points_to_probe_without_moving_partial_guards() {
    let source: Program = syn::parse_str(include_str!(
        "../../language/tests/fixtures/written_order.datalog"
    ))
    .unwrap();
    let target = compile(&source).unwrap();
    let context = &target.context;
    let probe = &context.layouts[1];
    assert_eq!(probe.columns, [1, 0]);
    assert_eq!(probe.key, 1);
    assert_eq!(context.layouts.len(), 6);
    assert_eq!(context.delta_layouts.len(), 2);
    let delta_probe = &context.delta_layouts[0];
    assert_eq!(delta_probe.columns, [1, 0]);
    assert_eq!(delta_probe.key, 1);
    let delta_name = &delta_probe.name;
    let all_delta = &context.delta_layouts[1].name;
    let probe_name = &probe.name;
    let assign_scan = &context.layouts[4].name;
    let steps = &context.strata[0].steps;
    let earlier = &target.holes[steps.iter().find(|r| r.variant == 1).unwrap().hole];
    let expected_probe: BodyItem = syn::parse_quote!(#probe_name(from,heap));
    assert_eq!(earlier.query.body[1], expected_probe);
    let delta_expected: BodyItem = syn::parse_quote!(#delta_name(from,heap));
    for (variant, position) in [(2, 1), (3, 3)] {
        let hole = &target.holes[steps.iter().find(|r| r.variant == variant).unwrap().hole];
        assert_eq!(hole.query.body[position], delta_expected);
    }

    let index = steps.iter().find(|r| r.variant == 4).unwrap().hole;
    let hole = &target.holes[index];
    let name = &hole.name;
    let declarations = &hole.declarations;
    let expected: cq_lang_cq::Program = syn::parse2(quote! {
        struct #name;
        #(#declarations)*
        #name(*heap,*to) :- #assign_scan(to,from),
            if *to != 0, if 10 / *to > 0,
            #probe_name(from,heap), #all_delta(heap,to);
    })
    .unwrap();
    assert_eq!(hole, &expected);
    assert_eq!(hole.query.body[3], expected_probe);
    check(&source, &target).unwrap();

    let mut prebound = target.clone();
    let all_bound = &context.layouts[5].name;
    prebound.holes[index].query.body[3] = syn::parse_quote!(#all_bound(heap,from));
    assert!(check(&source, &prebound).is_err());
    let mut moved_guard = target.clone();
    let guard = moved_guard.holes[index].query.body[1].clone();
    moved_guard.holes[index].query.body[1] = moved_guard.holes[index].query.body[2].clone();
    moved_guard.holes[index].query.body[2] = guard;
    assert!(check(&source, &moved_guard).is_err());
    let mut moved_delta = target.clone();
    let atom = moved_delta.holes[index].query.body[3].clone();
    moved_delta.holes[index].query.body[3] = moved_delta.holes[index].query.body[4].clone();
    moved_delta.holes[index].query.body[4] = atom;
    assert!(check(&source, &moved_delta).is_err());
}

#[test]
fn positive_constant_probes_preserve_repeats_wildcards_and_later_delta() {
    let source: Program = syn::parse_quote! {
        struct Constants;
        relation E(x:i32,y:i32,z:i32) = (e);
        relation R(x:i32);
        output R;
        R(*x) :- E(x,x,7), E(_,x,(1 + 2));
        R(*y) :- E(y,7,_), if *y != 0, R(x), E(x,x,7);
        strata { component [R]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta R = D;
            layout E [2,0,1] key 1;
            layout E [1,2,0] key 2;
            layout E [1,0,2] key 1;
            layout E [0,1,2] key 3;
            delta_layout R [0] key 0;
            variant 0 seed { R(*x) :- E(x,x,7), E(_,x,(1 + 2)); }
            variant 1 delta 2 { R(*y) :- E(y,7,_), if *y != 0, D(x), E(x,x,7); }
        }
    };
    let target = compile(&source).unwrap();
    let layouts = &target.context.layouts;
    let first = &layouts[0].name;
    let second = &layouts[1].name;
    let third = &layouts[2].name;
    let fourth = &layouts[3].name;
    let delta = &target.context.delta_layouts[0].name;
    let stratum = &target.context.strata[0];
    for (index, body) in [
        (
            stratum.seeds[0].hole,
            quote!(#first(7,x,x), #second(x,(1 + 2),_)),
        ),
        (
            stratum.steps[0].hole,
            quote!(#third(7,y,_), if *y != 0, #delta(x), #fourth(x,x,7)),
        ),
    ] {
        let hole = &target.holes[index];
        let name = &hole.name;
        let declarations = &hole.declarations;
        let head = &hole.query.head;
        let expected: cq_lang_cq::Program = syn::parse2(quote! {
            struct #name;
            #(#declarations)*
            #head :- #body;
        })
        .unwrap();
        assert_eq!(hole, &expected);
        let replay: cq_lang_cq::Program = syn::parse2(hole.to_token_stream()).unwrap();
        assert_eq!(hole, &replay);
    }
    check(&source, &target).unwrap();
}

#[test]
fn delta_hosts_are_shared_only_by_exact_demand_and_keep_written_bodies() {
    let source: Program = syn::parse_str(include_str!(
        "../../language/tests/fixtures/delta_probes.datalog"
    ))
    .unwrap();
    let target = compile(&source).unwrap();
    let context = &target.context;
    assert_eq!(context.delta_layouts.len(), 4);
    assert_eq!(context.layouts.len(), 1);
    let scan = &context.layouts[0].name;
    let constant = &context.delta_layouts[0].name;
    let keyed = &context.delta_layouts[1].name;
    let zero = &context.delta_layouts[2].name;
    let pair = &context.delta_layouts[3].name;
    let expected_bodies = [
        quote!(Recent(x, y, z)),
        quote!(#constant(7,x,y)),
        quote!(#scan(_,y,_), #keyed(y,x,z)),
        quote!(#scan(_,y,_), if *y != 0, if 10 / *y > 0, #keyed(y,x,z)),
        quote!(#scan(a,b,c), #zero(x,y,z)),
        quote!(#scan(x,y,_), #pair(x,y,z)),
        quote!(#constant(7,x,x)),
    ];
    for (step, body) in context.strata[0].steps.iter().zip(expected_bodies) {
        let hole = &target.holes[step.hole];
        let name = &hole.name;
        let declarations = &hole.declarations;
        let head = &hole.query.head;
        let expected: cq_lang_cq::Program = syn::parse2(quote! {
            struct #name;
            #(#declarations)*
            #head :- #body;
        })
        .unwrap();
        assert_eq!(hole, &expected);
        assert_eq!(
            *hole,
            syn::parse2::<cq_lang_cq::Program>(hole.to_token_stream()).unwrap()
        );
        cq_lang_cq::contract::check(hole).unwrap();
    }
    check(&source, &target).unwrap();
    let mutations: &[fn(&mut Fork)] = &[
        |f| f.context.delta_layouts.clear(),
        |f| {
            f.context.delta_layouts.pop();
        },
        |f| f.context.delta_layouts.reverse(),
        |f| {
            f.context
                .delta_layouts
                .push(f.context.delta_layouts[0].clone())
        },
        |f| f.context.delta_layouts[0].relation = 0,
        |f| f.context.delta_layouts[0].relation = usize::MAX,
        |f| f.context.delta_layouts[0].columns.swap(0, 1),
        |f| f.context.delta_layouts[0].key = 0,
        |f| f.context.delta_layouts[0].host = f.context.layouts[0].host.clone(),
        |f| f.context.delta_layouts[0].name = syn::parse_quote!(Recent),
        |f| {
            let hole = f.context.strata[0].steps[2].hole;
            f.holes[hole].query.body[1] = syn::parse_quote!(Recent(x, y, z));
        },
        |f| {
            let hole = f.context.strata[0].steps[2].hole;
            let name = &f.context.delta_layouts[1].name;
            f.holes[hole].query.body[1] = syn::parse_quote!(#name(x,y,z));
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = target.clone();
        mutate(&mut changed);
        assert!(
            check(&source, &changed).is_err(),
            "delta fork mutation {index}"
        );
    }
}

#[test]
fn delta_layout_reorders_heterogeneous_declaration_and_arguments_together() {
    let source: Program = syn::parse_quote! {
        struct ReverseDelta;
        relation E(x:i32,y:String) = (edges);
        relation R(x:i32,y:String);
        output R;
        R(*x,(*y).clone()) :- E(x,y);
        R(*x,(*y).clone()) :- E(_,y), R(x,y);
        strata { component [R]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta R = D;
            layout E [0,1] key 0;
            delta_layout R [1,0] key 1;
            variant 0 seed { R(*x,(*y).clone()) :- E(x,y); }
            variant 1 delta 1 { R(*x,(*y).clone()) :- E(_,y), D(x,y); }
        }
    };
    let target = compile(&source).unwrap();
    let layout = &target.context.delta_layouts[0];
    let hole = &target.holes[target.context.strata[0].steps[0].hole];
    let declaration = hole
        .declarations
        .iter()
        .find(|d| d.name == layout.name)
        .unwrap();
    assert_eq!(declaration.columns[0].ty, syn::parse_quote!(String));
    assert_eq!(declaration.columns[1].ty, syn::parse_quote!(i32));
    let name = &layout.name;
    let expected: BodyItem = syn::parse_quote!(#name(y,x));
    assert_eq!(hole.query.body[1], expected);
    check(&source, &target).unwrap();
}

#[test]
fn fork_preserves_generators_and_generated_join_values() {
    let source: Program = syn::parse_quote! {
        struct Generated;
        relation Seed(x:i32) = (seeds);
        relation Link(x:i32,y:i32) = (links);
        relation Reach(x:i32);
        output Reach;
        Reach(y) :- Seed(x), for y in [*x, *x];
        Reach(*z) :- Reach(x), if *x < 3, for y in [*x + 1], Link(y,z);
        strata { component [Reach]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta Reach = Recent;
            layout Seed [0] key 0;
            layout Link [0,1] key 0;
            variant 0 seed { Reach(y) :- Seed(x), for y in [*x, *x]; }
            variant 1 delta 0 {
                Reach(*z) :- Recent(x), if *x < 3, for y in [*x + 1], Link(y,z);
            }
        }
    };
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let stratum = &target.context.strata[0];
    for step in stratum.seeds.iter().chain(&stratum.steps) {
        let hole = &target.holes[step.hole];
        cq_lang_cq::contract::check(hole).unwrap();
        let original = &source.variants.as_ref().unwrap()[step.variant].query;
        let generators = |query: &cq_lang_cq::Query| {
            query
                .body
                .iter()
                .filter_map(|item| match item {
                    BodyItem::For(generator) => Some(generator.to_token_stream().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(generators(&hole.query), generators(original));
    }
    let mut changed = target.clone();
    let hole = stratum.steps[0].hole;
    changed.holes[hole].query.body[2] = syn::parse_quote!(for y in [*x + 2]);
    assert!(check(&source, &changed).is_err());
}

#[test]
fn device_keys_have_owned_schema_and_checked_membership_intrinsic() {
    let mut source = authored();
    source.insertion = Some(syn::parse_quote!(device_set_all_columns));
    source.negation_keys = Some(syn::parse_quote!(device_set_owned));
    let mut target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(
        target.context.representation,
        cq_datalog_fork::KeyRepresentation::DeviceSetOwned
    );
    let key = &target.context.strata[0].keys[0];
    let key_hole = &target.holes[key.hole];
    assert_eq!(
        key_hole.declarations.len(),
        2,
        "only completed input and output"
    );
    assert_eq!(
        key_hole.declarations.last().unwrap().columns[0].ty,
        syn::parse_quote!(i32)
    );
    let rule = &target.context.strata[0].seeds[0];
    assert_eq!(
        target.holes[rule.hole].declarations.len(),
        2,
        "only used layout and output"
    );
    assert!(
        target.holes[rule.hole]
            .to_token_stream()
            .to_string()
            .contains("datalog :: contains")
    );
    target.context.representation = cq_datalog_fork::KeyRepresentation::HashSetBorrowed;
    assert!(check(&source, &target).is_err());
}
