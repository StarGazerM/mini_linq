#[test]
fn parallel_holes_reuse_full_and_delta_layouts_without_fallback_builds() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    pool.install(|| {
        let edges: Vec<_> = (0..1100_i32)
            .flat_map(|x| [(3 * x, 3 * x + 1), (3 * x + 1, 3 * x + 2)])
            .collect();
        macro_rules! run {
                ($($mode:tt)*) => {
                    cq_datalog::datalog! {
                        pipeline { $($mode)* }
                        struct IndexedParallel;
                        relation Edge(x:i32,y:i32) = (&edges);
                        relation Reach(x:i32,y:i32);
                        output Reach;
                        Reach(*x,*y) :- Edge(x,y);
                        Reach(*x,*z) :- Reach(x,y), Reach(y,z);
                        Reach(*x,*z) :- Edge(x,y), Reach(y,z);
                        Reach(*x,*z) :- Edge(y,z), Reach(x,y);
                        Reach(*x,*y) :- Edge(x,y), Reach(x,y);
                    }.collect::<Vec<_>>()
                };
            }
        pool.broadcast(|_| cq_storage::counters::reset());
        let sequential = run!();
        let baseline = pool.broadcast(|_| cq_storage::counters::snapshot());
        // Core counters are thread-local: reset and sample every worker,
        // rather than accidentally inspecting only the installing thread.
        pool.broadcast(|_| cq_storage::counters::reset());
        let parallel = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds;);
        assert_eq!(
            parallel.iter().collect::<std::collections::BTreeSet<_>>(),
            sequential.iter().collect::<std::collections::BTreeSet<_>>()
        );
        assert_eq!(parallel.len(), 3300);
        let costs = pool.broadcast(|_| cq_storage::counters::snapshot());
        assert_eq!(costs.iter().map(|cost| cost.index_builds).sum::<u64>(), 0);
        let baseline_lookups = baseline.iter().map(|cost| cost.lookups).sum::<u64>();
        let sharded_lookups = costs.iter().map(|cost| cost.lookups).sum::<u64>();
        assert_eq!(sharded_lookups, baseline_lookups);
        eprintln!("native probes: sequential={baseline_lookups} sharded={sharded_lookups}; fallback builds=0");
        assert!(costs.iter().map(|cost| cost.rows_enumerated).sum::<u64>() > 0);
    });
}
