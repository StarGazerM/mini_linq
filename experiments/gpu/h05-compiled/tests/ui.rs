#[test]
fn declared_host_schema_is_a_rust_type_obligation() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
