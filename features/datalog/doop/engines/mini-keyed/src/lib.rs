#![recursion_limit = "1024"]
doop_shared::doop_engine!(run, pipeline {
    storage_rewrites = [cq_datalog_hasher::hasher(keyed)];
    round = [cq_datalog_hasher::hasher_round(keyed)];
});
