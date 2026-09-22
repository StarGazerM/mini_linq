//! Authored Pipeline boundaries tested without an Access compiler or frontend.
use cq_ir::iterator_pipeline;
#[cfg(feature = "distinct-iter")]
use cq_pipeline_lowering::distinct_iter::compile as compile_iterator_pipeline;
#[cfg(feature = "distinct-iter")]
use quote::quote;

#[cfg(feature = "distinct-iter")]
#[test]
fn aggregate_style_identifier_items_remain_an_explicit_join_boundary() {
    let plan: iterator_pipeline::Pipeline = syn::parse2(quote! {
        iter0 = scan (person,) in (people.iter()) yield (person,);
        iter1 = join iter0 as (person,) with total in (
            totals(*person)
        ) yield (person, total,);
        iter2 = project iter1 as (person, total,) yield ((
            ::core::clone::Clone::clone(person),
            total,
        ));
        iter3 = distinct iter2;
        return iter3;
    })
    .unwrap();
    iterator_pipeline::contract::check(&plan).unwrap();

    let actual = compile_iterator_pipeline(&plan).unwrap();
    let expected: syn::Expr = syn::parse2(quote! {
        ::core::iter::Iterator::flatten(
            ::core::iter::once_with(move || {
                let iter0 = ::core::iter::Iterator::map(
                    ::core::iter::IntoIterator::into_iter(people.iter()),
                    move |(person,)| (person,),
                );
                let iter1 = ::core::iter::Iterator::flat_map(
                    iter0,
                    move |(person,)| {
                        ::core::iter::Iterator::map(
                            ::core::iter::IntoIterator::into_iter(totals(*person)),
                            move |total| { let person = ::core::clone::Clone::clone(&person); (person, total,) },
                        )
                    },
                );
                let iter2 = ::core::iter::Iterator::map(
                    iter1,
                    move |#[allow(unused_variables)] (person, total,)| (
                        ::core::clone::Clone::clone(person),
                        total,
                    ),
                );
                let iter3 = {
                    let mut seen = mini_linq::distinct_state! { operator iter3; new };
                    ::core::iter::Iterator::filter(
                        iter2,
                        move |row| mini_linq::distinct_state! { operator iter3; insert(seen, row) },
                    )
                };
                iter3
            })
        )
    })
    .unwrap();

    assert_eq!(actual, expected);
}

fn rejects<T>(lower: impl FnOnce() -> syn::Result<T>, diagnostic: &str) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(lower))
        .unwrap_or_else(|_| panic!("a public compiler boundary panicked on invalid IR"));
    let error = result.err().expect("invalid IR must return a diagnostic");
    assert!(error.to_string().contains(diagnostic), "{error}");
}

#[test]
fn all_pipeline_endpoints_check_empty_and_incomplete_pipelines() {
    let empty: iterator_pipeline::Pipeline = syn::parse_quote! { return missing; };
    let incomplete: iterator_pipeline::Pipeline = syn::parse_quote! {
        iter0 = unit yield ();
        iter1 = project iter0 as () yield (42);
        return iter1;
    };
    for (source, diagnostic) in [
        (&empty, "must define a source and terminal distinct"),
        (&incomplete, "must end with `distinct`"),
    ] {
        #[cfg(feature = "distinct-iter")]
        rejects(
            || cq_pipeline_lowering::distinct_iter::compile(source),
            diagnostic,
        );
        #[cfg(feature = "collect-set")]
        rejects(
            || cq_pipeline_lowering::collect_set::compile(source),
            diagnostic,
        );
        #[cfg(feature = "support-iter")]
        rejects(
            || cq_pipeline_lowering::support_iter::compile(source),
            diagnostic,
        );
        #[cfg(feature = "exists")]
        rejects(|| cq_pipeline_lowering::exists::compile(source), diagnostic);
    }
}
