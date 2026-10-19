use cq_rewrite_passes::build_host_reads;
use cq_testkit::physical as ir;
use proc_macro2::TokenStream;
use quote::quote;

#[test]
fn retains_integral_host_without_changing_graph_or_keys() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x};
        r = pipe rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,r;
        p = pipe canon probe first [x] s,r;
        return p as answer(x,y);
    }};
    let actual = build_host_reads(&source, TokenStream::new()).unwrap();
    let mut expected = source.clone();
    expected.program.definitions[1].mode = ir::Mode::Let(Default::default());
    assert_eq!(actual, expected);
    ir::check_rewrite(&source, &actual).unwrap();
    assert_eq!(build_host_reads(&actual, quote!(r)).unwrap(), actual);
    let selected = cq_structure_selection::compile(&actual).unwrap();
    let selected = cq_rewrite_passes::sorted_runs(&selected, TokenStream::new()).unwrap();
    let facts = cq_testkit::structures::contract::check(&selected).unwrap();
    assert!(facts.built.contains_key("r"));
    assert!(
        !facts
            .fallbacks
            .keys()
            .any(|(host, reader, _)| host == "r" && reader != "scan")
    );
}

#[test]
fn skips_non_integral_and_lazy_membership() {
    for source in [
        syn::parse_quote! { physical {
            relation L(a:String); relation R(a:String);
            l = pipe rename L@0 {a -> x}; r = pipe rename R@1 {a -> x};
            s = pipe semi eager [x] l,r; return s as answer(x);
        }},
        syn::parse_quote! { physical {
            relation L(a:i64); relation R(a:i64);
            l = pipe rename L@0 {a -> x}; r = pipe rename R@1 {a -> x};
            s = pipe semi lazy [x] l,r; return s as answer(x);
        }},
    ] {
        assert_eq!(
            build_host_reads(&source, TokenStream::new()).unwrap(),
            source
        );
        assert!(build_host_reads(&source, quote!(r)).is_err());
    }
}

#[test]
fn public_pipeline_preserves_duplicates_until_canon() {
    let left = [(1i64,), (2,), (1,)];
    let right = [(1i64, 8i64), (1, 9), (2, 7), (1, 8)];
    let mut answer: Vec<_> = mini_linq::compile! {
        pipeline {
            physical = [cq_rewrites::build_host_reads];
            structures = [cq_rewrites::sorted_runs, cq_rewrites::retain_lookups];
        }
        struct RetainedHost;
        relation L(a:i64) = (&left);
        relation R(a:i64,b:i64) = (&right);
        answer(x,y) :- L(x), R(x,y);
    }
    .map(|(x, y)| (*x, *y))
    .collect();
    answer.sort_unstable();
    assert_eq!(answer, vec![(1, 8), (1, 9), (2, 7)]);
}
