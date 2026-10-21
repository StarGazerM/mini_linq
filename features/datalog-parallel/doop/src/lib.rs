//! Feature-selected DOOP CLI with independently compiled parallel engines.
//! Defaults enable all six engines; `--check` requires all six features.
pub mod harness;

#[cfg(all(
    test,
    feature = "mini",
    feature = "mini-fx",
    feature = "ascent",
    feature = "mini-par",
    feature = "mini-par-fx",
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
                            doop_mini_fx::run(facts()).compare(&mut expected).unwrap();
                            doop_mini_par_fx::run(facts())
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
