//! Hybrid parallel MiniLinq evaluators for the shared oracle corpus.
oracle_shared::oracle_programs!(programs, mini, ascent::ascent_par, oracle_shared::oracle_input, pipeline {
    storage_rewrites = [cq_datalog_parallel::shard_storage];
    round = [cq_datalog_parallel::partition];
    print = cq_datalog_parallel::print_rounds;
});
pub use programs::*;
