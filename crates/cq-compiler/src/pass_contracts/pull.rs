//! Exact Access-to-Pipeline correspondence, including binding flow and leaves.

use cq_ir::{iterator_pipeline, rust_access_plan};
use quote::format_ident;

/// Whether `target` is exactly the explicit iterator pipeline denoted by
/// `source`.
///
/// This is an independent pass relation: it compares the two typed values
/// directly and never invokes the lowering under test. Besides requiring the
/// target's local contract, it preserves source order, every Rust pattern and
/// leaf expression, the complete intermediate binding at each boundary, and
/// the head output expression.
pub(crate) fn iterator_pipeline_is_exact(
    source: &rust_access_plan::Plan,
    target: &iterator_pipeline::Pipeline,
) -> bool {
    if !iterator_pipeline::contract::well_formed(target) {
        return false;
    }

    let mut binding = Vec::new();
    let mut definitions = target.definitions.iter();
    let Some(first_definition) = definitions.next() else {
        return false;
    };
    let mut expected_stream_number = 0usize;
    if first_definition.stream != generated_stream(expected_stream_number) {
        return false;
    }
    expected_stream_number += 1;
    let first = source.body.first();
    let access_offset = match (first.map(|clause| &clause.access), first_definition) {
        (
            Some(rust_access_plan::RustAccess::For {
                pattern,
                source: rust_source,
                ..
            }),
            iterator_pipeline::Definition {
                operator: iterator_pipeline::Operator::Scan(scan),
                ..
            },
        ) => {
            let Some(introduced) = enumerating_variables(pattern) else {
                return false;
            };
            binding.extend(introduced);
            if scan.item_pattern != *pattern
                || scan.source != *rust_source
                || scan.binding != binding_expression(&binding)
            {
                return false;
            }
            1
        }
        (
            None | Some(rust_access_plan::RustAccess::If { .. }),
            iterator_pipeline::Definition {
                stream,
                operator: iterator_pipeline::Operator::Unit(unit),
                ..
            },
        ) if stream == &generated_stream(0) && unit.binding == syn::parse_quote!(()) => 0,
        _ => return false,
    };

    let mut previous = first_definition.stream.clone();
    for clause in source.body.iter().skip(access_offset) {
        let Some(definition) = definitions.next() else {
            return false;
        };
        if definition.stream != generated_stream(expected_stream_number) {
            return false;
        }
        expected_stream_number += 1;
        match (&clause.access, &definition.operator) {
            (
                rust_access_plan::RustAccess::For {
                    pattern,
                    source: rust_source,
                    ..
                },
                iterator_pipeline::Operator::Join(join),
            ) => {
                if join.input_stream != previous
                    || join.input_pattern != binding_pattern(&binding)
                    || join.item_pattern != *pattern
                    || join.source != *rust_source
                {
                    return false;
                }
                let Some(introduced) = enumerating_variables(pattern) else {
                    return false;
                };
                binding.extend(introduced);
                if join.binding != binding_expression(&binding) {
                    return false;
                }
            }
            (
                rust_access_plan::RustAccess::If { condition, .. },
                iterator_pipeline::Operator::Filter(filter),
            ) => {
                if filter.input_stream != previous
                    || filter.input_pattern != binding_pattern(&binding)
                    || filter.condition != *condition
                {
                    return false;
                }
            }
            _ => return false,
        }
        previous = definition.stream.clone();
    }

    let Some(project_definition) = definitions.next() else {
        return false;
    };
    if project_definition.stream != generated_stream(expected_stream_number) {
        return false;
    }
    expected_stream_number += 1;
    let iterator_pipeline::Operator::Project(project) = &project_definition.operator else {
        return false;
    };
    if project.input_stream != previous
        || project.input_pattern != binding_pattern(&binding)
        || project.output != source.head.output
    {
        return false;
    }

    let Some(distinct_definition) = definitions.next() else {
        return false;
    };
    if distinct_definition.stream != generated_stream(expected_stream_number) {
        return false;
    }
    let iterator_pipeline::Operator::Distinct(distinct) = &distinct_definition.operator else {
        return false;
    };
    distinct.input_stream == project_definition.stream
        && definitions.next().is_none()
        && target.return_stream.stream == distinct_definition.stream
}

fn generated_stream(number: usize) -> syn::Ident {
    format_ident!("iter{number}", span = proc_macro2::Span::mixed_site())
}

fn enumerating_variables(pattern: &syn::Pat) -> Option<Vec<syn::Ident>> {
    match pattern {
        syn::Pat::Tuple(pattern) => pattern.elems.iter().map(simple_pattern_ident).collect(),
        syn::Pat::Ident(_) => simple_pattern_ident(pattern).map(|ident| vec![ident]),
        _ => None,
    }
}

fn simple_pattern_ident(pattern: &syn::Pat) -> Option<syn::Ident> {
    let syn::Pat::Ident(pattern) = pattern else {
        return None;
    };
    (pattern.attrs.is_empty()
        && pattern.by_ref.is_none()
        && pattern.mutability.is_none()
        && pattern.subpat.is_none())
    .then(|| pattern.ident.clone())
}

fn binding_pattern(binding: &[syn::Ident]) -> syn::Pat {
    syn::parse_quote!((#(#binding,)*))
}

fn binding_expression(binding: &[syn::Ident]) -> syn::Expr {
    syn::parse_quote!((#(#binding,)*))
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;
    use crate::pass_contracts::pipeline::iterator_pipeline_rust_is_exact;

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
