#![recursion_limit = "2048"]

/// The dataset's process-global dictionary is the only symbol representation.
/// Device kernels receive its integer codes and never resolve strings.
pub struct DoopSymbols;

impl gpu_datalog_runtime::SymbolCodec<doop_shared::IStr> for DoopSymbols {
    fn code(value: &doop_shared::IStr) -> u32 {
        value.code()
    }

    // DECODE P: datalog output printer invokes DoopSymbols only for final host answer symbols.
    fn decode(code: u32) -> doop_shared::IStr {
        // DECODE P: process-global symbol code inversion restores the final DOOP host answer handle.
        doop_shared::IStr::from_code(code)
    }
}

doop_shared::doop_engine!(run, pipeline {
    incremental = ::cq_datalog::passes::incremental(driver(delta_first););
    storage_rewrites = [::cq_gpu_datalog::device_storage(symbols(IStr => [0] crate::DoopSymbols))];
    cq = pipeline {
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs, ::cq_gpu_sequence::select_catalog];
        sequence = ::mini_linq::passes::sequence;
        // Each arena hole reuses only its last validated count as a capacity hint;
        // root tiles are sized at launch to fill the device (GPU-RootSpread).
        // With relational heads (c1b75718) spread measured 0.953x [0.923, 0.960]
        // of no spread on Cassandra (P6b).
        decompose = [
            ::cq_gpu_sequence::decompose(codecs [
                CodecToken {
                    codec_id: Some(0),
                    codec: Some(crate::DoopSymbols),
                    ordered: true,
                    levels: [[host], [host, device], [host, device, block], [host, device, block, lane]],
                }
            ]),
            ::cq_sequence_rewrites::output(reserve_previous),
            ::cq_sequence_rewrites::roots(spread),
        ];
        stage = ::cq_gpu_sequence::stage;
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_sequence::translate_arena;
    };
    terminals = [::cq_gpu_sequence::translate_arena => batch];
    // DL-WaveLanes: four streams measured 0.723x [0.702, 0.765] of one on Cassandra.
    round = [::cq_gpu_datalog::bind_round(binding = 5), ::cq_gpu_datalog::lanes(streams = 4)];
    print = ::cq_gpu_datalog::print_rounds;
});
