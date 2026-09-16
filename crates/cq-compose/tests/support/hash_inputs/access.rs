use super::*;
pub(super) fn assert_unique_pull_plan(file: &syn::File, fixture: &PhysicalFixture) {
    let mut collector = PullMacroCollector::default();
    collector.visit_file(file);
    assert_eq!(
        collector.macros.len(),
        1,
        "{} must contain exactly one macro named `pull`",
        fixture.program_name,
    );

    let pull = collector.macros[0];
    assert!(
        is_absolute_mini_linq_pull(&pull.path),
        "{} must invoke the exact path `::mini_linq::pull!`",
        fixture.program_name,
    );

    let actual: rust_access_plan::Plan = syn::parse2(pull.tokens.clone()).unwrap();
    rust_access_plan::contract::check(&actual).unwrap();
    assert_query_plan_borrows_stored_tuples(&actual, fixture.program_name);
    let expected: rust_access_plan::Plan = syn::parse2(fixture.rust_access_plan.clone()).unwrap();
    assert_eq!(
        actual, expected,
        "{} must emit the exact occurrence-annotated Rust access plan",
        fixture.program_name,
    );

    let actual_pipeline = compile_pull(&actual).unwrap();
    iterator_pipeline::contract::check(&actual_pipeline).unwrap();
    let expected_pipeline: iterator_pipeline::Pipeline =
        syn::parse2(fixture.iterator_pipeline.clone()).unwrap();
    iterator_pipeline::contract::check(&expected_pipeline).unwrap();
    assert_eq!(
        actual_pipeline, expected_pipeline,
        "{} must expose every binary iterator boundary and intermediate binding",
        fixture.program_name,
    );

    let _: syn::Expr = compile_iterator_pipeline(&actual_pipeline).unwrap();
}

pub(super) fn assert_query_plan_borrows_stored_tuples(
    plan: &rust_access_plan::Plan,
    program_name: &str,
) {
    let mut copies = QueryTupleCopyAdapters::default();
    copies.visit_expr(&plan.head.output);
    for clause in &plan.body {
        match &clause.access {
            rust_access_plan::RustAccess::For { source, .. } => copies.visit_expr(source),
            rust_access_plan::RustAccess::If { condition, .. } => copies.visit_expr(condition),
        }
    }

    assert_eq!(
        copies.count, 0,
        "{program_name}'s query plan must borrow scans and index payload tuples; `.copied()`/`.cloned()` adapters belong only to eager physical construction",
    );
}

#[derive(Default)]
struct QueryTupleCopyAdapters {
    count: usize,
}

impl<'ast> Visit<'ast> for QueryTupleCopyAdapters {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "copied" || call.method == "cloned" {
            self.count += 1;
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}
