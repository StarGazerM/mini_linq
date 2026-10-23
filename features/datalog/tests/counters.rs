//! Core counters include the one-time raw EDB scan for canonicalization, but
//! exclude Datalog's canonical borrowed-row traversal, map population, and set
//! inserts. They expose per-hole fallback builds and repeated core full scans.
//! Run from `features/datalog` with:
//! `cargo test -p cq-datalog --features counters --test counters -- --nocapture`.
#[test]
fn reach_reuses_indexes_across_every_round() {
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
        }
        .collect();
        let cost = cq_storage::counters::snapshot();
        assert_eq!(rows, (0..n).map(|x| (x,)).collect());
        assert_eq!(cost.index_builds, 0);
        // Sequential scans retain the original whole-host Rows realization.
        // Raw inputs: n; grouped seed: 2; deltas: n; edge matches: n - 1.
        assert_eq!(cost.rows_enumerated, 3 * n as u64 + 1);
        assert_eq!(cost.distinct_insertions, 0);
        println!(
            "{n}\t{}\t{}\t{}\t{}",
            cost.index_builds, cost.rows_enumerated, cost.lookups, cost.distinct_insertions
        );
    }
}

#[test]
fn shared_owned_delta_layouts_probe_without_core_fallback_builds() {
    let edges: Vec<_> = [("a", "b"), ("b", "c"), ("c", "a"), ("a", "b")]
        .into_iter()
        .map(|(x, y)| (x.to_owned(), y.to_owned()))
        .collect();
    let flags = [(7_i32,), (7,)];
    cq_storage::counters::reset();
    let (rows, flags) = cq_datalog::datalog! {
        struct IndexedRuns;
        relation Edge(x:String,y:String) = (&edges);
        relation FlagInput(flag:i32) = (&flags);
        relation Reach(x:String,y:String);
        relation Flag(flag:i32);
        output Reach, Flag;
        Reach((*x).clone(),(*y).clone()) :- Edge(x,y);
        Reach((*x).clone(),(*z).clone()) :- Reach(x,y), Reach(y,z);
        Reach((*x).clone(),(*z).clone()) :- Edge(x,y), Reach(y,z);
        Reach((*x).clone(),(*z).clone()) :- Edge(y,z), Reach(x,y);
        Reach((*x).clone(),(*y).clone()) :- Edge(x,y), Reach(x,y);
        Reach((*x).clone(),(*y).clone()) :- Flag(f), if *f == 7, Reach(x,y);
        Flag(*f) :- FlagInput(f), Reach(x,y);
    };
    let rows: Vec<_> = rows.collect();
    let flags: Vec<_> = flags.collect();
    let cost = cq_storage::counters::snapshot();
    assert_eq!(rows.len(), 9);
    assert_eq!(
        rows.into_iter().collect::<std::collections::BTreeSet<_>>(),
        ["a", "b", "c"]
            .into_iter()
            .flat_map(|x| ["a", "b", "c"].map(|y| (x.to_owned(), y.to_owned())))
            .collect()
    );
    assert_eq!(flags, [(7,)]);
    // The same borrowed String key is probed by several holes; reversed,
    // all-column, and later key-zero layouts are native grouped hosts too.
    assert_eq!(cost.index_builds, 0);
}

#[test]
fn alternating_empty_delta_indexes_never_fall_back_to_vec_builds() {
    let edges = [(1_i32, 2_i32), (2, 3), (3, 4)];
    let seed = [(1_i32,), (1,)];
    cq_storage::counters::reset();
    let (even, odd) = cq_datalog::datalog! {
        struct Mutual;
        relation Edge(x:i32,y:i32) = (&edges);
        relation Seed(x:i32) = (&seed);
        relation Even(x:i32);
        relation Odd(x:i32);
        output Even, Odd;
        Even(*x) :- Seed(x);
        Odd(*y) :- Edge(x,y), Even(x);
        Even(*y) :- Edge(x,y), Odd(x);
    };
    assert_eq!(
        even.collect::<std::collections::BTreeSet<_>>(),
        [(1,), (3,)].into_iter().collect()
    );
    assert_eq!(
        odd.collect::<std::collections::BTreeSet<_>>(),
        [(2,), (4,)].into_iter().collect()
    );
    assert_eq!(cq_storage::counters::snapshot().index_builds, 0);
}

#[test]
fn borrowed_negative_keys_use_membership_without_building_indexes() {
    let domain: Vec<_> = (0..128_i32).map(|id| (id, format!("key-{id}"))).collect();
    let blocked: Vec<_> = (0..128_i32)
        .filter(|id| id % 3 == 0)
        .map(|id| (format!("key-{id}"),))
        .collect();
    cq_storage::counters::reset();
    let rows: std::collections::BTreeSet<_> = cq_datalog::datalog! {
        struct NegativeKeys;
        relation Domain(id:i32,key:String) = (&domain);
        relation Blocked(key:String) = (&blocked);
        relation Allowed(id:i32);
        output Allowed;
        Allowed(*id) :- Domain(id,key), !Blocked(key);
    }
    .collect();
    let cost = cq_storage::counters::snapshot();
    assert_eq!(
        rows,
        (0..128).filter(|id| id % 3 != 0).map(|id| (id,)).collect()
    );
    assert_eq!(cost.index_builds, 0);
    assert_eq!(cost.lookups, domain.len() as u64);
}

#[test]
fn recursive_negation_reuses_completed_keys_across_rounds() {
    for n in [32_i32, 64, 128] {
        let edges: Vec<_> = (0..n - 1).map(|x| (x, x + 1)).collect();
        let seed = [(0_i32,)];
        let blocked = [(n - 1,)];
        cq_storage::counters::reset();
        let rows: std::collections::BTreeSet<_> = cq_datalog::datalog! {
            struct NegativeReachability;
            relation Edge(x:i32,y:i32) = (&edges);
            relation Seed(x:i32) = (&seed);
            relation Blocked(x:i32) = (&blocked);
            relation Reach(x:i32);
            output Reach;
            Reach(*x) :- Seed(x), !Blocked(x);
            Reach(*y) :- Reach(x), Edge(x,y), !Blocked(y);
        }
        .collect();
        let cost = cq_storage::counters::snapshot();
        assert_eq!(rows, (0..n - 1).map(|x| (x,)).collect());
        assert_eq!(cost.index_builds, 0);
        assert!(cost.lookups >= n as u64);
    }
}

#[test]
fn empty_negative_keys_test_existence_without_building_indexes() {
    let domain: Vec<_> = (0..128_i32).map(|id| (id,)).collect();
    for blocked in [&[][..], &[(String::from("blocked"),)][..]] {
        cq_storage::counters::reset();
        let rows: std::collections::BTreeSet<_> = cq_datalog::datalog! {
            struct NegativeExistence;
            relation Domain(id:i32) = (&domain);
            relation Blocked(key:String) = (blocked);
            relation Allowed(id:i32);
            output Allowed;
            Allowed(*id) :- Domain(id), !Blocked(_);
        }
        .collect();
        let cost = cq_storage::counters::snapshot();
        let expected = if blocked.is_empty() {
            domain.iter().copied().collect()
        } else {
            std::collections::BTreeSet::new()
        };
        assert_eq!(rows, expected);
        assert_eq!(cost.index_builds, 0);
    }
}

#[test]
fn repeated_negative_variables_and_constants_use_prepared_keys_without_builds() {
    let domain: Vec<_> = (0..128_i32).map(|id| (id, format!("key-{id}"))).collect();
    let blocked: Vec<_> = (0..128_i32)
        .flat_map(|id| {
            let left = format!("key-{id}");
            let right = if id % 3 == 0 {
                left.clone()
            } else {
                format!("other-{id}")
            };
            [
                (String::from("blocked"), left.clone(), right, id),
                (String::from("ignored"), left.clone(), left, id),
            ]
        })
        .collect();
    cq_storage::counters::reset();
    let rows: std::collections::BTreeSet<_> = cq_datalog::datalog! {
        struct RepeatedNegativeKeys;
        relation Domain(id:i32,key:String) = (&domain);
        relation Blocked(kind:String,left:String,right:String,extra:i32) = (&blocked);
        relation Allowed(id:i32);
        output Allowed;
        Allowed(*id) :- Domain(id,key), !Blocked(String::from("blocked"),key,key,_);
    }
    .collect();
    let cost = cq_storage::counters::snapshot();
    assert_eq!(
        rows,
        (0..128).filter(|id| id % 3 != 0).map(|id| (id,)).collect()
    );
    assert_eq!(cost.index_builds, 0);
    assert_eq!(cost.lookups, domain.len() as u64);
}
