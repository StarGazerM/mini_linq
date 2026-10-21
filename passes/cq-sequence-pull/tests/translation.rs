use cq_lang_sequence::{Program, contract};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn pipeline(query: TokenStream) -> cq_lang_structures::Program {
    let cq: cq_lang_cq::Program = syn::parse2(query).unwrap();
    let normal = cq_lang_cq::normal::normalize(&cq).unwrap();
    let vo = cq_order::compile(&normal, &Default::default()).unwrap();
    let ra = cq_elaborate::compile(&vo).unwrap();
    let ix = cq_index_policy::compile(&ra).unwrap();
    let physical = cq_physical_lowering::compile(&ix).unwrap();
    cq_structure_selection::compile(&physical).unwrap()
}

fn compare(structures: cq_lang_structures::Program, data: TokenStream) -> TokenStream {
    let program = cq_sequence_lowering::compile(&structures).unwrap();
    cq_sequence_lowering::check(&structures, &program).unwrap();
    let analysis = contract::check(&program).unwrap();
    let sequence = cq_sequence_pull::translate(&program, &analysis)
        .unwrap_or_else(|error| panic!("{error}\n{}", program.to_token_stream()));
    let pull = cq_physical_realization::compile(&structures).unwrap();
    quote!({
        #data
        let actual: ::std::collections::BTreeSet<_> = (#sequence).into_iter().collect();
        let expected: ::std::collections::BTreeSet<_> = (#pull).collect();
        assert_eq!(actual, expected);
    })
}

#[test]
fn cq_pipeline_answers_equal_the_existing_pull_printer() {
    let data = quote! {
        let L = vec![(1i32, 2i32), (1, 2), (2, 3), (3, 4)];
        let R = vec![(2i32, 7i32), (2, 8), (3, 9), (5, 10)];
        let S = vec![(7i32, 42i32), (8, 41), (9, 42)];
    };
    let declarations = quote! {
        struct Query;
        relation L(a:i32,b:i32);
        relation R(b:i32,c:i32);
        relation S(c:i32,d:i32);
    };
    let queries = [
        quote!(answer(a,c) :- L(a,b), R(b,c);),
        quote!(answer(a,b,c) :- L(a,b), R(b,c), S(c,42);),
        quote!(answer(a) :- L(a,b), R(b,7);),
        quote!(answer(a,b) :- L(a,b), R(b,_);),
        quote!(answer(a) :- L(a,_);),
        quote!(answer(a,c) :- L(a,b), { R(b,c) } else { S(c,b) };),
        quote!(answer(a,c) :- L(a,b), { R(b,c) } | { S(c,b) };),
    ];
    let checks = queries
        .into_iter()
        .map(|query| compare(pipeline(quote!(#declarations #query)), data.clone()))
        .collect::<Vec<_>>();
    run(quote!(fn main() { #(#checks)* }));
}

fn run(source: TokenStream) {
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    let runtime = cq_testkit::rustc::Runtime::new(&source_path, false);
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
    }
}

#[test]
fn constructors_execute_and_choose_uses_the_else_arm() {
    let cases = [
        (
            quote!(out = store canon flatten tile 3 rows r;),
            quote!(vec![(&1i32,), (&2i32,)]),
        ),
        (quote!(out = store fold count rows r;), quote!(vec![3usize])),
        (quote!(out = store fold or rows r;), quote!(vec![true])),
        (quote!(out = store fold sum a rows r;), quote!(vec![4i32])),
        (
            quote!(out = store canon choose [len R < 2 => rows r, else => rows r];),
            quote!(vec![(&1i32,), (&2i32,)]),
        ),
    ];
    let checks = cases
        .into_iter()
        .map(|(body, expected)| {
            let program = simple(body);
            let analysis = contract::check(&program).unwrap();
            let output = cq_sequence_pull::translate(&program, &analysis).unwrap();
            quote!({
                let R = vec![(1i32,), (1,), (2,)];
                assert_eq!(#output, #expected);
            })
        })
        .collect::<Vec<_>>();
    run(quote!(fn main() { #(#checks)* }));
}

#[test]
fn translation_requires_host_grades_but_does_not_recheck_the_program() {
    let program = simple(quote!(out = store canon rows r;));
    let mut analysis = contract::check(&program).unwrap();
    analysis
        .levels
        .insert("out".into(), vec!["host".into(), "pool".into()]);
    assert!(cq_sequence_pull::translate(&program, &analysis).is_err());
    analysis.levels.insert("out".into(), vec!["host".into()]);
    assert!(
        cq_sequence_pull::translate(&program, &analysis)
            .unwrap()
            .to_token_stream()
            .to_string()
            .contains("collect")
    );
}

#[test]
fn stored_lookup_uses_the_recorded_range_cursor() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe canon probe first [a] l,r;
            return j as answer(a,b);
        }
    };
    let mut structures = cq_structure_selection::compile(&physical).unwrap();
    for entry in &mut structures.structures.entries {
        if let cq_lang_structures::Structure::Built { keyed, .. } = &mut entry.structure {
            keyed.ty = syn::parse_quote!(::cq_storage::SortedRun<_, _>);
        }
    }
    let program = Program {
        structures,
        sequence: syn::parse_quote!(sequence {
            right = store rows r;
            expanded = store flatten lookup r [a] rows l;
            out = store canon expanded;
        }),
        graded: None,
    };
    let analysis = contract::check(&program).unwrap();
    assert_eq!(
        analysis.capabilities[&("r".into(), vec!["a".into()])],
        cq_lang_sequence::Capability::Range
    );
    let output = cq_sequence_pull::translate(&program, &analysis).unwrap();
    assert!(output.to_token_stream().to_string().contains("RangeCursor"));
    run(quote!(
        fn main() {
            let L = vec![(1i32,), (3,), (1,)];
            let R = vec![(1i32, 7i32), (1, 8), (2, 9)];
            assert_eq!(#output, vec![(&1, &7), (&1, &8)]);
        }
    ));
}
