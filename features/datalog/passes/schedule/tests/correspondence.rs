use cq_datalog_testkit::check_refinement;
use std::collections::BTreeSet;

macro_rules! mutual {
    ($rows:expr; $($configuration:tt)*) => {
        cq_datalog::datalog! {
            pipeline { $($configuration)* }
            struct Mutual;
            relation E(x:i32) = ($rows);
            relation A(x:i32);
            relation B(x:i32);
            output A;
            A(*x) :- E(x);
            A(*x) :- B(x);
            A(*x) :- A(x);
            B(*x + 1) :- A(x), if *x < 5;
        }
    };
}

#[test]
fn sequential_facade_schedules_refine_unscheduled_mutual_recursion() {
    for rows in [vec![], vec![(0,), (0,), (2,)], vec![(3,), (7,)]] {
        let base = || mutual!(&rows;).collect::<BTreeSet<_>>();
        check_refinement(base, || {
            mutual!(&rows;
                round = [cq_datalog_schedule::schedule(whole)];
            )
            .collect::<BTreeSet<_>>()
        })
        .unwrap();
        check_refinement(base, || {
            mutual!(&rows;
                round = [cq_datalog_schedule::schedule(per_hole)];
            )
            .collect::<BTreeSet<_>>()
        })
        .unwrap();
        check_refinement(base, || {
            mutual!(&rows;
                round = [cq_datalog_schedule::schedule(by_head)];
            )
            .collect::<BTreeSet<_>>()
        })
        .unwrap();
        check_refinement(base, || {
            mutual!(&rows;
                round = [cq_datalog_schedule::schedule(strata(0 { seed [0] step [0] [1 2] }))];
            )
            .collect::<BTreeSet<_>>()
        })
        .unwrap();
        check_refinement(base, || {
            mutual!(&rows;
                round = [cq_datalog_schedule::schedule(grain(1), inline_below(0))];
            )
            .collect::<BTreeSet<_>>()
        })
        .unwrap();
        check_refinement(base, || {
            mutual!(&rows;
                round = [cq_datalog_schedule::schedule(by_head, grain(2))];
            )
            .collect::<BTreeSet<_>>()
        })
        .unwrap();
    }
}
