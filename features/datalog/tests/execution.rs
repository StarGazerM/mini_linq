//! Datalog entry stages, native negation, and fixed-point execution.

use std::collections::BTreeSet;

#[test]
fn documented_language_entry_executes_without_a_surface_wrapper() {
    let edges = [(1_i32, 2_i32), (2, 3)];
    let answers: BTreeSet<_> = cq_datalog::datalog! {
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Reach(x,y), Reach(y,z);
    }.collect();
    assert_eq!(answers, BTreeSet::from([(1, 2), (1, 3), (2, 3)]));
}

#[test]
#[deny(non_snake_case)]
fn source_stratified_and_seminaive_text_have_the_same_answer() {
    let edges = [(1, 2), (2, 3), (3, 4), (1, 2)];
    let source: BTreeSet<_> = cq_datalog::datalog! {
            struct Closure;
            relation Edge(x:i32,y:i32) = (edges);
            relation Reach(x:i32,y:i32);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
    }.collect();
    let stratified: BTreeSet<_> = cq_datalog::stratified! {
            struct Closure;
            relation Edge(x:i32,y:i32) = (edges);
            relation Reach(x:i32,y:i32);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
            strata { component [Reach]; }
    }.collect();
    let seminaive: BTreeSet<_> = cq_datalog::seminaive! {
            struct Closure;
            relation Edge(x:i32,y:i32) = (edges);
            relation Reach(x:i32,y:i32);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
            strata { component [Reach]; }
            seminaive {
                insert_new hash_set_all_columns;
                negation_keys hash_set_borrowed;
                delta Reach = Recent;
                layout Edge [0,1] key 0;
                layout Reach [0,1] key 1;
                layout Reach [1,0] key 1;
                variant 0 seed { Reach(*x,*y) :- Edge(x,y); }
                variant 1 delta 0 { Reach(*x,*z) :- Recent(x,y), Reach(y,z); }
                variant 1 delta 1 { Reach(*x,*z) :- Reach(x,y), Recent(y,z); }
            }
    }.collect();
    assert_eq!(source, BTreeSet::from([(1, 2), (1, 3), (1, 4), (2, 3), (2, 4), (3, 4)]));
    assert_eq!(source, stratified);
    assert_eq!(source, seminaive);
}

#[test]
fn native_negation_handles_projection_repetition_and_empty_keys() {
    let domain = [(1,), (2,), (3,)];
    let pairs = [(1, 1), (2, 3), (2, 3)];
    let empty: [(i32, i32); 0] = [];
    let repeated: BTreeSet<_> = cq_datalog::datalog! {
            struct Repeated;
            relation Domain(x:i32) = (domain);
            relation Pair(x:i32,y:i32) = (pairs);
            relation Out(x:i32);
            output Out;
            Out(*x) :- Domain(x), !Pair(x,x);
    }.collect();
    assert_eq!(repeated, BTreeSet::from([(2,), (3,)]));
    let projected: BTreeSet<_> = cq_datalog::datalog! {
            struct Projected;
            relation Domain(x:i32) = (domain);
            relation Pair(x:i32,y:i32) = (pairs);
            relation Out(x:i32);
            output Out;
            Out(*x) :- Domain(x), !Pair(x,_);
    }.collect();
    assert_eq!(projected, BTreeSet::from([(3,)]));
    let absent: BTreeSet<_> = cq_datalog::datalog! {
            struct Absent;
            relation Domain(x:i32) = (domain);
            relation Pair(x:i32,y:i32) = (empty);
            relation Out(x:i32);
            output Out;
            Out(*x) :- Domain(x), !Pair(_,_), !Pair(x,_);
    }.collect();
    assert_eq!(absent, BTreeSet::from(domain));
    let present: BTreeSet<_> = cq_datalog::datalog! {
            struct Present;
            relation Domain(x:i32) = (domain);
            relation Pair(x:i32,y:i32) = (pairs);
            relation Out(x:i32);
            output Out;
            Out(*x) :- Domain(x), !Pair(_,_);
    }.collect();
    assert!(present.is_empty());
}

#[test]
fn recursive_negation_reads_completed_lower_strata() {
    let edges = [(1, 2), (2, 3), (3, 4), (2, 5), (5, 6)];
    let blocked = [(3,)];
    let seeds = [(1,)];
    let answer: BTreeSet<_> = cq_datalog::datalog! {
            struct Layered;
            relation Edge(x:i32,y:i32) = (edges);
            relation BlockInput(x:i32) = (blocked);
            relation Seed(x:i32) = (seeds);
            relation Blocked(x:i32);
            relation Reach(x:i32);
            relation Out(x:i32);
            output Out;
            Blocked(*x) :- BlockInput(x);
            Reach(*x) :- Seed(x), !Blocked(x);
            Reach(*y) :- Reach(x), Edge(x,y), !Blocked(y);
            Out(*x) :- Reach(x), !Seed(x);
    }.collect();
    assert_eq!(answer, BTreeSet::from([(2,), (5,), (6,)]));
}

#[test]
fn mutual_recursion_publishes_one_snapshot_per_round() {
    let edges = [(1, 2), (2, 3), (3, 4)];
    let seed = [(1,)];
    let mut rounds = Vec::new();
    let answer: BTreeSet<_> = cq_datalog::datalog! {
            observe = (|round, count| rounds.push((round, count)));
            struct Mutual;
            relation Edge(x:i32,y:i32) = (edges);
            relation Seed(x:i32) = (seed);
            relation Even(x:i32);
            relation Odd(x:i32);
            output Odd;
            Even(*x) :- Seed(x);
            Odd(*y) :- Even(x), Edge(x,y);
            Even(*y) :- Odd(x), Edge(x,y);
    }.collect();
    assert_eq!(answer, BTreeSet::from([(2,), (4,)]));
    assert_eq!(rounds, [(0, 1), (1, 1), (2, 1), (3, 1)]);
}

#[test]
fn negation_keys_borrow_noncopy_columns() {
    let domain = [(String::from("keep"),), (String::from("drop"),)];
    let blocked = [(String::from("drop"), 7)];
    let answer: BTreeSet<_> = cq_datalog::datalog! {
            struct Borrowed;
            relation Domain(x:String) = (domain);
            relation Blocked(x:String,y:i32) = (blocked);
            relation Out(x:String);
            output Out;
            Out((*x).clone()) :- Domain(x), !Blocked(x,_);
    }.collect();
    assert_eq!(answer, BTreeSet::from([(String::from("keep"),)]));
}
