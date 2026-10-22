#[test]
fn unsupported_device_programs_are_rejected() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
