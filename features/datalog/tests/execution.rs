//! Datalog entry stages, native negation, and fixed-point execution.

use std::collections::BTreeSet;

#[test]
fn opt_in_frozen_filter_preserves_exact_recursive_answers() {
    let seed = [(0_i32, 1_i32)];
    let edges = [(1_i32, 2_i32), (1, 2), (2, 3), (3, 4), (4, 5)];
    let blocked = [(3_i32,)];
    let actual: BTreeSet<_> = cq_datalog::datalog! {
        arguments { hoist_invariants; };
        struct Filtered;
        relation Seed(x:i32,y:i32) = (seed);
        relation Edge(x:i32,y:i32) = (edges);
        relation Block(x:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Seed(x,y);
        Reach(*x,*z) :- Reach(x,y), Edge(y,z), !Block(z);
    }
    .collect();
    assert_eq!(actual, BTreeSet::from([(0, 1), (0, 2)]));
}

#[test]
fn push_holes_match_pull_for_owned_heads_and_constant_filtered_keys() {
    let names = [
        ("alice".to_owned(),),
        ("bob".to_owned(),),
        ("alice".to_owned(),),
    ];
    let modifiers = [
        ("blocked".to_owned(), "bob".to_owned()),
        ("other".to_owned(), "alice".to_owned()),
    ];
    macro_rules! run {
        ($printer:path) => {
            cq_datalog::datalog! {
                pipeline { cq = pipeline { print = $printer; }; }
                struct Names;
                relation Name(name:String) = (&names);
                relation Modifier(kind:String,name:String) = (&modifiers);
                relation Allowed(name:String,n:i32);
                output Allowed;
                Allowed(name.clone(), 0) :- Name(name), !Modifier("blocked",name);
                Allowed(name.clone(), *n + 1) :- Allowed(name,n), if *n < 2;
            }
            .collect::<BTreeSet<_>>()
        };
    }
    let pull = run!(::mini_linq::passes::translate_pull);
    let push = run!(::cq_datalog::passes::print_push);
    assert_eq!(push, pull);
    assert_eq!(
        push,
        BTreeSet::from([
            ("alice".to_owned(), 0),
            ("alice".to_owned(), 1),
            ("alice".to_owned(), 2),
        ])
    );
}

#[test]
fn push_key_holes_preserve_empty_tuple_and_empty_input_semantics() {
    let domain = [(1_i32,), (2,), (2,)];
    macro_rules! run {
        ($printer:path, $blocked:expr) => {
            cq_datalog::datalog! {
                pipeline { cq = pipeline { print = $printer; }; }
                struct Exists;
                relation Domain(x:i32) = (domain);
                relation Blocked(x:i32) = ($blocked);
                relation Out(x:i32);
                output Out;
                Out(*x) :- Domain(x), !Blocked(_);
            }
            .collect::<BTreeSet<_>>()
        };
    }
    let absent: [(i32,); 0] = [];
    for blocked in [absent.as_slice(), &[(9_i32,)]] {
        assert_eq!(
            run!(::cq_datalog::passes::print_push, blocked),
            run!(::mini_linq::passes::translate_pull, blocked),
        );
    }
    assert_eq!(
        run!(::cq_datalog::passes::print_push, absent),
        BTreeSet::from(domain)
    );
}

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
fn named_datalog_phases_and_nested_cq_configuration_execute() {
    let edges = [(1_i32, 2_i32), (2, 3)];
    let answers: BTreeSet<_> = cq_datalog::datalog! {
        pipeline {
            surface = [];
            stratify = cq_datalog::passes::stratify;
            incremental = cq_datalog::passes::incremental;
            storage = cq_datalog::passes::storage;
            storage_rewrites = [];
            fork = cq_datalog::passes::fork;
            round = [];
            print = cq_datalog::passes::print_rounds;
            cq = pipeline { plan = mini_linq::passes::plan(no_reduce); };
        }
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

mod pull_alias {
    pub use mini_linq::passes::translate_pull as emit;
}

#[test]
fn configured_pull_printer_alias_executes_recursive_and_key_holes() {
    let edges = [(1_i32, 2_i32), (2, 3), (3, 4)];
    let blocked = [(4_i32,)];
    let answers: BTreeSet<_> = cq_datalog::datalog! {
        pipeline {
            terminals = [crate::pull_alias::emit => pull];
            cq = pipeline {
                sequence = mini_linq::passes::sequence;
                grade = mini_linq::passes::sequential;
                print = crate::pull_alias::emit;
            };
        }
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        relation Allowed(x:i32,y:i32);
        output Allowed;
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Reach(x,y), Reach(y,z);
        Allowed(*x,*y) :- Reach(x,y), !Blocked(y);
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
    let stratified: BTreeSet<_> = cq_datalog::passes::incremental! {
            pipeline { cq_datalog::passes::storage, cq_datalog::passes::fork,
                cq_datalog::passes::print_rounds };
            struct Closure;
            relation Edge(x:i32,y:i32) = (edges);
            relation Reach(x:i32,y:i32);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
            strata { component [Reach]; }
    }
    .collect();
    let seminaive: BTreeSet<_> = cq_datalog::passes::storage! {
            pipeline { cq_datalog::passes::fork, cq_datalog::passes::print_rounds };
            struct Closure;
            relation Edge(x:i32,y:i32) = (edges);
            relation Reach(x:i32,y:i32);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
            strata { component [Reach]; }
            incremental {
                scheme seminaive_covering;
                delta Reach = Recent;
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
            Odd(*y) :- Edge(x,y), Even(x);
            Even(*y) :- Edge(x,y), Odd(x);
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
    let stratified = cq_datalog::passes::incremental! {
        pipeline { cq_datalog::passes::storage, cq_datalog::passes::fork,
            cq_datalog::passes::print_rounds };
        struct Replay;
        relation D(x:i32) = (domain);
        relation M(kind:i32,x:i32) = (blocked);
        relation A(x:i32); relation B(x:i32);
        output B, A;
        A(*x) :- D(x), !M(7,x);
        B(*x + 10) :- A(x);
        strata { component [A]; component [B]; }
    };
    let seminaive = cq_datalog::passes::storage! {
        pipeline { cq_datalog::passes::fork, cq_datalog::passes::print_rounds };
        struct Replay;
        relation D(x:i32) = (domain);
        relation M(kind:i32,x:i32) = (blocked);
        relation A(x:i32); relation B(x:i32);
        output B, A;
        A(*x) :- D(x), !M(7,x);
        B(*x + 10) :- A(x);
        strata { component [A]; component [B]; }
        incremental {
            scheme seminaive_covering;
            delta A = DA;
            delta B = DB;
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
    let stratified = cq_datalog::passes::incremental! {
        pipeline { cq_datalog::passes::storage, cq_datalog::passes::fork,
            cq_datalog::passes::print_rounds };
        struct Facts;
        relation E(name:String,value:i32) = (&input);
        output E;
        strata {}
    };
    let seminaive = cq_datalog::passes::storage! {
        pipeline { cq_datalog::passes::fork, cq_datalog::passes::print_rounds };
        struct Facts;
        relation E(name:String,value:i32) = (&input);
        output E;
        strata {}
        incremental {
            scheme seminaive_covering;
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

#[test]
fn duplicate_grouped_inputs_are_scanned_once_for_all_layouts_keys_and_outputs() {
    use cq_storage::rows::Rows;
    use std::cell::Cell;

    #[derive(Clone, Copy)]
    struct CountRows<'a, H> {
        host: H,
        scans: &'a Cell<usize>,
        visits: &'a Cell<usize>,
    }
    impl<H: Rows> Rows for CountRows<'_, H> {
        type Row = H::Row;
        fn rows(self) -> impl Iterator<Item = Self::Row> {
            self.scans.set(self.scans.get() + 1);
            self.host
                .rows()
                .inspect(move |_| self.visits.set(self.visits.get() + 1))
        }
    }
    fn counted(value: &str, calls: &Cell<usize>) -> String {
        calls.set(calls.get() + 1);
        value.to_owned()
    }

    let (facts, nodes, paths, shared_targets, missing, reach) = {
        let edges = std::collections::BTreeMap::from([
            (
                (String::from("a"),),
                vec![(String::from("b"),), (String::from("b"),)],
            ),
            (
                (String::from("b"),),
                vec![(String::from("c"),), (String::from("c"),)],
            ),
            (
                (String::from("c"),),
                vec![(String::from("d"),), (String::from("d"),)],
            ),
        ]);
        let domain: Vec<_> = ["a", "a", "b", "b", "c", "c", "d", "d"]
            .into_iter()
            .map(|name| (name.to_owned(),))
            .collect();
        let edge_scans = Cell::new(0);
        let edge_visits = Cell::new(0);
        let domain_scans = Cell::new(0);
        let domain_visits = Cell::new(0);
        let candidates = &Cell::new(0);
        let expressions = Cell::new(0);
        let input = CountRows {
            host: &edges,
            scans: &edge_scans,
            visits: &edge_visits,
        };
        let domain_input = CountRows {
            host: &domain,
            scans: &domain_scans,
            visits: &domain_visits,
        };
        let answer = cq_datalog::datalog! {
            observe = (|_, _| {
                assert_eq!(edge_scans.get(), 1);
                assert_eq!(domain_scans.get(), 1);
            });
            struct CanonicalInputs;
            relation E(x:String,y:String) = ({ expressions.set(expressions.get() + 1); input });
            relation D(x:String) = (domain_input);
            relation Paths(x:String,z:String);
            relation SharedTargets(x:String,z:String);
            relation Missing(x:String);
            relation Reach(x:String,y:String);
            output E, D, Paths, SharedTargets, Missing, Reach;
            Paths(counted(x,candidates),(*z).clone()) :- E(x,y), E(y,z);
            SharedTargets(counted(x,candidates),(*z).clone()) :- E(x,y), E(z,y);
            Missing((*x).clone()) :- D(x), !E(x,_);
            Reach((*x).clone(),(*y).clone()) :- E(x,y);
            Reach((*x).clone(),(*z).clone()) :- Reach(x,y), E(y,z);
        };
        assert_eq!(expressions.get(), 1);
        assert_eq!((edge_scans.get(), edge_visits.get()), (1, 6));
        assert_eq!((domain_scans.get(), domain_visits.get()), (1, 8));
        // Observe candidates before IDB set insertion: final set equality alone
        // would hide the duplicate Cartesian amplification in both joins.
        assert_eq!(candidates.get(), 5);
        assert_eq!(edges.len(), 3);
        assert_eq!(domain.len(), 8);
        answer
    };
    let facts: Vec<_> = facts.collect();
    let nodes: Vec<_> = nodes.collect();
    assert_eq!(facts.len(), 3);
    assert_eq!(nodes.len(), 4);
    assert_eq!(
        facts.into_iter().collect::<BTreeSet<_>>(),
        [("a", "b"), ("b", "c"), ("c", "d")]
            .into_iter()
            .map(|(x, y)| (x.to_owned(), y.to_owned()))
            .collect()
    );
    assert_eq!(
        paths.collect::<BTreeSet<_>>(),
        [("a", "c"), ("b", "d")]
            .into_iter()
            .map(|(x, y)| (x.to_owned(), y.to_owned()))
            .collect()
    );
    assert_eq!(
        shared_targets.collect::<BTreeSet<_>>(),
        ["a", "b", "c"]
            .into_iter()
            .map(|x| (x.to_owned(), x.to_owned()))
            .collect()
    );
    assert_eq!(missing.collect::<Vec<_>>(), [(String::from("d"),)]);
    assert_eq!(
        reach.collect::<BTreeSet<_>>(),
        [
            ("a", "b"),
            ("a", "c"),
            ("a", "d"),
            ("b", "c"),
            ("b", "d"),
            ("c", "d")
        ]
        .into_iter()
        .map(|(x, y)| (x.to_owned(), y.to_owned()))
        .collect()
    );
}

#[test]
fn canonical_negative_only_inputs_do_not_require_column_clones() {
    #[derive(Debug, Eq, PartialEq, Hash)]
    struct NoClone(String);
    let blocked = [
        (1, NoClone(String::from("reason"))),
        (1, NoClone(String::from("reason"))),
    ];
    let domain = [(1,), (1,), (2,), (2,)];
    let result = cq_datalog::datalog! {
        struct BorrowOnly;
        relation D(x:i32) = (&domain);
        relation Blocked(x:i32,reason:NoClone) = (&blocked);
        relation Out(x:i32);
        output Out;
        Out(*x) :- D(x), !Blocked(x,_);
    };
    assert_eq!(result.collect::<Vec<_>>(), [(2,)]);
    assert_eq!(blocked[0].1.0, "reason");
}

#[test]
fn later_delta_atoms_keep_written_bindings_guards_and_constant_probes() {
    let alloc = [(10, 1), (10, 1), (20, 1), (30, 1)];
    let assign = [(0, 1), (2, 1), (2, 1), (3, 2), (4, 3)];
    let tags = [(10, 7), (10, 7), (20, 8), (30, 7)];
    let closed = [(10, 3), (20, 3), (30, 4)];
    let names = [
        (10, String::from("ok")),
        (20, String::from("ok")),
        (30, String::from("ok")),
    ];
    let answer = cq_datalog::datalog! {
        struct PointsTo;
        relation Alloc(heap:i32,var:i32) = (&alloc);
        relation Assign(to:i32,from:i32) = (&assign);
        relation Tags(heap:i32,kind:i32) = (&tags);
        relation Closed(heap:i32,kind:i32) = (&closed);
        relation Names(heap:i32,name:String) = (&names);
        relation VarPointsTo(heap:i32,var:i32);
        output VarPointsTo;
        VarPointsTo(*heap,*var) :- Alloc(heap,var);
        VarPointsTo(*heap,*to) :-
            VarPointsTo(other,from), VarPointsTo(heap,from), Assign(to,from),
            if *to != 0, if 10 / *to > 0,
            Tags(heap,7), Closed(heap,1 + 2), Names(heap,"ok");
        VarPointsTo(*heap,*to) :-
            Assign(to,from), if *to != 0, if 10 / *to > 0,
            VarPointsTo(heap,from), VarPointsTo(heap,to),
            Tags(heap,7), Closed(heap,1 + 2), Names(heap,"ok");
    };
    assert_eq!(
        answer.collect::<BTreeSet<_>>(),
        BTreeSet::from([(10, 1), (20, 1), (30, 1), (10, 2), (10, 3), (10, 4)])
    );
}

#[test]
fn cyclic_closure_shares_delta_runs_across_holes_and_key_orders() {
    let edges = [(1, 2), (2, 3), (3, 1), (3, 4), (1, 2), (3, 4)];
    let rows: Vec<_> = cq_datalog::datalog! {
        struct SharedDelta;
        relation Edge(x:i32,y:i32) = (&edges);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Reach(x,y), Reach(y,z);
        Reach(*x,*z) :- Edge(x,y), Reach(y,z);
        Reach(*x,*z) :- Edge(y,z), Reach(x,y);
        Reach(*x,*y) :- Edge(x,y), Reach(x,y);
        Reach(*y,*z) :- Reach(1,y), Reach(y,z);
    }
    .collect();
    assert_eq!(rows.len(), 12);
    assert_eq!(
        rows.into_iter().collect::<BTreeSet<_>>(),
        (1..=3).flat_map(|x| (1..=4).map(move |y| (x, y))).collect()
    );
}

#[test]
fn empty_delta_replaces_the_old_run_while_another_member_advances() {
    use std::cell::Cell;
    fn record(probes: &Cell<usize>) -> bool {
        probes.set(probes.get() + 1);
        true
    }
    let probes = Cell::new(0);
    let probes = &probes;
    let edges = [(1, 2), (2, 3), (3, 4)];
    let seed = [(1,), (1,)];
    let (even, odd) = cq_datalog::datalog! {
        struct Alternating;
        relation Edge(x:i32,y:i32) = (&edges);
        relation Seed(x:i32) = (&seed);
        relation Even(x:i32);
        relation Odd(x:i32);
        output Even, Odd;
        Even(*x) :- Seed(x);
        Odd(*y) :- Edge(x,y), Even(x), if record(probes);
        Even(*y) :- Edge(x,y), Odd(x), if record(probes);
    };
    assert_eq!(even.collect::<BTreeSet<_>>(), BTreeSet::from([(1,), (3,)]));
    assert_eq!(odd.collect::<BTreeSet<_>>(), BTreeSet::from([(2,), (4,)]));
    // Each matching delta fact is visited only in its own publication, not
    // again while the other relation keeps this mutually recursive SCC alive.
    assert_eq!(probes.get(), 3);
}

#[test]
fn owned_delta_keys_and_later_zero_key_runs_preserve_guard_order() {
    let (reach, flags) = {
        let edges = [
            (String::from("a"), String::from("b"), 0_i32),
            (String::from("a"), String::from("b"), 1),
            (String::from("b"), String::from("c"), 2),
            (String::from("c"), String::from("a"), 1),
            (String::from("a"), String::from("b"), 1),
        ];
        let seed = [(String::from("a"),), (String::from("a"),)];
        let flag = [(7_i32,), (7,)];
        cq_datalog::datalog! {
            struct OwnedRuns;
            relation Edge(x:String,y:String,divisor:i32) = (&edges);
            relation Seed(x:String) = (&seed);
            relation Flag(flag:i32) = (&flag);
            relation Reach(x:String);
            relation SeenFlag(flag:i32);
            output Reach, SeenFlag;
            Reach((*x).clone()) :- Seed(x);
            Reach((*y).clone()) :- Edge(x,y,d), if *d != 0, if 8 / *d > 0, Reach(x);
            Reach((*x).clone()) :- SeenFlag(f), if *f == 7, Reach(x);
            SeenFlag(*f) :- Flag(f), Reach(x);
        }
    };
    let reach: Vec<_> = reach.collect();
    assert_eq!(reach.len(), 3);
    assert_eq!(
        reach.into_iter().collect::<BTreeSet<_>>(),
        ["a", "b", "c"]
            .into_iter()
            .map(|x| (x.to_owned(),))
            .collect()
    );
    assert_eq!(flags.collect::<Vec<_>>(), [(7,)]);
}

#[test]
fn nonrecursive_generators_join_generated_values_and_deduplicate() {
    let seeds = [(1_i32,), (1,), (3,)];
    let links = [(1_i32, 10_i32), (1, 10), (2, 20), (4, 40)];
    let rows: Vec<_> = cq_datalog::datalog! {
        struct Generated;
        relation Seed(x:i32) = (seeds);
        relation Link(x:i32,y:i32) = (links);
        relation Answer(x:i32,y:i32);
        output Answer;
        Answer(value,*label) :- Seed(x),
            for (value, keep) in [(*x, true), (*x + 1, true), (*x, false)],
            if *keep, Link(value,label);
    }
    .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(1, 10), (2, 20), (4, 40)])
    );
}

#[test]
fn recursive_finite_generators_converge_and_join_generated_values() {
    let seeds = [(0_i32,), (0,)];
    let links = [(1_i32, 10_i32), (2, 20), (2, 20), (3, 30), (4, 40)];
    let (reach, matched) = cq_datalog::datalog! {
        struct GeneratedClosure;
        relation Seed(x:i32) = (seeds);
        relation Link(x:i32,y:i32) = (links);
        relation Reach(x:i32);
        relation Matched(x:i32,y:i32);
        output Reach, Matched;
        Reach(*x) :- Seed(x);
        Reach(next) :- Reach(x), if *x < 4, for next in [*x + 1, *x + 1];
        Matched(next,*label) :- Reach(x), for next in [*x + 1, *x + 1],
            Link(next,label);
    };
    let reach: Vec<_> = reach.collect();
    let matched: Vec<_> = matched.collect();
    assert_eq!(reach.len(), 5);
    assert_eq!(matched.len(), 4);
    assert_eq!(
        reach.into_iter().collect::<BTreeSet<_>>(),
        (0..=4).map(|x| (x,)).collect()
    );
    assert_eq!(
        matched.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(1, 10), (2, 20), (3, 30), (4, 40)])
    );
}

#[test]
fn split_codegen_preserves_local_types_captures_and_mutable_observers() {
    struct Facts {
        seeds: Vec<(i32,)>,
        limit: i32,
    }
    let facts = Facts {
        seeds: vec![(0,)],
        limit: 3,
    };
    let increment = |value: i32| value + 1;
    let label = String::from("captured");
    let mut rounds = Vec::new();
    let (reach, labels) = cq_datalog::datalog! {
        observe = (|round, count| rounds.push((round, count)));
        struct Captured;
        relation Seed(x:i32) = (facts.seeds);
        relation Reach(x:i32);
        relation Label(x:i32,label:String);
        output Reach, Label;
        Reach(*x) :- Seed(x);
        Reach(increment(*x)) :- Reach(x), if *x < facts.limit;
        Label(*x,label.clone()) :- Reach(x);
    };
    assert_eq!(
        reach.collect::<BTreeSet<_>>(),
        BTreeSet::from([(0,), (1,), (2,), (3,)])
    );
    assert_eq!(
        labels.collect::<BTreeSet<_>>(),
        (0..=3).map(|x| (x, String::from("captured"))).collect()
    );
    assert!(!rounds.is_empty());
    assert_eq!(facts.seeds, [(0,)]);
}
