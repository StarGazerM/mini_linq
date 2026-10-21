//! Feature-selected sequential DOOP CLI; each engine compiles independently.
//! Defaults enable all engines; `--check` requires `mini,mini-fx,ascent`.
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
