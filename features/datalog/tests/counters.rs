//! Core counters exclude Datalog's own one-time map population and set inserts.
//! They expose accidental per-hole fallback builds and repeated full scans.
#[test]
fn reach_reuses_indexes_across_every_round() {
    let mut first_builds = None;
    for n in [2000_i32, 4000, 8000] {
        let edges: Vec<_> = (0..n - 1).map(|x| (x, x + 1)).collect();
        let seed = [(0_i32,)];
        cq_storage::counters::reset();
        let rows: std::collections::BTreeSet<_> = cq_datalog::datalog! {
            struct Reachability;
            relation Edge(x:i32,y:i32) = (edges);
            relation Seed(x:i32) = (seed);
            relation Reach(x:i32);
            output Reach;
            Reach(*x) :- Seed(x);
            Reach(*y) :- Reach(x), Edge(x,y);
        }.collect();
        let cost = cq_storage::counters::snapshot();
        assert_eq!(rows, (0..n).map(|x| (x,)).collect());
        assert_eq!(*first_builds.get_or_insert(cost.index_builds), cost.index_builds);
        assert!(cost.rows_enumerated <= 4 * n as u64 + 4, "{cost:?}");
        println!("{n}\t{}\t{}", cost.index_builds, cost.rows_enumerated);
    }
}
