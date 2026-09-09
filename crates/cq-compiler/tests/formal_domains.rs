//! Domain witnesses used by doc/formal; successful local checking is not a
//! claim that every physical lowering admits the same language fragment.
use cq_compiler::{
    compile_cq, compile_index_requirements, compile_materialized, compile_result_indexes,
};
use cq_ir::{cq, relational_plan};

#[test]
fn a_well_formed_empty_extension_is_outside_both_physical_domains() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32);
        answer(x) :- R(x), let () = ();
    };
    cq::contract::check(&source).unwrap();
    let logical = compile_cq(&source).unwrap();
    relational_plan::contract::check(&logical).unwrap();
    let indexed = compile_result_indexes(&logical).unwrap();
    let error = compile_materialized(&indexed).unwrap_err();
    assert!(error.to_string().contains("fully bound"), "{error}");
    // The default entry's support predicate currently overclaims this case:
    // its debug postcondition panics, while release returns the diagnostic.
    let lazy = std::panic::catch_unwind(|| compile_index_requirements(&indexed));
    if cfg!(debug_assertions) {
        let panic = lazy.expect_err("the current debug support contract overclaims this domain");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap();
        assert!(
            message.contains("Post-condition of compile_index_requirements violated"),
            "{message}"
        );
    } else {
        let error = lazy.unwrap().unwrap_err();
        assert!(error.to_string().contains("fully bound"), "{error}");
    }
}
