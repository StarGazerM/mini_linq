use std::collections::BTreeSet;

#[test]
fn negative_source_keeps_independent_literal_restrictions() {
    let r0 = &[(1i32,)];
    let r1 = &[(0i32, 1i32, 0i32)];
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
        relation r1(a:i32,x:i32,b:i32);
        relation answer(x:i32);
        answer(x) :- r0(x), !r1(1,x,1);
    };
    let actual: BTreeSet<(i32,)> = answer.to_host().unwrap().into_iter().collect();
    assert_eq!(actual, BTreeSet::from([(1,)]));
}

#[test]
fn variable_range_keeps_signed_candidates() {
    let r0 = &[(-1i32,)];
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
        answer(x) :- r0(x), where x in -3..=x;
    };
    let actual: BTreeSet<(i32,)> = answer.to_host().unwrap().into_iter().collect();
    assert_eq!(actual, BTreeSet::from([(-1,)]));
}

#[test]
fn selected_join_keeps_signed_keys() {
    let r0 = &[(1i32, -1i32)];
    let r1 = &[(-1i32,)];
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
        relation r0(a:i32,x:i32);
        relation r1(x:i32);
        relation answer(x:i32);
        answer(x) :- r1(x), r0(1,x);
    };
    let actual: BTreeSet<(i32,)> = answer.to_host().unwrap().into_iter().collect();
    assert_eq!(actual, BTreeSet::from([(-1,)]));
}
