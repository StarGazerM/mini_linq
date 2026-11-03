#![recursion_limit = "1024"]

use std::collections::HashSet;

#[test]
fn captured_literal_atom_preserves_its_restriction() {
    let r0 = [(1, -3), (2, 0), (3, -3)];
    let actual: HashSet<_> = ::mini_linq::compile! {
        pipeline { physical = [::cq_gpu_sequence_macros::capture_constants]; }
        struct LiteralAtom;
        relation r0(c0: i32, c1: i32); relation answer(c0: i32);
        answer(v0) :- r0(v0, -3);
    }
    .collect();
    assert_eq!(actual, HashSet::from([(&1,), (&3,)]));
}
