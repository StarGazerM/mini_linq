//! Final Project from selected stored input, as a separate query trace target.
//! Requires general physical selection. Do not supply a table of query-local
//! relation, index, or operator names to make this acceptance test pass.
#![recursion_limit = "512"]

use std::collections::{HashMap, HashSet};

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Value(i32);

mod wiring {
    pub(crate) use ::mini_linq::{
        access_to_set_loops as access, index_demands as choose_index_policy, realize_physical,
    };
    #[allow(unused_imports)]
    pub(crate) use cq_example_providers::{
        hash_distinct as distinct_state, hash_set_sink as set_sink,
    };
}
use wiring as mini_linq;

#[allow(non_snake_case)]
fn run(R: &HashMap<Value, Vec<Value>>) -> HashSet<(&Value,)> {
    ::mini_linq::cq_to_ra_written_order! {
        struct ProjectMap;
        relation R(a: Value, b: Value);
        answer(a) :- R(a,_b);
    }
}

#[test]
fn project_requests_rows_from_the_selected_stored_operand() {
    let relation = HashMap::from([
        (Value(1), vec![Value(10), Value(11), Value(11)]),
        (Value(2), vec![]),
        (Value(3), vec![Value(30)]),
    ]);
    let result = run(&relation);
    let expected = relation
        .iter()
        .flat_map(|(a, values)| values.iter().map(move |_| (a,)))
        .collect();
    assert_eq!(result, expected);
    assert_eq!(result.len(), 2);
    for (a,) in result {
        assert!(relation.keys().any(|v| std::ptr::eq(v, a)));
    }
    assert!(run(&HashMap::new()).is_empty());
}
