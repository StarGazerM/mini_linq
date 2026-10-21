//! Checked per-hole CQ choices and the optional Datalog pull adapter.
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
                    Reach(*x + 1) :- Reach(x), if *x < 70;
                    Reach(*x + 2) :- Reach(x), if *x < 69;
                }.collect::<BTreeSet<_>>()
            };
        }
        let default = evaluate!();
        let rewritten = evaluate! {
            round = [cq_datalog_test_passes::per_hole_pipeline(1, 2)];
        };
        let partitioned = evaluate! {
            terminals = [
                ::mini_linq::passes::print_pull => pull,
                ::cq_datalog::passes::print_hole => pull
            ];
            round = [cq_datalog_test_passes::per_hole_pipeline(1, 2),
                cq_datalog_test_passes::partition_holes];
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
        assert_eq!(partitioned, default);
    }
}

#[test]
fn partitioned_rule_and_key_holes_preserve_local_types_and_captures() {
    #[derive(Clone, Debug, Eq, PartialEq, Hash)]
    struct Payload(String);
    struct Facts {
        rows: Vec<(i32, Payload)>,
        limit: i32,
    }
    let facts = Facts {
        rows: vec![(0, Payload(String::from("keep")))],
        limit: 4,
    };
    let blocked = [(3_i32,)];
    let prefix_owner = String::from("ke");
    let prefix = &prefix_owner;
    let increment = |value: i32| value + 1;
    let mut rounds = Vec::new();
    let answers: BTreeSet<_> = cq_datalog::datalog! {
        pipeline {
            terminals = [
                ::mini_linq::passes::print_pull => pull,
                ::cq_datalog::passes::print_hole => pull
            ];
            round = [cq_datalog_test_passes::partition_holes];
        }
        observe = (|round, count| rounds.push((round, count)));
        struct Captures;
        relation Input(x:i32,payload:Payload) = (&facts.rows);
        relation Blocked(x:i32) = (&blocked);
        relation Reach(x:i32);
        relation Allowed(x:i32);
        output Allowed;
        Reach(*x) :- Input(x,payload), if payload.0.starts_with(prefix.as_str());
        Reach(increment(*x)) :- Reach(x), if *x < facts.limit;
        Allowed(*x) :- Reach(x), !Blocked(x);
    }
    .collect();
    assert_eq!(answers, BTreeSet::from([(0,), (1,), (2,), (4,)]));
    assert_eq!(facts.rows[0].1.0, "keep");
    assert_eq!(prefix_owner, "ke");
    assert!(!rounds.is_empty());
}

#[test]
fn explicit_partition_preserves_nonstatic_borrowed_rows() {
    let labels = [String::from("keep"), String::from("drop")];
    let rows = [(labels[0].as_str(),), (labels[1].as_str(),)];
    let wanted = String::from("keep");
    let wanted_ref = &wanted;
    let answers: BTreeSet<_> = cq_datalog::datalog! {
        pipeline {
            terminals = [::cq_datalog::passes::print_hole => pull];
            cq = pipeline { print = ::cq_datalog::passes::print_hole(partition(7)); };
        }
        struct Borrowed;
        relation Input(value:&str) = (&rows);
        relation Output(value:&str);
        output Output;
        Output(*value) :- Input(value), if *value == wanted_ref.as_str();
    }
    .collect();
    assert_eq!(answers, BTreeSet::from([("keep",)]));
    assert_eq!(labels[0], wanted);
}
