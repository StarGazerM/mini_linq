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

#[test]
fn parallel_negative_keys_are_shared_native_membership_hosts() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    pool.install(|| {
        let domain: Vec<_> = (0..2048_i32).map(|id| (id, format!("key-{id}"))).collect();
        let blocked: Vec<_> = (0..2048_i32)
            .map(|id| {
                let left = format!("key-{id}");
                let right = if id % 3 == 0 {
                    left.clone()
                } else {
                    format!("other-{id}")
                };
                (left, right)
            })
            .collect();
        pool.broadcast(|_| cq_storage::counters::reset());
        let rows: std::collections::BTreeSet<_> = cq_datalog::datalog! {
            pipeline {
                storage_rewrites = [cq_datalog_parallel::shard_storage];
                round = [cq_datalog_parallel::partition(threshold = 1, grain = 64)];
                print = cq_datalog_parallel::print_rounds;
            }
            struct NegativeParallel;
            relation Domain(id:i32,key:String) = (&domain);
            relation Blocked(left:String,right:String) = (&blocked);
            relation Allowed(id:i32);
            output Allowed;
            Allowed(*id) :- Domain(id,key), !Blocked(key,key);
        }
        .collect();
        let costs = pool.broadcast(|_| cq_storage::counters::snapshot());
        assert_eq!(
            rows,
            (0..2048).filter(|id| id % 3 != 0).map(|id| (id,)).collect()
        );
        assert_eq!(costs.iter().map(|cost| cost.index_builds).sum::<u64>(), 0);
        assert_eq!(
            costs.iter().map(|cost| cost.lookups).sum::<u64>(),
            domain.len() as u64
        );
    });
}
