//! Compile expressions using exactly the logical inputs reported by the helper.
//! This checks the supplied syntax/analysis utility without implementing a pass.
use std::{collections::BTreeSet, process::Command};

use cq_lang_cq as cq;
use quote::{format_ident, quote};

#[test]
fn reported_captures_compile_and_preserve_expression_results() {
    let environment: BTreeSet<_> = ["x", "y", "z"].map(String::from).into();
    let cases: &[(&str, &[&str], i32)] = &[
        ("x + y * 2", &["x", "y"], 19),
        ("{ let x = y; x + z }", &["y", "z"], 18),
        ("{ let x = x + 1; x + y }", &["x", "y"], 13),
        ("(|x| x + y)(z)", &["y", "z"], 18),
        (
            "match Some(x) { Some(y) if y > z => y + 1, _ => z }",
            &["x", "z"],
            11,
        ),
        (
            "{ let Some(x) = Some(y) else { return z }; x + 1 }",
            &["y", "z"],
            8,
        ),
        (
            "if let Some(x) = Some(y) { x + 1 } else { x }",
            &["x", "y"],
            8,
        ),
        (
            "{ let mut total = x; for x in [y, z] { total += x; } total }",
            &["x", "y", "z"],
            23,
        ),
        ("generic::<i32>(x)", &["x"], 5),
        ("(x, y).1", &["x", "y"], 7),
    ];
    let mut definitions = Vec::new();
    let mut assertions = Vec::new();
    for (index, (source, expected_names, expected_value)) in cases.iter().enumerate() {
        let atom: cq::Atom = syn::parse_str(&format!("R({source})")).unwrap();
        let expression = &atom.args[0];
        let free = cq::rust::free_variables(expression);
        assert!(!free.opaque, "{source}");
        let dependencies = free.dependencies(&environment);
        assert_eq!(
            dependencies.iter().map(String::as_str).collect::<Vec<_>>(),
            *expected_names,
            "{source}"
        );
        let parameters: Vec<_> = dependencies
            .iter()
            .map(|name| format_ident!("{name}"))
            .collect();
        let values = dependencies.iter().map(|name| match name.as_str() {
            "x" => 5_i32,
            "y" => 7,
            "z" => 11,
            _ => unreachable!(),
        });
        let function = format_ident!("case_{index}");
        definitions.push(quote!(fn #function(#(#parameters: i32),*) -> i32 { #expression }));
        assertions.push(quote!(assert_eq!(#function(#(#values),*), #expected_value);));
    }
    let program = quote! {
        fn generic<T>(value: T) -> T { value }
        #(#definitions)*
        fn main() { #(#assertions)* }
    };
    let directory = std::env::temp_dir().join(format!("cq-free-variables-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("main.rs");
    let binary = directory.join("captures");
    std::fs::write(&source, program.to_string()).unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compiled = Command::new(rustc)
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    let executed = compiled
        .status
        .success()
        .then(|| Command::new(&binary).output().unwrap());
    std::fs::remove_dir_all(&directory).unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = executed.unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}
