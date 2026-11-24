//! Datalog entry stages, native negation, and fixed-point execution.

use std::collections::BTreeSet;

use cq_load::StringDictionary;
use cq_storage::domain::exact;

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
fn invariant_projection_owns_string_aliases_on_pull_and_push() {
    type Name = String;
    let seed = [(Name::from("a"), Name::from("b"))];
    let edges = [
        (Name::from("b"), Name::from("c")),
        (Name::from("c"), Name::from("d")),
    ];
    let blocked = [(Name::from("d"),)];
    macro_rules! run {
        ($printer:path, $input:ident) => {
            cq_datalog::datalog! {
                arguments { hoist_invariants; };
                pipeline { cq = pipeline { print = $printer; print_input = $input; }; }
                struct FilteredNames;
                relation Seed(x:Name,y:String) = (&seed);
                relation Edge(x:Name,y:String) = (&edges);
                relation Block(x:Name) = (&blocked);
                relation Reach(x:Name,y:String);
                output Reach;
                Reach(x,y) :- Seed(x,y);
                Reach(x,z) :- Reach(x,y), Edge(y,z), !Block(z);
            }
            .collect::<BTreeSet<_>>()
        };
    }
    let expected = BTreeSet::from([
        (Name::from("a"), Name::from("b")),
        (Name::from("a"), Name::from("c")),
    ]);
    assert_eq!(
        run!(::mini_linq::passes::translate_pull, sequence),
        expected
    );
    assert_eq!(run!(::cq_datalog::passes::print_push, structures), expected);
}

#[test]
fn push_holes_match_pull_for_owned_heads_and_constant_filtered_keys() {
    // Names are owned non-Copy values; modifier kinds are dictionary codes.
    let (alice, bob) = (vec![1_i64], vec![2_i64]);
    let kinds = StringDictionary::new(["blocked", "other"]);
    let names = [(alice.clone(),), (bob.clone(),), (alice.clone(),)];
    let modifiers = [
        (exact(&kinds, "blocked"), bob.clone()),
        (exact(&kinds, "other"), alice.clone()),
    ];
    macro_rules! run {
        ($printer:path, $input:ident) => {
            cq_datalog::datalog! {
                pipeline { cq = pipeline { print = $printer; print_input = $input; }; }
                struct Names;
                relation Name(name:Vec<i64>) = (&names);
                relation Modifier(kind:cq_load::StringCode in (kinds),name:Vec<i64>) = (&modifiers);
                relation Allowed(name:Vec<i64>,n:i32);
                output Allowed;
                Allowed(name.clone(), 0) :- Name(name), !Modifier("blocked",name);
                Allowed(name.clone(), *n + 1) :- Allowed(name,n), where n < 2;
            }
            .collect::<BTreeSet<_>>()
        };
    }
    let pull = run!(::mini_linq::passes::translate_pull, sequence);
    let push = run!(::cq_datalog::passes::print_push, structures);
    assert_eq!(push, pull);
    assert_eq!(
        push,
        BTreeSet::from([(alice.clone(), 0), (alice.clone(), 1), (alice, 2)])
    );
}

#[test]
fn variable_heads_admit_borrowed_answer_columns_on_pull_and_push() {
    let (alice, bob) = (vec![1_i64], vec![2_i64]);
    let kinds = StringDictionary::new(["blocked", "other"]);
    let names = [(alice.clone(),), (bob.clone(),), (alice.clone(),)];
    let modifiers = [
        (exact(&kinds, "blocked"), bob.clone()),
        (exact(&kinds, "other"), alice.clone()),
    ];
    // Variable heads project stored (borrowed) non-Copy columns; `*n + 1` and
    // the constant are owned values in the same answer row (DL-AnswerOwnership).
    macro_rules! run {
        ($printer:path, $input:ident) => {
            cq_datalog::datalog! {
                pipeline { cq = pipeline { print = $printer; print_input = $input; }; }
                struct Names;
                relation Name(name:Vec<i64>) = (&names);
                relation Modifier(kind:cq_load::StringCode in (kinds),name:Vec<i64>) = (&modifiers);
                relation Allowed(name:Vec<i64>,n:i32);
                relation Pair(left:cq_load::StringCode in (kinds),right:Vec<i64>);
                output Allowed, Pair;
                Allowed(name, 0) :- Name(name), !Modifier("blocked",name);
                Allowed(name, *n + 1) :- Allowed(name,n), where n < 2;
                Pair(kind, name) :- Modifier(kind, name), Allowed(name, n);
            }
        };
    }
    let (allowed, pairs) = run!(::mini_linq::passes::translate_pull, sequence);
    let (pushed_allowed, pushed_pairs) = run!(::cq_datalog::passes::print_push, structures);
    let allowed: BTreeSet<_> = allowed.collect();
    let pairs: BTreeSet<_> = pairs.collect();
    assert_eq!(pushed_allowed.collect::<BTreeSet<_>>(), allowed);
    assert_eq!(pushed_pairs.collect::<BTreeSet<_>>(), pairs);
    assert_eq!(
        allowed,
        BTreeSet::from([(alice.clone(), 0), (alice.clone(), 1), (alice.clone(), 2)])
    );
    assert_eq!(pairs, BTreeSet::from([(exact(&kinds, "other"), alice)]));
}

#[test]
fn push_key_holes_preserve_empty_tuple_and_empty_input_semantics() {
    let domain = [(1_i32,), (2,), (2,)];
    macro_rules! run {
        ($printer:path, $input:ident, $blocked:expr) => {
            cq_datalog::datalog! {
                pipeline { cq = pipeline { print = $printer; print_input = $input; }; }
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
            run!(::cq_datalog::passes::print_push, structures, blocked),
            run!(::mini_linq::passes::translate_pull, sequence, blocked),
        );
    }
    assert_eq!(
        run!(::cq_datalog::passes::print_push, structures, absent),
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
    let domain = [(vec![1_i64],), (vec![2_i64],)];
    let blocked = [(vec![2_i64], 7)];
    let answer: BTreeSet<_> = cq_datalog::datalog! {
            struct Borrowed;
            relation Domain(x:Vec<i64>) = (domain);
            relation Blocked(x:Vec<i64>,y:i32) = (blocked);
            relation Out(x:Vec<i64>);
            output Out;
            Out((*x).clone()) :- Domain(x), !Blocked(x,_);
    }
    .collect();
    assert_eq!(answer, BTreeSet::from([(vec![1_i64],)]));
}

#[test]
fn multiple_outputs_are_owned_in_written_order_across_components() {
    let types = StringDictionary::new(["void", "String[]", "int", ""]);
    let (main, other) = (vec![1_i64], vec![2_i64]);
    // None of the returned iterators may borrow these owned inputs.
    let (descriptors, reach, empty) = {
        let methods = [
            (
                main.clone(),
                exact(&types, "void"),
                exact(&types, "String[]"),
            ),
            (other.clone(), exact(&types, "int"), exact(&types, "")),
        ];
        let edges = [(main.clone(), other.clone())];
        cq_datalog::datalog! {
            struct Methods;
            relation Method(name:Vec<i64>,result:cq_load::StringCode in (types),params:cq_load::StringCode in (types)) = (methods);
            relation Edge(caller:Vec<i64>,callee:Vec<i64>) = (edges);
            relation Reach(name:Vec<i64>);
            relation Empty(name:Vec<i64>);
            relation Descriptor(name:Vec<i64>,signature:Vec<cq_load::StringCode>);
            output Descriptor, Reach, Empty;
            Reach((*name).clone()) :- Method(name,"void","String[]");
            Reach((*super_).clone()) :- Reach(type_), Edge(type_,super_);
            Descriptor((*type_).clone(),vec![*ret,*params])
                :- Reach(type_), Method(type_,ret,params);
        }
    };
    let empty: std::vec::IntoIter<(Vec<i64>,)> = empty;
    assert_eq!(empty.count(), 0);
    assert_eq!(
        reach.collect::<BTreeSet<_>>(),
        BTreeSet::from([(main.clone(),), (other.clone(),)])
    );
    assert_eq!(
        descriptors.collect::<BTreeSet<_>>(),
        BTreeSet::from([
            (main, vec![exact(&types, "void"), exact(&types, "String[]")]),
            (other, vec![exact(&types, "int"), exact(&types, "")]),
        ])
    );
}

#[test]
fn literal_selections_and_filtered_negative_keys_preserve_string_values() {
    // One domain per compared column family: names (joined across every
    // relation) and modifier kinds. "missing" and "absent" are absent values.
    let names = StringDictionary::new(["main", "helper", "abstract", "λ,\""]);
    let kinds = StringDictionary::new(["public", "abstract", "static"]);
    let texts = StringDictionary::new(["literal"]);
    let name = |text: &str| (exact(&names, text),);
    let literal = exact(&texts, "literal");
    let nodes = [name("main"), name("helper"), name("abstract"), name("λ,\"")];
    let modifiers = [
        (exact(&kinds, "public"), exact(&names, "main"), 1),
        (exact(&kinds, "abstract"), exact(&names, "abstract"), 7),
        (exact(&kinds, "abstract"), exact(&names, "abstract"), 7),
        (exact(&kinds, "static"), exact(&names, "helper"), 2),
    ];
    let edges = [
        (exact(&names, "main"), exact(&names, "helper")),
        (exact(&names, "helper"), exact(&names, "abstract")),
        (exact(&names, "helper"), exact(&names, "λ,\"")),
    ];
    let (reached, concrete, no_static, absent, present, tags) = cq_datalog::datalog! {
        struct Constants;
        relation Node(name:cq_load::StringCode in (names)) = (nodes);
        relation Modifier(kind:cq_load::StringCode in (kinds),name:cq_load::StringCode in (names),flag:i32) = (modifiers);
        relation Edge(caller:cq_load::StringCode in (names),callee:cq_load::StringCode in (names)) = (edges);
        relation Concrete(name:cq_load::StringCode in (names));
        relation Reach(name:cq_load::StringCode in (names));
        relation NoStatic(name:cq_load::StringCode in (names));
        relation Absent(name:cq_load::StringCode in (names));
        relation Present(name:cq_load::StringCode in (names));
        relation Tags(name:cq_load::StringCode in (names),text:cq_load::StringCode,flag:i32,enabled:bool);
        output Reach, Concrete, NoStatic, Absent, Present, Tags;
        Concrete(*type_) :- Node(type_), !Modifier("abstract",type_,_);
        Reach(*type_) :- Modifier("public",type_,1), !Modifier("abstract",type_,_);
        Reach(*super_) :- Reach(type_), Edge(type_,super_), !Modifier("abstract",super_,_);
        NoStatic(*type_) :- Node(type_), !Modifier("static",type_,_);
        Absent(*type_) :- Node(type_), !Modifier("missing",_,_), !Modifier("abstract","absent",7);
        Present(*type_) :- Node(type_), !Modifier("abstract","abstract",7);
        Tags(*r#type,{ literal },-7,true) :- Concrete(r#type);
    };
    let concrete: BTreeSet<_> = concrete.collect();
    assert_eq!(
        concrete,
        BTreeSet::from([name("main"), name("helper"), name("λ,\"")])
    );
    assert_eq!(reached.collect::<BTreeSet<_>>(), concrete);
    assert_eq!(
        no_static.collect::<BTreeSet<_>>(),
        BTreeSet::from([name("main"), name("abstract"), name("λ,\"")])
    );
    assert_eq!(absent.collect::<BTreeSet<_>>(), nodes.into_iter().collect());
    assert_eq!(present.count(), 0);
    assert_eq!(
        tags.collect::<BTreeSet<_>>(),
        concrete
            .into_iter()
            .map(|(name,)| (name, literal, -7, true))
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
    let answer: std::vec::IntoIter<(Vec<i64>, i32)> = {
        let input = vec![(vec![1_i64], 1), (vec![1_i64], 1), (vec![2_i64], 2)];
        let answer = cq_datalog::datalog! {
            struct Facts;
            relation E(name:Vec<i64>,value:i32) = (&input);
            output E;
        };
        assert_eq!(input.len(), 3);
        answer
    };
    let answer: Vec<_> = answer.collect();
    assert_eq!(answer.len(), 2);
    assert_eq!(
        answer.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(vec![1_i64], 1), (vec![2_i64], 2)])
    );
    let empty = cq_datalog::datalog! {
        struct EmptyFacts;
        relation E(x:Vec<i64>) = (Vec::<(Vec<i64>,)>::new());
        output E;
    };
    assert_eq!(empty.count(), 0);
}

#[test]
fn mixed_edb_idb_outputs_keep_written_order_and_ownership() {
    let (derived, facts, empty) = {
        let input = vec![(vec![1_i64],), (vec![1_i64],), (vec![2_i64],)];
        let empty: Vec<(i32,)> = Vec::new();
        cq_datalog::datalog! {
            struct Mixed;
            relation E(name:Vec<i64>) = (&input);
            relation Empty(x:i32) = (&empty);
            relation A(name:Vec<i64>);
            output A, E, Empty;
            A([name.as_slice(), &[0]].concat()) :- E(name);
        }
    };
    let facts: Vec<_> = facts.collect();
    assert_eq!(facts.len(), 2);
    assert_eq!(
        facts.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(vec![1_i64],), (vec![2_i64],)])
    );
    assert_eq!(
        derived.collect::<BTreeSet<_>>(),
        BTreeSet::from([(vec![1_i64, 0],), (vec![2_i64, 0],)])
    );
    assert_eq!(empty.count(), 0);
}

#[test]
fn edb_only_replay_preserves_grouped_host_schema() {
    let mut input = std::collections::BTreeMap::new();
    input.insert((vec![1_i64],), vec![(1_i32,), (1,), (2,)]);
    let source = cq_datalog::datalog! {
        struct Facts;
        relation E(name:Vec<i64>,value:i32) = (&input);
        output E;
    };
    let stratified = cq_datalog::passes::incremental! {
        pipeline { cq_datalog::passes::storage, cq_datalog::passes::fork,
            cq_datalog::passes::print_rounds };
        struct Facts;
        relation E(name:Vec<i64>,value:i32) = (&input);
        output E;
        strata {}
    };
    let seminaive = cq_datalog::passes::storage! {
        pipeline { cq_datalog::passes::fork, cq_datalog::passes::print_rounds };
        struct Facts;
        relation E(name:Vec<i64>,value:i32) = (&input);
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
            BTreeSet::from([(vec![1_i64], 1), (vec![1_i64], 2)])
        );
    }
}

#[test]
fn owned_string_constants_filter_each_keyset_row_without_moving_captures() {
    // Owned `String` operands are closed values translated once into codes of
    // the compared column's domain; the owned head constant is a `Vec`.
    let xs = StringDictionary::new(["a", "b", "c"]);
    let kinds = StringDictionary::new(["static", "public"]);
    let code = |text: &str| (exact(&xs, text),);
    let domain = [code("a"), code("b"), code("c")];
    let modifiers = [
        (exact(&kinds, "static"), exact(&xs, "a")),
        (exact(&kinds, "static"), exact(&xs, "b")),
        (exact(&kinds, "public"), exact(&xs, "c")),
        (exact(&kinds, "static"), exact(&xs, "a")),
    ];
    let (allowed, absent, present, literal) = cq_datalog::datalog! {
        struct OwnedConstants;
        relation D(x:cq_load::StringCode in (xs)) = (domain);
        relation M(kind:cq_load::StringCode in (kinds),x:cq_load::StringCode in (xs)) = (modifiers);
        relation Allowed(x:cq_load::StringCode in (xs));
        relation Absent(x:cq_load::StringCode in (xs));
        relation Present(x:cq_load::StringCode in (xs));
        relation Literal(x:cq_load::StringCode in (xs),value:Vec<i64>);
        output Allowed, Absent, Present, Literal;
        Allowed(*x) :- D(x), !M(String::from("static"),x);
        Absent(*x) :- D(x), !M(String::from("missing"),_);
        Present(*x) :- D(x), !M(String::from("static"),String::from("b"));
        Literal(*x,vec![7_i64]) :- D(x);
    };
    assert_eq!(allowed.collect::<Vec<_>>(), [code("c")]);
    assert_eq!(
        absent.collect::<BTreeSet<_>>(),
        domain.iter().copied().collect()
    );
    assert_eq!(present.count(), 0);
    assert_eq!(
        literal.collect::<BTreeSet<_>>(),
        domain
            .into_iter()
            .map(|(name,)| (name, vec![7_i64]))
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

    let nodes = StringDictionary::new(["a", "b", "c", "d"]);
    let node = |text: &str| exact(&nodes, text);
    let (facts, nodes_out, paths, shared_targets, missing, reach) = {
        let edges = std::collections::BTreeMap::from([
            ((node("a"),), vec![(node("b"),), (node("b"),)]),
            ((node("b"),), vec![(node("c"),), (node("c"),)]),
            ((node("c"),), vec![(node("d"),), (node("d"),)]),
        ]);
        let domain: Vec<_> = ["a", "a", "b", "b", "c", "c", "d", "d"]
            .into_iter()
            .map(|name| (node(name),))
            .collect();
        let edge_scans = Cell::new(0);
        let edge_visits = Cell::new(0);
        let domain_scans = Cell::new(0);
        let domain_visits = Cell::new(0);
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
            relation E(x:cq_load::StringCode,y:cq_load::StringCode) = (input);
            relation D(x:cq_load::StringCode) = (domain_input);
            relation Paths(x:cq_load::StringCode,z:cq_load::StringCode);
            relation SharedTargets(x:cq_load::StringCode,z:cq_load::StringCode);
            relation Missing(x:cq_load::StringCode);
            relation Reach(x:cq_load::StringCode,y:cq_load::StringCode);
            output E, D, Paths, SharedTargets, Missing, Reach;
            Paths(*x,*z) :- E(x,y), E(y,z);
            SharedTargets(*x,*z) :- E(x,y), E(z,y);
            Missing(*x) :- D(x), !E(x,_);
            Reach(*x,*y) :- E(x,y);
            Reach(*x,*z) :- Reach(x,y), E(y,z);
        };
        // Count original-host traversal, not computed-head evaluations: pure
        // Rust expressions may move as soon as their free variables are bound.
        assert_eq!((edge_scans.get(), edge_visits.get()), (1, 6));
        assert_eq!((domain_scans.get(), domain_visits.get()), (1, 8));
        // Printer realization coverage separately checks that layouts, keys
        // and outputs share the canonical host rather than the duplicate input.
        assert_eq!(edges.len(), 3);
        assert_eq!(domain.len(), 8);
        answer
    };
    let facts: Vec<_> = facts.collect();
    let nodes_out: Vec<_> = nodes_out.collect();
    assert_eq!(facts.len(), 3);
    assert_eq!(nodes_out.len(), 4);
    assert_eq!(
        facts.into_iter().collect::<BTreeSet<_>>(),
        [("a", "b"), ("b", "c"), ("c", "d")]
            .into_iter()
            .map(|(x, y)| (node(x), node(y)))
            .collect()
    );
    assert_eq!(
        paths.collect::<BTreeSet<_>>(),
        [("a", "c"), ("b", "d")]
            .into_iter()
            .map(|(x, y)| (node(x), node(y)))
            .collect()
    );
    assert_eq!(
        shared_targets.collect::<BTreeSet<_>>(),
        ["a", "b", "c"]
            .into_iter()
            .map(|x| (node(x), node(x)))
            .collect()
    );
    assert_eq!(missing.collect::<Vec<_>>(), [(node("d"),)]);
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
        .map(|(x, y)| (node(x), node(y)))
        .collect()
    );
}

#[test]
fn canonical_negative_only_inputs_are_deduplicated_once() {
    let (reasons, codes) = StringDictionary::encode(["reason", "reason", "other"]);
    let blocked = [(1, codes[0]), (1, codes[1]), (3, codes[2])];
    let domain = [(1,), (1,), (2,), (2,), (3,)];
    let result = cq_datalog::datalog! {
        struct NegativeOnly;
        relation D(x:i32) = (&domain);
        relation Blocked(x:i32,reason:cq_load::StringCode) = (&blocked);
        relation Out(x:i32);
        output Out, Blocked;
        Out(*x) :- D(x), !Blocked(x,_);
    };
    let (out, blocked_rows) = result;
    assert_eq!(out.collect::<Vec<_>>(), [(2,)]);
    let mut blocked_rows: Vec<_> = blocked_rows.collect();
    blocked_rows.sort();
    assert_eq!(
        blocked_rows,
        [
            (1, exact(&reasons, "reason")),
            (3, exact(&reasons, "other"))
        ]
    );
}

#[test]
fn later_delta_atoms_keep_written_bindings_guards_and_constant_probes() {
    let alloc = [(10, 1), (10, 1), (20, 1), (30, 1)];
    let assign = [(0, 1), (2, 1), (2, 1), (3, 2), (4, 3)];
    let tags = [(10, 7), (10, 7), (20, 8), (30, 7)];
    let closed = [(10, 3), (20, 3), (30, 4)];
    let labels = StringDictionary::new(["ok"]);
    let names = [
        (10, exact(&labels, "ok")),
        (20, exact(&labels, "ok")),
        (30, exact(&labels, "ok")),
    ];
    let answer = cq_datalog::datalog! {
        struct PointsTo;
        relation Alloc(heap:i32,var:i32) = (&alloc);
        relation Assign(to:i32,from:i32) = (&assign);
        relation Tags(heap:i32,kind:i32) = (&tags);
        relation Closed(heap:i32,kind:i32) = (&closed);
        relation Names(heap:i32,name:cq_load::StringCode in (labels)) = (&names);
        relation VarPointsTo(heap:i32,var:i32);
        output VarPointsTo;
        VarPointsTo(*heap,*var) :- Alloc(heap,var);
        VarPointsTo(*heap,*to) :-
            VarPointsTo(other,from), VarPointsTo(heap,from), Assign(to,from),
            where to != 0, if 10 / *to > 0,
            Tags(heap,7), Closed(heap,1 + 2), Names(heap,"ok");
        VarPointsTo(*heap,*to) :-
            Assign(to,from), where to != 0, if 10 / *to > 0,
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
    let mut rounds = Vec::new();
    let edges = [(1, 2), (2, 3), (3, 4)];
    let seed = [(1,), (1,)];
    let (even, odd) = cq_datalog::datalog! {
        observe = (|round, count| rounds.push((round, count)));
        struct Alternating;
        relation Edge(x:i32,y:i32) = (&edges);
        relation Seed(x:i32) = (&seed);
        relation Even(x:i32);
        relation Odd(x:i32);
        output Even, Odd;
        Even(*x) :- Seed(x);
        Odd(*y) :- Edge(x,y), Even(x);
        Even(*y) :- Edge(x,y), Odd(x);
    };
    assert_eq!(even.collect::<BTreeSet<_>>(), BTreeSet::from([(1,), (3,)]));
    assert_eq!(odd.collect::<BTreeSet<_>>(), BTreeSet::from([(2,), (4,)]));
    // Each publication has exactly one fresh row, alternating between members.
    // Keeping the other member's previous delta would increase this count.
    // Layout reset/build order is checked independently by printer realization
    // tests; an opaque `if` call count would not measure delta-row visits.
    assert_eq!(rounds, [(0, 1), (1, 1), (2, 1), (3, 1)]);
}

#[test]
fn same_round_duplicates_from_one_hole_are_admitted_once() {
    let mut rounds = Vec::new();
    // Seed and step candidates each repeat within one hole's round.
    let edges = [(1, 5), (2, 5), (5, 7), (5, 7), (6, 7), (5, 8), (7, 8)];
    let seed = [(1,), (2,), (6,)];
    let reach = cq_datalog::datalog! {
        observe = (|round, count| rounds.push((round, count)));
        struct OneHole;
        relation Edge(x:i32,y:i32) = (&edges);
        relation Seed(x:i32) = (&seed);
        relation Out(y:i32);
        output Out;
        Out(*y) :- Seed(x), Edge(x,y);
        Out(*z) :- Out(y), Edge(y,z);
    };
    assert_eq!(
        reach.collect::<BTreeSet<_>>(),
        BTreeSet::from([(5,), (7,), (8,)])
    );
    // Seed candidates 5, 5, 7 admit {5, 7}; step candidates 7, 8, 8 admit {8}.
    assert_eq!(rounds, [(0, 2), (1, 1)]);
}

#[test]
fn same_round_duplicates_from_two_holes_are_admitted_once() {
    let mut rounds = Vec::new();
    let left = [(1,), (2,), (3,)];
    let right = [(2,), (3,), (4,)];
    let both = cq_datalog::datalog! {
        observe = (|round, count| rounds.push((round, count)));
        struct TwoHoles;
        relation Left(x:i32) = (&left);
        relation Right(x:i32) = (&right);
        relation Both(x:i32);
        output Both;
        Both(*x) :- Left(x);
        Both(*x) :- Right(x);
    };
    let rows: Vec<_> = both.collect();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(1,), (2,), (3,), (4,)])
    );
    // Six candidates from two seed holes publish one delta of four rows.
    assert_eq!(rounds, [(0, 4)]);
}

#[test]
fn owned_delta_keys_and_later_zero_key_runs_preserve_guard_order() {
    let (a, b, c) = (vec![1_i64], vec![2_i64], vec![3_i64]);
    let (reach, flags) = {
        let edges = [
            (a.clone(), b.clone(), 0_i32),
            (a.clone(), b.clone(), 1),
            (b.clone(), c.clone(), 2),
            (c.clone(), a.clone(), 1),
            (a.clone(), b.clone(), 1),
        ];
        let seed = [(a.clone(),), (a.clone(),)];
        let flag = [(7_i32,), (7,)];
        cq_datalog::datalog! {
            struct OwnedRuns;
            relation Edge(x:Vec<i64>,y:Vec<i64>,divisor:i32) = (&edges);
            relation Seed(x:Vec<i64>) = (&seed);
            relation Flag(flag:i32) = (&flag);
            relation Reach(x:Vec<i64>);
            relation SeenFlag(flag:i32);
            output Reach, SeenFlag;
            Reach((*x).clone()) :- Seed(x);
            Reach((*y).clone()) :- Edge(x,y,d), where d != 0, if 8 / *d > 0, Reach(x);
            Reach((*x).clone()) :- SeenFlag(f), where f == 7, Reach(x);
            SeenFlag(*f) :- Flag(f), Reach(x);
        }
    };
    let reach: Vec<_> = reach.collect();
    assert_eq!(reach.len(), 3);
    assert_eq!(
        reach.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([(a,), (b,), (c,)])
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
        Reach(next) :- Reach(x), where x < 4, for next in [*x + 1, *x + 1];
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
    let label = vec![7_i64];
    let mut rounds = Vec::new();
    let (reach, labels) = cq_datalog::datalog! {
        observe = (|round, count| rounds.push((round, count)));
        struct Captured;
        relation Seed(x:i32) = (facts.seeds);
        relation Reach(x:i32);
        relation Label(x:i32,label:Vec<i64>);
        output Reach, Label;
        Reach(*x) :- Seed(x);
        Reach(increment(*x)) :- Reach(x), where x < facts.limit;
        Label(*x,label.clone()) :- Reach(x);
    };
    assert_eq!(
        reach.collect::<BTreeSet<_>>(),
        BTreeSet::from([(0,), (1,), (2,), (3,)])
    );
    assert_eq!(
        labels.collect::<BTreeSet<_>>(),
        (0..=3).map(|x| (x, vec![7_i64])).collect()
    );
    assert!(!rounds.is_empty());
    assert_eq!(facts.seeds, [(0,)]);
}

#[test]
fn heapalloc_filters_captured_constants_before_known_membership() {
    struct Constants {
        object_array: i32,
        string_array: i32,
    }
    fn run(alloc: Vec<(i32, i32)>, constants: &Constants) -> BTreeSet<(i32, i32)> {
        cq_datalog::datalog! {
            struct Heaps;
            relation Alloc(h: i32, t: i32) = (alloc);
            relation Component(a: i32, c: i32) = ([(1, 11), (2, 12), (3, 13)]);
            relation Super(c: i32, t: i32) = ([(11, 11), (12, 12), (13, 13), (13, 1)]);
            relation Compat(h: i32, b: i32);
            output Compat;
            HeapAllocHelper: Compat(*heap, *baseheap) :-
                Alloc(heap, heaptype), Alloc(baseheap, baseheaptype),
                Component(baseheaptype, comptype), Super(comptype, heaptype),
                where baseheaptype != constants.object_array && baseheaptype != constants.string_array;
        }
        .collect()
    }
    let alloc = vec![
        (10, 1),
        (20, 2),
        (30, 3),
        (31, 3),
        (40, 11),
        (50, 12),
        (60, 13),
    ];
    assert_eq!(
        run(
            alloc.clone(),
            &Constants {
                object_array: 1,
                string_array: 2,
            },
        ),
        BTreeSet::from([(10, 30), (10, 31), (60, 30), (60, 31)])
    );
    assert_eq!(
        run(
            alloc,
            &Constants {
                object_array: 2,
                string_array: 3,
            },
        ),
        BTreeSet::from([(40, 10)])
    );
    assert!(
        run(
            Vec::new(),
            &Constants {
                object_array: 1,
                string_array: 2,
            },
        )
        .is_empty()
    );
}
