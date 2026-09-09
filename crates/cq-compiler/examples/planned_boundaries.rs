//! Write a complete executable example at each existing language boundary.
//! cargo run -p cq-compiler --example planned_boundaries -- doc/planned-example
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
    let relational = cq_compiler::compile_cq_planned(&source).unwrap();
    let indexed = cq_compiler::compile_result_indexes(&relational).unwrap();
    let staged = cq_compiler::compile_materialized(&indexed).unwrap();
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
            if m.mac.path.segments.last().unwrap().ident == "pull" {
                let source: rust_access_plan::Plan = syn::parse2(m.mac.tokens.clone()).unwrap();
                let target = cq_compiler::compile_pull(&source);
                *expr = syn::parse_quote!(::mini_linq::iterator_pipeline! { #target });
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
            if m.mac.path.segments.last().unwrap().ident == "iterator_pipeline" {
                let source: iterator_pipeline::Pipeline =
                    syn::parse2(m.mac.tokens.clone()).unwrap();
                *expr = cq_compiler::compile_iterator_pipeline(&source);
                return;
            }
        }
        syn::visit_mut::visit_expr_mut(self, expr);
    }
}

fn main() {
    let directory = std::env::args_os()
        .nth(1)
        .expect("supply the output directory");
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    for (name, file) in artifacts() {
        std::fs::write(directory.join(name), prettyplease::unparse(&file)).unwrap();
    }
}
