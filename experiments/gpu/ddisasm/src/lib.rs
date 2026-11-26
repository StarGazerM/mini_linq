#![recursion_limit = "2048"]
#![allow(non_snake_case, unused_variables)]

mod ids;

pub fn run(facts: datalog_ddisasm::Facts) -> Result<datalog_ddisasm::Results, &'static str> {
    let (facts, dictionary) = ids::Dictionary::encode(facts)?;
    Ok(engine::evaluate(facts, &dictionary))
}

mod engine {
datalog_ddisasm::ddisasm_engine!(evaluate, u32, [, dictionary: &crate::ids::Dictionary], |row| dictionary.decode(row), pipeline {
    incremental = ::cq_datalog::passes::incremental(driver(delta_first););
    storage_rewrites = [::cq_datalog_canonical::canonical(ordered), ::cq_gpu_datalog::device_storage];
    cq = pipeline {
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs, ::cq_gpu_sequence::select_catalog];
        sequence = ::mini_linq::passes::sequence;
        decompose = [::cq_gpu_sequence::decompose, ::cq_sequence_rewrites::output(reserve_previous), ::cq_sequence_rewrites::roots(spread)];
        stage = ::cq_gpu_sequence::stage;
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_sequence::translate_arena;
    };
    terminals = [::cq_gpu_sequence::translate_arena => batch];
    round = [::cq_gpu_datalog::bind_round(binding = 5, head_cursor)];
    print = ::cq_gpu_datalog::print_rounds;
});
}
