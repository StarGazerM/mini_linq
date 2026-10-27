use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct TestSymbol(u32);

struct TestCodec;
impl gpu_datalog_runtime::SymbolCodec<TestSymbol> for TestCodec {
    fn code(value: &TestSymbol) -> u32 { value.0 }
    fn decode(code: u32) -> TestSymbol { TestSymbol(code) }
}

fn sym(value: &str) -> TestSymbol {
    match value {
        "x" => TestSymbol(7),
        "y" => TestSymbol(19),
        _ => panic!("unknown test symbol"),
    }
}

type Answers = (BTreeSet<(TestSymbol,)>, BTreeSet<(TestSymbol,)>, BTreeSet<(TestSymbol, TestSymbol, TestSymbol, TestSymbol, TestSymbol)>);

macro_rules! engine {
    ($name:ident, {$($pipeline:tt)*}) => {
        fn $name(labels: Vec<(TestSymbol, TestSymbol)>, wide: Vec<(TestSymbol, TestSymbol, TestSymbol, TestSymbol, TestSymbol)>) -> Answers {
            let (guarded, selected, copied) = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Symbols;
                relation Label(t: TestSymbol, value: TestSymbol) = (labels);
                relation Wide(a: TestSymbol, b: TestSymbol, c: TestSymbol, d: TestSymbol, e: TestSymbol) = (wide);
                relation Guarded(value: TestSymbol);
                relation Selected(value: TestSymbol);
                relation Copied(a: TestSymbol, b: TestSymbol, c: TestSymbol, d: TestSymbol, e: TestSymbol);
                output Guarded, Selected, Copied;
                Guarded(*value) :- Label(t, value), if *t == sym("x");
                Selected(*value) :- Label(sym("x"), value);
                Copied(*a, *b, *c, *d, *e) :- Wide(a, b, c, d, e);
            };
            (guarded.collect(), selected.collect(), copied.collect())
        }
    };
}

engine!(cpu, {});
engine!(gpu, {
    storage_rewrites = [::cq_gpu_datalog::device_storage(symbols(TestSymbol => TestCodec))];
    cq = pipeline {
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
        sequence = ::mini_linq::passes::sequence;
        decompose = [::cq_gpu_sequence::plan];
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_sequence::translate_arena;
    };
    terminals = [::cq_gpu_sequence::translate_arena => batch];
    print = ::cq_gpu_datalog::print_rounds;
});

#[test]
fn symbols_guards_atom_constants_and_wide_rows_match_cpu() {
    let x = sym("x");
    let y = sym("y");
    let labels = vec![(x, y), (y, x), (x, x), (x, y)];
    let wide = vec![(x, y, x, y, x), (y, x, y, x, y), (x, y, x, y, x)];
    assert_eq!(gpu(labels.clone(), wide.clone()), cpu(labels, wide));
    assert_eq!(gpu(Vec::new(), Vec::new()), cpu(Vec::new(), Vec::new()));
}
