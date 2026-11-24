// The push printer cannot print an ordered merge, even with ordered hosts.
fn main() {
    let left = std::collections::BTreeMap::from([((1_i32,), vec![()])]);
    let right = std::collections::BTreeMap::from([((1_i32,), vec![()])]);
    let _ = mini_linq::compile! {
        pipeline {
            physical = [cq_rewrites::merge_joins];
            print_input = structures;
            print = own_push::push;
        }
        struct Intersection;
        relation left(x:i32); relation right(x:i32);
        answer(x) :- left(x), right(x);
    }.count();
}
