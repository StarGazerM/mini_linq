//! The three basic homework lowerings and the supplied pull-strategy lowerings.
//!
//! The third homework pass targets a mixed-stage Rust file containing one
//! local `pull!` invocation. Storage objects, their construction code, Rust
//! names, access expressions, output value, and the surrounding query API
//! must already be literal Rust syntax in that target. The supplied `pull`
//! pass first exposes a left-deep iterator plan; a separate semantic lowering
//! chooses the concrete lazy Rust iterator adapters and returns `syn::Expr`.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

use cq_ir::{cq, index_requirements, iterator_pipeline, relational_plan, rust_access_plan};

use crate::pass_contracts;

#[path = "staff_r1.rs"]
mod staff_r1;

/// Basic Homework 1: lower a CQ to canonical named relational algebra.
#[allow(unreachable_code, clippy::diverging_sub_expression)]
#[contracts::contract(
    debug_requires(
        cq::contract::well_formed(source),
        "source must satisfy the CQ local contract"
    ),
    debug_ensures(
        ret.as_ref()
            .is_ok_and(relational_plan::contract::well_formed)
    ),
    debug_ensures(
        ret.as_ref()
            .is_ok_and(|target| target.program == source.program)
    ),
    debug_ensures(
        ret.as_ref()
            .is_ok_and(|target| pass_contracts::relational_plan_is_exact(source, target))
    ),
)]
pub fn compile_cq(source: &cq::Module) -> syn::Result<relational_plan::Module> {
    // BASIC HOMEWORK 1: construct the complete canonical RelationalPlan.
    // OPTIONAL HW4 — STUDENT 3/6 (LOGICAL): extend this lowering as specified
    // in doc/HW4-OPTIONAL.md.
    staff_r1::compile_cq(source)
}

/// Basic Homework 2: add the exact equality-index overlay to a logical plan.
#[allow(unreachable_code, clippy::diverging_sub_expression)]
#[contracts::contract(
    debug_requires(
        relational_plan::contract::well_formed(source),
        "source must satisfy the RelationalPlan local contract"
    ),
    debug_ensures(
        pass_contracts::basic_left_deep_plan(source)
            -> ret.as_ref().is_ok_and(index_requirements::contract::well_formed)
    ),
    debug_ensures(
        pass_contracts::basic_left_deep_plan(source)
            -> ret.as_ref().is_ok_and(|target| pass_contracts::indexes_are_exact(source, target))
    ),
    debug_ensures(
        !pass_contracts::basic_left_deep_plan(source) -> ret.is_err()
    ),
)]
pub fn compile_relational_plan(
    source: &relational_plan::Module,
) -> syn::Result<index_requirements::Module> {
    // BASIC HOMEWORK 2: preserve the full logical plan and derive its keys.
    // OPTIONAL HW4 — STUDENT 4/6 (INDEX): extend canonical key derivation as
    // specified in doc/HW4-OPTIONAL.md.
    staff_r1::compile_relational_plan(source)
}

/// Basic Homework 3: lower the required indexed join to staged, ordinary Rust.
///
/// The returned file must contain the complete query API and concrete Rust
/// storage-building statements. Its core join is one
/// `::mini_linq::pull!` call whose body and head occurrences are paired with
/// their exact Rust patterns, sources, predicates, and output expression.
/// Every written index requirement, including an unused extra one, is built as
/// a concrete Rust collection. During its one source walk, this pass returns a
/// span-aware error if a canonical access requirement is missing.
#[allow(unreachable_code, clippy::diverging_sub_expression)]
#[contracts::contract(
    debug_requires(
        index_requirements::contract::well_formed(source),
        "source must satisfy the IndexRequirements local contract"
    ),
    debug_ensures(pass_contracts::physical_input_is_supported(source) -> ret.is_ok()),
    debug_ensures(!pass_contracts::physical_input_is_supported(source) -> ret.is_err()),
)]
pub fn compile_index_requirements(source: &index_requirements::Module) -> syn::Result<syn::File> {
    // BASIC HOMEWORK 3: lower the indexed logical plan to staged Rust.
    // OPTIONAL HW4 — STUDENT 6/6 (PHYSICAL): emit the extended accesses as
    // specified in doc/HW4-OPTIONAL.md.
    staff_r1::compile_index_requirements(source)
}

/// Supplied pull strategy: expose a conventional named iterator plan.
///
/// This pass chooses the source and binary operator tree, assigns every stream
/// a stable name, and records complete tuple bindings at every boundary. It
/// does not choose Rust iterator combinators or construct Rust closures.
#[contracts::contract(
    debug_requires(
        rust_access_plan::contract::well_formed(source),
        "source must satisfy the RustAccessPlan local contract"
    ),
    debug_ensures(pass_contracts::iterator_pipeline_is_exact(source, &ret))
)]
pub fn compile_pull(source: &rust_access_plan::Plan) -> iterator_pipeline::Pipeline {
    let mut binding = Vec::new();
    let mut clauses = source.body.iter();
    let mut definitions = Vec::<TokenStream>::new();
    let mut next_stream_number = 0usize;

    let mut current_stream = match clauses.next() {
        Some(
            clause @ rust_access_plan::Clause {
                access: rust_access_plan::RustAccess::For { .. },
                ..
            },
        ) => {
            let stream = next_stream(&mut next_stream_number);
            let definition = lower_root_scan(clause, &stream, &mut binding);
            definitions.push(definition);
            stream
        }
        Some(clause) => {
            let stream = next_stream(&mut next_stream_number);
            definitions.push(quote! {
                #stream = unit yield ();
            });
            let (next_stream, definition) =
                lower_operator(clause, &stream, &mut binding, &mut next_stream_number);
            definitions.push(definition);
            next_stream
        }
        None => {
            let stream = next_stream(&mut next_stream_number);
            definitions.push(quote! {
                #stream = unit yield ();
            });
            stream
        }
    };

    for clause in clauses {
        let (next_stream, definition) = lower_operator(
            clause,
            &current_stream,
            &mut binding,
            &mut next_stream_number,
        );
        definitions.push(definition);
        current_stream = next_stream;
    }

    let project_pattern = binding_pattern(&binding);
    let output = &source.head.output;
    let project_stream = next_stream(&mut next_stream_number);
    definitions.push(quote! {
        #project_stream = project #current_stream as #project_pattern yield (#output);
    });
    let distinct_stream = next_stream(&mut next_stream_number);
    definitions.push(quote! {
        #distinct_stream = distinct #project_stream;
    });

    syn::parse2(quote! {
        #(#definitions)*
        return #distinct_stream.
    })
    .expect("a well-formed RustAccessPlan must lower to an IteratorPipeline")
}

fn lower_root_scan(
    clause: &rust_access_plan::Clause,
    stream: &syn::Ident,
    binding: &mut Vec<syn::Ident>,
) -> TokenStream {
    let rust_access_plan::RustAccess::For {
        pattern, source, ..
    } = &clause.access
    else {
        unreachable!("the root scan caller selected a Rust `for` access")
    };
    binding.extend(pattern_variables(pattern));
    let output = binding_expression(binding);
    quote! {
        #stream = scan #pattern in (#source) yield #output;
    }
}

fn lower_operator(
    clause: &rust_access_plan::Clause,
    input_stream: &syn::Ident,
    binding: &mut Vec<syn::Ident>,
    next_stream_number: &mut usize,
) -> (syn::Ident, TokenStream) {
    let input = binding_pattern(binding);
    match &clause.access {
        rust_access_plan::RustAccess::For {
            pattern, source, ..
        } => {
            binding.extend(pattern_variables(pattern));
            let output = binding_expression(binding);
            let stream = next_stream(next_stream_number);
            let definition = quote! {
                #stream = join #input_stream as #input with #pattern in (#source) yield #output;
            };
            (stream, definition)
        }
        rust_access_plan::RustAccess::If { condition, .. } => {
            let stream = next_stream(next_stream_number);
            let definition = quote! {
                #stream = filter #input_stream as #input if (#condition);
            };
            (stream, definition)
        }
    }
}

fn next_stream(next_stream_number: &mut usize) -> syn::Ident {
    let stream = format_ident!("iter{}", *next_stream_number, span = Span::mixed_site());
    *next_stream_number += 1;
    stream
}

fn pattern_variables(pattern: &syn::Pat) -> impl Iterator<Item = syn::Ident> + '_ {
    let variables: Vec<syn::Ident> = match pattern {
        syn::Pat::Tuple(pattern) => pattern.elems.iter().map(simple_pattern_ident).collect(),
        syn::Pat::Ident(pattern) => vec![pattern.ident.clone()],
        _ => unreachable!(
            "the RustAccessPlan contract requires a simple tuple pattern for generated `for` accesses"
        ),
    };
    variables.into_iter()
}

fn simple_pattern_ident(element: &syn::Pat) -> syn::Ident {
    let syn::Pat::Ident(element) = element else {
        unreachable!("the RustAccessPlan contract requires simple identifier bindings")
    };
    element.ident.clone()
}

fn binding_pattern(binding: &[syn::Ident]) -> syn::Pat {
    syn::parse_quote!((#(#binding,)*))
}

fn binding_expression(binding: &[syn::Ident]) -> syn::Expr {
    syn::parse_quote!((#(#binding,)*))
}

/// Lower the named physical operators to the actual ordinary-Rust expression.
///
/// This is the final semantic lowering: it selects standard iterator adapters
/// and writes one Rust local per named operator. The returned [`syn::Expr`] is
/// already the target representation, so proc-macro emission is only
/// `ToTokens`; there is no later code generator that reconstructs the plan.
#[contracts::contract(
    debug_requires(
        iterator_pipeline::contract::well_formed(source),
        "source must satisfy the IteratorPipeline local contract"
    ),
    debug_ensures(pass_contracts::iterator_pipeline_rust_is_exact(source, &ret))
)]
pub fn compile_iterator_pipeline(source: &iterator_pipeline::Pipeline) -> syn::Expr {
    let locals = source
        .definitions
        .iter()
        .map(lower_operator_to_rust)
        .collect::<Vec<_>>();
    let return_stream = &source.return_stream.stream;
    syn::parse2(quote! {
        ::core::iter::Iterator::flatten(
            ::core::iter::once_with(move || {
                #(#locals)*
                #return_stream
            })
        )
    })
    .expect("a well-formed IteratorPipeline must lower to one Rust expression")
}

fn lower_operator_to_rust(definition: &iterator_pipeline::Definition) -> TokenStream {
    let stream = &definition.stream;
    match &definition.operator {
        iterator_pipeline::Operator::Unit(operator) => {
            let binding = &operator.binding;
            quote! {
                let #stream = ::core::iter::once(#binding);
            }
        }
        iterator_pipeline::Operator::Scan(operator) => {
            let pattern = &operator.item_pattern;
            let source = &operator.source;
            let binding = &operator.binding;
            quote! {
                let #stream = ::core::iter::Iterator::map(
                    ::core::iter::IntoIterator::into_iter(#source),
                    move |#pattern| #binding,
                );
            }
        }
        iterator_pipeline::Operator::Join(operator) => {
            let input_stream = &operator.input_stream;
            let input_pattern = &operator.input_pattern;
            let item_pattern = &operator.item_pattern;
            let source = &operator.source;
            let binding = &operator.binding;
            quote! {
                let #stream = ::core::iter::Iterator::flat_map(
                    #input_stream,
                    move |#input_pattern| {
                        ::core::iter::Iterator::map(
                            ::core::iter::IntoIterator::into_iter(#source),
                            move |#item_pattern| #binding,
                        )
                    },
                );
            }
        }
        iterator_pipeline::Operator::Filter(operator) => {
            let input_stream = &operator.input_stream;
            let input_pattern = &operator.input_pattern;
            let borrowed_pattern: syn::Pat = syn::parse_quote!(&#input_pattern);
            let condition = &operator.condition;
            quote! {
                let #stream = ::core::iter::Iterator::filter(
                    #input_stream,
                    move |#[allow(unused_variables)] #borrowed_pattern| #condition,
                );
            }
        }
        iterator_pipeline::Operator::Project(operator) => {
            let input_stream = &operator.input_stream;
            let input_pattern = &operator.input_pattern;
            let output = &operator.output;
            quote! {
                let #stream = ::core::iter::Iterator::map(
                    #input_stream,
                    move |#[allow(unused_variables)] #input_pattern| #output,
                );
            }
        }
        iterator_pipeline::Operator::Distinct(operator) => {
            let input_stream = &operator.input_stream;
            quote! {
                let #stream = {
                    let mut seen = ::std::collections::HashSet::new();
                    ::core::iter::Iterator::filter(
                        #input_stream,
                        move |row| seen.insert(::core::clone::Clone::clone(row)),
                    )
                };
            }
        }
    }
}
