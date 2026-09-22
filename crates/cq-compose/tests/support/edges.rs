use super::fixtures::LoweringFixture;
use cq_access_lowering::to_pipeline::compile as compile_pull;
use cq_compose::compile_cq;
use cq_index_policy::base_demands::compile as compile_input_indexes;
use cq_ir::{cq, index_requirements, iterator_pipeline, relational_plan, rust_access_plan};
use cq_pipeline_lowering::distinct_iter::compile as compile_iterator_pipeline;
pub fn assert_cq_to_relational_plan(fixture: LoweringFixture) {
    let source: cq::Module = syn::parse2(fixture.source).unwrap();
    cq::contract::check(&source).unwrap();
    let actual_logical = compile_cq(&source).unwrap();
    relational_plan::contract::check(&actual_logical).unwrap();
    let expected: index_requirements::Module = syn::parse2(fixture.expected).unwrap();
    index_requirements::contract::check(&expected).unwrap();
    assert_eq!(actual_logical, expected.relational_plan);
}

pub fn assert_relational_plan_to_index_requirements(fixture: LoweringFixture) {
    let expected: index_requirements::Module = syn::parse2(fixture.expected).unwrap();
    index_requirements::contract::check(&expected).unwrap();
    let actual = compile_input_indexes(&expected.relational_plan).unwrap();
    assert_eq!(actual, expected);
}

pub fn assert_pull_pipeline_lowers_to_iterator_expression(source: proc_macro2::TokenStream) {
    let source: rust_access_plan::Plan = syn::parse2(source).unwrap();
    rust_access_plan::contract::check(&source).unwrap();
    let pipeline = compile_pull(&source).unwrap();
    iterator_pipeline::contract::check(&pipeline).unwrap();
    let _: syn::Expr = compile_iterator_pipeline(&pipeline).unwrap();
}
