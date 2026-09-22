use cq_ir::relational_plan as ra;

pub(crate) fn compile_ra(source: &ra::Module) -> syn::Result<syn::File> {
    let indexed = cq_index_policy::result_demands::compile(source)?;
    assert_eq!(indexed.relational_plan, *source);
    compile_indexed(&indexed)
}

pub(crate) fn compile_indexed(
    source: &cq_ir::index_requirements::Module,
) -> syn::Result<syn::File> {
    use syn::visit_mut::VisitMut;
    let mut rust = cq_relation_lowering::materialized_hash::compile(
        source,
        &syn::parse_quote!(::mini_linq::access_to_set),
    )?;
    struct Lower {
        count: usize,
    }
    impl VisitMut for Lower {
        fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
            if let syn::Expr::Macro(m) = expr {
                if m.mac.path.segments.last().unwrap().ident == "access_to_set" {
                    let access =
                        syn::parse2::<cq_ir::rust_access_plan::Plan>(m.mac.tokens.clone()).unwrap();
                    cq_ir::rust_access_plan::contract::check(&access).unwrap();
                    let pipeline = cq_access_lowering::to_pipeline::compile(&access).unwrap();
                    cq_ir::iterator_pipeline::contract::check(&pipeline).unwrap();
                    *expr = cq_pipeline_lowering::collect_set::compile(&pipeline).unwrap();
                    self.count += 1;
                    return;
                }
            }
            syn::visit_mut::visit_expr_mut(self, expr);
        }
    }
    let mut lower = Lower { count: 0 };
    lower.visit_file_mut(&mut rust);
    assert_eq!(lower.count, source.relational_plan.plan.definitions.len());
    Ok(rust)
}
