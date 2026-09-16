//! Input-indexed storage, ownership, API and Access observations.
use super::fixtures::PhysicalFixture;
use cq_execution::pull::compile as compile_pull;
use cq_ir::{cq, index_requirements, iterator_pipeline, rust_access_plan};
use cq_pipeline::iterator::compile as compile_iterator_pipeline;
use cq_realize::hash_inputs::compile_input_indexed;
use proc_macro2::Span;
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeMap;
use syn::visit::Visit;
use syn::visit_mut::{self, VisitMut};
mod access;
mod api;
mod construction;
mod execution;
mod syntax;
use access::assert_unique_pull_plan;
use api::assert_staged_api;
use construction::*;
use execution::compile_and_run_staged_rust;
use syntax::*;
pub fn assert_index_requirements_to_staged_rust(fixture: PhysicalFixture) {
    let source: index_requirements::Module = syn::parse2(fixture.source.clone()).unwrap();
    index_requirements::contract::check(&source).unwrap();
    let file = compile_input_indexed(&source, &syn::parse_quote!(::mini_linq::pull)).unwrap();
    let text = file.to_token_stream().to_string();

    assert_eq!(source.relational_plan.program.name, fixture.program_name);
    assert_staged_api(&file, &source, &fixture);
    assert_unique_pull_plan(&file, &fixture);

    for required in fixture.required_rust {
        assert!(
            text.contains(required),
            "{} is missing staged-Rust fragment: {required}",
            fixture.program_name,
        );
    }

    compile_and_run_staged_rust(file, &fixture);
}

pub fn assert_missing_required_index_rejected(tokens: proc_macro2::TokenStream) {
    let source: index_requirements::Module = syn::parse2(tokens.clone()).unwrap();
    index_requirements::contract::check(&source).unwrap();

    let error = compile_input_indexed(&source, &syn::parse_quote!(::mini_linq::pull)).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("Edge"));
    assert!(message.contains('0'));
}
