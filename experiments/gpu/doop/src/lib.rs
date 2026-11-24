#![recursion_limit = "2048"]

#[cfg(feature = "manual-joins")]
pub mod manual;

#[cfg(feature = "manual-joins")]
pub mod manual_assign;

#[cfg(feature = "manual-joins")]
mod manual_dispatch;

#[cfg(feature = "capture-joins")]
mod capture;

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

/// The engine with its round printer: `print_rounds`, or with hand-written
/// rule holes under the `manual-joins` feature.
macro_rules! engine {
    () => { engine!(@emit [] []); };
    ($kind:ident ($($label:literal => $path:path),+ $(,)?)) => {
        engine!(@emit [, legacy($($label),+)] [, $kind($($label => $path),+)]);
    };
    (@emit [$($legacy:tt)*] [$($hooks:tt)*]) => {
        doop_shared::doop_engine!(run, pipeline {
            incremental = ::cq_datalog::passes::incremental(
                driver(delta_first);
                plan(VPT_Cast, delta(reachable), order(reachable, assigncast, varpointsto, heapallocation_type, supertypeof));
                plan(VPT_Cast, delta(varpointsto), order(varpointsto, assigncast, reachable, heapallocation_type, supertypeof));
                plan(AIPT_Store, delta(varpointsto@2), order(varpointsto@2, storearrayindex, reachable, heapallocation_type@5, componenttype, varpointsto@3, heapallocation_type@4, supertypeof));
                plan(AIPT_Store, delta(varpointsto@3), order(varpointsto@3, storearrayindex, reachable, heapallocation_type@4, varpointsto@2, heapallocation_type@5, componenttype, supertypeof));
                plan(VPT_LoadArray, delta(varpointsto), order(reachable, loadarrayindex, varpointsto, var_type, heapallocation_type, componenttype, arrayindexpointsto, supertypeof));
                plan(VPT_AssignLocal, delta(varpointsto), order(reachable, assignlocal, varpointsto));
                plan(VPT_LoadField, delta(instancefieldpointsto), order(loadinstancefield, reachable, varpointsto, instancefieldpointsto));
                plan(IFPT_Store, delta(varpointsto@3), order(storeinstancefield, reachable, varpointsto@3, varpointsto@2));
                plan(SFPT_Store, delta(varpointsto), order(storestaticfield, reachable, varpointsto));
                plan(Reachable_SpecialCall, delta(varpointsto), order(specialmethodinvocation_base, varpointsto, instruction_method, reachable, methodinvocation_method, thisvar));
                plan(CallGraphEdge_SpecialCall, delta(varpointsto), order(specialmethodinvocation_base, varpointsto, instruction_method, reachable, methodinvocation_method, thisvar));
                plan(VPT_SpecialCall, delta(varpointsto), order(specialmethodinvocation_base, varpointsto, instruction_method, reachable, methodinvocation_method, thisvar));
            );
            storage_rewrites = [::cq_datalog_canonical::canonical(ordered), ::cq_gpu_datalog::device_storage(symbols(IStr => [0] crate::DoopSymbols))];
            cq = pipeline {
                physical = [::cq_gpu_sequence::capture_constants];
                structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs, ::cq_gpu_sequence::select_catalog];
                sequence = ::mini_linq::passes::sequence;
                // Grouped heads reuse a phase hint; other holes keep their own growth slack.
                // Root tiles are sized at launch to fill the device (GPU-RootSpread).
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
            round = [::cq_gpu_datalog::bind_round(binding = 5, head_cursor $($legacy)*)];
            print = ::cq_gpu_datalog::print_rounds(
                frontier("VPT_Cast@0", "VPT_Cast@4", "AIPT_Store@2", "AIPT_Store@3",
                         "VPT_LoadArray@2", "VPT_LoadField@2") $($hooks)*
            );
        });
    };
}

#[cfg(not(any(feature = "manual-joins", feature = "capture-joins")))]
engine!();

#[cfg(all(feature = "manual-joins", not(feature = "capture-joins")))]
engine!(manual(
    "varpointsto@3" => crate::manual_dispatch::vpt,
    "reachable@3" => crate::manual_dispatch::reachable,
    "callgraphedge@3" => crate::manual_dispatch::edge,
    "VPT_Assign@0" => crate::manual_assign::assign_delta,
    "VPT_Assign@1" => crate::manual_assign::points_delta,
    "VPT_LoadArray@3" => crate::manual::vpt_load_array_delta_index
));

// The offline oracle always comes from a generated kernel, even if both
// features are enabled.
#[cfg(feature = "capture-joins")]
engine!(observe(
    "VPT_LoadArray@3" => crate::capture::observe,
    "VPT_SpecialCall@3" => crate::capture::observe,
    "VPT_LoadField@3" => crate::capture::observe,
    "varpointsto@3" => crate::capture::observe,
    "VPT_Cast@4" => crate::capture::observe,
    "VPT_Cast@0" => crate::capture::observe,
    "Reachable_SpecialCall@3" => crate::capture::observe,
    "reachable@3" => crate::capture::observe,
    "CallGraphEdge_SpecialCall@3" => crate::capture::observe,
    "callgraphedge@3" => crate::capture::observe,
    "varpointsto@0" => crate::capture::observe,
    "VPT_Assign@0" => crate::capture::observe,
    "VPT_Assign@1" => crate::capture::observe,
    "VPT_AssignLocal@0" => crate::capture::observe,
    "VPT_LoadField@0" => crate::capture::observe,
    "VPT_SpecialCall@0" => crate::capture::observe,
));
