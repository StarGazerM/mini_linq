mod wiring {
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        access_to_distinct_iter, access_to_set, choose_graph_or_prefix, cq_to_ra_clause_prefix,
        cq_to_ra_hypergraph, pipeline_to_distinct_iter, pipeline_to_set,
    };
    pub use ::mini_linq::{
        materialized_hash_program as choose_relation_lowering,
        result_index_demands as choose_index_policy,
    };
}
use mini_linq::{choose_graph_or_prefix, choose_index_policy};
use wiring as mini_linq;

type Amount = i32;
choose_graph_or_prefix! {
    struct AliasedSelection;
    relation R(value: Amount);
    answer(x) :- R(x), if (x >= -2147483648), if (x != 0);
}

#[allow(dead_code, clippy::all)]
mod cq_boundary {
    include!("../../../doc/planned-example/01-cq.rs");
}
#[allow(dead_code, clippy::all)]
mod ra_boundary {
    include!("../../../doc/planned-example/02-relational.rs");
}
#[allow(dead_code, clippy::all)]
mod index_boundary {
    include!("../../../doc/planned-example/03-indexes.rs");
}
#[allow(dead_code, clippy::all)]
mod access_boundary {
    include!("../../../doc/planned-example/04-access.rs");
}
#[allow(dead_code, clippy::all)]
mod pipeline_boundary {
    include!("../../../doc/planned-example/05-pipelines.rs");
}
#[allow(dead_code, clippy::all)]
mod rust_boundary {
    include!("../../../doc/planned-example/06-rust.rs");
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key(usize);
choose_graph_or_prefix! {
    struct NonOrderedKeys;
    relation R(key:Key,payload:String);
    relation S(key:Key,rank:u8);
    answer(p,n) :- R(k,p), S(k,n);
}

struct NotClone;
choose_graph_or_prefix! {
    struct UnusedInput;
    relation Used(value:i32);
    relation Unused(value:NotClone);
    answer(x) :- Used(x);
}

#[test]
fn ordinary_rust_ownership_and_unused_inputs_are_preserved() {
    assert_eq!(
        NonOrderedKeys::run([(Key(1), "a".into())], [(Key(1), 2)]),
        vec![("a".into(), 2)]
    );
    let unused = std::iter::from_fn(|| -> Option<(NotClone,)> {
        panic!("unused input must not be iterated")
    });
    assert_eq!(UnusedInput::run([(1,), (1,)], unused), vec![(1,)]);
}

choose_graph_or_prefix! {
    struct Selected;
    relation R(a: i32, b: i32);
    relation S(b: i32, c: i32);
    answer(c,a) :- R(a,b), S(b,c), if (c > 0), if (a < c);
}

choose_index_policy! {
    struct Direct;
    relation Input(key: String, payload: String);
    relation Guard(key: String);
    answer(v) :- Input(k,v), Guard(k);
    relational {
        r = rename Input {key -> k, payload -> v};
        s = rename Guard {key -> k};
        filtered = semijoin r with s;
        p = project filtered keep {v};
        output p as answer(v);
    }
}

#[test]
fn quotation_can_enter_at_each_existing_language_boundary() {
    assert_eq!(
        AliasedSelection::run([(-1,), (0,), (2,)]),
        vec![(-1,), (2,)]
    );
    let storage = Selected::build([(1, 2), (4, 2), (1, 2)], [(2, 3), (2, -1)]);
    assert_eq!(storage.materialize(), vec![(3, 1)]);
    assert_eq!(storage.query().collect::<Vec<_>>(), vec![(3, 1)]);
    assert_eq!(
        Direct::run(
            [("x".into(), "p".into()), ("y".into(), "q".into())],
            [("x".into(),)]
        ),
        vec![("p".into(),)]
    );
    // A lower local IR needs no source program wrapper.
    let unit = mini_linq::access_to_distinct_iter! { answer() => (()) :- unit() => if (true); };
    assert_eq!(unit.collect::<Vec<_>>(), vec![()]);
}

#[test]
fn complete_partial_expansions_compile_and_execute_at_all_six_boundaries() {
    let r = [(1, 2), (9, 7), (1, 2)];
    let s = [(2, 3), (7, -1)];
    let expected = vec![(1,)];
    assert_eq!(cq_boundary::PlannedExample::run(r, s), expected);
    assert_eq!(ra_boundary::PlannedExample::run(r, s), expected);
    assert_eq!(index_boundary::PlannedExample::run(r, s), expected);
    assert_eq!(access_boundary::PlannedExample::run(r, s), expected);
    assert_eq!(pipeline_boundary::PlannedExample::run(r, s), expected);
    assert_eq!(rust_boundary::PlannedExample::run(r, s), expected);
}
