pub fn value() -> i32 {
    let mut rows = cq_storage::sorted_by(vec![(42, ())]);
    rows.next().unwrap().0
}
