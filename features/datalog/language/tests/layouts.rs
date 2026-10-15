use cq_lang_datalog::{Layout, Program, check_rewrite, contract};
use quote::ToTokens;

const SEMINAIVE: &str = include_str!("fixtures/seminaive.datalog");

#[test]
fn layouts_are_exact_complete_permutations_with_valid_leading_keys() {
    let original: Program = syn::parse_str(SEMINAIVE).unwrap();
    let mutations: Vec<Box<dyn Fn(&mut Program)>> = vec![
        Box::new(|p| p.layouts.clear()),
        Box::new(|p| { p.layouts.pop(); }),
        Box::new(|p| p.layouts.push(p.layouts[0].clone())),
        Box::new(|p| p.layouts.reverse()),
        Box::new(|p| p.layouts[0].relation = syn::parse_quote!(Unknown)),
        Box::new(|p| p.layouts[0].relation = syn::parse_quote!(ReachChanges)),
        Box::new(|p| p.layouts[0].relation = syn::parse_quote!(Blocked)),
        Box::new(|p| p.layouts[0].columns.clear()),
        Box::new(|p| p.layouts[0].columns = vec![0]),
        Box::new(|p| p.layouts[0].columns = vec![0, 0]),
        Box::new(|p| p.layouts[0].columns = vec![0, 2]),
        Box::new(|p| p.layouts[0].columns = vec![0, 1, 2]),
        Box::new(|p| p.layouts[0].columns = vec![1, 0]),
        Box::new(|p| p.layouts[0].key = 3),
        Box::new(|p| p.layouts[0].key = 1),
        Box::new(|p| p.layouts[1].key = 0),
        Box::new(|p| p.layouts[2].columns = vec![0, 1]),
        Box::new(|p| p.layouts.push(Layout {
            relation: syn::parse_quote!(Blocked), columns: vec![0, 1], key: 0,
        })),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut changed = original.clone();
        mutate(&mut changed);
        assert!(contract::check(&changed).is_err(), "mutation {index}");
        assert!(check_rewrite(&original, &changed).is_err(), "rewrite {index}");
    }
}

#[test]
fn unscheduled_stages_cannot_hide_layout_decisions() {
    for source in [include_str!("fixtures/source.datalog"), include_str!("fixtures/stratified.datalog")] {
        let mut program: Program = syn::parse_str(source).unwrap();
        assert!(program.layouts.is_empty());
        program.layouts.push(Layout {
            relation: syn::parse_quote!(Edge), columns: vec![0, 1], key: 0,
        });
        assert!(contract::check(&program).is_err());
    }
}

#[test]
fn layout_syntax_has_one_unambiguous_position_and_round_trips() {
    let program: Program = syn::parse_str(SEMINAIVE).unwrap();
    assert_eq!(program.layouts.len(), 4);
    assert_eq!(program.layouts[2].columns, vec![1, 0]);
    let restored: Program = syn::parse2(program.to_token_stream()).unwrap();
    assert_eq!(restored.layouts, program.layouts);
    for malformed in [
        SEMINAIVE.replace("layout Edge [0,1] key 0;", "layout Edge [0,1] 0;"),
        SEMINAIVE.replace("layout Edge [0,1] key 0;", "layout Edge [0,1] key -1;"),
        SEMINAIVE.replace("delta Reach = ReachChanges;", "layout Edge [0,1] key 0; delta Reach = ReachChanges;"),
        SEMINAIVE.replace("variant 0 seed", "layout Edge [0,1] key 0; variant 0 seed")
            .replace("variant 1 delta 0", "layout Edge [0,1] key 0; variant 1 delta 0"),
    ] {
        assert!(syn::parse_str::<Program>(&malformed).is_err(), "{malformed}");
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
