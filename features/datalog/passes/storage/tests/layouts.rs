use cq_datalog_storage::{check, compile};
use cq_datalog_stratify as stratify;
use cq_lang_datalog_storage::{BodyItem, Program, contract, incremental};
use incremental::strata::source;
use quote::ToTokens;

fn scheduled(source: source::Program) -> (incremental::Program, Program) {
    let strata = stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let target = compile(&incremental).unwrap();
    (incremental, target)
}

fn choices(program: &Program) -> Vec<(String, Vec<usize>, usize)> {
    program
        .layouts
        .iter()
        .map(|layout| {
            (
                layout.relation.to_string(),
                layout.columns.clone(),
                layout.key,
            )
        })
        .collect()
}

#[test]
fn delta_binds_only_at_its_written_positive_position() {
    let (strata, target) = scheduled(syn::parse_quote! {
        struct Layouts;
        relation E(x:i32,y:i32,z:i32) = (e);
        relation F(x:i32,y:i32,z:i32) = (f);
        relation R(x:i32,y:i32,z:i32);
        output R;
        R(*x,*y,*z) :- E(x,y,z);
        R(*x,*z,*w) :- E(x,y,_), R(y,z,w), F(z,x,w), E(w,x,y), E(w,x,y);
        R(*x,*y,*z) :- R(x,y,z), R(z,y,x);
    });
    assert_eq!(
        choices(&target),
        vec![
            ("E".into(), vec![0, 1, 2], 0),
            ("F".into(), vec![0, 1, 2], 3),
            ("E".into(), vec![0, 1, 2], 3),
            ("R".into(), vec![0, 1, 2], 3),
            ("R".into(), vec![0, 1, 2], 0),
        ]
    );
    assert_eq!(target.incremental.variants[1].delta, Some(1));
    check(&strata, &target).unwrap();
    let mut wrong = target.clone();
    wrong.layouts[0].columns = vec![1, 0, 2];
    wrong.layouts[0].key = 1;
    assert!(check(&strata, &wrong).is_err());
    for variant in &target.incremental.variants {
        let mut erased = variant.query.clone();
        if let Some(position) = variant.delta {
            let BodyItem::Positive { atom } = &mut erased.body[position] else {
                unreachable!()
            };
            let BodyItem::Positive { atom: original } =
                &strata.strata.source.rules[variant.rule].body[position]
            else {
                unreachable!()
            };
            atom.relation = original.relation.clone();
        }
        assert_eq!(erased, strata.strata.source.rules[variant.rule]);
    }
}

#[test]
fn constants_are_keys_but_wildcards_repeats_and_lets_do_not_prebind_variables() {
    let (_, target) = scheduled(syn::parse_quote! {
        struct Bindings;
        relation E(x:i32,y:i32,z:i32) = (e);
        relation A(x:i32);
        output A;
        A(*x) :- E(x,x,_), let n = *x, E(x,x,7), E(_,x,n), E(_,_,(1 + 2));
    });
    assert_eq!(
        choices(&target),
        vec![
            ("E".into(), vec![0, 1, 2], 0),
            ("E".into(), vec![0, 1, 2], 3),
            ("E".into(), vec![1, 0, 2], 1),
            ("E".into(), vec![2, 0, 1], 1),
        ]
    );
    contract::check(&target).unwrap();
}

#[test]
fn disconnected_full_reads_are_scans_and_negative_reads_have_separate_policy() {
    let (_, target) = scheduled(syn::parse_quote! {
        struct Scans;
        relation E(x:i32) = (e);
        relation F(x:i32) = (f);
        relation Blocked(x:i32) = (blocked);
        relation A(x:i32);
        relation B(x:i32);
        output B;
        A(*x) :- E(x);
        B(*x) :- A(x), F(y), !Blocked(x);
    });
    assert_eq!(
        choices(&target),
        vec![
            ("E".into(), vec![0], 0),
            ("A".into(), vec![0], 0),
            ("F".into(), vec![0], 0),
        ]
    );
    assert_eq!(&target.policy.negation_keys, "borrowed");
}

#[test]
fn independent_strata_and_empty_idbs_are_explicit() {
    let (strata, target) = scheduled(syn::parse_quote! {
        struct Empty;
        relation E(x:i32) = (e);
        relation A(x:i32);
        relation Empty(x:i32);
        output Empty;
        A(*x) :- E(x);
    });
    assert_eq!(strata.strata.components.len(), 2);
    assert_eq!(target.incremental.deltas.len(), 2);
    assert_eq!(target.incremental.variants.len(), 1);
    assert_eq!(choices(&target), vec![("E".into(), vec![0], 0)]);
}

#[test]
fn authored_written_order_layouts_preserve_later_delta_probes() {
    let source: incremental::Program =
        syn::parse_str(include_str!("fixtures/written_order.datalog")).unwrap();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(
        choices(&target),
        vec![
            ("Alloc".into(), vec![0, 1], 0),
            ("VarPointsTo".into(), vec![1, 0], 1),
            ("Assign".into(), vec![1, 0], 1),
            ("VarPointsTo".into(), vec![0, 1], 0),
            ("Assign".into(), vec![0, 1], 0),
            ("VarPointsTo".into(), vec![0, 1], 2),
        ]
    );
    let replay: Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(target, replay);
    check(&source, &replay).unwrap();
}
#[test]
fn delta_demands_cover_constants_later_scans_and_shared_mixed_keys_exactly() {
    let source: incremental::Program =
        syn::parse_str(include_str!("fixtures/delta_probes.datalog")).unwrap();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let decisions: Vec<_> = target
        .delta_layouts
        .iter()
        .map(|l| (l.relation.to_string(), l.columns.clone(), l.key))
        .collect();
    assert_eq!(
        decisions,
        vec![
            ("R".into(), vec![2, 0, 1], 1),
            ("R".into(), vec![1, 0, 2], 1),
            ("R".into(), vec![0, 1, 2], 0),
            ("R".into(), vec![0, 1, 2], 2)
        ]
    );
    assert_eq!(choices(&target), vec![("E".into(), vec![0, 1, 2], 0)]);
    let replay: Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(target, replay);
    check(&source, &replay).unwrap();
}
#[test]
fn insertion_negation_and_read_semantics_are_checked() {
    let (_, source) = scheduled(
        syn::parse_quote! {struct Q;relation E(x:i32)=(e);relation R(x:i32);output R;R(*x):-E(x);},
    );
    let changes: &[fn(&mut Program)] = &[
        |p| p.policy.insertion = syn::parse_quote!(bag),
        |p| p.policy.negation_keys = syn::parse_quote!(owned),
        |p| p.reads.clear(),
        |p| p.reads[0].position = usize::MAX,
        |p| p.reads[0].index = usize::MAX,
    ];
    for mutate in changes {
        let mut changed = source.clone();
        mutate(&mut changed);
        assert!(contract::check(&changed).is_err());
    }
}

#[test]
fn default_providers_record_concrete_types_in_declaration_order() {
    let (_, target) = scheduled(syn::parse_quote! {
        struct Providers;
        relation E(x:i32,y:u64) = (e);
        relation R(y:u64,x:i32);
        output R;
        R(*y,*x) :- E(x,y);
        R(*y,*x) :- R(y,x), E(x,y);
    });
    let expected: Vec<cq_lang_datalog_storage::Providers> = vec![
        syn::parse_quote! {
            canonical_name { Canonical }
            canonical_set { ::cq_storage::HashSet<R> }
            all { ::std::vec::Vec<(i32, u64,)> }
            seen { ::cq_storage::HashSet<(i32, u64,)> }
            delta { ::std::vec::Vec<(i32, u64,)> }
            next { ::std::vec::Vec<(i32, u64,)> }
            all_init { ::std::vec::Vec::new() }
            delta_init { ::std::vec::Vec::new() }
            next_init { ::std::vec::Vec::new() }
        },
        syn::parse_quote! {
            canonical_name { Canonical }
            canonical_set { ::cq_storage::HashSet<R> }
            all { ::std::vec::Vec<(u64, i32,)> }
            seen { ::cq_storage::HashSet<(u64, i32,)> }
            delta { ::std::vec::Vec<(u64, i32,)> }
            next { ::std::vec::Vec<(u64, i32,)> }
            all_init { ::std::vec::Vec::new() }
            delta_init { ::std::vec::Vec::new() }
            next_init { ::std::vec::Vec::new() }
        },
    ];
    assert_eq!(target.providers, expected);
    let scan: syn::Type = syn::parse_quote!(
        ::std::collections::HashMap<(), ::std::vec::Vec<(i32, u64,)>, ::cq_storage::HashState>
    );
    assert_eq!(target.layouts[0].provider, scan);
    let lookup: syn::Type = syn::parse_quote!(
        ::std::collections::HashMap<(i32, u64,), ::std::vec::Vec<()>, ::cq_storage::HashState>
    );
    assert_eq!(target.layouts[1].provider, lookup);
    assert!(
        target
            .layouts
            .iter()
            .all(|layout| layout.maintenance == "admitted_rows")
    );
    assert!(
        target
            .delta_layouts
            .iter()
            .all(|layout| layout.maintenance == "replace_each_run")
    );
    let replay: Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(target, replay);
}
