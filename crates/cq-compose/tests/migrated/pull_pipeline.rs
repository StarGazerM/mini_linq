mod tests {
    use quote::quote;

    use cq_ir::{iterator_pipeline, rust_access_plan};
    use cq_pipeline::iterator::check as iterator_pipeline_rust_is_exact;
    use cq_pull::check as iterator_pipeline_is_exact;

    #[test]
    fn iterator_pipeline_relations_reject_leaf_order_and_rust_mutations() {
        let source: rust_access_plan::Plan = syn::parse2(quote! {
            answer(x, y) => ((x.clone(), y.clone(),)) :-
                Left(x) => for (x,) in (left.iter()),
                Right(x, y) => for (y,) in
                    (right.get(&(x.clone(),)).into_iter().flatten()),
                Keep(x, y) => if (keep(*x, *y));
        })
        .unwrap();
        rust_access_plan::contract::check(&source).unwrap();

        let exact: iterator_pipeline::Pipeline = syn::parse2(quote! {
            iter0 = scan (x,) in (left.iter()) yield (x,);
            iter1 = join iter0 as (x,) with (y,) in
                (right.get(&(x.clone(),)).into_iter().flatten()) yield (x, y,);
            iter2 = filter iter1 as (x, y,) if (keep(*x, *y));
            iter3 = project iter2 as (x, y,) yield ((x.clone(), y.clone(),));
            iter4 = distinct iter3;
            return iter4;
        })
        .unwrap();
        iterator_pipeline::contract::check(&exact).unwrap();
        assert!(iterator_pipeline_is_exact(&source, &exact));

        let mut changed_source = exact.clone();
        let iterator_pipeline::Operator::Join(join) = &mut changed_source.definitions[1].operator
        else {
            unreachable!()
        };
        join.source = syn::parse_quote!(other.get(&(x.clone(),)).into_iter().flatten());

        let mut changed_condition = exact.clone();
        let iterator_pipeline::Operator::Filter(filter) =
            &mut changed_condition.definitions[2].operator
        else {
            unreachable!()
        };
        filter.condition = syn::parse_quote!(!keep(*x, *y));

        let mut changed_output = exact.clone();
        let iterator_pipeline::Operator::Project(project) =
            &mut changed_output.definitions[3].operator
        else {
            unreachable!()
        };
        project.output = syn::parse_quote!((y.clone(), x.clone(),));

        let reordered: iterator_pipeline::Pipeline = syn::parse2(quote! {
            iter0 = scan (x,) in (left.iter()) yield (x,);
            iter1 = filter iter0 as (x,) if (ready(*x));
            iter2 = join iter1 as (x,) with (y,) in
                (right.get(&(x.clone(),)).into_iter().flatten()) yield (x, y,);
            iter3 = project iter2 as (x, y,) yield ((x.clone(), y.clone(),));
            iter4 = distinct iter3;
            return iter4;
        })
        .unwrap();

        for different_but_well_formed in
            [changed_source, changed_condition, changed_output, reordered]
        {
            iterator_pipeline::contract::check(&different_but_well_formed).unwrap();
            assert!(!iterator_pipeline_is_exact(
                &source,
                &different_but_well_formed
            ));
        }

        let exact_rust: syn::Expr = syn::parse2(quote! {
            ::core::iter::Iterator::flatten(
                ::core::iter::once_with(move || {
                    let iter0 = ::core::iter::Iterator::map(
                        ::core::iter::IntoIterator::into_iter(left.iter()),
                        move |(x,)| (x,),
                    );
                    let iter1 = ::core::iter::Iterator::flat_map(
                        iter0,
                        move |(x,)| {
                            ::core::iter::Iterator::map(
                                ::core::iter::IntoIterator::into_iter(
                                    right.get(&(x.clone(),)).into_iter().flatten()
                                ),
                                move |(y,)| { let x = ::core::clone::Clone::clone(&x); (x, y,) },
                            )
                        },
                    );
                    let iter2 = ::core::iter::Iterator::filter_map(
                        iter1,
                        move |#[allow(unused_variables)] (x, y,)| { if keep(*x, *y) { ::core::option::Option::Some((x, y,)) } else { ::core::option::Option::None } },
                    );
                    let iter3 = ::core::iter::Iterator::map(
                        iter2,
                        move |#[allow(unused_variables)] (x, y,)| (x.clone(), y.clone(),),
                    );
                    let iter4 = {
                        let mut seen = ::std::collections::HashSet::new();
                        ::core::iter::Iterator::filter(
                            iter3,
                            move |row| seen.insert(::core::clone::Clone::clone(row)),
                        )
                    };
                    iter4
                })
            )
        })
        .unwrap();
        assert!(iterator_pipeline_rust_is_exact(&exact, &exact_rust));

        let wrong_rust: syn::Expr = syn::parse2(quote! {
            ::core::iter::Iterator::flatten(
                ::core::iter::once_with(move || {
                    let iter0 = ::core::iter::Iterator::map(
                        ::core::iter::IntoIterator::into_iter(other.iter()),
                        move |(x,)| (x,),
                    );
                    iter0
                })
            )
        })
        .unwrap();
        assert!(!iterator_pipeline_rust_is_exact(&exact, &wrong_rust));
    }
}
