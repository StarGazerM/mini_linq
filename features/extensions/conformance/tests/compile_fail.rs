#[test]
fn invalid_extension_surfaces_and_unexpanded_clauses_are_rejected() {
    let status = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ui.py"))
        .status()
        .expect("run isolated extension consumers");
    assert!(status.success(), "extension compile-fail runner failed");
}
