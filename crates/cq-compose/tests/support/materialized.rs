//! Physical tuple-shape observations independent of the materialized emitter.
use crate::oracle;
use cq_ir::relational_plan as ra;

pub(super) fn assert_physical_widths(
    ra: &ra::Module,
    rust: &syn::File,
    database: &oracle::Database,
) {
    use syn::visit::Visit;
    struct Storage(std::collections::BTreeMap<usize, usize>);
    impl<'ast> Visit<'ast> for Storage {
        fn visit_local(&mut self, local: &'ast syn::Local) {
            if let syn::Pat::Type(p) = &local.pat {
                if let syn::Pat::Ident(name) = p.pat.as_ref() {
                    if let Some(number) = name.ident.to_string().strip_prefix("relation") {
                        let syn::Type::Path(path) = p.ty.as_ref() else {
                            panic!("storage must have explicit type")
                        };
                        let segment = path.path.segments.last().unwrap();
                        assert_eq!(segment.ident, "HashSet");
                        let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
                            panic!()
                        };
                        let Some(syn::GenericArgument::Type(syn::Type::Tuple(tuple))) =
                            args.args.first()
                        else {
                            panic!("expected tuple storage")
                        };
                        assert!(
                            self.0
                                .insert(number.parse().unwrap(), tuple.elems.len())
                                .is_none()
                        );
                    }
                }
            }
            syn::visit::visit_local(self, local);
        }
    }
    let mut storage = Storage(Default::default());
    storage.visit_file(rust);
    assert_eq!(storage.0.len(), ra.plan.definitions.len());
    let interpreted = oracle::evaluate_plan(ra.program.inputs(), &ra.plan, database);
    for (i, d) in ra.plan.definitions.iter().enumerate() {
        assert_eq!(
            storage.0[&i],
            interpreted.definitions[&cq_ir::symbol_name(&d.result)]
                .heading
                .len()
        );
    }
}
