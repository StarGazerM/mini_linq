use cq_ir::{cq, index_requirements, iterator_pipeline, relational_plan, rust_access_plan};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn program_tokens() -> TokenStream {
    quote! {
        pub struct TriangleProgram;
        relation R(c0: i32, c1: i32);
        relation S(c0: i32, c1: i32);
        relation T(c0: i32, c1: i32);
        triangle(x, y, z) :- R(x, y), S(y, z), T(z, x).
    }
}

fn index_tokens() -> TokenStream {
    let relational = relational_tokens();
    quote! {
        #relational
        indexes {
            S[0];
            T[0, 1];
        }
    }
}

fn relational_tokens() -> TokenStream {
    let program = program_tokens();
    quote! {
        #program
        relational {
            r0 = rename R {c0 -> x, c1 -> y};
            r1 = rename S {c0 -> y, c1 -> z};
            r2 = natural_join r0 with r1;
            r3 = rename T {c0 -> z, c1 -> x};
            r4 = natural_join r2 with r3;
            r5 = project r4 keep {x, y, z};
            output r5 as triangle(x, y, z).
        }
    }
}

fn annotated_access_tokens() -> TokenStream {
    quote! {
        triangle(x, y, z) => ((x.clone(), y.clone(), z.clone(),)) :-
            R(x, y) => for (x, y,) in (rows0.iter()),
            S(y, z) => for (z,) in
                (index0.get(&(y.clone(),)).into_iter().flatten()),
            T(z, x) => if (index1.contains(&(z.clone(), x.clone(),))).
    }
}

fn pipeline_tokens() -> TokenStream {
    quote! {
        iter0 = scan (x, y,) in (rows0.iter()) yield (x, y,);
        iter1 = join iter0 as (x, y,) with (z,) in (
            index0.get(&(y.clone(),)).into_iter().flatten()
        ) yield (x, y, z,);
        iter2 = filter iter1 as (x, y, z,) if (
            index1.contains(&(z.clone(), x.clone(),))
        );
        iter3 = project iter2 as (x, y, z,) yield ((x.clone(), y.clone(), z.clone(),));
        iter4 = distinct iter3;
        return iter4.
    }
}

fn assert_round_trips<T>(tokens: TokenStream)
where
    T: syn::parse::Parse + ToTokens + PartialEq + std::fmt::Debug,
{
    let first: T = syn::parse2(tokens).unwrap();
    let second: T = syn::parse2(first.to_token_stream()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn every_language_boundary_parses_directly() {
    let logical: cq::Module = syn::parse2(program_tokens()).unwrap();
    let relational: relational_plan::Module = syn::parse2(relational_tokens()).unwrap();
    let indexed: index_requirements::Module = syn::parse2(index_tokens()).unwrap();
    let annotated: rust_access_plan::Plan = syn::parse2(annotated_access_tokens()).unwrap();
    let pipeline: iterator_pipeline::Pipeline = syn::parse2(pipeline_tokens()).unwrap();

    cq::contract::check(&logical).unwrap();
    relational_plan::contract::check(&relational).unwrap();
    index_requirements::contract::check(&indexed).unwrap();
    rust_access_plan::contract::check(&annotated).unwrap();
    iterator_pipeline::contract::check(&pipeline).unwrap();
    assert!(cq::contract::well_formed(&logical));
    assert!(relational_plan::contract::well_formed(&relational));
    assert!(index_requirements::contract::well_formed(&indexed));
    assert!(matches!(annotated.head.output, syn::Expr::Tuple(_)));
    assert_eq!(pipeline.definitions.len(), 5);
}

#[test]
fn emitted_tokens_round_trip_for_every_language() {
    assert_round_trips::<cq::Module>(program_tokens());
    assert_round_trips::<relational_plan::Module>(relational_tokens());
    assert_round_trips::<index_requirements::Module>(index_tokens());
    assert_round_trips::<rust_access_plan::Plan>(annotated_access_tokens());
    assert_round_trips::<iterator_pipeline::Pipeline>(pipeline_tokens());
}

#[test]
fn the_grammars_are_distinct() {
    assert!(syn::parse2::<index_requirements::Module>(program_tokens()).is_err());
    assert!(syn::parse2::<index_requirements::Module>(relational_tokens()).is_err());
    assert!(syn::parse2::<cq::Module>(index_tokens()).is_err());
    assert!(syn::parse2::<relational_plan::Module>(index_tokens()).is_err());
    assert!(syn::parse2::<rust_access_plan::Plan>(index_tokens()).is_err());
    assert!(syn::parse2::<index_requirements::Module>(annotated_access_tokens()).is_err());
    assert!(syn::parse2::<rust_access_plan::Plan>(pipeline_tokens()).is_err());
    assert!(syn::parse2::<iterator_pipeline::Pipeline>(annotated_access_tokens()).is_err());
}

#[test]
fn cq_language_contract_enforces_the_narrow_relational_core() {
    let escaped_keyword: cq::Module = syn::parse2(quote! {
        struct P;
        relation R(c0: i32);
        r#relation(x) :- R(x).
    })
    .unwrap();
    cq::contract::check(&escaped_keyword).unwrap();

    for invalid in [
        quote! {
            struct P;
            relation R();
            answer(x) :- R(x).
        },
        quote! {
            struct P;
            relation R(c0: i32);
            answer(x) :- Missing(x).
        },
        quote! {
            struct P;
            relation R(c0: i32, c1: i32);
            answer(x) :- R(x).
        },
        quote! {
            struct P;
            relation R(c0: i32);
            answer(x) :- R(y).
        },
        quote! {
            struct P;
            relation R(c0: i32);
            relation r#R(c0: i32);
            answer(x) :- R(x).
        },
        quote! {
            struct P;
            relation R(c0: i32);
            r#R(x) :- R(x).
        },
        quote! {
            struct P;
            relation R(c0: i32, c1: i32);
            answer(x) :- R(x, r#x).
        },
    ] {
        let module: cq::Module = syn::parse2(invalid).unwrap();
        assert!(cq::contract::check(&module).is_err());
        assert!(!cq::contract::well_formed(&module));
    }
}

#[test]
fn relation_declarations_preserve_full_rust_column_types() {
    let module: cq::Module = syn::parse2(quote! {
        struct Typed;
        relation R(c0: &'static str, c1: Option<Vec<i32>>);
        answer(name) :- R(name, values).
    })
    .unwrap();

    let relation = &module.program.inputs[0];
    assert_eq!(relation.arity(), 2);
    assert_eq!(relation.columns.len(), 2);

    let reparsed: cq::Module = syn::parse2(module.to_token_stream()).unwrap();
    assert_eq!(module, reparsed);
}

#[test]
fn index_language_contract_is_local_and_separate_from_rust_physicalization() {
    let relational = relational_tokens();

    // This remains locally meaningful even though it is not the canonical
    // answer for the written triangle order.
    let noncanonical: index_requirements::Module = syn::parse2(quote! {
        #relational
        indexes { R[1]; }
    })
    .unwrap();
    index_requirements::contract::check(&noncanonical).unwrap();

    for invalid in [
        quote! {
            #relational
            indexes { S[]; }
        },
        quote! {
            #relational
            indexes { S[1, 0]; }
        },
        quote! {
            #relational
            indexes { S[0]; S[0]; }
        },
        quote! {
            #relational
            indexes { S[2]; }
        },
    ] {
        let module: index_requirements::Module = syn::parse2(invalid).unwrap();
        assert!(index_requirements::contract::check(&module).is_err());
        assert!(!index_requirements::contract::well_formed(&module));
    }
}
