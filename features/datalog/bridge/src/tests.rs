use super::*;
use quote::quote;

fn source() -> proc_macro2::TokenStream {
    quote! {
        struct Q;
        relation E(x:i32) = (input);
        relation R(x:i32);
        output R;
        R(*x) :- E(x);
    }
}

#[test]
fn ordinary_queue_and_arguments_are_preserved_per_hole() {
    let source = source();
    let expanded = expand(
        quote! {
            arguments {};
            pipeline { custom_surface(opaque(tokens)), planner(no_reduce), printer };
            #source
        },
        Stage::Source,
    )
    .unwrap()
    .to_string();
    assert_eq!(expanded.matches("custom_surface !").count(), 1);
    assert!(expanded.contains("arguments { opaque (tokens) }"));
    assert!(expanded.contains("pipeline { planner (no_reduce) , printer }"));
    assert!(!expanded.contains("stream ("));
    assert!(expanded.contains("HashMap"));
    assert!(expanded.contains("HashSet"));
    assert!(expanded.contains("cq_storage :: HashState"));
    assert!(expanded.contains("cq_storage :: HashSet"));
    assert!(expanded.contains("seen_1 . insert"));
    assert!(!expanded.contains("seen_1 . contains"));
    assert!(expanded.contains("while"));
}

#[test]
fn named_configuration_preserves_every_phase_and_planner_argument() {
    let source = source();
    let input: Input = syn::parse2(quote! {
        pipeline {
            surface = [surface(one)]; plan = custom_plan(two);
            ix = custom_ix(three); lower = custom_lower(four);
            physical = [physical_a(five), physical_b]; select = custom_select(six);
            structures = [structure(seven)]; print = custom_print(eight);
        }
        #source
    })
    .unwrap();
    let expected: cq_stage::Pipeline = syn::parse_quote!(pipeline {
        surface(one), custom_plan(two), custom_ix(three), custom_lower(four),
        physical_a(five), physical_b, custom_select(six), structure(seven), custom_print(eight)
    });
    assert_eq!(
        input.pipeline.to_token_stream().to_string(),
        expected.to_token_stream().to_string()
    );
}

#[test]
fn entries_enforce_their_authored_stage() {
    let source = source();
    let strata = quote!(strata { component [R]; });
    let schedule = quote!(seminaive {
        insert_new hash_set_all_columns;
        negation_keys hash_set_borrowed;
        delta R = Recent;
        layout E [0] key 0;
        variant 0 seed { R(*x) :- E(x); }
    });
    assert!(expand(source.clone(), Stage::Source).is_ok());
    assert!(expand(source.clone(), Stage::Stratified).is_err());
    assert!(expand(source.clone(), Stage::Seminaive).is_err());
    assert!(expand(quote!(#source #strata), Stage::Stratified).is_ok());
    assert!(expand(quote!(#source #strata), Stage::Source).is_err());
    assert!(expand(quote!(#source #strata #schedule), Stage::Seminaive).is_ok());
    assert!(expand(quote!(#source #strata #schedule), Stage::Stratified).is_err());
    assert!(expand(quote!(arguments { unexpected }; #source), Stage::Source).is_err());
}

#[test]
fn authored_round_expansion_matches_independent_fixture() {
    let source = source();
    let program: cq_lang_datalog::Program = syn::parse2(quote! {
        #source
        strata { component [R]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta R = Recent;
            layout E [0] key 0;
            variant 0 seed { R(*x) :- E(x); }
        }
    })
    .unwrap();
    let fork = cq_datalog_fork::fork(&program, |_| Ok(quote!(authored_rows()))).unwrap();
    let actual: syn::Expr = syn::parse2(render::render(fork)).unwrap();
    let expected: syn::Expr = syn::parse_str(include_str!("../tests/fixtures/round.rs")).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn lower_stratum_negation_builds_one_key_set_outside_rounds() {
    let program: cq_lang_datalog::Program = syn::parse_quote! {
        observe = (observer);
        struct Layered;
        relation Domain(x:String) = (domain);
        relation Blocked(x:String);
        relation Out(x:String);
        output Out;
        Blocked(x.clone()) :- Domain(x);
        Out(x.clone()) :- Domain(x), !Blocked(x), !Blocked(x);
        strata { component [Blocked]; component [Out]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta Blocked = DB;
            delta Out = DO;
            layout Domain [0] key 0;
            variant 0 seed { Blocked(x.clone()) :- Domain(x); }
            variant 1 seed { Out(x.clone()) :- Domain(x), !Blocked(x), !Blocked(x); }
        }
    };
    let fork = cq_datalog_fork::fork(&program, |hole| {
        if hole.name.to_string().contains("key_hole") {
            Ok(quote!(completed_keys()))
        } else {
            Ok(quote!(candidate_rows()))
        }
    })
    .unwrap();
    assert_eq!(fork.context.strata[1].keys.len(), 1);
    let expanded = render::render(fork).to_string();
    assert_eq!(expanded.matches("completed_keys ()").count(), 1);
    assert_eq!(expanded.matches("Rows :: rows").count(), 2);
    assert!(expanded.contains("observer"));
}

#[test]
fn full_idb_layouts_publish_only_admitted_rows_after_all_holes() {
    let program: cq_lang_datalog::Program = syn::parse_quote! {
        struct Closure;
        relation E(x:String,y:String) = (edges);
        relation R(x:String,y:String);
        output R;
        R((*x).clone(),(*y).clone()) :- E(x,y);
        R((*x).clone(),(*z).clone()) :- R(x,y), R(y,z);
        strata { component [R]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta R = D;
            layout E [0,1] key 0;
            layout R [0,1] key 1;
            layout R [0,1] key 0;
            variant 0 seed { R((*x).clone(),(*y).clone()) :- E(x,y); }
            variant 1 delta 0 { R((*x).clone(),(*z).clone()) :- D(x,y), R(y,z); }
            variant 1 delta 1 { R((*x).clone(),(*z).clone()) :- R(x,y), D(y,z); }
        }
    };
    let expanded = cq_datalog_fork::fork(&program, |hole| {
        let name = hole.name;
        Ok(quote!(#name()))
    })
    .unwrap();
    let next = &expanded.context.relations[1].next;
    let first_host = &expanded.context.layouts[1].host;
    let second_host = &expanded.context.layouts[2].host;
    let expected_updates = quote! {
        for row in &#next {
            #first_host.entry((::core::clone::Clone::clone(&row.0),))
                .or_default().push((::core::clone::Clone::clone(&row.1),));
            #second_host.entry(())
                .or_default().push((::core::clone::Clone::clone(&row.0),
                    ::core::clone::Clone::clone(&row.1),));
        }
    }
    .to_string();
    let rendered = render::render(expanded).to_string();
    // One publication after seeds and one after all recursive variants, not
    // one insertion into the full maps per rule or per rejected candidate.
    assert_eq!(rendered.matches(&expected_updates).count(), 2);
    assert_eq!(rendered.matches("Rows :: rows").count(), 2);
    let last_step = rendered.find("__cq_datalog_hole_2 ()").unwrap();
    assert!(rendered.rfind(&expected_updates).unwrap() > last_step);
}

#[test]
fn multioutput_render_moves_each_relation_in_output_order() {
    let source: cq_lang_datalog::Program = syn::parse_quote! {
        struct Ordered;
        relation E(x:i32) = (input);
        relation A(x:i32); relation B(x:String);
        output B, A;
        A(*x) :- E(x);
        B(x.to_string()) :- A(x);
    };
    let strata = cq_datalog_schedule::stratify::compile(&source).unwrap();
    let program = cq_datalog_schedule::seminaive::compile(&strata).unwrap();
    let fork = cq_datalog_fork::fork(&program, |_| Ok(quote!(rows()))).unwrap();
    let a = &fork.context.relations[1].all;
    let b = &fork.context.relations[2].all;
    let expected: syn::Expr = syn::parse_quote!((#b.into_iter(), #a.into_iter(),));
    let actual: syn::ExprBlock = syn::parse2(render::render(fork)).unwrap();
    assert_eq!(
        actual.block.stmts.last(),
        Some(&syn::Stmt::Expr(expected, None))
    );
    for (stage, program) in [
        (Stage::Source, source),
        (Stage::Stratified, strata),
        (Stage::Seminaive, program),
    ] {
        let replay = syn::parse2::<cq_lang_datalog::Program>(program.to_token_stream()).unwrap();
        assert!(expand(replay.to_token_stream(), stage).is_ok());
    }
}

#[test]
fn constant_key_projection_is_built_once_before_recursive_rounds() {
    let source: cq_lang_datalog::Program = syn::parse_quote! {
        struct Filtered;
        relation E(x:i32) = (input);
        relation M(kind:String,x:i32) = (modifiers);
        relation A(x:i32);
        output A;
        A(*x) :- E(x), !M("abstract",x);
        A(*x) :- A(x), !M("abstract",x), !M("abstract",x);
    };
    let strata = cq_datalog_schedule::stratify::compile(&source).unwrap();
    let program = cq_datalog_schedule::seminaive::compile(&strata).unwrap();
    let fork = cq_datalog_fork::fork(&program, |hole| {
        if hole.name.to_string().contains("key_hole") {
            Ok(quote!(filtered_keys()))
        } else {
            Ok(quote!(candidate_rows()))
        }
    })
    .unwrap();
    assert_eq!(fork.context.strata[0].keys.len(), 1);
    let rendered = render::render(fork).to_string();
    assert_eq!(rendered.matches("filtered_keys ()").count(), 1);
    assert!(rendered.find("filtered_keys ()").unwrap() < rendered.find("while").unwrap());
}

#[test]
fn edb_canonicalization_precedes_and_is_shared_by_layouts_keys_and_outputs() {
    let source: cq_lang_datalog::Program = syn::parse_quote! {
        struct CanonicalInputs;
        relation E(x:String,y:String) = (input());
        relation R(x:String,y:String);
        output E, R;
        R((*x).clone(),(*z).clone()) :- E(x,y), E(y,z), !E(z,x);
        R((*x).clone(),(*z).clone()) :- R(x,y), E(z,y), !E(z,x);
    };
    let strata = cq_datalog_schedule::stratify::compile(&source).unwrap();
    let program = cq_datalog_schedule::seminaive::compile(&strata).unwrap();
    let fork = cq_datalog_fork::fork(&program, |hole| {
        if hole.name.to_string().contains("key_hole") {
            Ok(quote!(canonical_keys()))
        } else {
            Ok(quote!(candidate_rows()))
        }
    })
    .unwrap();
    assert!(
        fork.context
            .layouts
            .iter()
            .filter(|layout| layout.relation == 0)
            .count()
            >= 3
    );
    let all = &fork.context.relations[0].all;
    let canonicalize =
        quote!(Canonical(::cq_storage::rows::Rows::rows(#all).collect())).to_string();
    let output = quote!(::cq_storage::rows::Rows::rows(#all).map).to_string();
    let rendered = render::render(fork).to_string();
    assert_eq!(rendered.matches("input ()").count(), 1);
    assert_eq!(rendered.matches(&canonicalize).count(), 1);
    assert_eq!(rendered.matches("struct Canonical").count(), 1);
    assert!(rendered.contains(&output));
    let canonicalized = rendered.find(&canonicalize).unwrap();
    assert!(canonicalized < rendered.find("HashMap").unwrap());
    assert!(canonicalized < rendered.find("canonical_keys ()").unwrap());
    assert!(canonicalized < rendered.find("while").unwrap());
    // Output collection must not introduce a second all-column hash set.
    assert_eq!(
        rendered
            .matches("collect :: < :: cq_storage :: HashSet")
            .count(),
        1
    );
}
