//! DOOP's unchanged 103-relation, 135-rule source instantiated as an extension.
//! `doop-parallel FACTS --check --threads N` compares all 26 full tuple sets.
//! `--engine mini|mini-par|ascent|ascent-par` isolates timing and RSS; the sweep
//! script checks six applications and six worker counts before measuring.
//! [`harness`] supplies engine descriptors and installs local Rayon pools;
//! CLI parsing, loading, full-tuple checks, timing and RSS use the base harness.
#![recursion_limit = "1024"]

pub mod harness;

#[macro_export]
macro_rules! relation_storage {
    ($rows:expr) => {
        $rows.into_iter().collect()
    };
}

pub mod mini_engine {
    pub use datalog_doop::mini_engine::run;
    #[cfg(not(feature = "phase-timing"))]
    datalog_doop::doop_engine!(run_parallel, pipeline {
        storage_rewrites = [cq_datalog_parallel::shard_storage];
        round = [cq_datalog_parallel::partition(threshold = 64)];
        print = cq_datalog_parallel::print_rounds;
    });
    #[cfg(feature = "phase-timing")]
    datalog_doop::doop_engine!(run_parallel, pipeline {
        storage_rewrites = [cq_datalog_parallel::shard_storage];
        fork = cq_datalog::passes::fork(timing = phases;);
        round = [cq_datalog_parallel::partition(threshold = 64)];
        print = cq_datalog_parallel::print_rounds;
    });
}

pub mod ascent_engine {
    pub use datalog_doop::ascent_engine::run;
    datalog_doop::doop_ascent_engine!(workers, ascent::ascent_par, crate::relation_storage);
    pub use workers::run as run_parallel;
}

#[cfg(test)]
mod tests {
    use datalog_doop::{Facts, interner::sym};

    fn facts() -> Facts {
        let class = sym("Example");
        let object = sym("java.lang.Object");
        let method = sym("<Example: void main(java.lang.String[])>");
        let abstract_method = sym("<Example: void abstractMethod()>");
        Facts {
            mainclass: vec![(class,)],
            _classtype: vec![(class,), (object,)],
            directsuperclass: vec![(class, object)],
            method_modifier: vec![
                (sym("public"), method),
                (sym("static"), method),
                (sym("abstract"), abstract_method),
            ],
            _method: vec![
                (
                    method,
                    sym("main"),
                    sym("java.lang.String[]"),
                    class,
                    sym("void"),
                    sym("([Ljava/lang/String;)V"),
                    1,
                ),
                (
                    abstract_method,
                    sym("abstractMethod"),
                    sym(""),
                    class,
                    sym("void"),
                    sym("()V"),
                    0,
                ),
            ],
            _assignheapallocation: vec![(
                sym("allocation"),
                0,
                sym("heap"),
                sym("local"),
                method,
                1,
            )],
            _normalheap: vec![(sym("heap"), class)],
            ..Facts::default()
        }
    }

    #[test]
    fn all_outputs_match_in_local_worker_pools() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                for threads in [1, 2, 4] {
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(threads)
                        .stack_size(16 * 1024 * 1024)
                        .build()
                        .unwrap()
                        .install(|| {
                            let mut expected = datalog_doop::ascent_engine::run(facts());
                            let mut sequential = datalog_doop::mini_engine::run(facts());
                            let mut actual = super::mini_engine::run_parallel(facts());
                            let mut ascent = super::ascent_engine::run_parallel(facts());
                            sequential.compare(&mut expected).unwrap();
                            actual.compare(&mut expected).unwrap();
                            ascent.compare(&mut expected).unwrap();
                            assert_eq!(actual.varpointsto, vec![(sym("heap"), sym("local"))]);
                            assert_eq!(actual.mainmethoddeclaration.len(), 1);
                            assert_eq!(actual.methodimplemented.len(), 1);
                        });
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
