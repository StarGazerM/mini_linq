//! Complete authored query artifacts at each compilation boundary.
use cq_ir::{cq, iterator_pipeline, rust_access_plan};
use quote::quote;
use syn::visit_mut::VisitMut;

pub fn artifacts() -> Vec<(&'static str, syn::File)> {
    let source: cq::Module = syn::parse_quote! {
        pub struct PlannedExample;
        relation R(a:i32,b:i32);
        relation S(b:i32,c:i32);
        answer(a) :- R(a,b), S(b,c), if (c > 0);
    };
    let relational = cq_compose::compile_cq_planned(&source).unwrap();
    let indexed = cq_index::result::compile(&relational).unwrap();
    let staged = cq_physical::materialized::compile_materialized(
        &indexed,
        &syn::parse_quote!(::mini_linq::collect_access),
    )
    .unwrap();
    let mut pipelines = staged.clone();
    ExposePipelines.visit_file_mut(&mut pipelines);
    let mut rust = pipelines.clone();
    LowerPipelines.visit_file_mut(&mut rust);
    vec![
        (
            "01-cq.rs",
            syn::parse2(quote!(::mini_linq::planned_query! { #source })).unwrap(),
        ),
        (
            "02-relational.rs",
            syn::parse2(quote!(
                ::mini_linq::planned_relational_plan! { #relational }
            ))
            .unwrap(),
        ),
        (
            "03-indexes.rs",
            syn::parse2(quote!(::mini_linq::materialized! { #indexed })).unwrap(),
        ),
        ("04-access.rs", staged),
        ("05-pipelines.rs", pipelines),
        ("06-rust.rs", rust),
    ]
}

struct ExposePipelines;
impl VisitMut for ExposePipelines {
    fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
        if let syn::Expr::Macro(m) = expr {
            if m.mac.path.segments.last().unwrap().ident == "collect_access" {
                let source: rust_access_plan::Plan = syn::parse2(m.mac.tokens.clone()).unwrap();
                let target = cq_pull::compile(&source).unwrap();
                *expr = syn::parse_quote!(::mini_linq::collect_pipeline! { #target });
                return;
            }
        }
        syn::visit_mut::visit_expr_mut(self, expr);
    }
}

struct LowerPipelines;
impl VisitMut for LowerPipelines {
    fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
        if let syn::Expr::Macro(m) = expr {
            if m.mac.path.segments.last().unwrap().ident == "collect_pipeline" {
                let source: iterator_pipeline::Pipeline =
                    syn::parse2(m.mac.tokens.clone()).unwrap();
                *expr = cq_pipeline::collect::compile(&source).unwrap();
                return;
            }
        }
        syn::visit_mut::visit_expr_mut(self, expr);
    }
}
