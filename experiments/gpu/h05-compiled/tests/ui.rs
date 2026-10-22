// The Sequence translator consumes resident columns, not host tuple iterators.
// Schema mismatches are rejected at the typed resident binding boundary.
#[test]
fn resident_host_schema_is_a_rust_type_obligation() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
