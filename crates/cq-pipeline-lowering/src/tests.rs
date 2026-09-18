use super::*;
use cq_ir::iterator_pipeline;
use quote::quote;

#[test]
fn support_stream_omits_only_the_returned_terminal_distinct() {
    let source: iterator_pipeline::Pipeline = syn::parse_quote! {
        iter0 = scan (x,) in (rows) yield (x,);
        iter1 = filter iter0 as (x,) if (keep(x));
        iter2 = project iter1 as (x,) yield (x % 2);
        iter3 = distinct iter2;
        return iter3;
    };
    let actual = support_iter::compile(&source).unwrap();
    assert!(support_iter::check(&source, &actual));
    assert!(!distinct_iter::check(&source, &actual));
    let text = quote!(#actual).to_string();
    assert!(!text.contains("HashSet"));
    assert!(!text.contains("iter3"));
    assert!(text.contains("keep (x)"));
    assert!(text.contains("x % 2"));
    assert!(!support_iter::check(
        &source,
        &distinct_iter::compile(&source).unwrap()
    ));
    let mut wrong_source = source.clone();
    let iterator_pipeline::Operator::Scan(scan) = &mut wrong_source.definitions[0].operator else {
        unreachable!();
    };
    scan.source = syn::parse_quote!(other_rows);
    assert!(!support_iter::check(&wrong_source, &actual));

    let mut wrong_return = source.clone();
    wrong_return.return_stream.stream = syn::parse_quote!(iter2);
    assert!(support_iter::compile(&wrong_return).is_err());
    assert!(!support_iter::check(&wrong_return, &actual));
    let mut missing_distinct = source;
    missing_distinct.definitions.pop();
    missing_distinct.return_stream.stream = syn::parse_quote!(iter2);
    assert!(support_iter::compile(&missing_distinct).is_err());
}

#[test]
fn set_sink_contract_checks_the_terminal_stream_and_collection_type() {
    let source: iterator_pipeline::Pipeline = syn::parse_quote! {
        iter0 = scan (x,) in (rows) yield (x,);
        iter1 = filter iter0 as (x,) if (keep(x));
        iter2 = project iter1 as (x,) yield (x % 2);
        iter3 = distinct iter2;
        return iter3;
    };
    let actual = collect_set::compile(&source).unwrap();
    let expected: syn::Expr = syn::parse_quote! {{
        let iter0 = ::core::iter::Iterator::map(
            ::core::iter::IntoIterator::into_iter(rows),
            move |(x,)| (x,),
        );
        let iter1 = ::core::iter::Iterator::filter_map(
            iter0,
            move |#[allow(unused_variables)] (x,)| {
                if keep(x) { ::core::option::Option::Some((x,)) }
                else { ::core::option::Option::None }
            },
        );
        let iter2 = ::core::iter::Iterator::map(
            iter1,
            move |#[allow(unused_variables)] (x,)| x % 2,
        );
        let iter3: ::std::collections::HashSet<_> = ::core::iter::Iterator::collect(iter2);
        iter3
    }};
    assert_eq!(actual, expected);
    assert!(collect_set::check(&source, &actual));
    assert!(!collect_set::check(
        &source,
        &distinct_iter::compile(&source).unwrap()
    ));

    for changed in [
        syn::parse_quote!(let iter3: ::std::vec::Vec<_> = ::core::iter::Iterator::collect(iter2);),
        syn::parse_quote!(let iter3: ::std::collections::HashSet<_> = ::core::iter::Iterator::collect(iter0);),
    ] {
        let mut target = expected.clone();
        let syn::Expr::Block(block) = &mut target else {
            unreachable!();
        };
        block.block.stmts[3] = changed;
        assert!(!collect_set::check(&source, &target));
    }
}
