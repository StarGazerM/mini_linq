use cq_testkit::{CASES, SCENARIOS};
use quote::ToTokens;
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let mut code = String::new();
    for case in CASES {
        let cq = case.parse_cq().unwrap();
        cq_testkit::cq::contract::check(&cq).unwrap();
        for host in ["slice", "hash", "ordered"] {
            emit(&mut code, "cq", case.name, host, &cq, case.cq, None);
            if host == "slice" {
                emit(
                    &mut code,
                    "cq",
                    case.name,
                    "slice_positional",
                    &cq,
                    case.cq,
                    None,
                );
            }
        }
    }
    for scenario in SCENARIOS {
        let cq = scenario.parse_cq().unwrap();
        let structures = scenario.parse_structures().unwrap();
        cq_testkit::structures::contract::check(&structures).unwrap();
        for host in ["slice", "hash", "ordered"] {
            emit(
                &mut code,
                "structures",
                scenario.name,
                host,
                &cq,
                scenario.cq,
                Some(structures.to_token_stream().to_string()),
            );
        }
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("corpus.rs"), code).unwrap();
}

fn emit(
    code: &mut String,
    stage: &str,
    name: &str,
    host: &str,
    cq: &cq_testkit::cq::Program,
    cq_text: &str,
    structures: Option<String>,
) {
    let mut declarations = String::new();
    for input in cq.inputs() {
        let relation = input.name.to_string();
        let width = input.columns.len();
        let fields = (0..width).map(|i| format!("row[{i}],")).collect::<String>();
        let types = "i32,".repeat(width);
        declarations.push_str(&format!("let raw_{relation}: Vec<({types})> = raw[{relation:?}].iter().map(|row| ({fields})).collect();\n"));
        if host.starts_with("slice") {
            declarations.push_str(&format!("let {relation} = raw_{relation}.as_slice();\n"));
        } else {
            let map = if host == "hash" {
                "HashMap"
            } else {
                "BTreeMap"
            };
            let payload = (1..width).map(|i| format!("row.{i},")).collect::<String>();
            let payload_types = "i32,".repeat(width - 1);
            declarations.push_str(&format!("let mut {relation}: std::collections::{map}<(i32,), Vec<({payload_types})>> = std::collections::{map}::new();\nfor row in &raw_{relation} {{ {relation}.entry((row.0,)).or_default().push(({payload})); }}\n"));
        }
    }
    let width = cq.query.head.args.len();
    let answer = (0..width)
        .map(|i| format!(" *row.{i},"))
        .collect::<String>();
    let advice = if host == "slice_positional" {
        let hosts = cq
            .inputs()
            .map(|input| input.name.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!("select = mini_linq::passes::select(positional({hosts}));")
    } else {
        String::new()
    };
    let (push, pull) = if let Some(source) = structures {
        (
            format!("own_push::push! {{ pipeline {{}}; {source} }}"),
            format!("mini_linq::passes::print_pull! {{ pipeline {{}}; {source} }}"),
        )
    } else {
        (
            format!(
                "mini_linq::compile! {{ pipeline {{ {advice} print = own_push::push; }} {cq_text} }}"
            ),
            format!("mini_linq::compile! {{ pipeline {{ {advice} }} {cq_text} }}"),
        )
    };
    let lookup = if stage == "cq" {
        format!("*cq_testkit::CASES.iter().find(|case| case.name == {name:?}).unwrap()")
    } else {
        format!(
            "*cq_testkit::SCENARIOS.iter().find(|scenario| scenario.name == {name:?}).unwrap().case().unwrap()"
        )
    };
    code.push_str(&format!(
        r#"
#[test]
#[allow(non_snake_case)]
fn {stage}_{name}_{host}() {{
    let case = {lookup};
    check(case, |raw| {{
        {declarations}
        let push: Vec<Vec<i32>> = ({push}).map(|row| vec![{answer}]).collect();
        let pull: Vec<Vec<i32>> = ({pull}).map(|row| vec![{answer}]).collect();
        (push, pull)
    }});
}}
"#
    ));
}
