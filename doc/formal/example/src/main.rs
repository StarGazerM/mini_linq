//! Reproduce the paper's worked example using the actual typed pass APIs.
//! All generated artifacts go beneath doc/formal/build/example by default.
//! Also executes both public macro routes and compiles/runs both final Rust files.
use cq_ir::{cq, iterator_pipeline, rust_access_plan};
use quote::quote;
use syn::visit_mut::VisitMut;

mini_linq::mini_linq! {
    pub struct LazyOverview;
    relation R(a:i32,b:i32);
    relation S(a:i32,b:i32);
    relation answer(a:i32,b:i32);
    answer(x,*y+1) :- R(x,k), let next:i32 = *k+1, S(next,y), if y > 0;
}
mini_linq::planned_query! {
    pub struct MaterializedOverview;
    relation R(a:i32,b:i32);
    relation S(a:i32,b:i32);
    relation answer(a:i32,b:i32);
    answer(x,*y+1) :- R(x,k), let next:i32 = *k+1, S(next,y), if y > 0;
}
struct ExposePipelines;
impl VisitMut for ExposePipelines {
    fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
        if mac.path.segments.last().unwrap().ident == "pull" {
            let access: rust_access_plan::Plan = syn::parse2(mac.tokens.clone()).unwrap();
            rust_access_plan::contract::check(&access).unwrap();
            let target = cq_compiler::compile_pull(&access);
            iterator_pipeline::contract::check(&target).unwrap();
            mac.path = syn::parse_quote!(::mini_linq::iterator_pipeline);
            mac.tokens = quote!(#target);
        }
    }
}
struct LowerPipelines;
impl VisitMut for LowerPipelines {
    fn visit_stmt_mut(&mut self, stmt: &mut syn::Stmt) {
        if let syn::Stmt::Macro(m) = stmt {
            if m.mac.path.segments.last().unwrap().ident == "iterator_pipeline" {
                let pipeline: iterator_pipeline::Pipeline =
                    syn::parse2(m.mac.tokens.clone()).unwrap();
                *stmt = syn::Stmt::Expr(
                    cq_compiler::compile_iterator_pipeline(&pipeline),
                    m.semi_token,
                );
                return;
            }
        }
        syn::visit_mut::visit_stmt_mut(self, stmt);
    }
    fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
        if let syn::Expr::Macro(m) = expr {
            if m.mac.path.segments.last().unwrap().ident == "iterator_pipeline" {
                let pipeline: iterator_pipeline::Pipeline =
                    syn::parse2(m.mac.tokens.clone()).unwrap();
                *expr = cq_compiler::compile_iterator_pipeline(&pipeline);
                return;
            }
        }
        syn::visit_mut::visit_expr_mut(self, expr);
    }
}
fn write(directory: &std::path::Path, name: &str, file: &syn::File) {
    std::fs::write(
        directory.join(format!("{name}.rs")),
        prettyplease::unparse(file),
    )
    .unwrap();
}
fn boundaries(directory: &std::path::Path, prefix: &str, source: &cq::Module, planned: bool) {
    let ra = if planned {
        cq_compiler::compile_cq_planned(source)
    } else {
        cq_compiler::compile_cq(source)
    }
    .unwrap();
    let ix = if planned {
        cq_compiler::compile_result_indexes(&ra)
    } else {
        cq_compiler::compile_relational_plan(&ra)
    }
    .unwrap();
    let staged = if planned {
        cq_compiler::compile_materialized(&ix)
    } else {
        cq_compiler::compile_index_requirements(&ix)
    }
    .unwrap();
    let (cq_macro, ra_macro, index_macro): (syn::Path, syn::Path, syn::Path) = if planned {
        (
            syn::parse_quote!(::mini_linq::planned_query),
            syn::parse_quote!(::mini_linq::planned_relational_plan),
            syn::parse_quote!(::mini_linq::materialized),
        )
    } else {
        (
            syn::parse_quote!(::mini_linq::mini_linq),
            syn::parse_quote!(::mini_linq::relational_plan),
            syn::parse_quote!(::mini_linq::index_requirements),
        )
    };
    write(
        directory,
        &format!("{prefix}-01-cq"),
        &syn::parse2(quote!(#cq_macro! { #source })).unwrap(),
    );
    write(
        directory,
        &format!("{prefix}-02-relational"),
        &syn::parse2(quote!(#ra_macro! { #ra })).unwrap(),
    );
    write(
        directory,
        &format!("{prefix}-03-indexes"),
        &syn::parse2(quote!(#index_macro! { #ix })).unwrap(),
    );
    write(directory, &format!("{prefix}-04-access"), &staged);
    let mut pipelines = staged;
    ExposePipelines.visit_file_mut(&mut pipelines);
    write(directory, &format!("{prefix}-05-pipeline"), &pipelines);
    let mut rust = pipelines;
    LowerPipelines.visit_file_mut(&mut rust);
    let main: syn::Item = syn::parse_quote! {
        fn main() {
            let r = [(1,6),(2,6),(3,8)];
            let s = [(7,10),(7,-2),(9,4)];
            let expected = vec![(1,11),(2,11),(3,5)];
            let actual = Overview::run(r,s);
            assert_eq!(actual, expected);
        }
    };
    rust.items.push(main);
    write(directory, &format!("{prefix}-06-rust"), &rust);
    let source_file = directory.join(format!("{prefix}-06-rust.rs"));
    let executable = directory.join(format!("{prefix}-run"));
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let build = std::process::Command::new(rustc)
        .args(["--edition=2024", "-Awarnings"])
        .arg(&source_file)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(
        std::process::Command::new(executable)
            .status()
            .unwrap()
            .success()
    );
}
fn main() {
    let directory = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../build/example")
        });
    std::fs::create_dir_all(&directory).unwrap();
    let source: cq::Module = syn::parse_quote! {
        pub struct Overview;
        relation R(a:i32,b:i32);
        relation S(a:i32,b:i32);
        relation answer(a:i32,b:i32);
        answer(x,*y+1) :- R(x,k), let next:i32 = *k+1, S(next,y), if y > 0;
    };
    let (ra, report) = cq_compiler::explain_cq_plan(
        &source,
        cq_compiler::PlanningOptions {
            strategy: cq_compiler::JoinPolicy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    std::fs::write(directory.join("report.txt"), format!("{report:#?}")).unwrap();
    cq_compiler::check_cq_plan(&source, &ra, &report).unwrap();
    boundaries(&directory, "lazy", &source, false);
    boundaries(&directory, "materialized", &source, true);
    let r = [(1, 6), (2, 6), (3, 8)];
    let s = [(7, 10), (7, -2), (9, 4)];
    let expected = vec![(1, 11), (2, 11), (3, 5)];
    let lazy = LazyOverview::run(r, s);
    assert_eq!(lazy, expected);
    let materialized = MaterializedOverview::run(r, s);
    assert_eq!(materialized, expected);
    let (answer, stats) = MaterializedOverview::run_with_stats(r, s);
    assert_eq!(answer, expected);
    assert_eq!(
        stats.results,
        [
            ("r0", 2, 3),
            ("r1", 3, 3),
            ("r2", 2, 3),
            ("r3", 4, 5),
            ("r4", 4, 3),
            ("r5", 4, 3)
        ]
    );
    assert_eq!(stats.index_rows, 3);
    assert_eq!(stats.join_pairs, 5);
    assert_eq!(stats.semijoin_probes, 0);
    let evidence = "Both public macro routes and both fully lowered Rust programs return exactly [(1,11),(2,11),(3,5)].\nNamed-result (arity,count): r0=(2,3), r1=(3,3), r2=(2,3), r3=(4,5), r4=(4,3), r5=(4,3).\nIndex rows=3; join pairs=5; semijoin probes=0.\n";
    std::fs::write(directory.join("validation.txt"), evidence).unwrap();
    print!("{evidence}");
}
