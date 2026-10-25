#![recursion_limit = "2048"]

/// The dataset's process-global dictionary is the only symbol representation.
/// Device kernels receive its integer codes and never resolve strings.
pub struct DoopSymbols;

impl gpu_datalog_runtime::SymbolCodec<doop_shared::IStr> for DoopSymbols {
    fn code(value: &doop_shared::IStr) -> u32 {
        value.code()
    }

    fn decode(code: u32) -> doop_shared::IStr {
        doop_shared::IStr::from_code(code)
    }
}

doop_shared::doop_engine!(run, pipeline {
    storage_rewrites = [::cq_gpu_datalog::device_storage(symbols(IStr => crate::DoopSymbols))];
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
