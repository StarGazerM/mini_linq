#![cfg(feature = "sequence-conformance")]

use cq_testkit::conformance::{Target, assert_conformant, assert_conformant_with_inputs, grade};

fn program() -> cq_lang_sequence::Program {
    syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32);
            consumer canonical;
            r = pipe rename R@0 {a -> a,b -> b};
            return r as answer(b,a);
        }
        structures {
            r host scan = ::core::iter::Map<_,_> [];
        }
        sequence {
            input = store rows r;
            result = store input;
        }
    }
}

#[test]
fn identical_programs_compare_after_the_answer_map_with_duplicate_inputs() {
    let program = program();
    let targets = [Target::Sequential, Target::Tasks];
    assert_conformant(&program, &targets);
    assert_conformant_with_inputs(
        &program,
        &targets,
        &[("R", &[&[1, 20], &[1, 20], &[2, 10], &[3, 20]])],
    );
}

#[test]
fn grading_changes_only_placement_and_preserves_authored_tasks_grades() {
    let original = program();
    let (tasks, _) = grade(&original, Target::Tasks).unwrap();
    assert_eq!(tasks.structures, original.structures);
    assert_eq!(tasks.sequence, original.sequence);
    assert_eq!(grade(&tasks, Target::Tasks).unwrap().0, tasks);
    let (sequential, _) = grade(&tasks, Target::Sequential).unwrap();
    assert_eq!(sequential, original);
}
