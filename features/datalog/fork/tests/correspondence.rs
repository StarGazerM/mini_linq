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
        assert!(!hole.query.body.iter().any(|item| matches!(item, BodyItem::Negation(_))));
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
    }).unwrap();
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
    rejected(|t| t.context.output = 0);
    rejected(|t| t.context.observer = Some(syn::parse_quote!(observer)));
    rejected(|t| t.context.relations[2].delta_host = None);
    rejected(|t| t.context.relations[2].delta_host = Some(syn::parse_quote!(Recent)));
    rejected(|t| t.context.relations[0].all = t.context.relations[1].all.clone());
    rejected(|t| t.context.strata[0].members.clear());
    rejected(|t| t.context.strata[0].steps[0].head = 0);
    rejected(|t| t.context.strata[0].steps[0].variant = 0);
    rejected(|t| t.context.strata[0].steps.clear());
    rejected(|t| { t.holes.pop(); });
    rejected(|t| t.holes.push(t.holes[0].clone()));
    rejected(|t| t.context.layouts.clear());
    rejected(|t| t.context.layouts[0].key = 2);
    rejected(|t| t.context.layouts[1].columns.swap(0,1));
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
    rejected(|t| t.context.strata[0].keys[0].columns = vec![0,0]);
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
    }).unwrap();
    assert_eq!(visited, compile(&source).unwrap().holes);
    assert_eq!(expanded.holes.len(), 3);
    let mut calls = 0;
    assert!(fork(&source, |hole| {
        calls += 1;
        Err(syn::Error::new_spanned(hole, "callback rejected hole"))
    }).is_err());
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
    let declaration = hole.declarations.iter().find(|d| d.name == layout.name).unwrap();
    assert_eq!(declaration.columns[0].ty, syn::parse_quote!(String));
    let name = &layout.name;
    let expected: BodyItem = syn::parse_quote!(#name(y,z));
    assert_eq!(hole.query.body[1], expected);
}
