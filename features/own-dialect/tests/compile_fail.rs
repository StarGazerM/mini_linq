use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MERGE: &str = r#"
physical {
    relation R(a: i32, b: i32);
    relation S(a: i32, c: i32);
    l = pipe rename R { a -> a, b -> b };
    r = pipe rename S { a -> a, c -> c };
    j = let canon merge[a] l,r;
    return j as answer(a,b,c);
}"#;

const SMALLEST: &str = r#"
physical {
    relation R(a: i32, b: i32);
    relation S(a: i32, c: i32);
    relation T(b: i32, c: i32);
    l = pipe rename R { a -> a, b -> b };
    r = pipe rename S { a -> a, c -> c };
    t = pipe rename T { b -> b, c -> c };
    j = pipe probe smallest[a] l,r;
    k = pipe semi eager[b,c] j,t;
    out = let canon project k keep { a,b,c };
    return out as answer(a,b,c);
}"#;

struct Compiler {
    directory: PathBuf,
    dependencies: PathBuf,
}

impl Compiler {
    fn new() -> Self {
        let dependencies = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("own-push-ui-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        Self {
            directory,
            dependencies,
        }
    }

    // Compile against this test's Cargo-built public dependencies. This also
    // works in the external gate, where every core artifact is packaged.
    fn compile(&self, name: &str, expression: &str) -> Output {
        let input = self.directory.join(format!("{name}.rs"));
        std::fs::write(
            &input,
            format!(
                r#"
#![allow(non_snake_case, unused_variables)]
fn main() {{
    let R = std::collections::BTreeMap::from([((1_i32,), vec![(2_i32,)])]);
    let S = std::collections::BTreeMap::from([((1_i32,), vec![(3_i32,)])]);
    let T = std::collections::BTreeMap::from([((2_i32,), vec![(3_i32,)])]);
    let _ = {expression};
}}"#
            ),
        )
        .unwrap();
        let mut rustc = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
        rustc
            .arg("--edition=2024")
            .arg("--emit=metadata")
            .arg("--crate-name")
            .arg(name)
            .arg("--out-dir")
            .arg(&self.directory)
            .arg("-L")
            .arg(format!("dependency={}", self.dependencies.display()));
        for (crate_name, extension) in [
            ("own_push", std::env::consts::DLL_EXTENSION),
            ("mini_linq", "rlib"),
            ("cq_storage", "rlib"),
            ("cq_load", "rlib"),
        ] {
            let artifact = artifact(&self.dependencies, crate_name, extension);
            rustc
                .arg("--extern")
                .arg(format!("{crate_name}={}", artifact.display()));
        }
        rustc.arg(input).output().unwrap()
    }
}

impl Drop for Compiler {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn artifact(directory: &Path, crate_name: &str, extension: &str) -> PathBuf {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!("lib{crate_name}-"))
                && path.extension().is_some_and(|ext| ext == extension)
        })
        .max_by_key(|path| std::fs::metadata(path).unwrap().modified().unwrap())
        .unwrap_or_else(|| panic!("missing Cargo-built artifact for {crate_name}"))
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "positive control failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn failure(output: Output) {
    assert!(!output.status.success(), "unsupported input compiled");
}

#[test]
fn unsupported_written_choices_are_refused_after_core_contracts() {
    let compiler = Compiler::new();
    for (name, physical) in [("merge", MERGE), ("smallest", SMALLEST)] {
        let scenario = cq_testkit::Scenario {
            physical,
            ..cq_testkit::SCENARIOS[0]
        };
        cq_testkit::physical::contract::check(&scenario.parse_physical().unwrap()).unwrap();
        let expression = |printer: &str| {
            format!("mini_linq::passes::select! {{ pipeline {{ {printer} }}; {physical} }}")
        };
        success(compiler.compile(
            &format!("{name}_pull"),
            &expression("mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull"),
        ));
        failure(compiler.compile(name, &expression("own_push::push")));
    }
}

#[test]
fn consumer_form_rejects_missing_or_mismatched_consumers() {
    let compiler = Compiler::new();
    let query = "struct Q; relation R(a:i32,b:i32); answer(a) :- R(a,b);";
    success(compiler.compile(
        "consumer_control",
        &format!("mini_linq::compile! {{ pipeline {{ print_input = structures; print = own_push::push_into(consumer {{ |_: (&i32,)| {{}} }};); }} {query} }}"),
    ));
    failure(
        compiler.compile(
            "missing_consumer",
            &format!(
                "mini_linq::compile! {{ pipeline {{ print_input = structures; print = own_push::push_into; }} {query} }}"
            ),
        ),
    );
    failure(
        compiler.compile(
            "mismatched_consumer",
            &format!("mini_linq::compile! {{ pipeline {{ print_input = structures; print = own_push::push_into(consumer {{ |_: (String,)| {{}} }};); }} {query} }}"),
        ),
    );
}

#[test]
fn printer_rejects_arguments_and_a_remaining_queue() {
    let compiler = Compiler::new();
    let query = "struct Q; relation R(a: i32,b: i32); answer(a) :- R(a,b);";
    success(compiler.compile(
        "control",
        &format!("mini_linq::compile! {{ pipeline {{ print_input = structures; print = own_push::push; }} {query} }}"),
    ));
    failure(
        compiler.compile(
            "arguments",
            &format!(
                "mini_linq::compile! {{ pipeline {{ print_input = structures; print = own_push::push(unused); }} {query} }}"
            ),
        ),
    );
    let scan = cq_testkit::SCENARIOS
        .iter()
        .find(|scenario| scenario.name == "scan")
        .unwrap();
    let source = format!("{} {}", scan.physical, scan.structures);
    failure(
        compiler.compile(
            "remaining",
            &format!(
                "own_push::push! {{ pipeline {{ mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull }}; {source} }}"
            ),
        ),
    );
}

#[test]
fn positional_advice_requires_a_positional_host() {
    let compiler = Compiler::new();
    for (name, printer) in [
        (
            "pull_positions",
            "mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull",
        ),
        ("push_positions", "own_push::push"),
    ] {
        failure(
            compiler.compile(
                name,
                &format!(
                    r#"mini_linq::passes::select! {{
            arguments {{ positional(R) }};
            pipeline {{ {printer} }};
            physical {{
                relation R(a:i32,b:i32);
                r = pipe rename R {{a -> x,b -> y}};
                s = pipe point select r if (y = 2);
                out = let canon project s keep {{x}}; return out as answer(x);
            }}
        }}"#
                ),
            ),
        );
    }
}

#[test]
fn positions_representation_is_explicitly_refused() {
    let compiler = Compiler::new();
    failure(
        compiler.compile(
            "positions_representation",
            r#"own_push::push! {
                pipeline {};
                physical {
                    relation R(a:i32,b:i32);
                    r = pipe rename R {a -> x,b -> y};
                    out = let canon project r keep {x,y};
                    return out as answer(x,y);
                }
                structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
                    r host scan = @host_scan as ::core::iter::Map<_,_> [];
                    out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
                    r repr = positions { R lineage {x -> a,y -> b} } owned [];
                }
            }"#,
        ),
    );
}

#[test]
fn lookup_witness_and_expansion_are_refused() {
    let compiler = Compiler::new();
    for (name, decision) in [
        (
            "lookup_witness",
            "s share = lookup r [k] first s force p; s repr = tuples witness { r [k] at s as ::cq_storage::RunCursor<'_, _> };",
        ),
        (
            "lookup_expand",
            "p stage = batch 4 expand;",
        ),
    ] {
        let source = format!(
            r#"own_push::push! {{
            pipeline {{}};
            physical {{
                relation R(a:i32,b:i32); relation S(a:i32,c:i32);
                l = pipe rename R {{a -> k,b -> b}};
                r = let rename S {{a -> k,c -> v}};
                s = pipe semi eager [k] l,r;
                p = pipe probe first [k] s,r;
                out = let canon project p keep {{k,v}};
                return out as answer(k,v);
            }}
            structures {{
                providers {{
                    host_scan = ::core::iter::Map<_, _> {{ reads [], build typed }};
                    sorted_run = ::cq_storage::SortedRun<_, _> {{
                        reads [exact, range, membership], supports [retain, expand, keys],
                        build typed, join_filter, run_filter, shared,
                        statistics_build [rows, max_group], statistics [
                            rows: rows occurrences exact run_finalize stat_rows host required build single,
                            keys: keys distinct_keys exact run_finalize stat_keys host required build single,
                            max_group: max_group max_bucket_occurrences exact run_finalize stat_max_group host required build single,
                            runs: runs runs exact run_finalize stat_runs host required build single
                        ]
                    }};
                    sort = ::cq_storage::dedup::Sort {{ reads [distinct], build inferred, shared }};
                }}
                l host scan = @host_scan as ::core::iter::Map<_,_> [];
                r host scan = @host_scan as ::core::iter::Map<_,_> [];
                r = @sorted_run as ::cq_storage::SortedRun<_,_> [k] supports [expand];
                out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
                {decision}
            }}
        }}"#
        );
        failure(compiler.compile(name, &source));
    }
}
