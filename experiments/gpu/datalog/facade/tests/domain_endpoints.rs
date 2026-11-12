use std::sync::Arc;
use gpu_datalog_runtime::{DomainCatalog, DomainInput, SymbolCodec};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Handle(u32);
struct Codec;
impl SymbolCodec<Handle> for Codec {
    fn code(value: &Handle) -> u32 { value.0 }
    // DECODE P: endpoint-test publication restores final logical handles.
    fn decode(code: u32) -> Handle { Handle(code) }
}

#[test]
fn absent_ordered_endpoints_inside_and_outside_the_domain_are_exact() -> gpu_datalog_runtime::Result<()> {
    let catalog = Arc::new(DomainCatalog::new(gpu_datalog_runtime::thread_pool()?, vec![
        DomainInput::Dictionary(vec![10, 30]),
    ])?);
    let rows = vec![(Handle(10),), (Handle(30),)];
    let (lower_all, upper_all, lower_none, upper_none, below, above) = cq_datalog::datalog! {
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
            print = ::cq_gpu_datalog::print_rounds(catalog);
        }
        struct Endpoints;
        relation Input(x:Handle) = (rows);
        relation LowerAll(x:Handle);
        relation UpperAll(x:Handle);
        relation LowerNone(x:Handle);
        relation UpperNone(x:Handle);
        relation Below(x:Handle);
        relation Above(x:Handle);
        output LowerAll, UpperAll, LowerNone, UpperNone, Below, Above;
        LowerAll(x) :- Input(x), where x < Handle(40);
        UpperAll(x) :- Input(x), where x > Handle(5);
        LowerNone(x) :- Input(x), where x < Handle(5);
        UpperNone(x) :- Input(x), where x > Handle(40);
        Below(x) :- Input(x), where x <= Handle(20);
        Above(x) :- Input(x), where x >= Handle(20);
    };
    assert_eq!(lower_all.collect::<Vec<_>>(), [(Handle(10),), (Handle(30),)]);
    assert_eq!(upper_all.collect::<Vec<_>>(), [(Handle(10),), (Handle(30),)]);
    assert_eq!(lower_none.count(), 0);
    assert_eq!(upper_none.count(), 0);
    assert_eq!(below.collect::<Vec<_>>(), [(Handle(10),)]);
    assert_eq!(above.collect::<Vec<_>>(), [(Handle(30),)]);
    Ok(())
}
