use cq_datalog_schedule::{seminaive, stratify};
use cq_lang_datalog::{BodyItem, Program, contract};
use quote::ToTokens;

fn scheduled(source: Program) -> (Program, Program) {
    let strata = stratify::compile(&source).unwrap();
    let target = seminaive::compile(&strata).unwrap();
    (strata, target)
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
    assert_eq!(target.variants.as_ref().unwrap()[1].delta, Some(1));
    seminaive::check(&strata, &target).unwrap();
    let mut wrong = target.clone();
    wrong.layouts[0].columns = vec![1, 0, 2];
    wrong.layouts[0].key = 1;
    assert!(seminaive::check(&strata, &wrong).is_err());
    for variant in target.variants.as_ref().unwrap() {
        let mut erased = variant.query.clone();
        if let Some(position) = variant.delta {
            let BodyItem::Positive { atom } = &mut erased.body[position] else {
                unreachable!()
            };
            let BodyItem::Positive { atom: original } = &strata.rules[variant.rule].body[position]
            else {
                unreachable!()
            };
            atom.relation = original.relation.clone();
        }
        assert_eq!(erased, strata.rules[variant.rule]);
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
    assert_eq!(target.negation_keys.as_ref().unwrap(), "hash_set_borrowed");
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
    assert_eq!(strata.strata.as_ref().unwrap().len(), 2);
    assert_eq!(target.deltas.len(), 2);
    assert_eq!(target.variants.as_ref().unwrap().len(), 1);
    assert_eq!(choices(&target), vec![("E".into(), vec![0], 0)]);
}

#[test]
fn later_delta_reuses_the_existing_var_points_to_column_one_layout() {
    let text = include_str!("../../language/tests/fixtures/written_order.datalog");
    let mut strata: Program = syn::parse_str(text).unwrap();
    let authored = strata.clone();
    strata.variants = None;
    strata.deltas.clear();
    strata.layouts.clear();
    strata.delta_layouts.clear();
    strata.insertion = None;
    strata.negation_keys = None;
    let target = seminaive::compile(&strata).unwrap();
    seminaive::check(&strata, &authored).unwrap();
    let expected: Program =
        syn::parse_str(&text.replace("RecentPointsTo", &target.deltas[0].name.to_string()))
            .unwrap();
    assert_eq!(target, expected);
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
    seminaive::check(&strata, &replay).unwrap();
}

#[test]
fn delta_demands_cover_constants_later_scans_and_shared_mixed_keys_exactly() {
    let text = include_str!("../../language/tests/fixtures/delta_probes.datalog");
    let authored: Program = syn::parse_str(text).unwrap();
    let mut source = authored.clone();
    source.strata = None;
    source.variants = None;
    source.deltas.clear();
    source.layouts.clear();
    source.delta_layouts.clear();
    source.insertion = None;
    source.negation_keys = None;
    let (strata, target) = scheduled(source);
    seminaive::check(&strata, &authored).unwrap();
    let expected: Program =
        syn::parse_str(&text.replace("Recent", &target.deltas[0].name.to_string())).unwrap();
    assert_eq!(target, expected);
    let decisions: Vec<_> = target
        .delta_layouts
        .iter()
        .map(|layout| {
            (
                layout.relation.to_string(),
                layout.columns.clone(),
                layout.key,
            )
        })
        .collect();
    assert_eq!(
        decisions,
        vec![
            ("R".into(), vec![2, 0, 1], 1),
            ("R".into(), vec![1, 0, 2], 1),
            ("R".into(), vec![0, 1, 2], 0),
            ("R".into(), vec![0, 1, 2], 2),
        ]
    );
    assert_eq!(choices(&target), vec![("E".into(), vec![0, 1, 2], 0)]);
    let replay: Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(target, replay);
    seminaive::check(&strata, &replay).unwrap();
}

#[test]
fn gpu_delta_driver_is_recorded_and_independently_checked() {
    let source: Program = syn::parse_quote! {
        struct Drivers;
        relation E(x:i32,y:i32) = (e);
        relation R(x:i32,y:i32);
        output R;
        R(*x,*y) :- E(x,y);
        R(*x,*z) :- E(x,y), R(y,z);
    };
    let strata = stratify::compile(&source).unwrap();
    let target = seminaive::compile_with_driver(
        &strata,
        seminaive::Representation::DeviceSet,
        seminaive::DriverPolicy::DeltaFirst,
    )
    .unwrap();
    let variant = &target.variants.as_ref().unwrap()[1];
    assert_eq!(variant.delta, Some(1));
    assert_eq!(variant.driver, Some(1));
    assert!(matches!(&variant.query.body[0], BodyItem::Positive { atom }
        if atom.relation == target.deltas[0].name));
    assert!(target.delta_layouts.is_empty());
    assert!(
        target
            .to_token_stream()
            .to_string()
            .contains("delta 1 driver 1")
    );
    let replay = syn::parse2(target.to_token_stream()).unwrap();
    seminaive::check(&strata, &replay).unwrap();
    let mut wrong = target.clone();
    wrong.variants.as_mut().unwrap()[1].driver = Some(0);
    assert!(contract::check(&wrong).is_err());
    let mut wrong = target.clone();
    let body = &mut wrong.variants.as_mut().unwrap()[1].query.body;
    let first = body[0].clone();
    body[0] = body[1].clone();
    body[1] = first;
    assert!(contract::check(&wrong).is_err());
    let mut wrong = target.clone();
    wrong.variants.as_mut().unwrap()[1].delta = Some(0);
    assert!(contract::check(&wrong).is_err());
    let cpu = seminaive::compile(&strata).unwrap();
    assert_eq!(cpu.variants.as_ref().unwrap()[1].driver, Some(0));
}

#[test]
fn delta_driver_never_crosses_before_constraints() {
    for clause in ["if *x > 0", "!Blocked(x)", "let copied = *x"] {
        let source: Program = syn::parse_str(&format!(
            "struct Barriers;
             relation E(x:i32) = (e);
             relation Blocked(x:i32) = (blocked);
             relation R(x:i32);
             output R;
             R(*x) :- E(x);
             R(*x) :- E(x), {clause}, R(x);"
        ))
        .unwrap();
        let strata = stratify::compile(&source).unwrap();
        let target = seminaive::compile_with_driver(
            &strata,
            seminaive::Representation::DeviceSet,
            seminaive::DriverPolicy::DeltaFirst,
        )
        .unwrap();
        assert_eq!(target.variants.as_ref().unwrap()[1].driver, Some(0));
        let mut wrong = target.clone();
        wrong.variants.as_mut().unwrap()[1].driver = Some(2);
        assert!(
            contract::check(&wrong)
                .unwrap_err()
                .to_string()
                .contains("Before")
        );
    }
}
