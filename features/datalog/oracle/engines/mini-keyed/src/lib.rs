oracle_shared::oracle_programs!(programs, mini, ascent::ascent, oracle_shared::oracle_input, pipeline {
    storage_rewrites = [cq_datalog_hasher::hasher(keyed)];
    round = [cq_datalog_hasher::hasher_round(keyed)];
    cq = pipeline {
        structures = [cq_rewrites::retype(dedup => ::cq_storage::HashSet<_>)];
    };
});
pub use programs::*;
