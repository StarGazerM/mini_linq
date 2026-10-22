//! Authored schedules, independent expected CQ, and boundary mutations.
use cq_datalog_fork::{check, compile};
use cq_datalog_storage::compile as schedule;
use cq_lang_cq::BodyItem;
use cq_lang_datalog_round::Program as Fork;
use cq_lang_datalog_round::cq as cq_lang_cq;
use cq_lang_datalog_storage::Program;

use quote::{ToTokens, quote};

fn authored() -> Program {
    schedule(&syn::parse_quote! {
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32,y:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y), !Blocked(y,_);
        Reach(*x,*z) :- Reach(x,y), Edge(y,z), !Blocked(z,_);
        strata { component [Reach]; }
        incremental { scheme seminaive_covering;


            delta Reach = Recent;


            variant 0 seed { Reach(*x,*y) :- Edge(x,y), !Blocked(y,_); }
            variant 1 delta 0 { Reach(*x,*z) :- Recent(x,y), Edge(y,z), !Blocked(z,_); }
        }
    })
    .unwrap()
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
    for entry in &target.holes {
        let hole = &entry.cq;
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
    let hole = &target.holes[stratum.steps[0].hole].cq;
    let name = &hole.name;
    let lookup = &context.layouts[1].name;
    let lookup_host = &context.layouts[1].host;
    let keys = &stratum.keys[0].name;
    let binding = &stratum.steps[0].scan.as_ref().unwrap().binding;
    let expected: cq_lang_cq::Program = syn::parse2(quote! {
        struct #name;
        relation #lookup(x:i32,y:i32) = (&#lookup_host);
        relation #binding(x:i32,y:i32) = (#binding);
        relation #name(x:i32,y:i32);
        #name(*x,*z) :- #binding(x,y), #lookup(y,z), #[total] if !#keys.contains(&(z,));
    })
    .unwrap();
    assert_eq!(hole, &expected);
}

#[test]
fn every_hole_declares_exactly_its_reads_and_output() {
    let source = authored();
    let target = compile(&source).unwrap();
    for entry in &target.holes {
        let hole = &entry.cq;
        let mut required =
            std::collections::BTreeSet::from([cq_syntax::symbol_name(&hole.query.head.relation)]);
        for item in &hole.query.body {
            if let BodyItem::Positive { atom } = item {
                required.insert(cq_syntax::symbol_name(&atom.relation));
            }
        }
        let actual: std::collections::BTreeSet<_> = hole
            .declarations
            .iter()
            .map(|declaration| cq_syntax::symbol_name(&declaration.name))
            .collect();
        assert_eq!(actual, required);
        assert_eq!(hole.declarations.len(), required.len());
    }
    let key = target.context.strata[0].keys[0].hole;
    assert_eq!(target.holes[key].cq.declarations.len(), 2);
    let step = target.context.strata[0].steps[0].hole;
    assert_eq!(target.holes[step].cq.declarations.len(), 3);
}

#[test]
fn standalone_round_checks_reject_missing_extra_and_misbound_inputs() {
    let source = authored();
    let target = compile(&source).unwrap();
    for hole in 0..target.holes.len() {
        for mutation in 0..4 {
            let mut changed = target.clone();
            let declarations = &mut changed.holes[hole].cq.declarations;
            match mutation {
                0 => {
                    declarations.remove(0);
                }
                1 => declarations.insert(
                    0,
                    syn::parse_quote! {
                        relation Unused(x:i32) = (unused);
                    },
                ),
                2 => declarations[0].value = Some(syn::parse_quote!(= (wrong_host))),
                _ => {
                    declarations.pop();
                }
            }
            assert!(cq_lang_datalog_round::check_storage(&source, &changed).is_err());
            assert!(cq_lang_datalog_round::contract::check(&changed).is_err());
        }
    }
}

fn rejected(change: impl FnOnce(&mut Fork)) {
    let source = authored();
    let mut target = compile(&source).unwrap();
    change(&mut target);
    assert!(check(&source, &target).is_err());
}

#[test]
fn checker_rejects_schedule_and_layout_mutations() {
    rejected(|t| t.context.outputs[0].relation = 0);
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
        if let BodyItem::Positive { atom } = &mut t.holes[hole].cq.query.body[1] {
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
        t.holes[hole].cq.query.body[2] = syn::parse_quote!(#[total] if true);
    });
    rejected(|t| {
        let hole = t.context.strata[0].keys[0].hole;
        t.holes[hole].cq.query.head.args.clear();
    });
}

#[test]
fn callback_is_checked_once_per_hole_and_errors_propagate() {
    let source = authored();
    let target = compile(&source).unwrap();
    let mut visited = Vec::new();
    let expanded = cq_datalog_print::splice_with(&target, |hole| {
        visited.push(hole.cq.clone());
        let name = &hole.cq.name;
        Ok(quote!(callback!(#name)))
    })
    .unwrap();
    assert_eq!(
        visited,
        target
            .holes
            .iter()
            .map(|h| h.cq.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(expanded.len(), 3);
    let mut calls = 0;
    assert!(
        cq_datalog_print::splice_with(&target, |hole| {
            calls += 1;
            Err(syn::Error::new_spanned(hole, "callback rejected hole"))
        })
        .is_err()
    );
    assert_eq!(calls, 1);
}

#[test]
fn reverse_layout_permutes_schema_and_arguments_together() {
    let source = schedule(&syn::parse_quote! {
        struct Reverse;
        relation E(x:i32,y:String) = (edges);
        relation R(x:i32,y:String);
        output R;
        R(*x,(*y).clone()) :- E(x,y);
        R(*z,(*y).clone()) :- R(x,y), E(z,y);
        strata { component [R]; }
        incremental { scheme seminaive_covering;


            delta R = D;


            variant 0 seed { R(*x,(*y).clone()) :- E(x,y); }
            variant 1 delta 0 { R(*z,(*y).clone()) :- D(x,y), E(z,y); }
        }
    })
    .unwrap();
    let target = compile(&source).unwrap();
    let layout = &target.context.layouts[1];
    let hole = &target.holes[target.context.strata[0].steps[0].hole].cq;
    let declaration = hole
        .declarations
        .iter()
        .find(|d| d.name == layout.name)
        .unwrap();
    assert_eq!(declaration.columns[0].ty, syn::parse_quote!(String));
    let name = &layout.name;
    let expected: BodyItem = syn::parse_quote!(#name(y,z));
    assert_eq!(hole.query.body[1], expected);
    for rule in target.context.strata[0]
        .seeds
        .iter()
        .chain(&target.context.strata[0].steps)
    {
        let binding = &rule.scan.as_ref().unwrap().binding;
        let declaration = target.holes[rule.hole]
            .cq
            .declarations
            .iter()
            .find(|declaration| declaration.name == *binding)
            .unwrap();
        assert_eq!(
            declaration.columns,
            source.incremental.strata.source.declarations[0].columns
        );
    }
}

fn constant_schedule() -> Program {
    schedule(&syn::parse_quote! {
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
        incremental { scheme seminaive_covering;


            delta A = DA;
            delta B = DB;


            variant 0 seed { A(*x) :- E(x), !Modifier("abstract",x), !Modifier("abstract",x), !Modifier("static",x); }
            variant 1 delta 0 { A(*x) :- DA(x), !Modifier("abstract",x); }
            variant 2 seed { B(*x) :- A(x), !Modifier("absent",_), !Modifier("abstract",7); }
        }
    }).unwrap()
}

#[test]
fn filtered_key_sets_are_shared_per_stratum_not_per_probe() {
    let source = constant_schedule();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(
        target
            .context
            .outputs
            .iter()
            .map(|output| output.relation)
            .collect::<Vec<_>>(),
        [3, 2]
    );
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
    for entry in &target.holes {
        let hole = &entry.cq;
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
    let BodyItem::Positive { atom } = &mut changed.holes[hole].cq.query.body[0] else {
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
    let source = schedule(&syn::parse_quote! {
        struct Facts;
        relation E(x:String) = (e);
        relation F(x:i32) = (f);
        output F, E;
        strata {}
        incremental { scheme seminaive_covering;


        }
    })
    .unwrap();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(
        target
            .context
            .outputs
            .iter()
            .map(|output| output.relation)
            .collect::<Vec<_>>(),
        [1, 0]
    );
    assert!(target.context.strata.is_empty());
    assert!(target.holes.is_empty());
    assert!(target.context.relations.iter().all(|r| r.delta.is_none()));
    let mut changed = target;
    changed.context.outputs.reverse();
    assert!(check(&source, &changed).is_err());
}

#[test]
fn owned_negative_constants_are_post_scan_filters_not_owned_prefixes() {
    let source = schedule(&syn::parse_quote! {
        struct OwnedConstant;
        relation D(x:String) = (domain);
        relation M(kind:String,x:String) = (modifiers);
        relation A(x:String);
        output A;
        A((*x).clone()) :- D(x), !M(String::from("static"),x);
        strata { component [A]; }
        incremental { scheme seminaive_covering;


            delta A = DA;

            variant 0 seed { A((*x).clone()) :- D(x), !M(String::from("static"),x); }
        }
    })
    .unwrap();
    let target = compile(&source).unwrap();
    let key = &target.context.strata[0].keys[0];
    assert_eq!(key.columns, [1]);
    let hole = &target.holes[key.hole].cq;
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
    changed.holes[hole_index].cq.query.body[1] = syn::parse_quote!(if true);
    assert!(check(&source, &changed).is_err());
}

#[test]
fn literal_key_filter_correspondence_preserves_atom_literals() {
    let source = constant_schedule();
    let target = compile(&source).unwrap();
    let key = &target.context.strata[0].keys[0];
    let hole = &target.holes[key.hole].cq;
    assert_eq!(hole.query.body.len(), 1);
    let BodyItem::Positive { atom } = &hole.query.body[0] else {
        unreachable!()
    };
    assert_eq!(atom.args[0], syn::parse_quote!("abstract"));
    assert_eq!(atom.variables().count(), 1);
    check(&source, &target).unwrap();
    let restored: cq_lang_cq::Program = syn::parse2(hole.to_token_stream()).unwrap();
    assert_eq!(&restored, hole);
    let normal = cq_lang_cq::normal::normalize(hole).unwrap();
    assert_eq!(normal.query.body.len(), 1);
    let BodyItem::Positive { atom } = &normal.query.body[0] else {
        unreachable!()
    };
    assert_eq!(atom.args[0], syn::parse_quote!("abstract"));

    let all_constants = &target.holes[target.context.strata[1].keys[1].hole].cq;
    assert_eq!(all_constants.query.body.len(), 1);
    let BodyItem::Positive { atom } = &all_constants.query.body[0] else {
        unreachable!()
    };
    assert_eq!(atom.variables().count(), 0);
    assert_eq!(atom.args[1], syn::parse_quote!(7));

    // A changed literal is still valid CQ, but changes the recorded selection.
    let hole_index = key.hole;
    let mut changed = target;
    let BodyItem::Positive { atom } = &mut changed.holes[hole_index].cq.query.body[0] else {
        unreachable!()
    };
    atom.args[0] = syn::parse_quote!("changed");
    cq_lang_cq::contract::check(&changed.holes[hole_index].cq).unwrap();
    assert!(check(&source, &changed).is_err());
}

#[test]
fn later_delta_reuses_var_points_to_probe_without_moving_partial_guards() {
    let source =
        schedule(&syn::parse_str(include_str!("fixtures/written_order.datalog")).unwrap()).unwrap();
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
    let steps = &context.strata[0].steps;
    let assign_scan = &steps
        .iter()
        .find(|rule| rule.variant == 4)
        .unwrap()
        .scan
        .as_ref()
        .unwrap()
        .binding;
    let earlier = &target.holes[steps.iter().find(|r| r.variant == 1).unwrap().hole].cq;
    let expected_probe: BodyItem = syn::parse_quote!(#probe_name(from,heap));
    assert_eq!(earlier.query.body[1], expected_probe);
    let delta_expected: BodyItem = syn::parse_quote!(#delta_name(from,heap));
    for (variant, position) in [(2, 1), (3, 3)] {
        let hole = &target.holes[steps.iter().find(|r| r.variant == variant).unwrap().hole].cq;
        assert_eq!(hole.query.body[position], delta_expected);
    }

    let index = steps.iter().find(|r| r.variant == 4).unwrap().hole;
    let hole = &target.holes[index].cq;
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
    prebound.holes[index].cq.query.body[3] = syn::parse_quote!(#all_bound(heap,from));
    assert!(check(&source, &prebound).is_err());
    let mut moved_guard = target.clone();
    let guard = moved_guard.holes[index].cq.query.body[1].clone();
    moved_guard.holes[index].cq.query.body[1] = moved_guard.holes[index].cq.query.body[2].clone();
    moved_guard.holes[index].cq.query.body[2] = guard;
    assert!(check(&source, &moved_guard).is_err());
    let mut moved_delta = target.clone();
    let atom = moved_delta.holes[index].cq.query.body[3].clone();
    moved_delta.holes[index].cq.query.body[3] = moved_delta.holes[index].cq.query.body[4].clone();
    moved_delta.holes[index].cq.query.body[4] = atom;
    assert!(check(&source, &moved_delta).is_err());
}

#[test]
fn positive_constant_probes_preserve_repeats_wildcards_and_later_delta() {
    let source = schedule(&syn::parse_quote! {
        struct Constants;
        relation E(x:i32,y:i32,z:i32) = (e);
        relation R(x:i32);
        output R;
        R(*x) :- E(x,x,7), E(_,x,(1 + 2));
        R(*y) :- E(y,7,_), if *y != 0, R(x), E(x,x,7);
        strata { component [R]; }
        incremental { scheme seminaive_covering;


            delta R = D;





            variant 0 seed { R(*x) :- E(x,x,7), E(_,x,(1 + 2)); }
            variant 1 delta 2 { R(*y) :- E(y,7,_), if *y != 0, D(x), E(x,x,7); }
        }
    })
    .unwrap();
    let target = compile(&source).unwrap();
    let layouts = &target.context.layouts;
    let first = &layouts[0].name;
    let second = &layouts[1].name;
    let third = &layouts[2].name;
    let fourth = &layouts[3].name;
    let delta = &target.context.delta_layouts[0].name;
    let stratum = &target.context.strata[0];
    assert!(stratum.seeds[0].scan.is_none());
    assert!(stratum.steps[0].scan.is_none());
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
        let hole = &target.holes[index].cq;
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
    let source =
        schedule(&syn::parse_str(include_str!("fixtures/delta_probes.datalog")).unwrap()).unwrap();
    let target = compile(&source).unwrap();
    let context = &target.context;
    assert_eq!(context.delta_layouts.len(), 4);
    assert_eq!(context.layouts.len(), 1);
    let constant = &context.delta_layouts[0].name;
    let keyed = &context.delta_layouts[1].name;
    let zero = &context.delta_layouts[2].name;
    let pair = &context.delta_layouts[3].name;
    let steps = &context.strata[0].steps;
    let bindings: Vec<_> = steps
        .iter()
        .map(|step| step.scan.as_ref().map(|scan| &scan.binding))
        .collect();
    let recent = bindings[0].unwrap();
    let scan2 = bindings[2].unwrap();
    let scan3 = bindings[3].unwrap();
    let scan4 = bindings[4].unwrap();
    let scan5 = bindings[5].unwrap();
    let expected_bodies = [
        quote!(#recent(x, y, z)),
        quote!(#constant(7,x,y)),
        quote!(#scan2(_,y,_), #keyed(y,x,z)),
        quote!(#scan3(_,y,_), if *y != 0, if 10 / *y > 0, #keyed(y,x,z)),
        quote!(#scan4(a,b,c), #zero(x,y,z)),
        quote!(#scan5(x,y,_), #pair(x,y,z)),
        quote!(#constant(7,x,x)),
    ];
    for (step, body) in context.strata[0].steps.iter().zip(expected_bodies) {
        let hole = &target.holes[step.hole].cq;
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
            f.holes[hole].cq.query.body[1] = syn::parse_quote!(Recent(x, y, z));
        },
        |f| {
            let hole = f.context.strata[0].steps[2].hole;
            let name = &f.context.delta_layouts[1].name;
            f.holes[hole].cq.query.body[1] = syn::parse_quote!(#name(x,y,z));
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
    let source = schedule(&syn::parse_quote! {
        struct ReverseDelta;
        relation E(x:i32,y:String) = (edges);
        relation R(x:i32,y:String);
        output R;
        R(*x,(*y).clone()) :- E(x,y);
        R(*x,(*y).clone()) :- E(_,y), R(x,y);
        strata { component [R]; }
        incremental { scheme seminaive_covering;


            delta R = D;


            variant 0 seed { R(*x,(*y).clone()) :- E(x,y); }
            variant 1 delta 1 { R(*x,(*y).clone()) :- E(_,y), D(x,y); }
        }
    })
    .unwrap();
    let target = compile(&source).unwrap();
    let layout = &target.context.delta_layouts[0];
    let hole = &target.holes[target.context.strata[0].steps[0].hole].cq;
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
    let source = schedule(&syn::parse_quote! {
        struct Generated;
        relation Seed(x:i32) = (seeds);
        relation Link(x:i32,y:i32) = (links);
        relation Reach(x:i32);
        output Reach;
        Reach(y) :- Seed(x), for y in [*x, *x];
        Reach(*z) :- Reach(x), if *x < 3, for y in [*x + 1], Link(y,z);
        strata { component [Reach]; }
        incremental { scheme seminaive_covering;


            delta Reach = Recent;


            variant 0 seed { Reach(y) :- Seed(x), for y in [*x, *x]; }
            variant 1 delta 0 {
                Reach(*z) :- Recent(x), if *x < 3, for y in [*x + 1], Link(y,z);
            }
        }
    })
    .unwrap();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let stratum = &target.context.strata[0];
    for step in stratum.seeds.iter().chain(&stratum.steps) {
        let hole = &target.holes[step.hole].cq;
        cq_lang_cq::contract::check(hole).unwrap();
        let original = &source.incremental.variants[step.variant].query;
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
    changed.holes[hole].cq.query.body[2] = syn::parse_quote!(for y in [*x + 2]);
    assert!(check(&source, &changed).is_err());
}

#[test]
fn device_keys_have_owned_schema_and_checked_membership_intrinsic() {
    let mut source = authored();
    source.policy.insertion = syn::parse_quote!(device_set_all_columns);
    source.policy.negation_keys = syn::parse_quote!(device_set_owned);
    let mut target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let replay: Fork = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(target, replay);
    check(&source, &replay).unwrap();
    assert_eq!(
        target.context.representation,
        cq_lang_datalog_round::KeyRepresentation::DeviceSetOwned
    );
    let key = &target.context.strata[0].keys[0];
    let key_hole = &target.holes[key.hole].cq;
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
        target.holes[rule.hole].cq.declarations.len(),
        2,
        "only used layout and output"
    );
    assert!(
        target.holes[rule.hole]
            .to_token_stream()
            .to_string()
            .contains("datalog :: contains")
    );
    target.context.representation = cq_lang_datalog_round::KeyRepresentation::Borrowed;
    assert!(check(&source, &target).is_err());
}

#[test]
fn frozen_device_layout_lifetime_is_an_exact_checked_certificate() {
    let mut source = authored();
    source.policy.insertion = syn::parse_quote!(device_set_all_columns);
    source.policy.negation_keys = syn::parse_quote!(device_set_owned);
    let target = compile(&source).unwrap();
    assert_eq!(
        target.context.layout_realization,
        cq_lang_datalog_round::LayoutRealization::MaintainedFrozenDeviceIndex
    );
    assert_eq!(target.context.strata[0].frozen_layouts, vec![1]);
    for layouts in [vec![], vec![0], vec![1, 1], vec![2]] {
        let mut wrong = target.clone();
        wrong.context.strata[0].frozen_layouts = layouts;
        assert!(
            check(&source, &wrong)
                .unwrap_err()
                .to_string()
                .contains("frozen device indexes")
        );
    }
    let mut wrong = target.clone();
    wrong.context.layout_realization =
        cq_lang_datalog_round::LayoutRealization::MaintainedGroupedHost;
    assert!(check(&source, &wrong).is_err());
    let cpu = compile(&authored()).unwrap();
    assert!(cpu.context.strata[0].frozen_layouts.is_empty());
}

#[test]
fn leveled_device_layouts_cover_only_changing_keyed_full_reads() {
    let mut source = schedule(&syn::parse_quote! {
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Reach(x,y), Reach(y,z);
        strata { component [Reach]; }
        incremental { scheme seminaive_covering;
            delta Reach = Recent;
            variant 0 seed { Reach(*x,*y) :- Edge(x,y); }
            variant 1 delta 0 { Reach(*x,*z) :- Recent(x,y), Reach(y,z); }
            variant 1 delta 1 { Reach(*x,*z) :- Reach(x,y), Recent(y,z); }
        }
    })
    .unwrap();
    source.policy.insertion = syn::parse_quote!(device_set_all_columns);
    source.policy.negation_keys = syn::parse_quote!(device_set_owned);
    let target = cq_datalog_fork::compile_with_ratio(&source, 8).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(target.context.leveled_ratio, 8);
    assert_eq!(
        target.context.changing_layout_realization,
        cq_lang_datalog_round::LayoutRealization::MaintainedLeveledDeviceIndex
    );
    assert_eq!(target.context.strata[0].leveled_layouts, [1]);
    assert!(target.context.strata[0].frozen_layouts.is_empty());
    for layouts in [vec![], vec![2], vec![1, 1], vec![3]] {
        let mut wrong = target.clone();
        wrong.context.strata[0].leveled_layouts = layouts;
        assert!(check(&source, &wrong).is_err());
    }
    let mut wrong = target.clone();
    wrong.context.changing_layout_realization =
        cq_lang_datalog_round::LayoutRealization::MaintainedFrozenDeviceIndex;
    assert!(check(&source, &wrong).is_err());
    assert!(cq_datalog_fork::compile_with_ratio(&source, 1).is_err());
    assert_eq!(compile(&source).unwrap().context.leveled_ratio, 4);
    assert!(
        compile(&authored()).unwrap().context.strata[0]
            .leveled_layouts
            .is_empty()
    );
}

#[test]
fn providers_are_copied_and_key_providers_are_opaque_rewrite_premises() {
    let mut source = authored();
    source.providers[2].all = syn::parse_quote!(custom::Rows<(i32, i32)>);
    source.providers[2].all_init = syn::parse_quote!(custom::Rows::new());
    source.layouts[0].provider = syn::parse_quote!(custom::Index);
    let target = compile(&source).unwrap();
    assert_eq!(target.context.relations[2].providers, source.providers[2]);
    assert_eq!(
        target.context.layouts[0].provider,
        source.layouts[0].provider
    );
    assert_eq!(
        target.context.layouts[0].maintenance,
        source.layouts[0].maintenance
    );
    let mut alternative = target.clone();
    alternative.context.strata[0].keys[0].provider = syn::parse_quote!(custom::BorrowedKeys<_>);
    cq_lang_datalog_round::check_rewrite(&target, &alternative).unwrap();
    let replay: Fork = syn::parse2(alternative.to_token_stream()).unwrap();
    assert_eq!(alternative, replay);
    check(&source, &replay).unwrap();
}
