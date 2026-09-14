//! Public typed boundaries reject invalid parsed IR without profile-dependent panics.

use cq_ir::{cq, index_requirements, iterator_pipeline, relational_plan, rust_access_plan};

fn rejects<T>(lower: impl FnOnce() -> syn::Result<T>, diagnostic: &str) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(lower))
        .unwrap_or_else(|_| panic!("a public compiler boundary panicked on invalid IR"));
    let error = result.err().expect("invalid IR must return a diagnostic");
    assert!(error.to_string().contains(diagnostic), "{error}");
}

fn relational_source() -> relational_plan::Module {
    syn::parse_quote! {
        struct P;
        relation R(c0: i32);
        answer(x) :- R(x);
        relational {
            r0 = rename R { c0 -> x };
            r1 = project r0 keep { x };
            output r1 as answer(x);
        }
    }
}

#[test]
fn cq_checks_declarations_before_lowering() {
    let source: cq::Module = syn::parse_quote! {
        struct P;
        relation R(c0: i32);
        answer(x) :- Missing(x);
    };
    rejects(|| cq_compiler::compile_cq(&source), "not declared");
}

#[test]
fn relational_checks_operand_scope_before_index_selection() {
    let mut source = relational_source();
    let relational_plan::Operator::Project(project) = &mut source.plan.definitions[1].operator
    else {
        unreachable!();
    };
    project.input = syn::parse_quote!(missing);
    rejects(
        || cq_compiler::compile_relational_plan(&source),
        "must name a result defined earlier",
    );
}

#[test]
fn indexes_check_duplicate_and_out_of_range_requirements_before_realization() {
    let source = relational_source();
    for (requirements, diagnostic) in [
        (quote::quote!(R[0]; R[0];), "duplicated"),
        (quote::quote!(R[8];), "exceeds relation arity"),
    ] {
        let indexed: index_requirements::Module = syn::parse_quote! {
            #source
            indexes { #requirements }
        };
        rejects(
            || cq_compiler::compile_index_requirements(&indexed),
            diagnostic,
        );
    }
}

#[test]
fn access_checks_fresh_bindings_before_pipeline_construction() {
    let source: rust_access_plan::Plan = syn::parse_quote! {
        answer(x) => ((x,)) :- R(x) => if (true);
    };
    rejects(|| cq_compiler::compile_pull(&source), "fresh variables");
}

#[test]
fn all_pipeline_endpoints_check_empty_and_incomplete_pipelines() {
    let empty: iterator_pipeline::Pipeline = syn::parse_quote! { return missing; };
    let incomplete: iterator_pipeline::Pipeline = syn::parse_quote! {
        iter0 = unit yield ();
        iter1 = project iter0 as () yield (42);
        return iter1;
    };
    for (source, diagnostic) in [
        (&empty, "must define a source and projection"),
        (&incomplete, "must end with `distinct`"),
    ] {
        rejects(
            || cq_compiler::compile_iterator_pipeline(source),
            diagnostic,
        );
        rejects(|| cq_compiler::compile_collect_pipeline(source), diagnostic);
        rejects(|| cq_compiler::compile_stream_pipeline(source), diagnostic);
    }
}
