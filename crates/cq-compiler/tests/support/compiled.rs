use cq_ir::relational_plan as ra;

pub(crate) fn compile_ra(source: &ra::Module) -> syn::Result<syn::File> {
    let indexed = cq_compiler::compile_result_indexes(source)?;
    assert_eq!(indexed.relational_plan, *source);
    compile_indexed(&indexed)
}

pub(crate) fn compile_indexed(
    source: &cq_ir::index_requirements::Module,
) -> syn::Result<syn::File> {
    use syn::visit_mut::VisitMut;
    let mut rust = cq_compiler::compile_materialized(source)?;
    struct Lower {
        count: usize,
    }
    impl VisitMut for Lower {
        fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
            if let syn::Expr::Macro(m) = expr {
                if m.mac.path.segments.last().unwrap().ident == "pull" {
                    let access =
                        syn::parse2::<cq_ir::rust_access_plan::Plan>(m.mac.tokens.clone()).unwrap();
                    cq_ir::rust_access_plan::contract::check(&access).unwrap();
                    let pipeline = cq_compiler::compile_pull(&access);
                    cq_ir::iterator_pipeline::contract::check(&pipeline).unwrap();
                    *expr = cq_compiler::compile_iterator_pipeline(&pipeline);
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

pub(crate) fn run_rust(tokens: proc_macro2::TokenStream) {
    check_rust(tokens, None);
}

pub(crate) fn check_rust(tokens: proc_macro2::TokenStream, expected_error: Option<&str>) {
    use std::{
        fs,
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mini-linq-stage1-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.rs");
    fs::write(&file, prettyplease::unparse(&syn::parse2(tokens).unwrap())).unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let built = Command::new(rustc)
        .arg("--edition=2024")
        .arg("-Awarnings")
        .arg(&file)
        .arg("-o")
        .arg(dir.join("run"))
        .output()
        .unwrap();
    if let Some(message) = expected_error {
        assert!(
            !built.status.success() && String::from_utf8_lossy(&built.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        fs::remove_dir_all(dir).unwrap();
        return;
    }
    assert!(
        built.status.success(),
        "{}\nsource: {}",
        String::from_utf8_lossy(&built.stderr),
        file.display()
    );
    let run = Command::new(dir.join("run")).output().unwrap();
    assert!(
        run.status.success(),
        "{}\nsource: {}",
        String::from_utf8_lossy(&run.stderr),
        file.display()
    );
    fs::remove_dir_all(dir).unwrap();
}
