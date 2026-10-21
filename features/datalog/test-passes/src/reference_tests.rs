//! Execute native reference expressions from the actual targets returned by PassCase.
use cq_datalog_testkit::PassCase;
use proc_macro2::TokenStream;
use quote::quote;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TemporaryDirectory(PathBuf);

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Compile opaque Rust natively, without the integer interpreter or production printer.
pub(crate) fn check(source: TokenStream, target: TokenStream, prelude: TokenStream) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let executable = std::env::current_exe().expect("test executable path");
    let dependencies = executable.parent().expect("Cargo dependency directory");
    let library = std::fs::read_dir(dependencies)
        .expect("Cargo dependency artifacts")
        .map(|entry| entry.expect("Cargo dependency entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("libcq_datalog_testkit-") && name.ends_with(".rlib")
                })
        })
        .max_by_key(|path| {
            (
                path.metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok(),
                path.clone(),
            )
        })
        .expect("cq_datalog_testkit rlib beside the test executable");
    let directory = TemporaryDirectory(std::env::temp_dir().join(format!(
        "cq-datalog-reference-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    )));
    std::fs::create_dir(&directory.0).expect("create reference fixture directory");
    let program = quote! {
        fn main() {
            #prelude
            ::cq_datalog_testkit::check_refinement(|| #source, || #target).unwrap();
        }
    };
    let source_path = directory.0.join("main.rs");
    let binary = directory.0.join("run");
    std::fs::write(&source_path, program.to_string()).expect("write reference fixture");
    let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--edition=2024")
        .arg(&source_path)
        .arg("-L")
        .arg(format!("dependency={}", dependencies.display()))
        .arg("--extern")
        .arg(format!("cq_datalog_testkit={}", library.display()))
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("compile native reference fixture");
    success("compile native reference fixture", &compiled);
    let executed = Command::new(&binary)
        .output()
        .expect("execute native reference fixture");
    success("execute native reference fixture", &executed);
}

fn success(operation: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{operation}: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn default_pipeline_pass_cases_refine_their_actual_targets() {
    let source: cq_lang_datalog_source::Program = syn::parse_quote! {
        struct Reachability;
        relation E(x:i32,y:i32) = (rows);
        relation Blocked(x:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        relation Visible(x:i32,y:i32);
        output Reach, Visible;
        Reach(*x,*y) :- E(x,y);
        Reach(*x,*z) :- Reach(x,y), E(y,z);
        Visible(*x,*y) :- Reach(x,y), !Blocked(y);
    };
    let prelude = quote! {
        let rows: [(i32, i32); 6] = [(0, 1), (1, 2), (2, 3), (3, 1), (0, 1), (9, 10)];
        let blocked: [(i32,); 1] = [(2,)];
    };
    let source_reference = super::reference::emit(&source).unwrap();
    let strata = PassCase {
        source,
        expected_next: None,
        compile: Box::new(cq_datalog_stratify::compile),
        check: Box::new(cq_datalog_stratify::check),
        mutations: vec![],
        answers: None,
    }
    .run(None)
    .unwrap();
    let strata_reference = super::reference::emit_strata(&strata).unwrap();
    check(source_reference, strata_reference.clone(), prelude.clone());

    let incremental = PassCase {
        source: strata,
        expected_next: None,
        compile: Box::new(cq_datalog_incremental::compile),
        check: Box::new(cq_datalog_incremental::check),
        mutations: vec![],
        answers: None,
    }
    .run(None)
    .unwrap();
    let incremental_reference = super::reference_incremental::emit(&incremental).unwrap();
    check(
        strata_reference,
        incremental_reference.clone(),
        prelude.clone(),
    );

    let storage = PassCase {
        source: incremental,
        expected_next: None,
        compile: Box::new(cq_datalog_storage::compile),
        check: Box::new(cq_datalog_storage::check),
        mutations: vec![],
        answers: None,
    }
    .run(None)
    .unwrap();
    // Storage's alpha map is its embedded Incremental program, not its layouts.
    let storage_reference = super::reference_incremental::emit(&storage.incremental).unwrap();
    check(
        incremental_reference,
        storage_reference.clone(),
        prelude.clone(),
    );

    let round = PassCase {
        source: storage,
        expected_next: None,
        compile: Box::new(cq_datalog_fork::compile),
        check: Box::new(cq_datalog_fork::check),
        mutations: vec![],
        answers: None,
    }
    .run(None)
    .unwrap();
    check(
        storage_reference,
        super::reference_round::emit(&round).unwrap(),
        prelude,
    );
}
