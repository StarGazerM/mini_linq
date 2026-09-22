//! Executable composition through IX, Physical, and provider-owned Access.
//! Requires general physical selection. Do not supply a table of query-local
//! relation, index, or operator names to make this acceptance test pass.
#![recursion_limit = "512"]

#[cfg(test)]
#[path = "support/trace.rs"]
mod trace;

use std::collections::{BTreeMap, HashMap, HashSet};

// Deliberately neither Clone nor Copy: every output field must stay borrowed.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Value(i32);

mod wiring {
    pub(crate) use ::mini_linq::{access_to_set_loops as access, realize_physical};
    #[allow(unused_imports)]
    pub(crate) use cq_example_providers::{
        hash_distinct as distinct_state, hash_set_sink as set_sink,
    };
}
use wiring as mini_linq;

#[allow(non_snake_case)]
fn run<'a>(
    L: &'a [Value],
    R: &'a HashMap<Value, Vec<Value>>,
    S: &'a BTreeMap<Value, Vec<Value>>,
) -> HashSet<(&'a Value, &'a Value, &'a Value)> {
    ::mini_linq::index_demands! {
        struct Heterogeneous;
        relation L(a: Value);
        relation R(a: Value, b: Value);
        relation S(b: Value, c: Value);
        answer(a, b, c) :- L(a), R(a, b), S(b, c);
        relational {
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            output j1 as answer(a, b, c);
        }
    }
}

fn oracle<'a>(
    left: &'a [Value],
    middle: &'a HashMap<Value, Vec<Value>>,
    right: &'a BTreeMap<Value, Vec<Value>>,
) -> HashSet<(&'a Value, &'a Value, &'a Value)> {
    let mut result = HashSet::new();
    // Enumerate the input relations independently; do not repeat index probes.
    for a in left {
        for (ra, values) in middle {
            for b in values {
                for (sb, values) in right {
                    for c in values {
                        if a == ra && b == sb {
                            result.insert((a, b, c));
                        }
                    }
                }
            }
        }
    }
    result
}

#[test]
fn mixed_providers_preserve_join_results_and_borrow_non_clone_fields() {
    let left = [Value(1), Value(1), Value(2), Value(9)];
    let middle = HashMap::from([
        (Value(1), vec![Value(10), Value(10), Value(11)]),
        (Value(2), vec![Value(20)]),
        (Value(3), vec![Value(30)]),
    ]);
    let right = BTreeMap::from([
        (Value(10), vec![Value(100), Value(101), Value(101)]),
        (Value(11), vec![]),
        (Value(20), vec![Value(200)]),
        (Value(99), vec![Value(999)]),
    ]);
    let actual = run(&left, &middle, &right);
    assert_eq!(actual, oracle(&left, &middle, &right));
    assert_eq!(actual.len(), 3);
    for (a, b, c) in actual {
        assert!(left.iter().any(|item| std::ptr::eq(item, a)));
        assert!(middle.values().flatten().any(|item| std::ptr::eq(item, b)));
        assert!(right.values().flatten().any(|item| std::ptr::eq(item, c)));
    }
}

#[test]
fn any_empty_input_or_missing_join_value_produces_empty_set() {
    let left = [Value(1)];
    let middle = HashMap::from([(Value(1), vec![Value(10)])]);
    let right = BTreeMap::from([(Value(10), vec![Value(100)])]);
    assert!(run(&[], &middle, &right).is_empty());
    assert!(run(&left, &HashMap::new(), &right).is_empty());
    assert!(run(&left, &middle, &BTreeMap::new()).is_empty());
    assert!(run(&[Value(2)], &middle, &right).is_empty());
    assert!(
        run(
            &left,
            &middle,
            &BTreeMap::from([(Value(11), vec![Value(100)])])
        )
        .is_empty()
    );
}

#[test]
fn incompatible_recorded_providers_and_index_columns_are_compile_errors() {
    use std::{fs, process::Command};

    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent().unwrap().parent().unwrap();
    let directory = std::env::temp_dir().join(format!(
        "mini-linq-physical-rejection-{}",
        std::process::id()
    ));
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(directory.join("Cargo.toml"), format!(r#"
[package]
name = "physical-rejection-fixture"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
mini-linq = {{ path = {:?}, default-features = false, features = ["index-demands", "physical-selection", "physical-realization", "access-to-set-loops"] }}
cq-example-providers = {{ path = {:?} }}
"#, manifest, workspace.join("crates/cq-example-providers"))).unwrap();

    // Compile the same real query. A passing control prevents an unrelated
    // fixture/build failure from making a rejection test pass accidentally.
    let source = include_str!("physical_realization.rs").to_owned() + "\nfn main() {}\n";
    let cases = [
        ("valid", source.clone(), None),
        (
            "wrong relation representation",
            source.replace(
                "R = ::cq_example_providers::hash_relation;",
                "R = ::cq_example_providers::sorted_relation;",
            ),
            Some("BTreeMap"),
        ),
        (
            "wrong index representation",
            source.replace(
                "i0 = ::cq_example_providers::hash_relation;",
                "i0 = ::cq_example_providers::sorted_relation;",
            ),
            Some("BTreeMap"),
        ),
        (
            "unsupported indexed column",
            source.replace("l = rename L {a -> a};", "l = rename L {a -> b};"),
            Some("selected map index can reuse only its physical first-column index"),
        ),
    ];
    for (name, source, expected_error) in cases {
        fs::write(directory.join("src/main.rs"), source).unwrap();
        let output = Command::new(env!("CARGO"))
            .args(["check", "--offline", "--quiet"])
            .current_dir(&directory)
            .env(
                "CARGO_TARGET_DIR",
                workspace.join("target/physical-rejection-fixtures"),
            )
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        match expected_error {
            None => assert!(output.status.success(), "valid control failed: {stderr}"),
            Some(message) => {
                assert!(!output.status.success(), "{name} unexpectedly compiled");
                assert!(
                    stderr.contains(message),
                    "{name} rejected for the wrong reason: {stderr}"
                );
            }
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

/// Run with CQ_REALIZATION_TRACE pointing at a fresh trace_macros.py artifact.
/// Keeping this explicit prevents an old checked-in capture from proving that
/// the current compiler still emits the authored boundaries.
#[test]
#[ignore = "requires a fresh real proc-macro trace in CQ_REALIZATION_TRACE"]
fn captured_boundaries_equal_authored_ix_physical_and_access() {
    trace::verify(
        None,
        include_str!("fixtures/physical-realization/expected.ix"),
        include_str!("fixtures/physical-realization/expected.selection"),
        include_str!("fixtures/physical-realization/expected.rs"),
    );
}
