use cq_lang_datalog_strata::{Program, check_rewrite, contract};
type Mutation = Box<dyn Fn(&mut Program)>;
const STRATIFIED: &str = include_str!("fixtures/stratified.datalog");
fn scheduled() -> Program {
    syn::parse_str(STRATIFIED).unwrap()
}
#[test]
fn schedule_cover_and_order_mutations_are_rejected() {
    let original = scheduled();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| {
            p.components.clear();
        }),
        Box::new(|p| {
            p.components[0].clear();
        }),
        Box::new(|p| {
            p.components.pop();
        }),
        Box::new(|p| {
            p.components.reverse();
        }),
        Box::new(|p| {
            p.components[0].push(syn::parse_quote!(Reach));
        }),
        Box::new(|p| {
            p.components[0].push(syn::parse_quote!(Edge));
        }),
        Box::new(|p| {
            p.components[0].push(syn::parse_quote!(Unknown));
        }),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut program = original.clone();
        mutate(&mut program);
        assert!(contract::check(&program).is_err(), "mutation {index}");
        assert!(
            check_rewrite(&original, &program).is_err(),
            "mutation {index}"
        );
        assert!(
            check_rewrite(&program, &original).is_err(),
            "mutation {index}"
        );
    }
}

#[test]
fn mutual_sccs_cannot_be_split_but_member_order_can_change() {
    let mutual: Program = syn::parse_quote! {
        struct Mutual;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output A;
        A(*x) :- E(x);
        A(*x) :- B(x);
        B(*x) :- A(x);
        strata { component [A,B]; }
    };
    contract::check(&mutual).unwrap();
    let mut split = mutual.clone();
    split.components = vec![vec![syn::parse_quote!(A)], vec![syn::parse_quote!(B)]];
    assert!(contract::check(&split).is_err());
    let mut reordered = mutual.clone();
    reordered.components[0].reverse();
    contract::check(&reordered).unwrap();
    check_rewrite(&mutual, &reordered).unwrap();
}

#[test]
fn coarser_components_preserve_source_and_reclassify_changing_positions() {
    let original = scheduled();
    let mut coarser = original.clone();
    coarser.components = vec![vec![syn::parse_quote!(Result), syn::parse_quote!(Reach)]];
    let analysis = contract::check(&coarser).unwrap();
    assert_eq!(
        analysis.changing_positions,
        vec![vec![], vec![0, 1], vec![0]]
    );
    check_rewrite(&original, &coarser).unwrap();
    check_rewrite(&coarser, &original).unwrap();

    let mut changed_source = coarser.clone();
    changed_source.source.rules.swap(0, 1);
    contract::check(&changed_source).unwrap();
    assert!(check_rewrite(&original, &changed_source).is_err());
}

#[test]
fn independent_components_can_be_reordered_or_merged() {
    let original: Program = syn::parse_quote! {
        struct Independent;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output A, B;
        A(*x) :- E(x);
        B(*x) :- E(x);
        strata { component [A]; component [B]; }
    };
    let mut reordered = original.clone();
    reordered.components.reverse();
    check_rewrite(&original, &reordered).unwrap();
    let mut merged = original.clone();
    merged.components = vec![vec![syn::parse_quote!(B), syn::parse_quote!(A)]];
    check_rewrite(&original, &merged).unwrap();
}

#[test]
fn negative_idb_dependencies_must_remain_in_completed_lower_components() {
    let original: Program = syn::parse_quote! {
        struct Negative;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output B;
        A(*x) :- E(x);
        B(*x) :- E(x), !A(x);
        strata { component [A]; component [B]; }
    };
    contract::check(&original).unwrap();
    let mut merged = original.clone();
    merged.components = vec![vec![syn::parse_quote!(A), syn::parse_quote!(B)]];
    assert!(contract::check(&merged).is_err());
    assert!(check_rewrite(&original, &merged).is_err());
    let mut reversed = original.clone();
    reversed.components.reverse();
    assert!(contract::check(&reversed).is_err());
}
