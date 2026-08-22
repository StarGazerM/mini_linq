use cq_compiler::{compile_iterator_pipeline, compile_pull};
use cq_ir::{iterator_pipeline, rust_access_plan};
use quote::quote;
use syn::visit::{self, Visit};

fn annotated_query() -> rust_access_plan::Plan {
    let source: rust_access_plan::Plan = syn::parse2(quote! {
        answer(x, y) => ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
        )) :-
            Left(x) => for (x,) in (left.iter()),
            Right(x, y) => for (y,) in
                (right.get(&(::core::clone::Clone::clone(x),)).into_iter().flatten()),
            Different(x, y) => if (*x != *y).
    })
    .unwrap();
    rust_access_plan::contract::check(&source).unwrap();
    source
}

#[test]
fn pull_exposes_every_binary_operator_and_intermediate_binding() {
    let actual = compile_pull(&annotated_query());
    iterator_pipeline::contract::check(&actual).unwrap();

    let expected: iterator_pipeline::Pipeline = syn::parse2(quote! {
        iter0 = scan (x,) in (left.iter()) yield (x,);
        iter1 = join iter0 as (x,) with (y,) in (
            right.get(&(::core::clone::Clone::clone(x),)).into_iter().flatten()
        ) yield (x, y,);
        iter2 = filter iter1 as (x, y,) if (*x != *y);
        iter3 = project iter2 as (x, y,) yield ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
        ));
        iter4 = distinct iter3;
        return iter4.
    })
    .unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn pipeline_emits_the_exact_lazy_rust_adapter_pipeline() {
    let plan = compile_pull(&annotated_query());
    let actual = compile_iterator_pipeline(&plan);

    let expected: syn::Expr = syn::parse2(quote! {
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
                                right.get(&(::core::clone::Clone::clone(x),)).into_iter().flatten()
                            ),
                            move |(y,)| (x, y,),
                        )
                    },
                );
                let iter2 = ::core::iter::Iterator::filter(
                    iter1,
                    move |#[allow(unused_variables)] &(x, y,)| *x != *y,
                );
                let iter3 = ::core::iter::Iterator::map(
                    iter2,
                    move |#[allow(unused_variables)] (x, y,)| (
                        ::core::clone::Clone::clone(x),
                        ::core::clone::Clone::clone(y),
                    ),
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

    assert_eq!(actual, expected);
}

#[test]
fn emitted_operators_stream_intermediates_without_materializing_them() {
    let iterator = compile_iterator_pipeline(&compile_pull(&annotated_query()));

    let mut shape = IteratorShape::default();
    shape.visit_expr(&iterator);

    assert_eq!(
        shape.maps, 3,
        "scan, joined binding, and project stay distinct"
    );
    assert_eq!(
        shape.flat_maps, 1,
        "one binary join consumes its left stream"
    );
    assert_eq!(
        shape.filters, 2,
        "the predicate and result-set distinct remain operator boundaries"
    );
    assert_eq!(
        shape.flattens, 1,
        "only the deferred root wrapper is flattened"
    );
    assert_eq!(shape.deferred_roots, 1, "root construction is deferred");
    assert_eq!(shape.for_loops, 0, "the result is one resumable iterator");
    assert_eq!(
        shape.intermediate_materializations, 0,
        "intermediate bindings remain iterator items",
    );
    assert_eq!(
        shape.distinct_state_updates, 1,
        "set semantics use one lazy result-key set",
    );
}

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
        return iter3.
    })
    .unwrap();
    iterator_pipeline::contract::check(&plan).unwrap();

    let actual = compile_iterator_pipeline(&plan);
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
                            move |total| (person, total,),
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
                    let mut seen = ::std::collections::HashSet::new();
                    ::core::iter::Iterator::filter(
                        iter2,
                        move |row| seen.insert(::core::clone::Clone::clone(row)),
                    )
                };
                iter3
            })
        )
    })
    .unwrap();

    assert_eq!(actual, expected);
}

#[derive(Default)]
struct IteratorShape {
    maps: usize,
    flat_maps: usize,
    filters: usize,
    flattens: usize,
    deferred_roots: usize,
    for_loops: usize,
    intermediate_materializations: usize,
    distinct_state_updates: usize,
}

impl<'ast> Visit<'ast> for IteratorShape {
    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        let syn::Expr::Path(function) = expression.func.as_ref() else {
            visit::visit_expr_call(self, expression);
            return;
        };
        let Some(name) = function.path.segments.last().map(|segment| &segment.ident) else {
            visit::visit_expr_call(self, expression);
            return;
        };

        if name == "map" {
            self.maps += 1;
        } else if name == "flat_map" {
            self.flat_maps += 1;
        } else if name == "filter" {
            self.filters += 1;
        } else if name == "flatten" {
            self.flattens += 1;
        } else if name == "once_with" {
            self.deferred_roots += 1;
        }

        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_for_loop(&mut self, expression: &'ast syn::ExprForLoop) {
        self.for_loops += 1;
        visit::visit_expr_for_loop(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        match expression.method.to_string().as_str() {
            "collect" | "extend" => self.intermediate_materializations += 1,
            "insert" => self.distinct_state_updates += 1,
            _ => {}
        }
        visit::visit_expr_method_call(self, expression);
    }
}
