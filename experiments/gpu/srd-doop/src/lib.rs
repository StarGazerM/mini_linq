#![recursion_limit = "2048"]

srd_doop_shared::srd_doop_engine!(run, pipeline {
    incremental = ::cq_datalog::passes::incremental(driver(delta_first););
    storage_rewrites = [::cq_gpu_datalog::device_storage];
    cq = pipeline {
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
        sequence = ::mini_linq::passes::sequence;
        // Reuse only each arena hole's last validated count as a capacity hint.
        decompose = [
            ::cq_gpu_sequence::plan,
            ::cq_sequence_rewrites::output(reserve_previous),
        ];
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_sequence::translate_arena;
    };
    terminals = [::cq_gpu_sequence::translate_arena => batch];
    print = ::cq_gpu_datalog::print_rounds;
});
