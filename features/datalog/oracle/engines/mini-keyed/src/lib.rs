oracle_shared::oracle_programs!(programs, mini, ascent::ascent, oracle_shared::oracle_input, pipeline {
    storage_rewrites = [cq_datalog_hasher::hasher(keyed)];
});
pub use programs::*;
