use cq_lang_datalog::{BodyItem, Delta, Program, check_rewrite, contract};
use cq_stage::Language;
use quote::{ToTokens, quote};

type Mutation = Box<dyn Fn(&mut Program)>;

const SOURCE: &str = include_str!("fixtures/source.datalog");
const STRATIFIED: &str = include_str!("fixtures/stratified.datalog");
const SEMINAIVE: &str = include_str!("fixtures/seminaive.datalog");

fn scheduled() -> Program {
    syn::parse_str(SEMINAIVE).unwrap()
}

#[test]
fn authored_stages_check_and_round_trip_independently() {
    for text in [
        SOURCE,
        STRATIFIED,
        SEMINAIVE,
        include_str!("fixtures/written_order.datalog"),
        include_str!("fixtures/delta_probes.datalog"),
    ] {
        let program: Program = syn::parse_str(text).unwrap();
        contract::check(&program).unwrap();
        assert_eq!(
            program.negation_keys.as_ref().map(ToString::to_string),
            program
                .variants
                .as_ref()
                .map(|_| "hash_set_borrowed".to_owned()),
        );
        program.check().unwrap();
        let roundtrip: Program = syn::parse2(program.to_token_stream()).unwrap();
        assert_eq!(program, roundtrip);
        contract::check(&roundtrip).unwrap();
    }
    let source: Program = syn::parse_str(SOURCE).unwrap();
    let stratified: Program = syn::parse_str(STRATIFIED).unwrap();
    check_rewrite(&source, &stratified).unwrap();
    check_rewrite(&stratified, &scheduled()).unwrap();
    Program::check_rewrite(&stratified, &scheduled()).unwrap();
}

#[test]
fn analysis_supplies_declared_graph_schedule_and_delta_facts() {
    let analysis = contract::check(&scheduled()).unwrap();
    assert_eq!(analysis.graph.names, ["Edge", "Blocked", "Reach", "Result"]);
    assert_eq!(analysis.positions["Reach"], 2);
    assert_eq!(analysis.components, vec![vec![2], vec![3]]);
    assert_eq!(analysis.component, vec![None, None, Some(0), Some(1)]);
    assert_eq!(
        analysis.changing_positions,
        vec![vec![], vec![0, 1], vec![]]
    );
    assert_eq!(analysis.deltas["Reach"], "ReachChanges");
    assert!(analysis.dependencies.contains(&(3, 1, true)));
    assert!(analysis.graph.reach[3][0]);
    assert_eq!(
        contract::check(&syn::parse_str(SOURCE).unwrap())
            .unwrap()
            .component,
        vec![None; 4]
    );
}

#[test]
fn schedule_partition_and_order_mutations_are_rejected() {
    let original = scheduled();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| {
            p.strata = None;
        }),
        Box::new(|p| {
            p.strata.as_mut().unwrap()[0].clear();
        }),
        Box::new(|p| {
            p.strata.as_mut().unwrap().pop();
        }),
        Box::new(|p| {
            p.strata.as_mut().unwrap().reverse();
        }),
        Box::new(|p| {
            p.strata.as_mut().unwrap()[0].push(syn::parse_quote!(Reach));
        }),
        Box::new(|p| {
            p.strata.as_mut().unwrap()[0].push(syn::parse_quote!(Edge));
        }),
        Box::new(|p| {
            p.strata.as_mut().unwrap()[0].push(syn::parse_quote!(Unknown));
        }),
        Box::new(|p| {
            p.strata = Some(vec![vec![
                syn::parse_quote!(Reach),
                syn::parse_quote!(Result),
            ]]);
        }),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut program = original.clone();
        mutate(&mut program);
        assert!(contract::check(&program).is_err(), "mutation {index}");
    }
}

#[test]
fn delta_declarations_are_fresh_complete_unique_and_idb_only() {
    let original = scheduled();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| {
            p.deltas.clear();
        }),
        Box::new(|p| {
            p.deltas.pop();
        }),
        Box::new(|p| {
            p.deltas.push(p.deltas[0].clone());
        }),
        Box::new(|p| {
            p.deltas[1].name = p.deltas[0].name.clone();
        }),
        Box::new(|p| {
            p.deltas[0].name = syn::parse_quote!(Edge);
        }),
        Box::new(|p| {
            p.deltas[0].name = syn::parse_quote!(edges);
        }),
        Box::new(|p| {
            p.deltas[0].name = syn::parse_quote!(x);
        }),
        Box::new(|p| {
            p.deltas[0].relation = syn::parse_quote!(Unknown);
        }),
        Box::new(|p| {
            p.deltas[0].relation = syn::parse_quote!(Edge);
        }),
        Box::new(|p| {
            p.variants = None;
            p.insertion = None;
            p.negation_keys = None;
        }),
        Box::new(|p| {
            p.insertion = None;
        }),
        Box::new(|p| {
            p.insertion = Some(syn::parse_quote!(bag));
        }),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut program = original.clone();
        mutate(&mut program);
        assert!(contract::check(&program).is_err(), "mutation {index}");
    }
    let mut source: Program = syn::parse_str(SOURCE).unwrap();
    source.deltas.push(Delta {
        relation: syn::parse_quote!(Reach),
        name: syn::parse_quote!(Fresh),
    });
    assert!(contract::check(&source).is_err());
}

#[test]
fn negation_key_policy_is_required_supported_and_seminaive_only() {
    let original = scheduled();
    for policy in [None, Some(syn::parse_quote!(btree_set_owned))] {
        let mut program = original.clone();
        program.negation_keys = policy;
        let error = contract::check(&program).unwrap_err().to_string();
        assert!(error.contains("negation_keys"), "{error}");
        assert!(check_rewrite(&original, &program).is_err());
    }
    for text in [SOURCE, STRATIFIED] {
        let mut program: Program = syn::parse_str(text).unwrap();
        program.negation_keys = Some(syn::parse_quote!(hash_set_borrowed));
        let error = contract::check(&program).unwrap_err().to_string();
        assert!(error.contains("require seminaive variants"), "{error}");
    }
    let unknown = SEMINAIVE.replace("hash_set_borrowed", "btree_set_owned");
    let program: Program = syn::parse_str(&unknown).unwrap();
    assert!(contract::check(&program).is_err());
    let annotation = "negation_keys hash_set_borrowed;";
    let missing = SEMINAIVE.replace(annotation, "");
    assert!(syn::parse_str::<Program>(&missing).is_err());
    let early = missing.replace(
        "insert_new hash_set_all_columns;",
        "negation_keys hash_set_borrowed; insert_new hash_set_all_columns;",
    );
    assert!(syn::parse_str::<Program>(&early).is_err());
    for text in [SOURCE, STRATIFIED] {
        assert!(syn::parse_str::<Program>(&format!("{text}\n{annotation}")).is_err());
    }
}

#[test]
fn variants_require_exact_coverage_and_erasure() {
    let original = scheduled();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| {
            p.variants.as_mut().unwrap().pop();
        }),
        Box::new(|p| {
            let v = p.variants.as_ref().unwrap()[0].clone();
            p.variants.as_mut().unwrap().push(v);
        }),
        Box::new(|p| {
            p.variants.as_mut().unwrap()[0].rule = 100;
        }),
        Box::new(|p| {
            p.variants.as_mut().unwrap()[1].delta = None;
        }),
        Box::new(|p| {
            p.variants.as_mut().unwrap()[0].delta = Some(0);
        }),
        Box::new(|p| {
            p.variants.as_mut().unwrap()[1].delta = Some(100);
        }),
        Box::new(|p| {
            p.variants.as_mut().unwrap()[0].query.head.args[0] = syn::parse_quote!(*y);
        }),
        Box::new(|p| {
            p.variants.as_mut().unwrap()[1].query =
                syn::parse_quote!(Reach(*x,*z) :- Reach(x,y), Reach(y,z););
        }),
        Box::new(|p| {
            p.variants.as_mut().unwrap()[1].query =
                syn::parse_quote!(Reach(*x,*z) :- ReachChanges(x,y), ReachChanges(y,z););
        }),
        Box::new(|p| {
            p.variants.as_mut().unwrap()[3].query = syn::parse_quote!(Result(*x,*y) :- Reach(x,y););
        }),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut program = original.clone();
        mutate(&mut program);
        assert!(contract::check(&program).is_err(), "mutation {index}");
    }
}

#[test]
fn malformed_variants_fail_standalone_cq_validation_before_erasure() {
    for (query, expected) in [
        (
            quote!(Reach(*x,*z) :- ReachChanges(x), Reach(y,z);),
            "arity",
        ),
        (quote!(Reach(*x) :- ReachChanges(x,y), Reach(y,z);), "arity"),
        (
            quote!(Reach(*missing,*z) :- ReachChanges(x,y), Reach(y,z);),
            "range restricted",
        ),
        (
            quote!(Reach(*x,*z) :- Missing(x,y), Reach(y,z);),
            "not declared",
        ),
        (
            quote!(Reach(*x,*z) :- if *x > 0, ReachChanges(x,y), Reach(y,z);),
            "before",
        ),
        (
            quote!(Reach(*x,*z) :- ReachChanges(x,y), !Blocked(z,_), Reach(y,z);),
            "before",
        ),
        (
            quote!(Reach(*x,*z) :- ReachChanges(x,y), Reach(y,z), !Blocked(x);),
            "arity",
        ),
    ] {
        let mut program = scheduled();
        program.variants.as_mut().unwrap()[1].query = syn::parse2(query).unwrap();
        let error = contract::check(&program).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
        assert!(
            !error.contains("erasure"),
            "standalone check must run first: {error}"
        );
    }
}

#[test]
fn negative_validation_projection_does_not_extend_core_cq() {
    let program = scheduled();
    contract::check(&program).unwrap();
    let declarations = &program.declarations;
    let query = &program.rules[2];
    let cq: cq_lang_cq::Program =
        syn::parse2(quote!(struct Check; #(#declarations)* #query)).unwrap();
    assert!(
        cq_lang_cq::contract::check(&cq)
            .unwrap_err()
            .to_string()
            .contains("extended CQ")
    );
    assert!(matches!(query.body[1], BodyItem::Negation(_)));
}

#[test]
fn unsafe_unstratified_modes_and_optional_syntax_are_rejected() {
    for source in [
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), !A(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); relation B(x:i32); output A; A(*x) :- E(x), !B(x); B(*x) :- A(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*y) :- E(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), !E(y);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- !E(x), E(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), !E(*x+1);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), neg!{E(x)};",
        "struct P; relation E(x:i32) = (e); output E; E(*x) :- E(x);",
    ] {
        let program: Program = syn::parse_str(source).unwrap();
        assert!(contract::check(&program).is_err(), "{source}");
    }
    // Frozen core has no relation modes; these fail during parsing, not checking.
    assert!(
        syn::parse_str::<Program>(
            "struct P; relation E(x:i32) parameter; relation A(x:i32); output A; A(*x) :- E(x);"
        )
        .is_err()
    );
}

#[test]
fn negation_rejects_keys_bound_only_by_let() {
    for binding in [quote!(let y = *x), quote!(let y:i32 = *x)] {
        let program: Program = syn::parse2(quote! {
            struct OwnedKey;
            relation D(x:i32) = (domain);
            relation R(x:i32) = (blocked);
            relation Out(x:i32);
            output Out;
            Out(*x) :- D(x), #binding, !R(y);
        })
        .unwrap();
        let error = contract::check(&program).unwrap_err().to_string();
        assert!(error.contains("bound by a positive atom"), "{error}");
    }
}

#[test]
fn rewrites_preserve_every_recorded_decision() {
    let original = scheduled();
    let mut renamed = scheduled();
    renamed.deltas[0].name = syn::parse_quote!(Alternative);
    for variant in renamed.variants.as_mut().unwrap() {
        if let Some(position) = variant.delta {
            let BodyItem::Positive { atom } = &mut variant.query.body[position] else {
                unreachable!()
            };
            atom.relation = syn::parse_quote!(Alternative);
        }
    }
    contract::check(&renamed).unwrap();
    assert!(check_rewrite(&original, &renamed).is_err());
    let mut reordered = original.clone();
    reordered.variants.as_mut().unwrap().reverse();
    contract::check(&reordered).unwrap();
    assert!(check_rewrite(&original, &reordered).is_err());
    let mut changed = original.clone();
    changed.observer = Some(syn::parse_quote!(observer));
    assert!(check_rewrite(&original, &changed).is_err());
}

#[test]
fn mutual_sccs_cannot_be_split_and_independent_decisions_are_preserved() {
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
    split.strata = Some(vec![vec![syn::parse_quote!(A)], vec![syn::parse_quote!(B)]]);
    assert!(contract::check(&split).is_err());
    let mut reordered = mutual.clone();
    reordered.strata.as_mut().unwrap()[0].reverse();
    contract::check(&reordered).unwrap();
    assert!(check_rewrite(&mutual, &reordered).is_err());
    let mut removed = mutual.clone();
    removed.strata = None;
    contract::check(&removed).unwrap();
    assert!(check_rewrite(&mutual, &removed).is_err());
}

#[test]
fn rewrites_preserve_source_name_schemas_rules_output_and_observer() {
    let original: Program = syn::parse_str(SOURCE).unwrap();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| p.name = syn::parse_quote!(Renamed)),
        Box::new(|p| p.outputs = vec![syn::parse_quote!(Reach)]),
        Box::new(|p| p.observer = Some(syn::parse_quote!(observe_round))),
        Box::new(|p| p.declarations[0].columns[0].ty = syn::parse_quote!(i64)),
        Box::new(|p| p.rules.swap(0, 1)),
        Box::new(|p| p.rules[0].head.args[0] = syn::parse_quote!(*y)),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut changed = original.clone();
        mutate(&mut changed);
        contract::check(&changed).unwrap();
        assert!(
            check_rewrite(&original, &changed).is_err(),
            "mutation {index}"
        );
    }
}

#[test]
fn generators_round_trip_and_survive_schedule_correspondence() {
    let source: Program = syn::parse_quote! {
        struct Generated;
        relation Seed(x:i32) = (seeds);
        relation Reach(x:i32);
        output Reach;
        Reach(y) :- Seed(x), for (y, _) in [(*x, ())];
        Reach(y) :- Reach(x), if *x < 3, for y in [*x + 1, *x + 1];
    };
    let stratified: Program = syn::parse2(quote! {
        #source
        strata { component [Reach]; }
    })
    .unwrap();
    let scheduled: Program = syn::parse2(quote! {
        #stratified
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta Reach = Recent;
            layout Seed [0] key 0;
            variant 0 seed { Reach(y) :- Seed(x), for (y, _) in [(*x, ())]; }
            variant 1 delta 0 {
                Reach(y) :- Recent(x), if *x < 3, for y in [*x + 1, *x + 1];
            }
        }
    })
    .unwrap();
    for program in [&source, &stratified, &scheduled] {
        contract::check(program).unwrap();
        let reparsed: Program = syn::parse2(program.to_token_stream()).unwrap();
        assert_eq!(*program, reparsed);
    }
    check_rewrite(&source, &stratified).unwrap();
    check_rewrite(&stratified, &scheduled).unwrap();
    let mut changed = scheduled.clone();
    changed.variants.as_mut().unwrap()[1].query.body[2] = syn::parse_quote!(for y in [*x + 2]);
    assert!(check_rewrite(&scheduled, &changed).is_err());
}

#[test]
fn generated_bindings_follow_let_negation_scope_restrictions() {
    let program: Program = syn::parse_quote! {
        struct GeneratedKey;
        relation D(x:i32) = (domain);
        relation R(x:i32) = (blocked);
        relation Out(x:i32);
        output Out;
        Out(y) :- D(x), for y in [*x], !R(y);
    };
    let error = contract::check(&program).unwrap_err().to_string();
    assert!(error.contains("bound by a positive atom"), "{error}");
}
