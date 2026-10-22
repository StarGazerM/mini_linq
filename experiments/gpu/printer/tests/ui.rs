#[test]
fn unsupported_structures_fail_at_the_public_macro_boundary() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
