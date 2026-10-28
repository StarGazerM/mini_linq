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
    incremental = ::cq_datalog::passes::incremental(driver(delta_first););
    storage_rewrites = [::cq_gpu_datalog::device_storage(symbols(IStr => crate::DoopSymbols))];
    cq = pipeline {
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs, ::cq_gpu_sequence::select_catalog];
        sequence = ::mini_linq::passes::sequence;
        // Each arena hole reuses only its last validated count as a capacity hint.
        decompose = [
            ::cq_gpu_sequence::plan,
            ::cq_sequence_rewrites::output(reserve_previous),
        ];
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_sequence::translate_arena;
    };
    terminals = [::cq_gpu_sequence::translate_arena => batch];
    round = [::cq_gpu_datalog::bind_round(binding = 5)];
    print = ::cq_gpu_datalog::print_rounds;
});
