#![recursion_limit = "1024"]
#[cfg(not(feature = "phase-timing"))]
doop_shared::doop_engine!(run, pipeline {
    storage_rewrites = [cq_datalog_parallel::shard_storage];
    round = [cq_datalog_parallel::partition(threshold = 64)];
    print = cq_datalog_parallel::print_rounds;
});
#[cfg(feature = "phase-timing")]
doop_shared::doop_engine!(run, pipeline {
    storage_rewrites = [cq_datalog_parallel::shard_storage];
    fork = cq_datalog::passes::fork(timing = phases;);
    round = [cq_datalog_parallel::partition(threshold = 64)];
    print = cq_datalog_parallel::print_rounds;
});
