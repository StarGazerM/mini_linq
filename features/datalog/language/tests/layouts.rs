use cq_lang_datalog::{Layout, Program, check_rewrite, contract};
use quote::ToTokens;

const SEMINAIVE: &str = include_str!("fixtures/seminaive.datalog");

#[test]
fn layouts_are_exact_complete_permutations_with_valid_leading_keys() {
    let original: Program = syn::parse_str(SEMINAIVE).unwrap();
    let mutations: &[fn(&mut Program)] = &[
        |p| p.layouts.clear(),
        |p| {
            p.layouts.pop();
        },
        |p| p.layouts.push(p.layouts[0].clone()),
        |p| p.layouts.reverse(),
        |p| p.layouts[0].relation = syn::parse_quote!(Unknown),
        |p| p.layouts[0].relation = syn::parse_quote!(ReachChanges),
        |p| p.layouts[0].relation = syn::parse_quote!(Blocked),
        |p| p.layouts[0].columns.clear(),
        |p| p.layouts[0].columns = vec![0],
        |p| p.layouts[0].columns = vec![0, 0],
        |p| p.layouts[0].columns = vec![0, 2],
        |p| p.layouts[0].columns = vec![0, 1, 2],
        |p| p.layouts[0].columns = vec![1, 0],
        |p| p.layouts[0].key = 3,
        |p| p.layouts[0].key = 1,
        |p| p.layouts[1].key = 0,
        |p| p.layouts[2].columns = vec![1, 0],
        |p| {
            p.layouts.push(Layout {
                relation: syn::parse_quote!(Blocked),
                columns: vec![0, 1],
                key: 0,
            })
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = original.clone();
        mutate(&mut changed);
        assert!(contract::check(&changed).is_err(), "mutation {index}");
        assert!(
            check_rewrite(&original, &changed).is_err(),
            "rewrite {index}"
        );
    }
}

#[test]
fn unscheduled_stages_cannot_hide_layout_decisions() {
    for source in [
        include_str!("fixtures/source.datalog"),
        include_str!("fixtures/stratified.datalog"),
    ] {
        let mut program: Program = syn::parse_str(source).unwrap();
        assert!(program.layouts.is_empty());
        program.layouts.push(Layout {
            relation: syn::parse_quote!(Edge),
            columns: vec![0, 1],
            key: 0,
        });
        assert!(contract::check(&program).is_err());
    }
}

#[test]
fn layout_syntax_has_one_unambiguous_position_and_round_trips() {
    let program: Program = syn::parse_str(SEMINAIVE).unwrap();
    assert_eq!(program.layouts.len(), 3);
    assert_eq!(program.layouts[2].columns, vec![0, 1]);
    let restored: Program = syn::parse2(program.to_token_stream()).unwrap();
    assert_eq!(restored.layouts, program.layouts);
    for malformed in [
        SEMINAIVE.replace("layout Edge [0,1] key 0;", "layout Edge [0,1] 0;"),
        SEMINAIVE.replace("layout Edge [0,1] key 0;", "layout Edge [0,1] key -1;"),
        SEMINAIVE.replace(
            "delta Reach = ReachChanges;",
            "layout Edge [0,1] key 0; delta Reach = ReachChanges;",
        ),
        SEMINAIVE
            .replace("variant 0 seed", "layout Edge [0,1] key 0; variant 0 seed")
            .replace(
                "variant 1 delta 0",
                "layout Edge [0,1] key 0; variant 1 delta 0",
            ),
    ] {
        assert!(
            syn::parse_str::<Program>(&malformed).is_err(),
            "{malformed}"
        );
    }
}

#[test]
fn variant_order_does_not_redefine_layout_order_but_rewrites_preserve_both() {
    let source: Program = syn::parse_str(SEMINAIVE).unwrap();
    let mut target = source.clone();
    target.variants.as_mut().unwrap().reverse();
    contract::check(&target).unwrap();
    assert_eq!(source.layouts, target.layouts);
    assert!(check_rewrite(&source, &target).is_err());
    let mut erased = source.clone();
    erased.variants = None;
    erased.deltas.clear();
    erased.layouts.clear();
    erased.insertion = None;
    erased.negation_keys = None;
    contract::check(&erased).unwrap();
    assert!(check_rewrite(&source, &erased).is_err());
}

#[test]
fn source_order_not_later_delta_bindings_determines_var_points_to_keys() {
    let source: Program = syn::parse_str(include_str!("fixtures/written_order.datalog")).unwrap();
    contract::check(&source).unwrap();
    let mut reordered = source.clone();
    reordered.variants.as_mut().unwrap().reverse();
    contract::check(&reordered).unwrap();
    assert!(check_rewrite(&source, &reordered).is_err());

    // The later delta binds heap only after VarPointsTo(heap,from).
    // Its col1 host is shared with an earlier variant, not replaced by col0+1.
    assert_eq!(source.layouts[1].columns, [1, 0]);
    assert_eq!(source.layouts[1].key, 1);
    let mut missing = source.clone();
    missing.layouts.remove(1);
    assert!(contract::check(&missing).is_err());
    let mut prebound = source.clone();
    prebound.layouts[3].key = 1;
    prebound.layouts[3].columns = vec![1, 0];
    assert!(contract::check(&prebound).is_err());
    let replay: Program = syn::parse2(source.to_token_stream()).unwrap();
    assert_eq!(source, replay);
    contract::check(&replay).unwrap();
}

#[test]
fn authored_constant_keys_do_not_bind_wildcards_or_repeated_new_variables() {
    let source: Program = syn::parse_quote! {
        struct Constants;
        relation E(x:i32,y:i32,z:i32) = (e);
        relation R(x:i32);
        output R;
        R(*x) :- E(x,x,7), E(_,x,(1 + 2));
        strata { component [R]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta R = D;
            layout E [2,0,1] key 1;
            layout E [1,2,0] key 2;
            variant 0 seed { R(*x) :- E(x,x,7), E(_,x,(1 + 2)); }
        }
    };
    contract::check(&source).unwrap();
    for index in 0..2 {
        let mut missing_constant = source.clone();
        missing_constant.layouts[index].key -= 1;
        assert!(contract::check(&missing_constant).is_err());
    }
    let mut repeated = source.clone();
    repeated.layouts[0].key = 2;
    assert!(contract::check(&repeated).is_err());
    let mut wildcard = source.clone();
    wildcard.layouts[1].key = 3;
    assert!(contract::check(&wildcard).is_err());
}
