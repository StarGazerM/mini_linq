#![recursion_limit = "2048"]

#[cfg(any(
    all(feature = "advice-aipt", any(feature = "advice-load", feature = "advice-fieldflow", feature = "advice-dispatch")),
    all(feature = "advice-load", any(feature = "advice-fieldflow", feature = "advice-dispatch")),
    all(feature = "advice-fieldflow", feature = "advice-dispatch"),
    all(feature = "default-query", any(feature = "advice-aipt", feature = "advice-load", feature = "advice-fieldflow", feature = "advice-dispatch"))
))]
compile_error!("select at most one advice experiment, using the factored SRD query");

// Baseline and individually authored P3.2 experiments share the same pipeline.
macro_rules! engine {
    ($($advice:tt)*) => {
srd_doop_shared::srd_doop_engine!(run, incremental(driver(delta_first); $($advice)*), pipeline {
    storage_rewrites = [::cq_gpu_datalog::device_storage];
    cq = pipeline {
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs, ::cq_gpu_sequence::select_catalog];
        stage = ::cq_gpu_sequence::stage;
        sequence = ::mini_linq::passes::sequence;
        // Reuse only each arena hole's last validated count as a capacity hint.
        decompose = [
            ::cq_gpu_sequence::decompose,
            ::cq_sequence_rewrites::output(reserve_previous),
        ];
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_sequence::translate_arena;
    };
    terminals = [::cq_gpu_sequence::translate_arena => batch];
    round = [::cq_gpu_datalog::bind_round(binding = 5)];
    print = ::cq_gpu_datalog::print_rounds;
});
    };
}

#[cfg(not(any(feature = "advice-aipt", feature = "advice-load",
    feature = "advice-fieldflow", feature = "advice-dispatch")))]
engine!();

#[cfg(feature = "advice-aipt")]
engine!(
    plan(AIPT_Store, delta(r_reachablesortedindex), order(r_reachablesortedindex, r_varpointsto@1, r_varpointsto@2(v_heap, v_frm), r_nonstringheapallocsupertype),
        smallest(r_varpointsto@2, r_nonstringheapallocsupertype));
    plan(AIPT_Store, delta(r_varpointsto@1), order(r_varpointsto@1, r_reachablesortedindex, r_varpointsto@2(v_heap, v_frm), r_nonstringheapallocsupertype),
        smallest(r_varpointsto@2, r_nonstringheapallocsupertype));
    plan(AIPT_Store, delta(r_varpointsto@2), order(r_varpointsto@2, r_reachablesortedindex, r_varpointsto@1(v_baseheap, v_base), r_nonstringheapallocsupertype),
        smallest(r_varpointsto@1, r_nonstringheapallocsupertype));
);

#[cfg(feature = "advice-load")]
engine!(
    plan(VPT_LoadArray, delta(r_reachable), order(r_reachable, r_loadarrayindex, r_var_type, r_varpointsto, r_arraytypecompat, r_arrayindexpointsto),
        smallest(r_varpointsto, r_arraytypecompat));
);

#[cfg(feature = "advice-fieldflow")]
engine!(
    // The existing packed layout cannot realize smallest(fld -> baseheap).
    // This supported control adds only the membership lookup before expansion.
    plan(FieldFlow_rule, delta(r_reachableloadinstancefield), order(r_reachableloadinstancefield, r_varpointsto, r_storeobjfrom(v_baseheap, v_fld), r_storeobjfrom));
    plan(FieldFlow_rule, delta(r_varpointsto), order(r_varpointsto, r_reachableloadinstancefield, r_storeobjfrom(v_baseheap, v_fld), r_storeobjfrom));
);

#[cfg(feature = "advice-dispatch")]
engine!(
    // Smallest's canonical prefix order differs from the packed helper layout.
    plan(VirtualDispatch_MultiHead_0, delta(r_reachableinstruction), order(r_reachableinstruction, r_virtualmethodinvocation, r_varpointsto, r_heaphelper(v_simplename, v_descriptor, v_heap), r_heaphelper));
    plan(VirtualDispatch_MultiHead_1, delta(r_reachableinstruction), order(r_reachableinstruction, r_virtualmethodinvocation, r_varpointsto, r_heaphelper(v_simplename, v_descriptor, v_heap), r_heaphelper));
    plan(VirtualDispatch_MultiHead_2, delta(r_reachableinstruction), order(r_reachableinstruction, r_virtualmethodinvocation, r_varpointsto, r_heaphelper(v_simplename, v_descriptor, v_heap), r_heaphelper));
);
