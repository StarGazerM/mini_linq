//! Feature-selected DOOP CLI with independently compiled parallel engines.
//! Defaults enable all six engines; `--check` requires all six features:
//! `mini,mini-keyed,ascent,mini-par,mini-par-keyed,ascent-par`.
//! Shared generators preserve DOOP's 103-relation, 135-rule source.
//! `doop-parallel FACTS --check --threads N` compares all 26 full tuple sets.
//! `--engine mini|mini-keyed|mini-par|mini-par-keyed|ascent|ascent-par` isolates
//! timing and RSS. `mini` and `mini-par` use Fx storage and hole deduplication;
//! the `-keyed` alternatives use keyed aHash. Default CLI execution runs `mini`
//! and `ascent`; Cargo enables all six engines.
//! For 15 workers, use
//! `doop-parallel /tmp/doop-data/cassandra --engine mini-par --threads 15`;
//! `doop-parallel /tmp/doop-data/cassandra --check --threads 15` compares every
//! engine with sequential Ascent. Run each timing engine in a separate process.
//! [`harness`] supplies engine descriptors and installs local Rayon pools;
//! CLI parsing, loading, full-tuple checks, timing and RSS use `doop-shared`.
//!
//! `hasher-samples.csv`, `hasher-summary.csv` and `hasher-evidence.json` preserve
//! the historical `dadaed6` opt-in measurements unchanged. In those artifacts,
//! `mini`/`mini-par` mean keyed aHash and `mini-fx`/`mini-par-fx` mean Fx.
//! They are not measurements of today's Fx-default engines or keyed selectors.
//! Use `hash_sweep.py` to collect a new, separately labeled comparison.
pub mod harness;

#[cfg(all(
    test,
    feature = "mini",
    feature = "mini-keyed",
    feature = "ascent",
    feature = "mini-par",
    feature = "mini-par-keyed",
    feature = "ascent-par"
))]
mod tests {
    use doop_shared::{Facts, interner::sym};

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
                for threads in [1, 2, 4, 15] {
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(threads)
                        .stack_size(16 * 1024 * 1024)
                        .build()
                        .unwrap()
                        .install(|| {
                            let mut expected = doop_ascent::run(facts());
                            let mut sequential = doop_mini::run(facts());
                            let mut actual = doop_mini_par::run(facts());
                            let mut ascent = doop_ascent_par::run(facts());
                            sequential.compare(&mut expected).unwrap();
                            actual.compare(&mut expected).unwrap();
                            ascent.compare(&mut expected).unwrap();
                            doop_mini_keyed::run(facts())
                                .compare(&mut expected)
                                .unwrap();
                            doop_mini_par_keyed::run(facts())
                                .compare(&mut expected)
                                .unwrap();
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
