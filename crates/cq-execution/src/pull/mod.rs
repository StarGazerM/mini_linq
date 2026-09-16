//! Pull execution exposes an iterator pipeline for the selected Access region.

use cq_ir::{iterator_pipeline, rust_access_plan};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

mod check;

/// Supplied pull strategy: expose a conventional named iterator plan.
///
/// This pass chooses the source and binary operator tree, assigns every stream
/// a stable name, and records complete tuple bindings at every boundary. It
/// does not choose Rust iterator combinators or construct Rust closures.
#[contracts::contract(
    debug_ensures(
        rust_access_plan::contract::well_formed(source) -> ret.as_ref()
            .is_ok_and(|target| check::iterator_pipeline_is_exact(source, target))
    )
)]
pub fn compile(source: &rust_access_plan::Plan) -> syn::Result<iterator_pipeline::Pipeline> {
    rust_access_plan::contract::check(source)?;
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
        return #distinct_stream;
    })
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

pub use check::iterator_pipeline_is_exact as check;
