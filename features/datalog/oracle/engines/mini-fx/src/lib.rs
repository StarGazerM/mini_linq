oracle_shared::oracle_programs!(programs, mini, ascent::ascent, oracle_shared::oracle_input, pipeline {
    storage_rewrites = [cq_datalog_hasher::hasher(fx)];
    round = [cq_datalog_hasher::hasher_round(fx)];
    cq = pipeline {
        structures = [cq_rewrites::retype(dedup => ::std::collections::HashSet<_, ::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>>)];
    };
});
pub use programs::*;
