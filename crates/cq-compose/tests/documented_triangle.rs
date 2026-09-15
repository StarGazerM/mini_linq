const DSL: &str = include_str!("../../../doc/DSL.md");

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use cq_pipeline::iterator::compile as compile_iterator_pipeline;
use cq_pull::compile as compile_pull;
use syn::visit_mut::{self, VisitMut};

static TEMPORARY_NUMBER: AtomicU64 = AtomicU64::new(0);

fn first_rust_fence_after(heading: &str) -> &str {
    let (_, section) = DSL
        .split_once(heading)
        .unwrap_or_else(|| panic!("missing documented triangle stage `{heading}`"));
    let (_, after_fence) = section
        .split_once("```rust\n")
        .unwrap_or_else(|| panic!("stage `{heading}` has no Rust fence"));
    let (source, _) = after_fence
        .split_once("\n```")
        .unwrap_or_else(|| panic!("stage `{heading}` has an unterminated Rust fence"));
    source
}

#[test]
fn every_complete_triangle_artifact_is_valid_rust_syntax_without_placeholders() {
    for heading in [
        "### 1. Entire CQ",
        "### 2. Entire RelationalPlan program",
        "### 3. Entire IndexRequirements program",
        "### 4. Entire staged Rust file with one RustAccessPlan",
        "### 5. Entire staged Rust file with one IteratorPipeline",
        "### 6. Entire final Rust file after iterator-pipeline lowering",
    ] {
        let source = first_rust_fence_after(heading);
        assert!(!source.contains("..."), "{heading} contains an ellipsis");
        assert!(!source.contains("/*"), "{heading} contains a placeholder");
        syn::parse_file(source)
            .unwrap_or_else(|error| panic!("{heading} is not valid Rust syntax: {error}"));
    }
}

#[test]
fn documented_domain_irs_are_well_formed_and_final_rust_executes() {
    let cq_file = documented_file("### 1. Entire CQ");
    let cq: cq_ir::cq::Module = syn::parse2(only_item_macro_tokens(&cq_file)).unwrap();
    cq_ir::cq::contract::check(&cq).unwrap();

    let relational_file = documented_file("### 2. Entire RelationalPlan program");
    let relational: cq_ir::relational_plan::Module =
        syn::parse2(only_item_macro_tokens(&relational_file)).unwrap();
    cq_ir::relational_plan::contract::check(&relational).unwrap();

    let indexes_file = documented_file("### 3. Entire IndexRequirements program");
    let indexes: cq_ir::index_requirements::Module =
        syn::parse2(only_item_macro_tokens(&indexes_file)).unwrap();
    cq_ir::index_requirements::contract::check(&indexes).unwrap();

    compile_and_run_documented_final_rust(first_rust_fence_after(
        "### 6. Entire final Rust file after iterator-pipeline lowering",
    ));
}

#[test]
fn documented_pull_changes_only_the_marked_expression() {
    let mut pulled = documented_file("### 4. Entire staged Rust file with one RustAccessPlan");
    let iterator_plan = documented_file("### 5. Entire staged Rust file with one IteratorPipeline");

    let mut pull_expander = PullExpander::default();
    pull_expander.visit_file_mut(&mut pulled);
    assert_eq!(pull_expander.expansions, 1);
    assert_eq!(
        normalize_formatting(&pulled),
        normalize_formatting(&iterator_plan)
    );
}

#[test]
fn documented_iterator_pipeline_changes_only_the_marked_expression() {
    let mut iterator_plan =
        documented_file("### 5. Entire staged Rust file with one IteratorPipeline");
    let final_rust =
        documented_file("### 6. Entire final Rust file after iterator-pipeline lowering");

    let mut iterator_plan_expander = IteratorPipelineExpander::default();
    iterator_plan_expander.visit_file_mut(&mut iterator_plan);
    assert_eq!(iterator_plan_expander.expansions, 1);
    assert_eq!(
        normalize_formatting(&iterator_plan),
        normalize_formatting(&final_rust),
    );
}

/// The documented stages are pretty-printed before being embedded. Reparse
/// both sides after the same pretty-print pass so tuple-comma trivia cannot
/// masquerade as a lowering difference.
fn normalize_formatting(file: &syn::File) -> syn::File {
    syn::parse_file(&prettyplease::unparse(file)).unwrap()
}

fn documented_file(heading: &str) -> syn::File {
    syn::parse_file(first_rust_fence_after(heading)).unwrap()
}

fn only_item_macro_tokens(file: &syn::File) -> proc_macro2::TokenStream {
    let [syn::Item::Macro(item)] = file.items.as_slice() else {
        panic!("documented domain IR must be exactly one macro invocation")
    };
    item.mac.tokens.clone()
}

fn compile_and_run_documented_final_rust(source: &str) {
    let temporary = TemporaryRustProgram::new();
    let source_path = temporary.directory.join("documented_triangle.rs");
    let executable_path = temporary.directory.join("documented_triangle");
    let source = format!(
        "{source}\nfn main() {{ let storage = TriangleProgram::build([(1, 2)], [(2, 3)], [(3, 1)]); assert_eq!(storage.query().next(), Some((1, 2, 3))); assert_eq!(storage.materialize(), vec![(1, 2, 3)]); assert_eq!(TriangleProgram::run([(1, 2)], [(2, 3)], [(3, 1)]), vec![(1, 2, 3)]); }}\n"
    );
    fs::write(&source_path, &source).unwrap();

    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let compilation = Command::new(&rustc)
        .arg("--edition=2024")
        .arg(&source_path)
        .arg("-o")
        .arg(&executable_path)
        .output()
        .unwrap();
    assert!(
        compilation.status.success(),
        "documented final Rust did not compile\nstdout:\n{}\nstderr:\n{}\nsource:\n{source}",
        String::from_utf8_lossy(&compilation.stdout),
        String::from_utf8_lossy(&compilation.stderr),
    );

    let execution = Command::new(&executable_path).output().unwrap();
    assert!(
        execution.status.success(),
        "documented final Rust compiled but failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&execution.stdout),
        String::from_utf8_lossy(&execution.stderr),
    );
}

struct TemporaryRustProgram {
    directory: PathBuf,
}

impl TemporaryRustProgram {
    fn new() -> Self {
        let number = TEMPORARY_NUMBER.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "mini-linq-documented-triangle-{}-{number}",
            std::process::id(),
        ));
        fs::create_dir(&directory).unwrap();
        Self { directory }
    }
}

impl Drop for TemporaryRustProgram {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn is_macro(path: &syn::Path, name: &str) -> bool {
    path.leading_colon.is_some()
        && path.segments.len() == 2
        && path.segments[0].ident == "mini_linq"
        && path.segments[1].ident == name
}

#[derive(Default)]
struct PullExpander {
    expansions: usize,
}

impl VisitMut for PullExpander {
    fn visit_stmt_mut(&mut self, statement: &mut syn::Stmt) {
        let syn::Stmt::Macro(source) = statement else {
            visit_mut::visit_stmt_mut(self, statement);
            return;
        };
        if !is_macro(&source.mac.path, "pull") {
            visit_mut::visit_stmt_mut(self, statement);
            return;
        }

        self.expansions += 1;
        let plan: cq_ir::rust_access_plan::Plan = syn::parse2(source.mac.tokens.clone()).unwrap();
        cq_ir::rust_access_plan::contract::check(&plan).unwrap();
        let lowered = compile_pull(&plan).unwrap();
        cq_ir::iterator_pipeline::contract::check(&lowered).unwrap();
        *statement = syn::parse2(quote::quote! {
            ::mini_linq::iterator_pipeline! { #lowered }
        })
        .unwrap();
    }
}

#[derive(Default)]
struct IteratorPipelineExpander {
    expansions: usize,
}

impl VisitMut for IteratorPipelineExpander {
    fn visit_stmt_mut(&mut self, statement: &mut syn::Stmt) {
        let syn::Stmt::Macro(source) = statement else {
            visit_mut::visit_stmt_mut(self, statement);
            return;
        };
        if !is_macro(&source.mac.path, "iterator_pipeline") {
            visit_mut::visit_stmt_mut(self, statement);
            return;
        }

        self.expansions += 1;
        let plan: cq_ir::iterator_pipeline::Pipeline =
            syn::parse2(source.mac.tokens.clone()).unwrap();
        cq_ir::iterator_pipeline::contract::check(&plan).unwrap();
        *statement = syn::Stmt::Expr(compile_iterator_pipeline(&plan).unwrap(), None);
    }
}
