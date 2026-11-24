use std::collections::BTreeSet;

#[test]
fn computed_answer_gate_is_evaluated_on_host() {
    for limit in [1i32, 3] {
        let r0 = &[(1i32,)];
        let answer = mini_linq::compile! {
            pipeline {
                physical = [cq_gpu_sequence::capture_constants];
                structures = [cq_gpu_sequence::bind_runs, cq_gpu_sequence::select_catalog];
                sequence = mini_linq::passes::sequence;
                stage = cq_gpu_sequence::stage;
                decompose = [cq_gpu_sequence::decompose];
                grade = cq_gpu_sequence::machine_grade;
                print = cq_gpu_sequence::translate;
            }
            struct Q;
            relation r0(x:i32);
            relation answer(x:i32);
            answer(x) :- r0(x), if (limit != 3);
        };
        let actual: BTreeSet<(i32,)> = answer.to_host().unwrap().into_iter().collect();
        let expected = if limit != 3 {
            BTreeSet::from([(1,)])
        } else {
            BTreeSet::new()
        };
        assert_eq!(actual, expected);
    }
}

#[test]
fn computed_closed_binding_has_a_host_relational_restriction() {
    for limit in [1i32, -1, -2] {
        let r0 = &[(1i32,)];
        let answer = mini_linq::compile! {
            pipeline {
                physical = [cq_gpu_sequence::capture_constants];
                structures = [cq_gpu_sequence::bind_runs, cq_gpu_sequence::select_catalog];
                sequence = mini_linq::passes::sequence;
                stage = cq_gpu_sequence::stage;
                decompose = [cq_gpu_sequence::decompose];
                grade = cq_gpu_sequence::machine_grade;
                print = cq_gpu_sequence::translate;
            }
            struct Q;
            relation r0(x:i32);
            relation answer(x:i32);
            answer(x) :- r0(x), let y: i32 = (limit + 1), where y > 0;
        };
        let actual: BTreeSet<(i32,)> = answer.to_host().unwrap().into_iter().collect();
        let expected = if limit + 1 > 0 {
            BTreeSet::from([(1,)])
        } else {
            BTreeSet::new()
        };
        assert_eq!(actual, expected);
    }
}

#[test]
fn answer_gate_preserves_once_only_capture_and_input_evaluation() {
    for limit in [1i32, 3] {
        let input_calls = std::cell::Cell::new(0);
        let capture_calls = std::cell::Cell::new(0);
        let answer = mini_linq::compile! {
            pipeline {
                physical = [cq_gpu_sequence::capture_constants];
                structures = [cq_gpu_sequence::bind_runs, cq_gpu_sequence::select_catalog];
                sequence = mini_linq::passes::sequence;
                stage = cq_gpu_sequence::stage;
                decompose = [cq_gpu_sequence::decompose];
                grade = cq_gpu_sequence::machine_grade;
                print = cq_gpu_sequence::translate;
            }
            struct Q;
            relation r0(x:i32) = ({ input_calls.set(input_calls.get() + 1); [(1i32,)] });
            relation answer(x:i32);
            answer(x) :- r0(x), if (limit != 3),
                let y: i32 = ({ capture_calls.set(capture_calls.get() + 1); limit + 1 }),
                where y > 0;
        };
        let actual: Vec<(i32,)> = answer.to_host().unwrap();
        let expected = if limit != 3 { vec![(1,)] } else { vec![] };
        assert_eq!(actual, expected);
        assert_eq!(capture_calls.get(), usize::from(limit != 3));
        assert_eq!(input_calls.get(), usize::from(limit != 3));
    }
}

#[test]
fn closed_range_subject_uses_row_upper_bound() {
    let parameter_negative_1 = -1i32;
    let r0 = &[(-3i32,), (0,), (1,), (2,)];
    let answer = mini_linq::compile! {
        pipeline {
            physical = [cq_gpu_sequence::capture_constants];
            structures = [cq_gpu_sequence::bind_runs, cq_gpu_sequence::select_catalog];
            sequence = mini_linq::passes::sequence;
            stage = cq_gpu_sequence::stage;
            decompose = [cq_gpu_sequence::decompose];
            grade = cq_gpu_sequence::machine_grade;
            print = cq_gpu_sequence::translate;
        }
        struct Q;
        relation r0(x:i32);
        relation answer(x:i32, y:i32);
        answer(x, y) :- r0(x), let y: i32 = (parameter_negative_1 + 2),
            where y in -2..=x;
    };
    let actual: BTreeSet<(i32, i32)> = answer.to_host().unwrap().into_iter().collect();
    assert_eq!(actual, BTreeSet::from([(1, 1), (2, 1)]));
}

#[test]
fn closed_range_subject_uses_row_lower_bound() {
    let parameter = -1i32;
    let r0 = &[(-3i32,), (0,), (1,), (2,)];
    let answer = mini_linq::compile! {
        pipeline {
            physical = [cq_gpu_sequence::capture_constants];
            structures = [cq_gpu_sequence::bind_runs, cq_gpu_sequence::select_catalog];
            sequence = mini_linq::passes::sequence;
            stage = cq_gpu_sequence::stage;
            decompose = [cq_gpu_sequence::decompose];
            grade = cq_gpu_sequence::machine_grade;
            print = cq_gpu_sequence::translate;
        }
        struct Q;
        relation r0(x:i32);
        relation answer(x:i32, y:i32);
        answer(x, y) :- r0(x), let y: i32 = (parameter + 2),
            where y in x..=2;
    };
    let actual: BTreeSet<(i32, i32)> = answer.to_host().unwrap().into_iter().collect();
    assert_eq!(actual, BTreeSet::from([(-3, 1), (0, 1), (1, 1)]));
}

#[test]
fn closed_range_subject_uses_two_row_bounds() {
    let parameter = -1i32;
    let r0 = &[(0i32, 2i32), (1, 1), (2, 3), (-2, 0), (2, 0)];
    let answer = mini_linq::compile! {
        pipeline {
            physical = [cq_gpu_sequence::capture_constants];
            structures = [cq_gpu_sequence::bind_runs, cq_gpu_sequence::select_catalog];
            sequence = mini_linq::passes::sequence;
            stage = cq_gpu_sequence::stage;
            decompose = [cq_gpu_sequence::decompose];
            grade = cq_gpu_sequence::machine_grade;
            print = cq_gpu_sequence::translate;
        }
        struct Q;
        relation r0(low:i32, high:i32);
        relation answer(low:i32, high:i32);
        answer(low, high) :- r0(low, high), let y: i32 = (parameter + 2),
            where y in low..=high;
    };
    let actual: BTreeSet<(i32, i32)> = answer.to_host().unwrap().into_iter().collect();
    assert_eq!(actual, BTreeSet::from([(0, 2), (1, 1)]));
}

#[test]
fn gpu_8_closed_range_with_row_bound_replay() {
    let parameter_negative_1 = -1i32;
    let r0 = &[(2i32, -3i32), (2, -1), (0, 0)];
    let answer = mini_linq::compile! {
        pipeline {
            physical = [cq_gpu_sequence::capture_constants];
            structures = [cq_gpu_sequence::bind_runs, cq_gpu_sequence::select_catalog];
            sequence = mini_linq::passes::sequence;
            stage = cq_gpu_sequence::stage;
            decompose = [cq_gpu_sequence::decompose];
            grade = cq_gpu_sequence::machine_grade;
            print = cq_gpu_sequence::translate;
        }
        struct Q;
        relation r0(x:i32, y:i32);
        relation answer(x:i32);
        answer(v2) :- r0(v0, -3),
            let v1: i32 = (v0.to_owned() - -3),
            let v2: i32 = (parameter_negative_1 + 2),
            if ((v2.to_owned() + v0.to_owned()) != (-(2))),
            where v1 != -1, where v2 in -2..=v0;
    };
    let actual: BTreeSet<(i32,)> = answer.to_host().unwrap().into_iter().collect();
    assert_eq!(actual, BTreeSet::from([(1,)]));
}
