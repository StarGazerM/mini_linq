//! The physical rules admit an extension with no fresh names: its existing
//! Let-labelled singleton access still evaluates the Rust expression.
use cq_compose::compile_cq;
use cq_index::result::compile as compile_result_indexes;
use cq_ir::{cq, relational_plan};
use cq_physical::lazy::compile_lazy_indexed;
use cq_physical::materialized::compile_materialized;

#[test]
fn an_empty_extension_is_supported_by_both_physical_routes() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32);
        answer(x) :- R(x), let () = ();
    };
    cq::contract::check(&source).unwrap();
    let logical = compile_cq(&source).unwrap();
    relational_plan::contract::check(&logical).unwrap();
    let indexed = compile_result_indexes(&logical).unwrap();
    compile_materialized(&indexed, &syn::parse_quote!(::mini_linq::collect_access)).unwrap();
    compile_lazy_indexed(&indexed, &syn::parse_quote!(::mini_linq::pull)).unwrap();
}
