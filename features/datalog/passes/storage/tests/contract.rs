use cq_lang_datalog_storage::{Layout, Program, check_rewrite, contract, incremental};
fn authored(text: &str) -> Program {
    cq_datalog_storage::compile(&syn::parse_str(text).unwrap()).unwrap()
}
use quote::ToTokens;

const INCREMENTAL: &str = include_str!("fixtures/incremental.datalog");

#[test]
fn layouts_require_complete_permutations_and_available_leading_keys() {
    let original: Program = authored(INCREMENTAL);
    let mutations: &[fn(&mut Program)] = &[
        |p| p.layouts.clear(),
        |p| {
            p.layouts.pop();
        },
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
        |p| p.layouts[2].columns = vec![1, 0],
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
fn storage_choices_are_not_accepted_by_logical_languages() {
    let source = authored(INCREMENTAL);
    assert!(syn::parse2::<incremental::Program>(source.to_token_stream()).is_err());
    assert!(syn::parse2::<incremental::strata::Program>(source.to_token_stream()).is_err());
    assert!(syn::parse2::<incremental::strata::source::Program>(source.to_token_stream()).is_err());
}
#[test]
fn layout_syntax_round_trips_and_rejects_malformed_records() {
    let source = authored(INCREMENTAL);
    let tokens = source.to_token_stream();
    assert_eq!(syn::parse2::<Program>(tokens.clone()).unwrap(), source);
    for malformed in [
        tokens.to_string().replacen("key { 0", "key { -1", 1),
        tokens.to_string().replacen("columns", "unknown", 1),
    ] {
        assert!(syn::parse_str::<Program>(&malformed).is_err());
    }
}

#[test]
fn variant_order_does_not_redefine_layout_order_but_rewrites_preserve_both() {
    let source: Program = authored(INCREMENTAL);
    let mut target = source.clone();
    target.incremental.variants.reverse();
    let analysis = incremental::contract::check(&target.incremental).unwrap();
    (target.reads, target.negations) =
        cq_lang_datalog_storage::read_plan(&target, &analysis).unwrap();
    contract::check(&target).unwrap();
    assert_eq!(source.layouts, target.layouts);
    assert_eq!(source.delta_layouts, target.delta_layouts);
    assert!(check_rewrite(&source, &target).is_err());
    let mut erased = source.clone();
    erased.reads.clear();
    assert!(contract::check(&erased).is_err());
    assert!(check_rewrite(&source, &erased).is_err());
}

#[test]
fn source_order_not_later_delta_bindings_determines_var_points_to_keys() {
    let source: Program = authored(include_str!("fixtures/written_order.datalog"));
    contract::check(&source).unwrap();
    let mut reordered = source.clone();
    reordered.incremental.variants.reverse();
    let analysis = incremental::contract::check(&reordered.incremental).unwrap();
    (reordered.reads, reordered.negations) =
        cq_lang_datalog_storage::read_plan(&reordered, &analysis).unwrap();
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
fn authored_constant_keys_are_optional_but_new_variables_and_wildcards_are_not_keys() {
    let source = cq_datalog_storage::compile(&syn::parse_quote! {
        struct Constants;
        relation E(x:i32,y:i32,z:i32) = (e);
        relation R(x:i32);
        output R;
        R(*x) :- E(x,x,7), E(_,x,(1 + 2));
        strata { component [R]; }
        incremental { scheme seminaive_covering;


            delta R = D;


            variant 0 seed { R(*x) :- E(x,x,7), E(_,x,(1 + 2)); }
        }
    })
    .unwrap();
    contract::check(&source).unwrap();
    for index in 0..2 {
        let mut missing_constant = source.clone();
        missing_constant.layouts[index].key -= 1;
        check_rewrite(&source, &missing_constant).unwrap();
    }
    let mut repeated = source.clone();
    repeated.layouts[0].key = 2;
    assert!(contract::check(&repeated).is_err());
    let mut wildcard = source.clone();
    wildcard.layouts[1].key = 3;
    assert!(contract::check(&wildcard).is_err());
}

#[test]
fn delta_layouts_require_idb_relations_and_available_keys() {
    let original: Program = authored(include_str!("fixtures/written_order.datalog"));
    let mutations: &[fn(&mut Program)] = &[
        |p| p.delta_layouts.clear(),
        |p| {
            p.delta_layouts.pop();
        },
        |p| p.delta_layouts.reverse(),
        |p| p.delta_layouts[0].relation = syn::parse_quote!(Unknown),
        |p| p.delta_layouts[0].relation = syn::parse_quote!(RecentPointsTo),
        |p| p.delta_layouts[0].relation = syn::parse_quote!(Assign),
        |p| p.delta_layouts[0].columns.clear(),
        |p| p.delta_layouts[0].columns = vec![0],
        |p| p.delta_layouts[0].columns = vec![0, 0],
        |p| p.delta_layouts[0].columns = vec![0, 2],
        |p| p.delta_layouts[0].columns = vec![0, 1, 2],
        |p| p.delta_layouts[0].columns = vec![0, 1],
        |p| p.delta_layouts[0].key = 2,
        |p| p.delta_layouts[0].key = 3,
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = original.clone();
        mutate(&mut changed);
        assert!(contract::check(&changed).is_err(), "delta mutation {index}");
        assert!(
            check_rewrite(&original, &changed).is_err(),
            "delta rewrite {index}"
        );
    }
}

#[test]
fn alternate_layouts_and_recorded_reads_are_admissible() {
    let original = authored(INCREMENTAL);
    let mut alternate = original.clone();
    let index = alternate.layouts.len();
    alternate.layouts.push(Layout {
        relation: syn::parse_quote!(Edge),
        columns: vec![1, 0],
        key: 0,
    });
    alternate.reads[0].index = index;
    alternate.reads[0].columns = vec![1, 0];
    check_rewrite(&original, &alternate).unwrap();
    cq_datalog_storage::check(&original.incremental, &alternate).unwrap();

    // Layout storage order is not an IR obligation when bindings are updated.
    let count = alternate.layouts.len();
    alternate.layouts.reverse();
    for read in &mut alternate.reads {
        if read.kind == "full" {
            read.index = count - 1 - read.index;
        }
    }
    alternate.reads.reverse();
    check_rewrite(&original, &alternate).unwrap();

    // Every declared extra structure participates in the fixed maintenance policy.
    alternate.layouts.push(original.layouts[0].clone());
    alternate.delta_layouts.push(Layout {
        relation: syn::parse_quote!(Result),
        columns: vec![1, 0],
        key: 0,
    });
    check_rewrite(&original, &alternate).unwrap();
}

#[test]
fn every_read_must_serve_exactly_its_positive_occurrence() {
    let original = authored(INCREMENTAL);
    let mutations: &[fn(&mut Program)] = &[
        |p| p.reads.push(p.reads[0].clone()),
        |p| p.reads[0].variant = usize::MAX,
        |p| p.reads[0].position = usize::MAX,
        |p| p.reads[0].index = usize::MAX,
        |p| p.reads[0].kind = syn::parse_quote!(other),
        |p| p.reads[0].kind = syn::parse_quote!(delta),
        |p| p.reads[0].kind = syn::parse_quote!(direct),
        |p| p.reads[0].columns = vec![1, 0],
        |p| p.reads[1].kind = syn::parse_quote!(full),
        |p| p.reads[1].index = 0,
        |p| p.reads[1].columns = vec![1, 0],
        |p| p.negations.clear(),
        |p| p.negations[0].component = usize::MAX,
        |p| p.negations[0].relation = 0,
        |p| p.negations[0].columns.clear(),
        |p| p.negations[0].filters.push((1, syn::parse_quote!(7))),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = original.clone();
        mutate(&mut changed);
        assert!(contract::check(&changed).is_err(), "read mutation {index}");
    }
}

#[test]
fn rust_bindings_are_available_only_after_their_written_clause() {
    let original = authored(
        "struct Q; relation E(x:i32,y:i32)=(e); relation R(x:i32); output R;
         R(*x) :- E(x,_), let n = x, for y in [x], E(n,y);
         strata { component [R]; }
         incremental { scheme seminaive_covering; delta R = D;
           variant 0 seed { R(*x) :- E(x,_), let n = x, for y in [x], E(n,y); }
         }",
    );
    let mut alternate = original.clone();
    let index = alternate.layouts.len();
    alternate.layouts.push(Layout {
        relation: syn::parse_quote!(E),
        columns: vec![1, 0],
        key: 2,
    });
    alternate.reads[1].index = index;
    alternate.reads[1].columns = vec![1, 0];
    check_rewrite(&original, &alternate).unwrap();
    alternate.reads[0].index = index;
    alternate.reads[0].columns = vec![1, 0];
    assert!(contract::check(&alternate).is_err());
}

#[test]
fn direct_delta_scans_may_replace_later_indexed_probes() {
    let original = authored(INCREMENTAL);
    let mut alternate = original.clone();
    let read = alternate
        .reads
        .iter_mut()
        .find(|r| r.kind == "delta")
        .unwrap();
    read.kind = syn::parse_quote!(direct);
    read.index = 2;
    read.columns = vec![0, 1];
    check_rewrite(&original, &alternate).unwrap();
}
