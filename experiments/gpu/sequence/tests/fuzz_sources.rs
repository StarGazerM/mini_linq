use std::collections::BTreeSet;

#[test]
fn self_join_catalog_retains_host_reads() {
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
        answer(x) :- r0(x), r0(x);
    };
    let actual: BTreeSet<(i32,)> = answer.to_host().unwrap().into_iter().collect();
    assert_eq!(actual, BTreeSet::from([(1,)]));
}

#[test]
fn filtered_cartesian_source_records_rows_and_length() {
    for (row, expected) in [
        ((2i32, 3i32), BTreeSet::from([(1,)])),
        ((3i32, 2i32), BTreeSet::new()),
    ] {
        let r0 = &[row];
        let r1 = &[(1i32,)];
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
            relation r0(a:i32,b:i32);
            relation r1(x:i32);
            relation answer(x:i32);
            answer(x) :- r0(a,b), r1(x), if a.to_owned() < b.to_owned();
        };
        let actual: BTreeSet<(i32,)> = answer.to_host().unwrap().into_iter().collect();
        assert_eq!(actual, expected);
    }
}
