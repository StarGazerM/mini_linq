#[test]
fn invalid_extension_surfaces_and_unexpanded_clauses_are_rejected() {
    let result = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/ui/extensions.py"
        ))
        .env(
            "CARGO",
            std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()),
        )
        .status()
        .unwrap();
    assert!(result.success(), "extension compile-fail runner failed");
}
