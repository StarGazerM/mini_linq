//! Borrowed String layouts remain immutable across scoped worker evaluation.
use std::collections::BTreeSet;

#[test]
fn owned_columns_recursive_probes_and_frozen_negation_match_ascent() {
    rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(|| {
        let edges: Vec<_> = (0..1100).flat_map(|i| {
            let a = format!("{i}:a:λ");
            let b = format!("{i}:b:λ");
            let c = format!("{i}:c:λ");
            [(a.clone(), b.clone()), (b, c), (a.clone(), a)]
        }).collect();
        let blocked: Vec<_> = (0..1100).step_by(3).map(|i| (format!("{i}:c:λ"),)).collect();
        macro_rules! run {
            ($($config:tt)*) => {{
                let (reach, visible, input) = cq_datalog::datalog! {
                    pipeline { $($config)* }
                    struct Owned;
                    relation Edge(x:String,y:String) = (&edges);
                    relation Blocked(x:String) = (&blocked);
                    relation Reach(x:String,y:String);
                    relation Visible(x:String,y:String);
                    output Reach, Visible, Edge;
                    Reach((*x).clone(),(*y).clone()) :- Edge(x,y);
                    Reach((*x).clone(),(*z).clone()) :- Reach(x,y), Reach(y,z);
                    Reach((*x).clone(),(*z).clone()) :- Edge(y,z), Reach(x,y);
                    Visible((*x).clone(),(*y).clone()) :- Reach(x,y), !Blocked(y);
                };
                (reach.collect::<BTreeSet<_>>(), visible.collect::<BTreeSet<_>>(), input.collect::<BTreeSet<_>>())
            }};
        }
        let expected = run!();
        let actual = run!(round=[cq_datalog_parallel::partition]; print=cq_datalog_parallel::print_rounds;);
        assert_eq!(actual, expected);
        ascent::ascent! {
            struct Oracle;
            relation edge(String,String);
            relation blocked(String);
            relation reach(String,String);
            relation visible(String,String);
            reach(x.clone(),y.clone()) <-- edge(x,y);
            reach(x.clone(),z.clone()) <-- reach(x,y), reach(y,z);
            reach(x.clone(),z.clone()) <-- edge(y,z), reach(x,y);
            visible(x.clone(),y.clone()) <-- reach(x,y), !blocked(y);
        }
        let mut oracle = Oracle { edge: edges, blocked, ..Default::default() };
        oracle.run();
        assert_eq!(actual.0, oracle.reach.into_iter().collect());
        assert_eq!(actual.1, oracle.visible.into_iter().collect());
        assert_eq!(actual.2, oracle.edge.into_iter().collect());
    });
}
