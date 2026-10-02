pub fn value() -> i32 {
    external_sibling_support::value()
}

#[test]
fn resolves_sibling_and_packaged_core() {
    assert_eq!(value(), 42);
}
