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
    }
    .collect();
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
    }
    .collect();
    let stratified: BTreeSet<_> = cq_datalog::stratified! {
            struct Closure;
            relation Edge(x:i32,y:i32) = (edges);
            relation Reach(x:i32,y:i32);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
            strata { component [Reach]; }
    }
    .collect();
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
    }
    .collect();
    assert_eq!(
        source,
        BTreeSet::from([(1, 2), (1, 3), (1, 4), (2, 3), (2, 4), (3, 4)])
    );
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
    }
    .collect();
    assert_eq!(repeated, BTreeSet::from([(2,), (3,)]));
    let projected: BTreeSet<_> = cq_datalog::datalog! {
            struct Projected;
            relation Domain(x:i32) = (domain);
            relation Pair(x:i32,y:i32) = (pairs);
            relation Out(x:i32);
            output Out;
            Out(*x) :- Domain(x), !Pair(x,_);
    }
    .collect();
    assert_eq!(projected, BTreeSet::from([(3,)]));
    let absent: BTreeSet<_> = cq_datalog::datalog! {
            struct Absent;
            relation Domain(x:i32) = (domain);
            relation Pair(x:i32,y:i32) = (empty);
            relation Out(x:i32);
            output Out;
            Out(*x) :- Domain(x), !Pair(_,_), !Pair(x,_);
    }
    .collect();
    assert_eq!(absent, BTreeSet::from(domain));
    let present: BTreeSet<_> = cq_datalog::datalog! {
            struct Present;
            relation Domain(x:i32) = (domain);
            relation Pair(x:i32,y:i32) = (pairs);
            relation Out(x:i32);
            output Out;
            Out(*x) :- Domain(x), !Pair(_,_);
    }
    .collect();
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
    }
    .collect();
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
    }
    .collect();
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
    }
    .collect();
    assert_eq!(answer, BTreeSet::from([(String::from("keep"),)]));
}

#[test]
fn multiple_outputs_are_owned_in_written_order_across_components() {
    // None of the returned iterators may borrow these input strings.
    let (descriptors, reach, empty) = {
        let methods = [
            (
                String::from("main"),
                String::from("void"),
                String::from("String[]"),
            ),
            (String::from("other"), String::from("int"), String::new()),
        ];
        let edges = [(String::from("main"), String::from("other"))];
        cq_datalog::datalog! {
            struct Methods;
            relation Method(name:String,result:String,params:String) = (methods);
            relation Edge(caller:String,callee:String) = (edges);
            relation Reach(name:String);
            relation Empty(name:String);
            relation Descriptor(name:String,text:String);
            output Descriptor, Reach, Empty;
            Reach((*name).clone()) :- Method(name,"void","String[]");
            Reach((*super_).clone()) :- Reach(type_), Edge(type_,super_);
            Descriptor((*type_).clone(),format!("{}({})",ret,params))
                :- Reach(type_), Method(type_,ret,params);
        }
    };
    let empty: std::vec::IntoIter<(String,)> = empty;
    assert_eq!(empty.count(), 0);
    assert_eq!(
        reach.collect::<BTreeSet<_>>(),
        BTreeSet::from([(String::from("main"),), (String::from("other"),)])
    );
    assert_eq!(
        descriptors.collect::<BTreeSet<_>>(),
        BTreeSet::from([
            (String::from("main"), String::from("void(String[])")),
            (String::from("other"), String::from("int()")),
        ])
    );
}

#[test]
fn literal_selections_and_filtered_negative_keys_preserve_string_values() {
    let nodes = [
        (String::from("main"),),
        (String::from("helper"),),
        (String::from("abstract"),),
        (String::from("λ,\""),),
    ];
    let modifiers = [
        (String::from("public"), String::from("main"), 1),
        (String::from("abstract"), String::from("abstract"), 7),
        (String::from("abstract"), String::from("abstract"), 7),
        (String::from("static"), String::from("helper"), 2),
    ];
    let edges = [
        (String::from("main"), String::from("helper")),
        (String::from("helper"), String::from("abstract")),
        (String::from("helper"), String::from("λ,\"")),
    ];
    let (reached, concrete, no_static, absent, present, tags) = cq_datalog::datalog! {
        struct Constants;
        relation Node(name:String) = (nodes);
        relation Modifier(kind:String,name:String,flag:i32) = (modifiers);
        relation Edge(caller:String,callee:String) = (edges);
        relation Concrete(name:String);
        relation Reach(name:String);
        relation NoStatic(name:String);
        relation Absent(name:String);
        relation Present(name:String);
        relation Tags(name:String,text:&'static str,flag:i32,enabled:bool);
        output Reach, Concrete, NoStatic, Absent, Present, Tags;
        Concrete((*type_).clone()) :- Node(type_), !Modifier("abstract",type_,_);
        Reach((*type_).clone()) :- Modifier("public",type_,1), !Modifier("abstract",type_,_);
        Reach((*super_).clone()) :- Reach(type_), Edge(type_,super_), !Modifier("abstract",super_,_);
        NoStatic((*type_).clone()) :- Node(type_), !Modifier("static",type_,_);
        Absent((*type_).clone()) :- Node(type_), !Modifier("missing",_,_), !Modifier("abstract","absent",7);
        Present((*type_).clone()) :- Node(type_), !Modifier("abstract","abstract",7);
        Tags((*r#type).clone(),"literal",-7,true) :- Concrete(r#type);
    };
    let concrete: BTreeSet<_> = concrete.collect();
    assert_eq!(
        concrete,
        BTreeSet::from([
            (String::from("main"),),
            (String::from("helper"),),
            (String::from("λ,\""),),
        ])
    );
    assert_eq!(reached.collect::<BTreeSet<_>>(), concrete);
    assert_eq!(
        no_static.collect::<BTreeSet<_>>(),
        BTreeSet::from([
            (String::from("main"),),
            (String::from("abstract"),),
            (String::from("λ,\""),),
        ])
    );
    assert_eq!(absent.collect::<BTreeSet<_>>(), nodes.into_iter().collect());
    assert_eq!(present.count(), 0);
    assert_eq!(
        tags.collect::<BTreeSet<_>>(),
        concrete
            .into_iter()
            .map(|(name,)| (name, "literal", -7, true))
            .collect()
    );
}

#[test]
fn negative_closed_expressions_and_empty_inputs() {
    fn symbol(text: &str) -> i32 {
        if text == "abstract" { 7 } else { 8 }
    }
    for modifiers in [vec![], vec![(7, 1), (8, 2), (7, 1)]] {
        let domain = [(1,), (2,), (3,)];
        let (concrete, absent) = cq_datalog::datalog! {
            struct Interned;
            relation D(x:i32) = (domain);
            relation M(kind:i32,x:i32) = (&modifiers);
            relation Concrete(x:i32); relation Absent(x:i32);
            output Concrete, Absent;
            Concrete(*x) :- D(x), !M(symbol("abstract"),x);
            Absent(*x) :- D(x), !M(symbol("abstract"),_);
        };
        let expected = if modifiers.is_empty() {
            BTreeSet::from(domain)
        } else {
            BTreeSet::from([(2,), (3,)])
        };
        assert_eq!(concrete.collect::<BTreeSet<_>>(), expected);
        assert_eq!(absent.count(), if modifiers.is_empty() { 3 } else { 0 });
    }
}

#[test]
fn multioutput_replay_retains_constants_and_result_order() {
    let domain = [(1,), (2,), (3,)];
    let blocked = [(7, 2)];
    let source = cq_datalog::datalog! {
        struct Replay;
        relation D(x:i32) = (domain);
        relation M(kind:i32,x:i32) = (blocked);
        relation A(x:i32); relation B(x:i32);
        output B, A;
        A(*x) :- D(x), !M(7,x);
        B(*x + 10) :- A(x);
    };
    let stratified = cq_datalog::stratified! {
        struct Replay;
        relation D(x:i32) = (domain);
        relation M(kind:i32,x:i32) = (blocked);
        relation A(x:i32); relation B(x:i32);
        output B, A;
        A(*x) :- D(x), !M(7,x);
        B(*x + 10) :- A(x);
        strata { component [A]; component [B]; }
    };
    let seminaive = cq_datalog::seminaive! {
        struct Replay;
        relation D(x:i32) = (domain);
        relation M(kind:i32,x:i32) = (blocked);
        relation A(x:i32); relation B(x:i32);
        output B, A;
        A(*x) :- D(x), !M(7,x);
        B(*x + 10) :- A(x);
        strata { component [A]; component [B]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta A = DA;
            delta B = DB;
            layout D [0] key 0;
            layout A [0] key 0;
            variant 0 seed { A(*x) :- D(x), !M(7,x); }
            variant 1 seed { B(*x + 10) :- A(x); }
        }
    };
    for (b, a) in [source, stratified, seminaive] {
        assert_eq!(b.collect::<BTreeSet<_>>(), BTreeSet::from([(11,), (13,)]));
        assert_eq!(a.collect::<BTreeSet<_>>(), BTreeSet::from([(1,), (3,)]));
    }
}

#[test]
fn single_edb_output_is_owned_deduplicated_and_needs_no_rules() {
    let answer: std::vec::IntoIter<(String, i32)> = {
        let input = vec![
            (String::from("main"), 1),
            (String::from("main"), 1),
            (String::from("λ"), 2),
        ];
        let answer = cq_datalog::datalog! {
            struct Facts;
            relation E(name:String,value:i32) = (&input);
            output E;
        };
        assert_eq!(input.len(), 3);
        answer
    };
    let answer: Vec<_> = answer.collect();
    assert_eq!(answer.len(), 2);
    assert_eq!(
        answer.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(String::from("main"), 1), (String::from("λ"), 2),])
    );
    let empty = cq_datalog::datalog! {
        struct EmptyFacts;
        relation E(x:String) = (Vec::<(String,)>::new());
        output E;
    };
    assert_eq!(empty.count(), 0);
}

#[test]
fn mixed_edb_idb_outputs_keep_written_order_and_ownership() {
    let (derived, facts, empty) = {
        let input = vec![
            (String::from("a"),),
            (String::from("a"),),
            (String::from("b"),),
        ];
        let empty: Vec<(i32,)> = Vec::new();
        cq_datalog::datalog! {
            struct Mixed;
            relation E(name:String) = (&input);
            relation Empty(x:i32) = (&empty);
            relation A(name:String);
            output A, E, Empty;
            A(format!("{}!",name)) :- E(name);
        }
    };
    let facts: Vec<_> = facts.collect();
    assert_eq!(facts.len(), 2);
    assert_eq!(
        facts.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(String::from("a"),), (String::from("b"),)])
    );
    assert_eq!(
        derived.collect::<BTreeSet<_>>(),
        BTreeSet::from([(String::from("a!"),), (String::from("b!"),)])
    );
    assert_eq!(empty.count(), 0);
}

#[test]
fn edb_only_replay_preserves_grouped_host_schema() {
    let mut input = std::collections::BTreeMap::new();
    input.insert((String::from("a"),), vec![(1_i32,), (1,), (2,)]);
    let source = cq_datalog::datalog! {
        struct Facts;
        relation E(name:String,value:i32) = (&input);
        output E;
    };
    let stratified = cq_datalog::stratified! {
        struct Facts;
        relation E(name:String,value:i32) = (&input);
        output E;
        strata {}
    };
    let seminaive = cq_datalog::seminaive! {
        struct Facts;
        relation E(name:String,value:i32) = (&input);
        output E;
        strata {}
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
        }
    };
    for result in [source, stratified, seminaive] {
        let result: Vec<_> = result.collect();
        assert_eq!(result.len(), 2);
        assert_eq!(
            result.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([(String::from("a"), 1), (String::from("a"), 2)])
        );
    }
}

#[test]
fn owned_string_constants_filter_each_keyset_row_without_moving_captures() {
    let domain = [
        (String::from("a"),),
        (String::from("b"),),
        (String::from("c"),),
    ];
    let modifiers = [
        (String::from("static"), String::from("a")),
        (String::from("static"), String::from("b")),
        (String::from("public"), String::from("c")),
        (String::from("static"), String::from("a")),
    ];
    let (allowed, absent, present, literal) = cq_datalog::datalog! {
        struct OwnedConstants;
        relation D(x:String) = (domain);
        relation M(kind:String,x:String) = (modifiers);
        relation Allowed(x:String);
        relation Absent(x:String);
        relation Present(x:String);
        relation Literal(x:String,value:String);
        output Allowed, Absent, Present, Literal;
        Allowed((*x).clone()) :- D(x), !M(String::from("static"),x);
        Absent((*x).clone()) :- D(x), !M(String::from("missing"),_);
        Present((*x).clone()) :- D(x), !M(String::from("static"),String::from("b"));
        Literal((*x).clone(),"java.lang.String".to_owned()) :- D(x);
    };
    assert_eq!(allowed.collect::<Vec<_>>(), [(String::from("c"),)]);
    assert_eq!(
        absent.collect::<BTreeSet<_>>(),
        domain.iter().cloned().collect()
    );
    assert_eq!(present.count(), 0);
    assert_eq!(
        literal.collect::<BTreeSet<_>>(),
        domain
            .into_iter()
            .map(|(name,)| (name, String::from("java.lang.String")))
            .collect()
    );
}
