//! SRDatalog DOOP `HeapAllocHelper` as a Datalog rule on the GPU route: an
//! acyclic body with a captured-constant selection written last. CPU and GPU
//! must agree exactly; the plan must be the output-bounded one (see
//! doc/PLANNING-REDESIGN.md and facade/mini-linq/tests/doop_heapalloc.rs).
use std::collections::BTreeSet;

struct Constants {
    c_object_array: i32,
    c_string_array: i32,
}

macro_rules! engine {
    ($name:ident, {$($pipeline:tt)*}) => {
        fn $name(
            alloc: Vec<(i32, i32)>,
            component: Vec<(i32, i32)>,
            supertype: Vec<(i32, i32)>,
            constants: &Constants,
        ) -> BTreeSet<(i32, i32)> {
            let answer = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Heaps;
                relation Alloc(h: i32, t: i32) = (alloc);
                relation Component(a: i32, c: i32) = (component);
                relation Super(c: i32, t: i32) = (supertype);
                relation Compat(h: i32, b: i32);
                output Compat;
                HeapAllocHelper: Compat(*heap, *baseheap) :-
                    Alloc(heap, heaptype), Alloc(baseheap, baseheaptype),
                    Component(baseheaptype, comptype), Super(comptype, heaptype),
                    if *baseheaptype != constants.c_object_array && *baseheaptype != constants.c_string_array;
            };
            answer.collect()
        }
    };
}

engine!(cpu, {});
engine!(gpu, {
    storage_rewrites = [::cq_gpu_datalog::device_storage];
    cq = pipeline {
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs, ::cq_gpu_sequence::select_catalog];
        sequence = ::mini_linq::passes::sequence;
        decompose = [::cq_gpu_sequence::plan];
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_sequence::translate_arena;
    };
    terminals = [::cq_gpu_sequence::translate_arena => batch];
    round = [::cq_gpu_datalog::bind_round(binding = 5)];
    print = ::cq_gpu_datalog::print_rounds;
});

const OBJECT_ARRAY: i32 = 1;
const STRING_ARRAY: i32 = 2;
const INT_ARRAY: i32 = 3;
const ELEMENT: [i32; 3] = [11, 12, 13];

fn instance() -> (Vec<(i32, i32)>, Vec<(i32, i32)>, Vec<(i32, i32)>) {
    let mut alloc = Vec::new();
    let mut heap = 0;
    for (ty, count) in [(OBJECT_ARRAY, 100), (STRING_ARRAY, 100), (INT_ARRAY, 20), (ELEMENT[0], 100), (ELEMENT[1], 100), (ELEMENT[2], 100)] {
        for _ in 0..count {
            alloc.push((heap, ty));
            heap += 1;
        }
    }
    let component = vec![(OBJECT_ARRAY, ELEMENT[0]), (STRING_ARRAY, ELEMENT[1]), (INT_ARRAY, ELEMENT[2])];
    let supertype = ELEMENT.iter().map(|&e| (e, e)).chain([(ELEMENT[2], OBJECT_ARRAY)]).collect();
    (alloc, component, supertype)
}

#[test]
fn heapalloc_matches_cpu_exactly() {
    let (alloc, component, supertype) = instance();
    let constants = Constants { c_object_array: OBJECT_ARRAY, c_string_array: STRING_ARRAY };
    let expected = cpu(alloc.clone(), component.clone(), supertype.clone(), &constants);
    assert_eq!(expected.len(), 4000);
    assert_eq!(gpu(alloc, component, supertype, &constants), expected);
    assert!(gpu(Vec::new(), Vec::new(), Vec::new(), &constants).is_empty());
}
