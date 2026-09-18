//! IndexRequirements to borrowed Access regions and selected Native resources.
//!
//! This scan implementation accepts an empty index overlay. It resolves RA
//! headings and resource ownership; execution and iterator emission remain in
//! the caller-selected Access and Pipeline passes.

mod domain;
mod lower;

use cq_ir::{index_requirements as ix, symbol_name};
use proc_macro2::Span;
use quote::quote;
use std::collections::BTreeMap;

/// Macro boundaries, supplied by the caller rather than implementation imports.
pub struct Options {
    pub inputs: BTreeMap<String, syn::Path>,
    /// Access -> lazy support iterator, preserving borrowed items.
    pub stream: syn::Path,
    /// Access -> lazy distinct answer iterator.
    pub output: syn::Path,
    /// Access -> first-witness boolean, without draining after a witness.
    pub exists: syn::Path,
    /// Native Build/Scan -> shared, demand-driven replay resource/cursor.
    pub replay: syn::Path,
}

pub fn compile(source: &ix::Module, options: &Options) -> syn::Result<syn::File> {
    let information = domain::check(source, &options.inputs)?;
    let source = &source.relational_plan;
    let arguments = domain::arguments(source);
    let lifetime = syn::Lifetime::new("'__cq_input", Span::mixed_site());
    let parameters = domain::parameters(source, &options.inputs, &arguments, &lifetime);
    let output_type = domain::output_type(
        source,
        &information[&symbol_name(&source.plan.output.input)],
        &lifetime,
    )?;
    let mut lowering = lower::Lowering::new(source, options, arguments, information);
    let definitions = lowering.definitions()?;
    let output = lowering.output()?;
    let (visibility, name) = (&source.program.visibility, &source.program.name);
    syn::parse2(quote! {
        #visibility struct #name;
        impl #name {
            pub fn iter<#lifetime>(#(#parameters),*)
                -> impl ::core::iter::Iterator<Item = #output_type> + #lifetime
            {
                #definitions
                #output
            }
        }
    })
}
