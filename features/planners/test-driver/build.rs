//! Generate compiled refinement consumers from the public testkit's model.
//! All 200 seeds use opaque Rust; every planner invocation is compiled.
use cq_planners_plan::{Planner, cq};
use cq_testkit::fuzz::Case;
use quote::ToTokens;
use std::fmt::Write;

fn executable(case: &Case, name: &str, invocation: &str) -> String {
    let mut code = format!("pub(super) fn {name}(data: &[Vec<Vec<i32>>]) -> cq_testkit::Rows {{\n");
    for (index, relation) in case.query.schema.iter().enumerate() {
        let fields = (0..relation.arity)
            .map(|column| format!("row[{column}],"))
            .collect::<String>();
        writeln!(
            code,
            "let input_{index}: Vec<_> = data[{index}].iter().map(|row| ({fields})).collect();\nlet {} = input_{index}.as_slice();",
            relation.name
        )
        .unwrap();
    }
    for (name, value) in case.parameters() {
        writeln!(code, "let {name}: i32 = {value};").unwrap();
    }
    let fields = (0..case.output_arity())
        .map(|column| format!("crate::integer(row.{column})"))
        .collect::<Vec<_>>()
        .join(",");
    writeln!(code, "{invocation}.map(|row| vec![{fields}]).collect()\n}}").unwrap();
    code
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let directory = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let mut consumers = String::new();
    let mut registry = String::from("const EXECUTABLES: &[ExecutableCase] = &[\n");
    for ordinal in 0..200 {
        let seed = ordinal * 2;
        let case = cq_testkit::fuzz::generate(seed);
        assert!(case.requires_rust_oracle());
        // The independent model printer preserves parentheses, uniform owned
        // i32 expressions and tautological/contradictory random constraints.
        // Keep those authored terms intact; these allowances affect syntax
        // advice only, never compilation, correspondence or refinement checks.
        writeln!(
            consumers,
            "#[allow(unused_parens, clippy::double_parens, clippy::cmp_owned, clippy::eq_op, clippy::double_comparisons, clippy::impossible_comparisons)] mod seed_{seed} {{"
        )
        .unwrap();

        // Opaque blocks retain the original short-circuit Rust guards. Exact
        // written elaboration disables alias solving and relational hoisting.
        let source: cq::Program = syn::parse_str(&case.written_cq()).unwrap();
        let normal = cq::normal::normalize(&source).unwrap();
        let written = cq_order::compile_written(&normal).unwrap();
        let reference = cq_elaborate::compile_written(&written).unwrap();
        let invocation = format!(
            "mini_linq::passes::index_demands! {{ pipeline {{ mini_linq::passes::lower, mini_linq::passes::select, mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull }}; {} }}",
            reference.to_token_stream()
        );
        consumers.push_str(&executable(&case, "reference", &invocation));

        for (index, planner) in Planner::ALL.iter().enumerate() {
            let invocation = format!(
                "mini_linq::compile! {{ pipeline {{ plan = cq_planners::order({}); }} {} }}",
                planner.to_token_stream(),
                case.cq()
            );
            consumers.push_str(&executable(&case, &format!("target_{index}"), &invocation));
        }
        consumers.push_str("}\n");
        let targets = (0..Planner::ALL.len())
            .map(|index| format!("seed_{seed}::target_{index}"))
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            registry,
            "ExecutableCase {{ seed: {seed}, reference: seed_{seed}::reference, targets: [{targets}] }},"
        )
        .unwrap();
    }
    registry.push_str("];\n");
    consumers.push_str(&registry);
    std::fs::write(directory.join("refinement.rs"), consumers).unwrap();
}
