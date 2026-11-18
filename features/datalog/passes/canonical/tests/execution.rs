//! The selected canonical order is a storage decision only: answers, negation
//! and outputs of a re-ordered relation are unchanged on the default engine.
use std::collections::BTreeSet;

macro_rules! paths {
    ($edges:expr, $next:expr, $blocked:expr; $($configuration:tt)*) => {
        cq_datalog::datalog! {
            pipeline { $($configuration)* }
            struct Paths;
            relation Edge(x:i32,y:i64) = ($edges);
            relation Next(y:i64,z:i64) = ($next);
            relation Blocked(x:i32,y:i64) = ($blocked);
            relation Reach(x:i32,y:i64);
            relation Into(y:i64,x:i32);
            relation Open(x:i32,y:i64);
            output Reach, Into, Open;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Next(y,z);
            Into(*y,*x) :- Next(_,y), Reach(x,y);
            Open(*x,*y) :- Edge(x,y), !Reach(_,y), !Blocked(x,y);
        }
    };
}

#[test]
fn reordered_canonical_layouts_leave_every_answer_unchanged() {
    let edges = [(1, 2i64), (2, 3), (4, 2), (7, 9), (8, 5)];
    let next = [(2i64, 3i64), (3, 4), (4, 2), (9, 9)];
    let blocked = [(8, 5i64)];
    let (reach, into, open) = paths!(edges, next, blocked;);
    let (ordered_reach, ordered_into, ordered_open) = paths!(edges, next, blocked;
        storage_rewrites = [cq_datalog_canonical::canonical(ordered)];
    );
    let set = |rows: Vec<(i32, i64)>| rows.into_iter().collect::<BTreeSet<_>>();
    let reach = set(reach.collect());
    assert!(reach.contains(&(1, 4)) && reach.contains(&(4, 4)) && !reach.contains(&(8, 2)));
    assert_eq!(set(ordered_reach.collect()), reach);
    let pairs = |rows: Vec<(i64, i32)>| rows.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(pairs(ordered_into.collect()), pairs(into.collect()));
    assert_eq!(set(ordered_open.collect()), set(open.collect()));
}
