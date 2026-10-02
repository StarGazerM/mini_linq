pub fn value() -> i32 {
    let mut rows = cq_storage::sorted_by(vec![(42, ())]);
    assert_eq!(rows.next(), Some((42, ())));
    external_sibling_leaf::value()
}
