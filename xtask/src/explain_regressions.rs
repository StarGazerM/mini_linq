use std::ffi::OsString;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use cq_access_lowering::to_pipeline::compile as compile_pull;
use cq_ir::{iterator_pipeline, rust_access_plan};
use cq_pipeline_lowering::distinct_iter::compile as compile_iterator_pipeline;
use quote::quote;
use syn::visit_mut::{self, VisitMut};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    path: PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        loop {
            let nonce = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mini-linq-explain-test-{}-{nonce}",
                std::process::id(),
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!(
                    "failed to create explain test directory {}: {error}",
                    path.display()
                ),
            }
        }
    }

    fn case_directory(&self, case: &QueryCase) -> PathBuf {
        self.path.join("target/mini-linq-explain").join(case.name)
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap_or_else(|error| {
            panic!(
                "failed to remove explain test directory {}: {error}",
                self.path.display()
            )
        });
    }
}

fn triangle() -> &'static QueryCase {
    cq_workloads::case("triangle").unwrap()
}

#[test]
fn expression_artifacts_preserve_literals_arguments_and_result_schema() {
    let source: cq::Module = syn::parse_quote! {
        struct Expressions;
        relation R(value: i32);
        relation answer(value: String);
        answer(format!(" :: ( & * ! > {x}")) :-
            R(x), let label = " :: ( & * ! >".to_owned(),
            if *x > 0 && !label.is_empty();
    };
    let rendered = format_cq_program(&source.program);
    let restored: cq::Module = syn::parse_str(&rendered).unwrap();
    assert_eq!(restored, source);
    let extended: relational_plan::Operator = syn::parse_quote! {
        extend r0 with {label: String} value ((" :: ( & * ! >".to_owned(),))
    };
    let restored: relational_plan::Operator =
        syn::parse_str(&format_relational_operator(&extended)).unwrap();
    assert_eq!(restored, extended);
}

#[test]
fn formatted_query_and_access_bodies_roundtrip_with_semicolons() {
    for body in [
        "R(x)",
        "R(x), R(x)",
        r#"R("a , b :: c ( d )"), if accept(x, " ; ")"#,
        "R(x), let y = (|a, b| a + b)(x, 1), !R(f(y))",
        "R(x), agg y = (factory(config))(value) in R(f(x, value))",
    ] {
        let program: cq::Program = syn::parse_str(&format!(
            "struct P; relation R(a:i32); answer(x) :- {body};"
        ))
        .unwrap();
        assert_eq!(
            syn::parse_str::<cq::Program>(&format_cq_program(&program)).unwrap(),
            program
        );
    }
    let plan: rust_access_plan::Plan = syn::parse_str(
        "answer(x) => ((x,)) :- R(x) => for (x,) in (rows.iter()), R(x) => if (x.is_positive());",
    )
    .unwrap();
    assert_eq!(
        syn::parse_str::<rust_access_plan::Plan>(&format_rust_access_plan(&plan)).unwrap(),
        plan
    );
}

#[test]
fn options_require_one_known_shape_and_default_to_final_rust() {
    let options = Options::parse(vec!["triangle".to_owned()]).unwrap();
    assert_eq!(options.case, "triangle");
    assert_eq!(options.through, Stage::FinalRust);

    assert!(Options::parse(Vec::new()).is_err());
    assert!(Options::parse(vec!["triangle".into(), "extra".into()]).is_err());
    assert!(Options::parse(vec!["triangle".into(), "--bad".into()]).is_err());
    assert!(Options::parse(vec!["triangle".into(), "--through".into()]).is_err());
    assert!(
        Options::parse(vec![
            "triangle".into(),
            "--through".into(),
            "not-a-stage".into(),
        ])
        .is_err()
    );
}

#[test]
fn unknown_catalog_case_is_rejected_before_creating_artifacts() {
    let root = TestRoot::new();
    let result = run(vec!["not-a-case".to_owned()], &root.path);

    assert!(result.is_err());
    assert!(!root.path.join("target/mini-linq-explain").exists());
}

#[test]
fn cq_stage_stops_without_calling_or_writing_later_stages() {
    let root = TestRoot::new();
    explain_case(triangle(), Stage::Cq, &root.path).unwrap();
    let directory = root.case_directory(triangle());

    assert!(directory.join("README.md").is_file());
    assert!(directory.join(Stage::Cq.file_name()).is_file());
    for stage in [
        Stage::RelationalPlan,
        Stage::IndexRequirements,
        Stage::RustAccessPlan,
        Stage::IteratorPipeline,
        Stage::FinalRust,
    ] {
        assert!(!directory.join(stage.file_name()).exists());
    }
}

#[test]
fn a_shallower_run_removes_stale_later_artifacts() {
    let root = TestRoot::new();
    let stale = Artifacts::new(&root.path, triangle(), Stage::FinalRust).unwrap();
    for stage in Stage::ALL {
        stale.write_stage(stage, "stale\n").unwrap();
    }
    explain_case(triangle(), Stage::Cq, &root.path).unwrap();
    let directory = root.case_directory(triangle());

    assert!(directory.join(Stage::Cq.file_name()).is_file());
    for stage in Stage::ALL.into_iter().skip(1) {
        assert!(!directory.join(stage.file_name()).exists());
    }
}

// These end-to-end checks become runnable after all three basic passes.
#[test]
fn every_catalog_case_reaches_complete_final_rust() {
    let root = TestRoot::new();

    for case in cq_workloads::catalog::all() {
        explain_case(case, Stage::FinalRust, &root.path)
            .unwrap_or_else(|error| panic!("`{}` did not reach final Rust: {error}", case.name));
        let final_source =
            fs::read_to_string(root.case_directory(case).join(Stage::FinalRust.file_name()))
                .unwrap();
        syn::parse_file(&final_source).unwrap_or_else(|error| {
            panic!("`{}` final artifact is not Rust syntax: {error}", case.name)
        });
    }
}

#[test]
fn triangle_final_artifact_compiles_as_standalone_rust() {
    let root = TestRoot::new();
    explain_case(triangle(), Stage::FinalRust, &root.path).unwrap();
    let source = root
        .case_directory(triangle())
        .join(Stage::FinalRust.file_name());
    for stage in Stage::ALL {
        let path = root.case_directory(triangle()).join(stage.file_name());
        let text = fs::read_to_string(&path).unwrap();
        syn::parse_file(&text).unwrap_or_else(|error| {
            panic!(
                "{} is not a complete Rust macro/file artifact: {error}",
                path.display()
            )
        });
    }
    let library = root.path.join("triangle.rlib");
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let output = Command::new(rustc)
        .arg("--edition=2024")
        .arg("--crate-name")
        .arg("mini_linq_explained_triangle")
        .arg("--crate-type=lib")
        .arg(&source)
        .arg("-o")
        .arg(&library)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "final artifact did not compile\nstdout:\n{}\nstderr:\n{}\nsource: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        source.display(),
    );
}

#[test]
fn custom_ir_and_nested_plan_bodies_are_readable_multiline_code() {
    let root = TestRoot::new();
    explain_case(triangle(), Stage::IteratorPipeline, &root.path).unwrap();
    let directory = root.case_directory(triangle());

    let cq = fs::read_to_string(directory.join(Stage::Cq.file_name())).unwrap();
    assert!(cq.contains("mini_linq! {\n    pub struct"));
    assert!(cq.contains("\n    relation R(c0: i32, c1: i32);"));
    assert!(cq.contains("\n        R(x, y),"));

    let relational = fs::read_to_string(directory.join(Stage::RelationalPlan.file_name())).unwrap();
    assert!(relational.contains("\n    relational {\n        r0"));
    assert!(relational.contains("\n        output r5 as triangle(x, y, z);"));
    assert!(relational.contains("natural_join r0 with r1"));
    assert!(relational.contains("natural_join r2 with r3"));

    let access = fs::read_to_string(directory.join(Stage::RustAccessPlan.file_name())).unwrap();
    assert!(access.contains("::mini_linq::access_to_distinct_iter! {\n"));
    assert!(access.contains("\n                R(x, y) => for"));
    assert!(access.contains("\n                T(z, x) => if"));

    let pipeline = fs::read_to_string(directory.join(Stage::IteratorPipeline.file_name())).unwrap();
    assert!(pipeline.contains("::mini_linq::pipeline_to_distinct_iter! {\n"));
    assert!(pipeline.contains("\n            iter0 = scan"));
    assert!(pipeline.contains("if (\n"));
    assert!(pipeline.contains("yield (\n"));
    assert!(pipeline.contains("\n            return iter"));
    assert!(pipeline.lines().all(|line| line.len() <= 100));
}

#[test]
fn structural_lowerings_replace_only_their_one_typed_macro() {
    let mut file: syn::File = syn::parse2(quote! {
        fn query() {
            unrelated!();
            ::mini_linq::access_to_distinct_iter! {
                answer(x) => (x) :- R(x) => for (x,) in ([1].iter());
            }
        }
    })
    .unwrap();
    let item_count = file.items.len();

    lower_pull_once(&mut file).unwrap();
    assert_eq!(file.items.len(), item_count);
    assert_eq!(count_macros(&file, NestedMacro::Pull), 0);
    assert_eq!(count_macros(&file, NestedMacro::IteratorPipeline), 1);
    assert!(file.to_token_stream().to_string().contains("unrelated !"));

    lower_iterator_pipeline_once(&mut file).unwrap();
    assert_eq!(file.items.len(), item_count);
    assert_eq!(count_macros(&file, NestedMacro::IteratorPipeline), 0);
    assert!(file.to_token_stream().to_string().contains("unrelated !"));
    assert!(file.to_token_stream().to_string().contains("once_with"));
}

#[test]
fn duplicate_target_macros_are_rejected_before_rewriting() {
    let mut file: syn::File = syn::parse2(quote! {
        fn query() {
            ::mini_linq::access_to_distinct_iter! {
                answer(x) => (x) :- R(x) => for x in ([1].iter());
            };
            ::mini_linq::access_to_distinct_iter! {
                answer(x) => (x) :- R(x) => for x in ([1].iter());
            }
        }
    })
    .unwrap();
    let before = file.clone();

    assert!(lower_pull_once(&mut file).is_err());
    assert_eq!(file, before);
}

#[test]
fn error_report_preserves_earlier_artifacts_and_names_the_edge() {
    let root = TestRoot::new();
    let artifacts = Artifacts::new(&root.path, triangle(), Stage::FinalRust).unwrap();
    artifacts.write_stage(Stage::Cq, "cq\n").unwrap();
    artifacts
        .write_stage(Stage::RelationalPlan, "relational\n")
        .unwrap();

    let result: Result<()> = artifacts.attempt("RelationalPlan -> IndexRequirements", || {
        Err::<(), _>("deliberate test failure")
    });
    assert!(result.is_err());

    let directory = root.case_directory(triangle());
    assert_eq!(
        fs::read_to_string(directory.join(Stage::Cq.file_name())).unwrap(),
        "cq\n"
    );
    assert_eq!(
        fs::read_to_string(directory.join(Stage::RelationalPlan.file_name())).unwrap(),
        "relational\n"
    );
    let report = fs::read_to_string(directory.join(ERROR_FILE)).unwrap();
    assert!(report.contains("RelationalPlan -> IndexRequirements"));
    assert!(report.contains("deliberate test failure"));
    assert!(
        !directory
            .join(Stage::IndexRequirements.file_name())
            .exists()
    );
}

#[test]
fn panic_report_preserves_earlier_artifact_and_names_the_edge() {
    let root = TestRoot::new();
    let artifacts = Artifacts::new(&root.path, triangle(), Stage::FinalRust).unwrap();
    artifacts.write_stage(Stage::Cq, "cq\n").unwrap();

    let result: Result<()> = artifacts.attempt(
        "CQ -> RelationalPlan",
        || -> std::result::Result<(), &'static str> { panic!("student pass is not implemented") },
    );
    assert!(result.is_err());

    let directory = root.case_directory(triangle());
    assert!(directory.join(Stage::Cq.file_name()).is_file());
    let report = fs::read_to_string(directory.join(ERROR_FILE)).unwrap();
    assert!(report.contains("CQ -> RelationalPlan"));
    assert!(report.contains("compiler panicked: student pass is not implemented"));
    assert!(!directory.join(Stage::RelationalPlan.file_name()).exists());
}

fn count_macros(file: &syn::File, target: NestedMacro) -> usize {
    let mut copy = file.clone();
    let mut counter = MacroCounter { target, count: 0 };
    counter.visit_file_mut(&mut copy);
    counter.count
}
