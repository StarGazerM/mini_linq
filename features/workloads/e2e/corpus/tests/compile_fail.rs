//! Handpicked negative programs: one visible fixture per rejection obligation.
//! The runner pins construct/trait diagnostics rather than generated source spans.
#[test]
fn rejected_configurations_and_invalid_constructs() {
    let result = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/ui/check.py"))
        .env(
            "CARGO",
            std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()),
        )
        .status()
        .unwrap();
    assert!(result.success(), "compile-fail runner failed");
}
