use std::sync::Arc;
use gpu_datalog_runtime::{DomainCatalog, DomainInput, SymbolCodec};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Handle(u32);
struct Codec;
impl SymbolCodec<Handle> for Codec {
    fn code(value: &Handle) -> u32 { value.0 }
    // DECODE P: final boundary-test answers restore logical handles for assertions.
    fn decode(code: u32) -> Handle { Handle(code) }
}

// An alias ensures schema generation asks Scalar for bounds, not Rust spelling.
type Derived = i32;

macro_rules! execute {
    ($catalog:ident; $($source:tt)*) => {
        cq_datalog::datalog! {
            pipeline {
                storage_rewrites = [::cq_gpu_datalog::domain_storage(symbols(Handle => [0] Codec))];
                cq = pipeline {
                    surface = [::cq_gpu_datalog::encode_domains];
                    physical = [::cq_gpu_sequence::capture_constants];
                    structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
                    sequence = ::mini_linq::passes::sequence;
                    decompose = [::cq_gpu_sequence::decompose];
                    stage = ::cq_gpu_sequence::stage;
                    grade = ::cq_gpu_sequence::machine_grade;
                    print = ::cq_gpu_datalog::translate_domain_arena;
                };
                terminals = [::cq_gpu_datalog::translate_domain_arena => batch];
                round = [::cq_gpu_datalog::bind_round(binding = 6)];
                print = ::cq_gpu_datalog::print_rounds($catalog);
            }
            $($source)*
        }
    };
}

// The schema and execution consume precisely the same declarations and rules.
macro_rules! boundary_program {
    ($($callback:ident)::+; $rows:ident; $($prefix:tt)*) => {
        $($callback)::+! {
            $($prefix)*
            struct Boundaries;
            relation Middle(n:Derived);
            relation Input(x:Handle) = ($rows);
            relation Tail(n:Derived);
            relation Absent(x:Handle);
            relation Below(x:Handle);
            relation Above(x:Handle);
            relation Exact(n:Derived);
            output Middle, Tail, Absent, Below, Above, Exact;
            Middle(-17) :- Input(x);
            Middle(-2147483648) :- Input(x);
            Middle(2147483647) :- Input(x);
            Tail(*n / 2) :- Middle(n);
            Absent(x) :- Input(x), where x == Handle(20);
            Below(x) :- Input(x), where x < Handle(20);
            Above(x) :- Input(x), where x > Handle(20);
            Exact(n) :- Middle(n), where n == -17;
        }
    };
}

#[test]
fn schema_covers_interior_and_trailing_derived_numeric_components() {
    let (relations, types, required) = boundary_program!(cq_gpu_datalog::domain_schema; rows;
        symbols(Handle => [0] Codec);
    );
    assert_eq!(relations, &[
        ("Middle", &[0][..]), ("Input", &[1][..]), ("Tail", &[2][..]),
        ("Absent", &[1][..]), ("Below", &[1][..]), ("Above", &[1][..]),
        ("Exact", &[0][..]),
    ]);
    assert_eq!(types, &[
        (Some((i32::MIN as i128, i32::MAX as i128)), false),
        (None, true),
        (Some((i32::MIN as i128, i32::MAX as i128)), false),
    ]);
    let required: Vec<(u32, u32)> = required();
    assert!(required.is_empty());
}

#[test]
fn derived_only_i32_and_absent_symbol_predicates_are_exact() -> gpu_datalog_runtime::Result<()> {
    let (_, types, _) = boundary_program!(cq_gpu_datalog::domain_schema; rows;
        symbols(Handle => [0] Codec);
    );
    let inputs = types.iter().map(|&(bounds, edb)| {
        Ok(match bounds {
            Some((minimum, maximum)) => {
                assert!(!edb, "this program has no numeric EDB column");
                DomainInput::Signed(Some((i64::try_from(minimum)?, i64::try_from(maximum)?)))
            }
            None => DomainInput::Dictionary(vec![10, 30]),
        })
    }).collect::<gpu_datalog_runtime::Result<Vec<_>>>()?;
    let catalog = Arc::new(DomainCatalog::new(gpu_datalog_runtime::thread_pool()?, inputs)?);
    assert_eq!(catalog.bits(0)?, 32);
    assert_eq!(catalog.bits(1)?, 1);
    assert_eq!(catalog.bits(2)?, 32);
    let rows = vec![(Handle(10),), (Handle(30),)];
    let (middle, tail, absent, below, above, exact) = boundary_program!(execute; rows; catalog;);
    let mut middle: Vec<_> = middle.collect();
    let mut tail: Vec<_> = tail.collect();
    middle.sort_unstable();
    tail.sort_unstable();
    assert_eq!(middle, [(i32::MIN,), (-17,), (i32::MAX,)]);
    assert_eq!(tail, [(i32::MIN / 2,), (-8,), (i32::MAX / 2,)]);
    assert_eq!(absent.count(), 0);
    assert_eq!(below.collect::<Vec<_>>(), [(Handle(10),)]);
    assert_eq!(above.collect::<Vec<_>>(), [(Handle(30),)]);
    assert_eq!(exact.collect::<Vec<_>>(), [(-17,)]);
    Ok(())
}

macro_rules! observed_program {
    ($($callback:ident)::+; $rows:ident; $($prefix:tt)*) => {
        $($callback)::+! {
            $($prefix)*
            struct Observed;
            relation Input(n:i32,x:Handle) = ($rows);
            relation Output(n:i32);
            output Output;
            Output(n) :- Input(n,x);
            Output(*n + 1) :- Output(n), where n < 3;
        }
    };
}

#[test]
#[should_panic(expected = "Sequence device count or fold overflow")]
fn derived_number_outside_observed_edb_bounds_is_rejected() {
    let (_, types, _) = observed_program!(cq_gpu_datalog::domain_schema; rows;
        symbols(Handle => [0] Codec);
    );
    assert_eq!(types, &[
        (Some((i32::MIN as i128, i32::MAX as i128)), true), (None, true),
    ]);
    let catalog = Arc::new(DomainCatalog::new(gpu_datalog_runtime::thread_pool().unwrap(), vec![
        DomainInput::Signed(Some((1, 2))), DomainInput::Dictionary(vec![10]),
    ]).unwrap());
    // 3 fits the observed physical signed width. The producer must reject it
    // through its existing failure flag before returning a candidate batch.
    assert_eq!(catalog.bits(0).unwrap(), 3);
    let rows = vec![(1i32, Handle(10)), (2, Handle(10))];
    let output = observed_program!(execute; rows; catalog;);
    let _ = output.collect::<Vec<_>>();
}
