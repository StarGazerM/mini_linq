use super::{edges::assert_pull_pipeline_lowers_to_iterator_expression, fixtures};
use cq_access_lowering::to_pipeline::compile as compile_pull;
use cq_ir::{cq, index_requirements, iterator_pipeline, rust_access_plan};
use cq_pipeline_lowering::distinct_iter::compile as compile_iterator_pipeline;
use quote::quote;
pub fn assert_semantically_invalid_sources_fail_local_contracts() {
    let invalid = fixtures::invalid_sources();

    let cq: cq::Module = syn::parse2(invalid.cq).unwrap();
    assert!(
        cq::contract::check(&cq).is_err(),
        "the parsed CQ fixture must fail its local semantic contract",
    );
    assert!(!cq::contract::well_formed(&cq));

    let indexes: index_requirements::Module = syn::parse2(invalid.index_requirements).unwrap();
    assert!(
        index_requirements::contract::check(&indexes).is_err(),
        "the parsed IndexRequirements fixture must fail its local semantic contract",
    );
    assert!(!index_requirements::contract::well_formed(&indexes));

    let access_plan: rust_access_plan::Plan = syn::parse2(invalid.rust_access_plan).unwrap();
    assert!(
        rust_access_plan::contract::check(&access_plan).is_err(),
        "the parsed RustAccessPlan fixture must fail its local semantic contract",
    );
    assert!(!rust_access_plan::contract::well_formed(&access_plan));
}

pub fn assert_malformed_sources_fail_direct_syn_parsing() {
    let cq = quote! { struct MissingProgramBody; };
    assert!(
        syn::parse2::<cq::Module>(cq).is_err(),
        "the malformed CQ fixture must fail direct syn parsing",
    );

    let indexes = quote! { indexes {} };
    assert!(
        syn::parse2::<index_requirements::Module>(indexes).is_err(),
        "the malformed IndexRequirements fixture must fail direct syn parsing",
    );

    let access_plan = quote! { answer(x) => ([x]) :- };
    assert!(
        syn::parse2::<rust_access_plan::Plan>(access_plan).is_err(),
        "the malformed RustAccessPlan fixture must fail direct syn parsing",
    );
}

pub fn assert_pull_rejects_invalid_typed_source() {
    let source: rust_access_plan::Plan =
        syn::parse2(fixtures::invalid_sources().rust_access_plan).unwrap();
    assert!(rust_access_plan::contract::check(&source).is_err());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compile_pull(&source)));
    assert!(
        result.is_ok_and(|result| result.is_err()),
        "compile_pull must return a diagnostic without panicking in every build profile",
    );
}

pub fn assert_fixtures_well_formed() {
    for fixture in fixtures::cq_to_relational_plan_and_indexes() {
        let source: cq::Module = syn::parse2(fixture.source).unwrap();
        cq::contract::check(&source).unwrap();
        let expected: index_requirements::Module = syn::parse2(fixture.expected).unwrap();
        index_requirements::contract::check(&expected).unwrap();
    }

    for fixture in fixtures::index_requirements_to_staged_rust() {
        let source: index_requirements::Module = syn::parse2(fixture.source).unwrap();
        index_requirements::contract::check(&source).unwrap();
        let access_plan: rust_access_plan::Plan =
            syn::parse2(fixture.rust_access_plan.clone()).unwrap();
        rust_access_plan::contract::check(&access_plan).unwrap();
        let pipeline: iterator_pipeline::Pipeline =
            syn::parse2(fixture.pipeline_to_distinct_iter.clone()).unwrap();
        iterator_pipeline::contract::check(&pipeline).unwrap();
        assert_eq!(compile_pull(&access_plan).unwrap(), pipeline);
        let _: syn::Expr = compile_iterator_pipeline(&pipeline).unwrap();
        assert_pull_pipeline_lowers_to_iterator_expression(fixture.rust_access_plan);
        let runtime_assertions = fixture.runtime_assertions;
        let _: syn::Block = syn::parse2(quote!({ #runtime_assertions })).unwrap();
    }
}
