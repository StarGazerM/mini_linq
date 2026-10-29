//! Public conformance tools for MiniLinq passes and external dialects. This crate
//! keeps language/runtime-only dependencies. CPU Sequence planning and execution
//! live in the external `cq-sequence-conformance` crate. All fixtures are included
//! here; consumers need no paths into a repository checkout.
//! Add `cq-testkit` as a dev-dependency in a core language, pass, runtime, or
//! facade crate. External feature crates can use the same API.
//!
//! * `corpus` embeds 30 scenarios at CQ, valuation order (VO), relational algebra
//!   (RA), index demands (IX), Physical, and Structures stages. The IX and
//!   Structures texts are suffix fragments; their parsed accessors join them to
//!   the preceding stage. Parsing does not imply language contract validation.
//!   No policy fixtures are included. `cases` retains the 25 independently authored
//!   input/answer cases, including raw duplicate inputs. `Scenario::case` maps five
//!   alternative plans to their original authored cases.
//! * `values` models finite Set semantics over `i32`. `Rows` is a sorted set
//!   of ordered tuples; output column order matters and duplicate witnesses
//!   collapse. `Relation` provides named headings, projection, and natural join.
//!   The equality premise is Leibniz equality: equal values are interchangeable
//!   in every supported observation. Rust `i32` satisfies it. These oracles do not
//!   model custom `Eq`/`Ord`, bags, NULL, collation, effects, or evaluation counts.
//! * `cq_semantics` enumerates active-domain valuations and tests the original
//!   base relations. `semantics` independently interprets the RA DAG with nested
//!   loops. `valuation_semantics` interprets the written VO schedule and reductions.
//!   They never call pass compilation, planner, heading-analysis, or key helpers.
//!   Inputs must satisfy the language contracts, be finite, have the declared
//!   widths, and provide every declared table. The supported fragment consists of
//!   positive atoms with variables and integer literals, relational integer
//!   constraints, and total integer comparison guards. Literal positions filter
//!   original source columns and do not contribute to occurrence headings.
//!   Wildcards/expressions in atom arguments, computed bindings, Rust generators
//!   (`for`), negation, aggregation, arbitrary Rust, and extension clauses are
//!   outside this fragment. Generators use compiled Rust references,
//!   including iterator-advancement panics; no interpreter evaluates arbitrary Rust.
//!   Unsupported expressions or malformed inputs may panic. Constraint parameters,
//!   when interpreting individual constraints, belong in the external binding map.
//! * `databases` supplies deterministic binary-domain tables by seed. Its empty
//!   and full seeds are 0 and 1. Domain enumeration is exponential in input arity
//!   and is intended for small conformance inputs.
//! * `rewrites` supplies authored merge, canonical projection, smallest-branch,
//!   and ordered deduplication candidates. Callers still check correspondence
//!   and execution; the helpers use language contracts, never compiler passes.
//! * `harness` takes closures supplied by the caller. It checks compilation against
//!   an optional fixture, the source/target check, rejection of every supplied
//!   mutation, and optional execution against authored case answers. It validates
//!   only the fixtures and mutations supplied, not universal pass correctness.
//!   Checker panics propagate and are not treated as mutation rejection.
//!   Refinement helpers compare two executable closures: both must return the
//!   same answer. Either executable panicking violates the total-Rust premise;
//!   a reference panic never licenses a target result. They catch Rust unwinding
//!   panics, not aborts, and do not promise effect or evaluation-count equivalence.
//! * `fuzz` generates bounded abstract queries/data deterministically by seed
//!   using SplitMix64. Its MiniLinq and SQL printers consume that model separately;
//!   neither printer parses the other or invokes compilation. `Coverage` counts
//!   cases carrying each feature, once per feature per observation. Total scalar
//!   queries have independently printable SQL. Partial Rust terms and all finite
//!   generators require executable written-order refinement; their SQL text is
//!   an explanatory comment, even when a generator's expression is total.
//! * The optional std-only `golden` feature enables strict, byte-for-byte canonical
//!   CSV comparison and Git fixture inventory. Golden encoding requires sorted,
//!   distinct rows of the declared width; it rejects CRLF, leading zeros, and
//!   missing final newlines. Git inventory needs the Git CLI.
//! * The optional `duckdb` feature also enables `golden` and adds an in-memory
//!   integer SQL oracle. SQL must select the desired set and ordering explicitly.
//!   `evaluate_fuzz` routes opaque Rust operations and generators elsewhere and
//!   checks DISTINCT. Default and `golden`-only builds do not build or link DuckDB.
//!   The bundled native engine can make the first `duckdb` feature build slower.
//!
//! ```rust
//! use cq_testkit::{CASES, cq_semantics};
//!
//! for case in CASES {
//!     let source = case.parse_cq()?;
//!     cq_testkit::cq::contract::check(&source)?;
//!     assert_eq!(cq_semantics::evaluate_cq(&source, &case.database()), case.expected_rows());
//! }
//! # Ok::<(), syn::Error>(())
//! ```
//!
//! This tiny identity pass illustrates the closure API. Real passes supply their
//! own independent source/target checker and invalid-target mutations.
//!
//! ```rust
//! use cq_testkit::{PassCase, harness::PassFailure};
//!
//! let pass = PassCase {
//!     source: 3_i32,
//!     expected_next: Some(3),
//!     compile: Box::new(|source: &i32| Ok::<_, &'static str>(*source)),
//!     check: Box::new(|source: &i32, target: &i32| {
//!         if source == target { Ok(()) } else { Err("not identity") }
//!     }),
//!     mutations: vec![Box::new(|target: &i32| target + 1)],
//!     answers: None,
//! };
//! assert_eq!(pass.run(None)?, 3);
//! # Ok::<(), PassFailure<&'static str, &'static str>>(())
//! ```
//!
//! `pass.run(Some(case))` uses `case.database()` and `case.expected_rows()` if an
//! `answers` closure is supplied. Supplying an answer closure without a case is an
//! error. `harness::check_refinement` and `check_refinement_on` accept executable
//! reference/target closures, so the compile-and-run driver stays in the caller.
//! The authored blocks corpus and recursive CQ/VO/RA interpreters cover union, fallback, locals, nesting and sugar.
//! Fuzz prints SQL UNION and fallback UNION ALL with NOT EXISTS against original branches.
//! Opaque total Rust uses an independent compiled written-order reference.
//!
//! Core blocks denote sets: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust expressions remain opaque, pure and total, placed wherever their free
//! variables are bound; there are no Before chains. Only explicit `where`
//! predicates supply relational constraints and equalities. Only differing
//! literals on one interface column prove union arms disjoint; no data keys or
//! constraints are assumed.
//!
#![forbid(unsafe_code)]

pub mod blocks;
pub mod cases;
pub mod conformance;
pub mod corpus;
pub mod cq_semantics;
pub mod databases;
#[cfg(feature = "duckdb")]
pub mod duckdb;
pub mod fuzz;
#[cfg(feature = "golden")]
pub mod golden;
pub mod harness;
mod positional_cases;
pub mod random;
pub mod rewrites;
pub mod rustc;
pub mod semantics;
pub mod valuation_semantics;
pub mod values;

// Re-export the language crates so fixture accessor types can be named without
// callers adding dependencies merely to refer to them.
pub use cq_lang_cq as cq;
pub use cq_lang_ix as ix;
pub use cq_lang_physical as physical;
pub use cq_lang_ra as ra;
pub use cq_lang_structures as structures;
pub use cq_lang_vo as vo;

pub use cases::{CASES, Case};
pub use corpus::{SCENARIOS, Scenario};
pub use harness::PassCase;
pub use values::{Binding, Database, Relation, Rows};

/// Fallible I/O and DuckDB helpers preserve their underlying error.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Rustc configured to link dependencies of the storage source in conformance
/// fixtures. Call from a Cargo-built test executable so its dependency artifacts
/// are available beside it. This does not link compiler passes into the fixture.
/// Each backend must have the same Rust type identity as cq-testkit's linked
/// dependency. Other builds in a shared target directory are not substitutes.
pub fn runtime_rustc() -> std::process::Command {
    static ARTIFACTS: std::sync::LazyLock<(
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    )> = std::sync::LazyLock::new(|| {
        let executable = std::env::current_exe().expect("test executable path");
        let dependencies = executable
            .parent()
            .expect("Cargo dependency directory")
            .to_path_buf();
        let ahash = linked_runtime_backend(
            &dependencies,
            "ahash",
            "ahash::RandomState",
            &format!("{:?}", std::any::TypeId::of::<ahash::RandomState>()),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let hashbrown = linked_runtime_backend(
            &dependencies,
            "hashbrown",
            "hashbrown::HashTable<()>",
            &format!("{:?}", std::any::TypeId::of::<hashbrown::HashTable<()>>()),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let rayon = linked_runtime_backend(
            &dependencies,
            "rayon",
            "rayon::ThreadPool",
            &format!("{:?}", std::any::TypeId::of::<rayon::ThreadPool>()),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        (dependencies, ahash, hashbrown, rayon)
    });
    let (dependencies, ahash, hashbrown, rayon) = &*ARTIFACTS;
    let mut command =
        std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
    command
        .arg("-L")
        .arg(format!("dependency={}", dependencies.display()))
        .arg("--extern")
        .arg(format!("ahash={}", ahash.display()))
        .arg("--extern")
        .arg(format!("hashbrown={}", hashbrown.display()))
        .arg("--extern")
        .arg(format!("rayon={}", rayon.display()));
    command
}

fn linked_runtime_backend(
    dependencies: &std::path::Path,
    name: &str,
    typ: &str,
    expected: &str,
) -> std::result::Result<std::path::PathBuf, String> {
    let prefix = format!("lib{name}-");
    let mut candidates: Vec<_> = std::fs::read_dir(dependencies)
        .expect("Cargo dependency artifacts")
        .map(|entry| entry.expect("Cargo dependency entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".rlib"))
        })
        .collect();
    candidates.sort();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("cq-linked-backend-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("probe.rs");
    let executable = directory.join("probe");
    std::fs::write(
        &source,
        format!("fn main() {{ println!(\"{{:?}}\", std::any::TypeId::of::<{typ}>()); }}"),
    )
    .unwrap();
    let mut matches = Vec::new();
    let mut diagnostics = String::new();
    for candidate in &candidates {
        let output =
            std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .args(["--edition=2024", "--crate-name", "cq_backend_identity"])
                .arg(&source)
                .arg("-o")
                .arg(&executable)
                .arg("-L")
                .arg(format!("dependency={}", dependencies.display()))
                .arg("--extern")
                .arg(format!("{name}={}", candidate.display()))
                .output()
                .expect("compile the linked-backend identity probe");
        if !output.status.success() {
            diagnostics.push_str(&format!(
                "{}: {}\n",
                candidate.display(),
                String::from_utf8_lossy(&output.stderr)
            ));
            continue;
        }
        let output = std::process::Command::new(&executable)
            .output()
            .expect("run the linked-backend identity probe");
        let actual = String::from_utf8_lossy(&output.stdout);
        if output.status.success() && actual.trim() == expected {
            matches.push(candidate.clone());
        } else {
            diagnostics.push_str(&format!(
                "{}: identity {:?}, status {}, stderr {}\n",
                candidate.display(),
                actual.trim(),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
    if matches.len() != 1 {
        return Err(format!(
            "expected exactly one {name} backend with linked identity {expected}; \
             matches: {matches:?}; candidates: {candidates:?}\n{diagnostics}"
        ));
    }
    Ok(matches.pop().unwrap())
}

#[cfg(test)]
mod runtime_backend_tests {
    use super::linked_runtime_backend;

    #[test]
    fn backend_selection_distinguishes_identical_apis_by_linked_identity() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("cq-backend-identities-{stamp}"));
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("backend.rs");
        std::fs::write(&source, "pub struct Marker;").unwrap();
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        // Both real rlibs have the same crate name and public API. Only their
        // crate identities differ; the decoy is deliberately built last.
        for identity in ["linked", "decoy"] {
            let output = std::process::Command::new(&rustc)
                .args([
                    "--edition=2024",
                    "--crate-name",
                    "backend",
                    "--crate-type",
                    "rlib",
                ])
                .arg("-C")
                .arg(format!("metadata={identity}"))
                .arg("-C")
                .arg(format!("extra-filename=-{identity}"))
                .arg(&source)
                .arg("--out-dir")
                .arg(&directory)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let linked = directory.join("libbackend-linked.rlib");
        let reference = directory.join("reference.rs");
        std::fs::write(
            &reference,
            "fn main() { println!(\"{:?}\", std::any::TypeId::of::<backend::Marker>()); }",
        )
        .unwrap();
        let executable = directory.join("reference");
        let output = std::process::Command::new(&rustc)
            .arg("--edition=2024")
            .arg(&reference)
            .arg("--extern")
            .arg(format!("backend={}", linked.display()))
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = std::process::Command::new(&executable).output().unwrap();
        assert!(output.status.success());
        let expected = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            linked_runtime_backend(&directory, "backend", "backend::Marker", expected.trim(),)
                .unwrap(),
            linked
        );
        let missing = linked_runtime_backend(
            &directory,
            "backend",
            "backend::Marker",
            "not the linked identity",
        )
        .unwrap_err();
        assert!(missing.contains("matches: []") && missing.contains("libbackend-decoy"));
        std::fs::copy(&linked, directory.join("libbackend-alias.rlib")).unwrap();
        let ambiguous =
            linked_runtime_backend(&directory, "backend", "backend::Marker", expected.trim())
                .unwrap_err();
        assert!(ambiguous.contains("libbackend-linked") && ambiguous.contains("libbackend-alias"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
