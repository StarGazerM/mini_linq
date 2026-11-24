//! Shared executable fixture for expanded-identity and completed-run tests.
use std::collections::BTreeSet;

pub const SOURCE: &str = stringify! {
    struct CalibrationReach;
    relation Edge(x: i32, y: i32) = (edges);
    relation Seed(x: i32) = (seeds);
    relation Reach(x: i32);
    output Reach;
    Init: Reach(*x) :- Seed(x);
    Step: Reach(*y) :- Reach(x), Edge(x,y), where x >= 0, where x < 10;
};

pub fn reach(edges: &[(i32, i32)], seeds: &[(i32,)]) -> BTreeSet<(i32,)> {
    cq_datalog::datalog! {
        struct CalibrationReach;
        relation Edge(x: i32, y: i32) = (edges);
        relation Seed(x: i32) = (seeds);
        relation Reach(x: i32);
        output Reach;
        Init: Reach(*x) :- Seed(x);
        Step: Reach(*y) :- Reach(x), Edge(x,y), where x >= 0, where x < 10;
    }.into_iter().collect()
}

#[test]
fn ordinary_execution_remains_exact_without_an_observer() {
    assert_eq!(reach(&[(0,1),(0,1),(1,2),(2,3)], &[(0,),(0,)]), BTreeSet::from([(0,),(1,),(2,),(3,)]));
}
