#![recursion_limit = "2048"]
#[cfg(not(feature = "phase-timing"))]
srd_doop_shared::srd_doop_engine!(run, pipeline {
    storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(sharded))];
    round = [cq_datalog_parallel::partition(threshold = 64)];
    print = cq_datalog_parallel::print_rounds;
});
#[cfg(feature = "phase-timing")]
srd_doop_shared::srd_doop_engine!(run, pipeline {
    storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(sharded))];
    fork = cq_datalog::passes::fork(timing = phases;);
    round = [cq_datalog_parallel::partition(threshold = 64)];
    print = cq_datalog_parallel::print_rounds;
});
