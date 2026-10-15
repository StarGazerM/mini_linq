use cq_datalog_schedule::{seminaive, stratify};
use cq_lang_datalog::{Program, contract};

fn scheduled(source: Program) -> (Program, Program) {
    let strata = stratify::compile(&source).unwrap();
    let target = seminaive::compile(&strata).unwrap();
    (strata, target)
}

fn choices(program: &Program) -> Vec<(String, Vec<usize>, usize)> {
    program.layouts.iter().map(|layout| {
        (layout.relation.to_string(), layout.columns.clone(), layout.key)
    }).collect()
}

#[test]
fn delta_drives_first_even_when_it_is_not_the_first_positive() {
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
    assert_eq!(choices(&target), vec![
        ("E".into(), vec![0,1,2], 0),
        ("E".into(), vec![1,0,2], 1),
        ("F".into(), vec![0,1,2], 3),
        ("E".into(), vec![0,1,2], 3),
        ("R".into(), vec![0,1,2], 3),
    ]);
    assert_eq!(target.variants.as_ref().unwrap()[1].delta, Some(1));
    seminaive::check(&strata, &target).unwrap();
    let mut wrong = target.clone();
    wrong.layouts[1].columns = vec![0,1,2];
    wrong.layouts[1].key = 0;
    assert!(seminaive::check(&strata, &wrong).is_err());
}

#[test]
fn repeated_columns_constants_wildcards_and_lets_do_not_create_early_keys() {
    let (_, target) = scheduled(syn::parse_quote! {
        struct Bindings;
        relation E(x:i32,y:i32,z:i32) = (e);
        relation A(x:i32);
        output A;
        A(*x) :- E(x,x,_), let n = *x, E(x,x,7), E(_,x,n);
    });
    assert_eq!(choices(&target), vec![
        ("E".into(), vec![0,1,2], 0),
        ("E".into(), vec![0,1,2], 2),
        ("E".into(), vec![1,0,2], 1),
    ]);
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
    assert_eq!(choices(&target), vec![
        ("E".into(), vec![0], 0),
        ("A".into(), vec![0], 0),
        ("F".into(), vec![0], 0),
    ]);
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
