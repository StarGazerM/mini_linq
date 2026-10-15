//! Row counts are checked against hand-enumerated inputs, independently of the printer.
use cq_storage::counters;

#[test]
fn counts_filter_join_dedup_and_final_answer() {
    let l = [(1_i32,), (1,), (2,), (3,)];
    let r = [(1_i32, 10_i32), (1, 11), (2, 20), (9, 90)];
    counters::reset();
    let rows: Vec<_> = mini_linq::compile! {
        pipeline { plan = mini_linq::passes::plan(no_reduce); }
        struct ProfileJoin;
        relation L(a:i32) = (l);
        relation R(a:i32,b:i32) = (r);
        answer(a,b) :- L(a), R(a,b), if *b < 20;
    }
    .collect();
    assert_eq!(
        rows.into_iter()
            .map(|(a, b)| (*a, *b))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([(1, 10), (1, 11)])
    );
    let ops = counters::operators();
    println!("{}", counters::render_operators(&ops));
    let answer = ops.iter().find(|r| r.label.kind == "ANSWER").unwrap();
    assert_eq!(answer.rows_out, 2);
    assert!(
        ops.iter()
            .any(|r| r.label.kind == "RANGE" && r.rows_in == 4 && r.rows_out == 2)
    );
    assert!(
        ops.iter()
            .any(|r| r.label.kind == "DEDUP" && r.rows_in > r.rows_out)
    );
}

#[test]
fn positional_selection_counts_survivors() {
    let l = [(1_i64, 10_i64), (1, 20), (2, 30), (3, 40), (5, 15)];
    let r = [(1_i64, 1_i64), (2, 0), (3, 1), (9, 1)];
    counters::reset();
    let answer = cq_testkit::positional_reduced!(mini_linq::passes::print_pull, l, r);
    assert_eq!(answer.len(), 2);
    let ops = counters::operators();
    println!("{}", counters::render_operators(&ops));
    assert!(ops.iter().any(|r| r.rows_inserted > 0));
}

#[test]
fn generated_profile_handle_does_not_capture_rust_names() {
    let __cq_profile_0 = 2;
    let __cq_profile = 1;
    let data = [(1_i32,), (2,), (3,)];
    let rows: Vec<_> = mini_linq::compile! {
        pipeline {}
        struct Hygiene; relation R(a:i32) = (data);
        answer(a) :- R(a), if *a > __cq_profile && *a <= __cq_profile_0;
    }
    .map(|(a,)| *a)
    .collect();
    assert_eq!(rows, [2]);
    let rows: Vec<_> = mini_linq::compile! {
        pipeline {}
        struct HeadHygiene; relation R(a:i32) = (data);
        relation answer(value:i32);
        answer(*a + __cq_profile_0) :- R(a);
    }
    .map(|(a,)| a)
    .collect();
    assert_eq!(rows, [3, 4, 5]);
}

#[test]
fn fused_smallest_results_share_the_combined_operator_span() {
    let r = [(1_i32, 2_i32), (1, 3), (4, 5)];
    let s = [(2_i32, 6_i32), (2, 7), (3, 6), (5, 8)];
    let t = [(1_i32, 6_i32), (4, 8), (4, 9)];
    counters::reset();
    let rows:std::collections::BTreeSet<_>=mini_linq::compile! {
        pipeline {plan = core_pass_fixtures::plan_generic; physical = [core_pass_fixtures::canonical_projects, core_pass_fixtures::smallest];}
        struct FusedProfile;
        relation R(a:i32,b:i32)=(r); relation S(b:i32,c:i32)=(s); relation T(a:i32,c:i32)=(t);
        answer(a,b,c) :- R(a,b), S(b,c), T(a,c);
    }.map(|(a,b,c)|(*a,*b,*c)).collect();
    assert_eq!(
        rows,
        std::collections::BTreeSet::from([(1, 2, 6), (1, 3, 6), (4, 5, 8)])
    );
    for op in counters::operators()
        .iter()
        .filter(|r| r.label.kind == "PROBE" && r.rows_out > 0)
    {
        assert!(
            op.elapsed > std::time::Duration::ZERO,
            "missing span for {}",
            op.label.result
        );
    }
}

#[test]
fn positional_projects_count_their_input_rows() {
    let data = [(1_i64, 10_i64), (2, 20), (3, 30), (3, 40)];
    counters::reset();
    let rows: Vec<_> = mini_linq::compile! {
        pipeline {select = mini_linq::passes::select(positional(R));}
        struct ProjectProfile; relation R(x:i64,y:i64)=(data);
        answer(x,y) :- R(x,y), if *x >= 2;
    }
    .collect();
    assert_eq!(rows.len(), 3);
    let records = counters::operators();
    let projects: Vec<_> = records
        .iter()
        .filter(|r| r.label.kind == "PROJECT" && r.rows_out > 0)
        .collect();
    assert!(!projects.is_empty());
    for p in projects {
        assert_eq!(p.rows_in, p.rows_out);
    }
}
