//! Exercise the public stage macros, generated providers, and CQ hole dedup.
use std::collections::BTreeSet;

macro_rules! closure {
    ($edges:expr, $blocked:expr; $($configuration:tt)*) => {
        cq_datalog::datalog! {
            pipeline { $($configuration)* }
            struct Closure;
            relation Edge(x:i32,y:i32) = ($edges);
            relation Blocked(x:i32) = ($blocked);
            relation Reach(x:i32,y:i32);
            relation Allowed(x:i32,y:i32);
            output Allowed;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
            Allowed(*x,*y) :- Reach(x,y), !Blocked(y);
        }
    };
}

#[test]
fn recursive_duplicate_heavy_negation_matches_base_and_restored_default() {
    let edges = [(1, 2), (1, 2), (2, 3), (3, 4), (2, 4), (4, 1)];
    let blocked = [(4,), (4,)];
    let base: BTreeSet<_> = closure!(edges, blocked;).collect();
    let fx: Vec<_> = closure!(edges, blocked;
        storage_rewrites = [cq_datalog_hasher::hasher(fx)];
        round = [cq_datalog_hasher::hasher_round(fx)];
        cq = pipeline {
            structures = [cq_rewrites::retype(dedup =>
                ::std::collections::HashSet<_,
                    ::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>>)];
        };
    )
    .collect();
    let restored: BTreeSet<_> = closure!(edges, blocked;
        storage_rewrites = [cq_datalog_hasher::hasher(fx), cq_datalog_hasher::hasher(default)];
        round = [cq_datalog_hasher::hasher_round(fx), cq_datalog_hasher::hasher_round(default)];
    )
    .collect();
    let expected: BTreeSet<_> = (1..=4).flat_map(|x| (1..=3).map(move |y| (x, y))).collect();
    assert_eq!(base, expected);
    assert_eq!(fx.len(), expected.len());
    assert_eq!(fx.into_iter().collect::<BTreeSet<_>>(), expected);
    assert_eq!(restored, expected);
}

#[test]
fn borrowed_string_edbs_and_owned_outputs_use_selected_canonical_sets() {
    let output: std::vec::IntoIter<(String,)> = {
        let rows = vec![
            (String::from("alpha"),),
            (String::from("alpha"),),
            (String::from("beta"),),
        ];
        cq_datalog::datalog! {
            pipeline {
                storage_rewrites = [cq_datalog_hasher::hasher(fx)];
                round = [cq_datalog_hasher::hasher_round(fx)];
            }
            struct CanonicalOutput;
            relation Names(name:String) = (&rows);
            output Names;
        }
    };
    let rows: Vec<_> = output.collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(String::from("alpha"),), (String::from("beta"),)])
    );
}

#[test]
fn empty_inputs_and_empty_idbs_remain_empty() {
    let edges: [(i32, i32); 0] = [];
    let blocked: [(i32,); 0] = [];
    let answer: Vec<_> = closure!(edges, blocked;
        storage_rewrites = [cq_datalog_hasher::hasher(fx)];
        round = [cq_datalog_hasher::hasher_round(fx)];
    )
    .collect();
    assert!(answer.is_empty());
}

#[test]
fn zero_column_negation_keys_preserve_existential_blocking() {
    let rows = [(1_i32,), (1,), (2,)];
    let blocked = [(9_i32,), (9,)];
    let answer: Vec<_> = cq_datalog::datalog! {
        pipeline {
            storage_rewrites = [cq_datalog_hasher::hasher(fx)];
            round = [cq_datalog_hasher::hasher_round(fx)];
            cq = pipeline {
                structures = [cq_rewrites::retype(dedup =>
                    ::std::collections::HashSet<_,
                        ::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>>)];
            };
        }
        struct Existential;
        relation E(x:i32) = (rows);
        relation Blocked(x:i32) = (blocked);
        relation Allowed(x:i32);
        output Allowed;
        Allowed(*x) :- E(x), !Blocked(_);
    }
    .collect();
    assert!(answer.is_empty());
}
