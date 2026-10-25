//! Feature-selected sequential DOOP CLI; each engine compiles independently.
//! Defaults enable all engines; `--check` requires `mini,mini-keyed,ascent`.
//! Shared generators preserve all 103 relations, 135 rules and body atom order
//! from `flowlog-rs/flowlog-bench/programs/oracle/flowlog/doop/default.dl`.
//! Loading and the process-wide u32 string interner live in `doop-shared`.
//!
//! From `features/datalog`, build with `cargo build --release -p datalog-doop`.
//! Fetch with `python3 doop/fetch.py cassandra`, then invoke the built binary:
//! `doop /tmp/doop-data/cassandra --check`.
//! `--check` compares all 26 full tuple sets against sequential Ascent.
//! Uninstrumented output order is unspecified.
//! `--engine mini|mini-keyed|ascent` isolates timing and peak RSS.
//! `mini` uses Fx storage and canonical consumers by default; `mini-keyed` uses
//! keyed aHash for both via explicit Storage and Round pipeline configuration.
//! Default CLI execution runs `mini` and `ascent`; Cargo enables all three.
//! Enable `phase-timing` for per-round instrumentation separately from timing runs.
//! Compute timing includes derived indexes and materializing the 26 outputs;
//! load timing includes parsing and interning, not engine initialization.
//! No counters are enabled. Do not compile concurrently with measured runs.
//! [`harness`] supplies engine descriptors to the shared harness, which owns
//! CLI parsing, full-set checks, loading, timing and process high-water RSS.
//!
//! `codegen-evidence.json` and `../oracle/hasher-summary.tsv` retain
//! historical pre-cutover measurements and engine names: `mini` meant keyed
//! aHash and `mini-fx` meant opt-in Fx. They do not measure today's Fx-default
//! engines or keyed `cq_storage` providers. The optional partitioned hole
//! terminal measured in the codegen evidence has since been removed.
pub mod harness;

#[cfg(all(test, feature = "mini", feature = "ascent"))]
mod tests {
    use doop_shared::Facts;
    use doop_shared::interner::sym;

    #[test]
    fn all_outputs_match_on_small_program() {
        // The unoptimized 103-relation expansions exceed libtest's 2 MiB
        // worker stack. Release benchmark execution uses the normal main stack.
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(compare_small_program)
            .unwrap()
            .join()
            .unwrap();
    }

    fn compare_small_program() {
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
                method_descriptor: vec![
                    (method, sym("void(java.lang.String[])")),
                    (abstract_method, sym("void()")),
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
        let mut mini = doop_mini::run(facts());
        let mut ascent = doop_ascent::run(facts());
        mini.compare(&mut ascent).unwrap();
        assert_eq!(mini.varpointsto, vec![(sym("heap"), sym("local"))]);
        assert_eq!(mini.mainmethoddeclaration.len(), 1);
        assert_eq!(mini.methodimplemented.len(), 1);
        assert!(mini.method_descriptor.contains(&(
            sym("<Example: void main(java.lang.String[])>"),
            sym("void(java.lang.String[])"),
        )));
    }
}
