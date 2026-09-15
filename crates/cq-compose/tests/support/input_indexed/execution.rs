use super::*;
pub(super) fn compile_and_run_staged_rust(mut file: syn::File, fixture: &PhysicalFixture) {
    let mut expander = IteratorPipelineExpander::default();
    expander.visit_file_mut(&mut file);
    if let Some(error) = expander.error {
        panic!(
            "{} emitted a Rust access plan that the supplied pull lowering rejected: {error}",
            fixture.program_name,
        );
    }
    assert_eq!(
        expander.pull_expansions, 1,
        "{} must lower one RustAccessPlan",
        fixture.program_name
    );
    assert_eq!(
        expander.pipeline_expansions, 1,
        "{} must lower the resulting iterator pipeline exactly once",
        fixture.program_name,
    );

    // Generated collection paths must not rely on the caller's prelude
    // bindings. Declared column types themselves intentionally remain exactly
    // the Rust types written by the caller.
    let shadows: syn::File = syn::parse2(quote! {
        #[allow(dead_code)]
        struct Vec;
    })
    .unwrap();
    file.items.splice(0..0, shadows.items);

    let runtime_assertions = &fixture.runtime_assertions;
    let main: syn::ItemFn = syn::parse2(quote! {
        fn main() {
            #runtime_assertions
        }
    })
    .unwrap();
    file.items.push(syn::Item::Fn(main));

    super::super::rust::run_rust(file.to_token_stream());
}

#[derive(Default)]
struct IteratorPipelineExpander {
    pull_expansions: usize,
    pipeline_expansions: usize,
    error: Option<syn::Error>,
}

impl VisitMut for IteratorPipelineExpander {
    fn visit_stmt_mut(&mut self, statement: &mut syn::Stmt) {
        let macro_invocation = match statement {
            syn::Stmt::Macro(statement) if statement.semi_token.is_none() => {
                Some(statement.mac.clone())
            }
            _ => None,
        };
        let Some(macro_invocation) = macro_invocation else {
            visit_mut::visit_stmt_mut(self, statement);
            return;
        };

        if is_absolute_mini_linq_pull(&macro_invocation.path) {
            self.pull_expansions += 1;
            let lowered =
                syn::parse2::<rust_access_plan::Plan>(macro_invocation.tokens).and_then(|plan| {
                    let target = compile_pull(&plan)?;
                    iterator_pipeline::contract::check(&target)?;
                    syn::parse2(quote! {
                        ::mini_linq::iterator_pipeline! { #target }
                    })
                });
            match lowered {
                Ok(lowered) => {
                    *statement = lowered;
                    self.visit_stmt_mut(statement);
                }
                Err(error) => self.error = Some(error),
            }
        } else if is_absolute_mini_linq_iterator_pipeline(&macro_invocation.path) {
            self.pipeline_expansions += 1;
            let lowered = syn::parse2::<iterator_pipeline::Pipeline>(macro_invocation.tokens)
                .and_then(|plan| compile_iterator_pipeline(&plan));
            match lowered {
                Ok(lowered) => *statement = syn::Stmt::Expr(lowered, None),
                Err(error) => self.error = Some(error),
            }
        } else {
            visit_mut::visit_stmt_mut(self, statement);
        }
    }
}
