//! A selected host layout is advisory to ordinary CQ: a different probe key
//! falls back correctly. Measure both shapes without changing the CQ pipeline.
use std::collections::{BTreeSet, HashMap};

fn main() {
    println!("layout\tnodes\tbuilds\trows_enumerated\tlookups");
    for n in [2000_i32, 4000, 8000] {
        let mut by_source: HashMap<(i32,), Vec<(i32,)>> = HashMap::new();
        let mut by_target: HashMap<(i32,), Vec<(i32,)>> = HashMap::new();
        for x in 0..n - 1 {
            by_source.entry((x,)).or_default().push((x + 1,));
            by_target.entry((x + 1,)).or_default().push((x,));
        }
        let seeds = [(n / 2,), (n / 2 + 1,)];
        cq_storage::counters::reset();
        let matched: BTreeSet<_> = mini_linq::mini_linq! {
            struct Matched;
            relation Seed(y:i32) = (seeds);
            relation Edge(y:i32,x:i32) = (&by_target);
            relation answer(x:i32,y:i32);
            answer(*x,*y) :- Seed(y), Edge(y,x);
        }.collect();
        let matched_cost = cq_storage::counters::snapshot();
        cq_storage::counters::reset();
        let mismatched: BTreeSet<_> = mini_linq::mini_linq! {
            struct Mismatched;
            relation Seed(y:i32) = (seeds);
            relation Edge(x:i32,y:i32) = (&by_source);
            relation answer(x:i32,y:i32);
            answer(*x,*y) :- Seed(y), Edge(x,y);
        }.collect();
        let mismatched_cost = cq_storage::counters::snapshot();
        let expected = BTreeSet::from([(n / 2 - 1, n / 2), (n / 2, n / 2 + 1)]);
        assert_eq!(matched, expected);
        assert_eq!(mismatched, expected);
        for (name, cost) in [("matched", matched_cost), ("mismatched", mismatched_cost)] {
            println!("{name}\t{n}\t{}\t{}\t{}", cost.index_builds,
                cost.rows_enumerated, cost.lookups);
        }
    }
}
