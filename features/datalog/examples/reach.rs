//! Run with `cargo run --manifest-path features/datalog/Cargo.toml --example reach`.
use std::collections::BTreeSet;

fn main() {
    let edges = [(1, 2), (2, 3), (3, 2), (3, 4), (8, 9)];
    let seeds = [(1,)];
    let reachable: BTreeSet<_> = cq_datalog::datalog! {
        struct Reachability;
        relation edge(source: i32, target: i32) = (edges);
        relation seed(node: i32) = (seeds);
        relation reach(node: i32);
        output reach;
        reach(*x) :- seed(x);
        reach(*y) :- reach(x), edge(x, y);
    }.collect();
    assert_eq!(reachable, BTreeSet::from([(1,), (2,), (3,), (4,)]));
    for (node,) in reachable { println!("{node}"); }
}
