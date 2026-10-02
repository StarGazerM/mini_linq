//! Compile generated negative configurations and small invalid-input fixtures.
//! The runner pins trait/construct substrings and attributes each diagnostic.
#[test]
fn rejected_configurations_and_invalid_constructs() {
    let result = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ui/check.py"))
        .arg(env!("OUT_DIR"))
        .env(
            "CARGO",
            std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()),
        )
        .status()
        .unwrap();
    assert!(result.success(), "compile-fail runner failed");
}
