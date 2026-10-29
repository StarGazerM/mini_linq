//! Checked per-hole CQ choices preserve default and reference execution.
//! Typed references compare the recorded holes before and after their rewrites.
use std::collections::BTreeSet;

#[test]
fn independently_selected_batch_and_positions_holes_match_default_and_ascent() {
    for seeds in [Vec::<(i32,)>::new(), vec![(1,), (1,), (8,)], vec![(0,)]] {
        macro_rules! evaluate {
            ($($configuration:tt)*) => {
                cq_datalog::datalog! {
                    pipeline { $($configuration)* }
                    struct Reachability;
                    relation Seed(x:i32) = (&seeds);
                    relation Reach(x:i32);
                    output Reach;
                    Reach(*x) :- Seed(x);
                    Reach(*x + 1) :- Reach(x), where x < 70;
                    Reach(*x + 2) :- Reach(x), where x < 69;
                }.into_iter().collect::<BTreeSet<_>>()
            };
        }
        let default = evaluate!();
        let rewritten = evaluate! {
            round = [cq_datalog_test_passes::per_hole_pipeline(1, 2)];
        };
        let oracle = ascent::ascent_run! {
            relation seed(i32) = seeds.clone();
            relation reach(i32);
            reach(x) <-- seed(x);
            reach(x + 1) <-- reach(x), if *x < 70;
            reach(x + 2) <-- reach(x), if *x < 69;
        };
        let oracle: BTreeSet<_> = oracle.reach.into_iter().collect();
        assert_eq!(default, oracle);
        assert_eq!(rewritten, default);
        let incremental = evaluate! {
            print = cq_datalog_test_passes::eval_incremental_from_round;
        };
        let reference = evaluate! { print = cq_datalog_test_passes::eval_round; };
        let rewritten_reference = evaluate! {
            round = [cq_datalog_test_passes::per_hole_pipeline(1, 2)];
            print = cq_datalog_test_passes::eval_round;
        };
        assert_eq!(incremental, oracle);
        assert_eq!(reference, incremental);
        assert_eq!(rewritten_reference, reference);
    }
}
