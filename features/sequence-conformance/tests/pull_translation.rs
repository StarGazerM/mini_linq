use cq_lang_sequence::{Program, contract};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn assert_answers(
    structures: cq_lang_structures::Program,
    data: TokenStream,
    expected: TokenStream,
) -> TokenStream {
    let program = cq_sequence_lowering::compile(&structures).unwrap();
    cq_sequence_lowering::check(&structures, &program).unwrap();
    let analysis = contract::check(&program).unwrap();
    let sequence = cq_sequence_pull::translate(&program, &analysis)
        .unwrap_or_else(|error| panic!("{error}\n{}", program.to_token_stream()));
    quote!({
        #data
        let actual: ::std::collections::BTreeSet<_> = (#sequence).into_iter().collect();
        let expected: ::std::collections::BTreeSet<_> = (#expected).into_iter().collect();
        assert_eq!(actual, expected);
    })
}

fn run(source: TokenStream) {
    static DEPENDENCIES: std::sync::LazyLock<Vec<(String, std::path::PathBuf)>> =
        std::sync::LazyLock::new(|| {
            vec![cq_testkit::rustc::linked_dependency::<
                cq_storage::SortedRun<i32, i32>,
            >(
                "cq_storage", "cq_storage::SortedRun<i32, i32>"
            )]
        });
    let runtime = cq_testkit::rustc::Runtime::with_dependencies(DEPENDENCIES.clone());
    runtime.run(&source.to_string());
}

fn simple(body: TokenStream) -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        }
    };
    Program {
        structures: cq_structure_selection::compile(&physical).unwrap(),
        sequence: syn::parse2(quote!(sequence { #body })).unwrap(),
        graded: None,
        evidence: Default::default(),
    }
}

#[test]
fn sequential_and_tasks_share_the_sequence_answer_contract() {
    use cq_sequence_conformance::{Target, assert_conformant};
    let program = simple(quote! {
        input = store rows r;
        out = store tile 2 canon input;
    });
    assert_conformant(&program, &[Target::Sequential, Target::Tasks]);
}

#[test]
fn retained_union_fallback_readers_keep_their_recorded_providers() {
    let source = syn::parse_quote! {
        struct Fuzz;
        relation r0(c0:i32);
        relation r1(c0:i32,c1:i32,c2:i32);
        relation r2(c0:i32,c1:i32,c2:i32);
        answer(v0,v1) :- r0(v0),
            { { r1(v0,v1,v3) } | { r1(v0,v1,v3) } } else { r2(v0,v1,v4) };
    };
    let pipeline = cq_workloads::pipelines::Pipeline {
        plan: cq_workloads::pipelines::Planner::ALL
            .iter()
            .position(|p| p.name() == "elimination_fine")
            .unwrap(),
        merge: true,
        smallest: true,
        ..Default::default()
    };
    let physical = pipeline.physical(&source).unwrap();
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let program = cq_sequence_lowering::compile(&structures).unwrap();
    let analysis = contract::check(&program).unwrap();
    let output = cq_sequence_pull::translate(&program, &analysis)
        .unwrap_or_else(|error| panic!("{error}\n{}", program.to_token_stream()));
    run(quote!(
        fn main() {
            let r0 = vec![(1i32,), (2,), (3,)];
            let r1 = vec![(1i32, 4i32, 5i32), (1, 4, 6)];
            let r2 = vec![(1i32, 9i32, 0i32), (2, 7, 8)];
            let actual: ::std::collections::BTreeSet<_> = (#output).into_iter().collect();
            let expected: ::std::collections::BTreeSet<_> =
                [(&1i32, &4i32), (&2, &7)].into_iter().collect();
            assert_eq!(actual, expected);
        }
    ));
}

#[test]
fn computed_branch_merges_use_the_retained_ordered_index() {
    let source = syn::parse_quote! {
        struct Fuzz;
        relation r0(c0:i32);
        answer(v0) :- r0(v0), let v1:i32 = std::convert::identity(v0.to_owned()),
            { if *v1 == 1 } | { if *v1 == 2 };
    };
    let pipeline = cq_workloads::pipelines::Pipeline {
        plan: cq_workloads::pipelines::Planner::ALL
            .iter()
            .position(|p| p.name() == "transfer")
            .unwrap(),
        merge: true,
        canon: true,
        smallest: true,
        ..Default::default()
    };
    let physical = pipeline.physical(&source).unwrap();
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let check = assert_answers(
        structures,
        quote! {
            let r0 = vec![(1i32,), (2,), (1,), (3,)];
        },
        quote!([(&1i32,), (&2,)]),
    );
    run(quote!(fn main() { #check }));
}
