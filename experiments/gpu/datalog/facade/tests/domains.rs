use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use gpu_datalog_runtime::{DomainCatalog, DomainInput, SymbolCodec};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Handle(u32);
struct BatchCodec;
static BATCHES: AtomicUsize = AtomicUsize::new(0);
static SCALAR_DECODES: AtomicUsize = AtomicUsize::new(0);
impl SymbolCodec<Handle> for BatchCodec {
    fn code(value: &Handle) -> u32 { value.0 }
    fn decode(code: u32) -> Handle {
        SCALAR_DECODES.fetch_add(1, Ordering::Relaxed);
        // DECODE P: diagnostic scalar publication records calls to the non-bulk inverse.
        Handle(code)
    }
    fn decode_many(codes: Vec<u32>) -> Vec<Handle> {
        BATCHES.fetch_add(1, Ordering::Relaxed);
        // DECODE P: the fixture publishes a whole logical handle column in one call.
        codes.into_iter().map(Handle).collect()
    }
}

#[test]
fn dense_symbol_and_signed_columns_publish_in_bulk() -> gpu_datalog_runtime::Result<()> {
    let pool = gpu_datalog_runtime::thread_pool()?;
    let catalog = Arc::new(DomainCatalog::new(pool, vec![
        DomainInput::Dictionary(vec![10, 20]),
        DomainInput::Signed(Some((-2, 3))),
    ])?);
    let rows = vec![(Handle(20), 3i32), (Handle(10), -2)];
    BATCHES.store(0, Ordering::Relaxed);
    SCALAR_DECODES.store(0, Ordering::Relaxed);
    let output = cq_datalog::datalog! {
        pipeline {
            storage_rewrites = [::cq_gpu_datalog::domain_storage(symbols(Handle => [0] BatchCodec))];
            cq = pipeline {
                surface = [::cq_gpu_datalog::encode_domains];
                physical = [::cq_gpu_sequence::capture_constants];
                structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
                sequence = ::mini_linq::passes::sequence;
                decompose = [::cq_gpu_sequence::decompose];
                stage = ::cq_gpu_sequence::stage;
                grade = ::cq_gpu_sequence::machine_grade;
                print = ::cq_gpu_sequence::translate_arena;
            };
            terminals = [::cq_gpu_sequence::translate_arena => batch];
            round = [::cq_gpu_datalog::bind_round(binding = 6)];
            print = ::cq_gpu_datalog::print_rounds(catalog);
        }
        struct Domains;
        relation Input(x:Handle,n:i32) = (rows);
        relation Output(n:i32,x:Handle);
        output Output;
        Output(n,x) :- Input(x,n), where x != Handle(99);
    };
    let mut actual: Vec<_> = output.collect();
    actual.sort_unstable();
    assert_eq!(actual, [(-2, Handle(10)), (3, Handle(20))]);
    assert_eq!(BATCHES.load(Ordering::Relaxed), 1);
    assert_eq!(SCALAR_DECODES.load(Ordering::Relaxed), 0);
    Ok(())
}
