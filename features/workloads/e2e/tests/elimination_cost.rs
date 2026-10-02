//! Minimal reproducer for disconnected private-column products in Elimination/Fine.
#[test]
fn shared_keys_are_bound_before_disconnected_leaf_products() {
    let rows: Vec<_> = (0..24).map(|v| (v, v)).collect();
    let r = rows.as_slice();
    let s = rows.as_slice();
    let t = rows.as_slice();
    let visits = std::cell::Cell::new(0);
    let observe = |_: &i32, _: &i32| {
        visits.set(visits.get() + 1);
        true
    };
    let got: std::collections::BTreeSet<_> = ::mini_linq::compile! {
        pipeline { plan = cq_planners::order(elimination, fine); }
        struct LeafProducts;
        relation r(k: i32, a: i32); relation s(k: i32, b: i32); relation t(k: i32, c: i32);
        answer(k) :- r(k, a), s(k, b), t(k, c), if observe(b, c);
    }
    .map(|(k,)| *k)
    .collect();
    assert_eq!(got, (0..24).collect());
    assert_eq!(
        visits.get(),
        24,
        "disconnected prefixes should not form a Cartesian product"
    );
}
