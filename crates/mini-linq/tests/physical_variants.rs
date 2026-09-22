//! Heterogeneous provider choices, noncanonical layouts, and outside providers.
//! Requires general physical selection. Do not supply a table of query-local
//! relation, index, or operator names to make this acceptance test pass.
#![recursion_limit = "512"]

use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Value(i32);

// The result representation is defined by the caller, without a dependency
// from either the common realizer or the example providers back to this file.
macro_rules! nested_output {
    (
        reply_to ($($callback:tt)+) { $($state:tt)* }
        plan { $($plan:tt)* }
        environment { $($environment:tt)* }
        request { output $name:ident
            fields { 0 = ($a:expr); 1 = ($b:expr); 2 = ($c:expr); }
            whole { $($whole:tt)* }
        }
    ) => {
        $($callback)+! {
            resume { $($state)* }
            witness {
                row ((($a, $b), $c))
                fields { 0 = ($a); 1 = ($b); 2 = ($c); }
            }
        }
    };
}
pub(crate) use nested_output as nested_provider;

macro_rules! chain_ra {
    ($l:ident, $r:ident, $s:ident) => {
        ::mini_linq::index_demands! {
            struct Chain;
            relation $l(a: Value);
            relation $r(a: Value, b: Value);
            relation $s(b: Value, c: Value);
            answer(a, b, c) :- $l(a), $r(a, b), $s(b, c);
            relational {
                l = rename $l {a -> a};
                r = rename $r {a -> a, b -> b};
                s = rename $s {b -> b, c -> c};
                j0 = natural_join l with r;
                j1 = natural_join j0 with s;
                output j1 as answer(a, b, c);
            }
        }
    };
}

mod hash_last {
    use super::*;

    mod wiring {
        pub(crate) use ::mini_linq::{access_to_set_loops as access, realize_physical};
        #[allow(unused_imports)]
        pub(crate) use cq_example_providers::{
            hash_distinct as distinct_state, hash_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [Value],
        R: &'a HashMap<Value, Vec<Value>>,
        S: &'a HashMap<Value, Vec<Value>>,
    ) -> HashSet<(&'a Value, &'a Value, &'a Value)> {
        chain_ra!(L, R, S)
    }
}

mod nested_result {
    use super::*;

    mod wiring {
        pub(crate) use ::mini_linq::{access_to_set_loops as access, realize_physical};
        #[allow(unused_imports)]
        pub(crate) use cq_example_providers::{
            hash_distinct as distinct_state, hash_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [Value],
        R: &'a HashMap<Value, Vec<Value>>,
        S: &'a BTreeMap<Value, Vec<Value>>,
    ) -> HashSet<((&'a Value, &'a Value), &'a Value)> {
        chain_ra!(L, R, S)
    }
}

mod reversed_layout {
    use super::*;

    mod wiring {
        pub(crate) use ::mini_linq::{access_to_set_loops as access, realize_physical};
        #[allow(unused_imports)]
        pub(crate) use cq_example_providers::{
            hash_distinct as distinct_state, hash_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a HashMap<Value, Vec<Value>>,
        R: &'a BTreeMap<Value, Vec<Value>>,
    ) -> HashSet<(&'a Value, &'a Value, &'a Value)> {
        ::mini_linq::index_demands! {
            struct Reversed;
            relation L(first: Value, second: Value);
            relation R(first: Value, second: Value);
            answer(a, x, z) :- L(z, x), R(z, a);
            relational {
                l = rename L {first -> z, second -> x};
                r = rename R {first -> z, second -> a};
                j = natural_join l with r;
                output j as answer(a, x, z);
            }
        }
    }
}

mod name_collision {
    use super::*;

    mod wiring {
        pub(crate) use ::mini_linq::{access_to_set_loops as access, realize_physical};
        #[allow(unused_imports)]
        pub(crate) use cq_example_providers::{
            hash_distinct as distinct_state, hash_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [Value],
        R: &'a HashMap<Value, Vec<Value>>,
    ) -> HashSet<(&'a Value, &'a Value)> {
        let __cq_store_L = 11;
        let __cq_index_i0 = 17;
        let output = ::mini_linq::index_demands! {
            struct Collision;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            answer(__cq_index_i0, b) :- L(__cq_index_i0), R(__cq_index_i0, b);
            relational {
                l = rename L {a -> __cq_index_i0};
                r = rename R {a -> __cq_index_i0, b -> b};
                j = natural_join l with r;
                output j as answer(__cq_index_i0, b);
            }
        };
        assert_eq!((__cq_store_L, __cq_index_i0), (11, 17));
        output
    }
}

#[test]
fn replacing_only_one_relation_family_preserves_results() {
    let left = [Value(1), Value(2), Value(8)];
    let middle = HashMap::from([
        (Value(1), vec![Value(10), Value(11), Value(11)]),
        (Value(2), vec![Value(20)]),
    ]);
    let right = HashMap::from([
        (Value(10), vec![Value(100)]),
        (Value(11), vec![Value(110), Value(111)]),
        (Value(90), vec![Value(900)]),
    ]);
    let actual = hash_last::run(&left, &middle, &right);
    let mut expected = HashSet::new();
    for a in &left {
        for (ra, values) in &middle {
            for b in values {
                for (sb, values) in &right {
                    for c in values {
                        if a == ra && b == sb {
                            expected.insert((a, b, c));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 3);
}

#[test]
fn map_rows_and_index_ordinal_follow_rename_mapping() {
    let left = HashMap::from([
        (Value(1), vec![Value(10), Value(11)]),
        (Value(2), vec![Value(20)]),
    ]);
    let right = BTreeMap::from([
        (Value(1), vec![Value(100), Value(101)]),
        (Value(9), vec![Value(900)]),
    ]);
    let actual = reversed_layout::run(&left, &right);
    let mut expected = HashSet::new();
    for (z, values) in &left {
        for x in values {
            for (rz, values) in &right {
                for a in values {
                    if z == rz {
                        expected.insert((a, x, z));
                    }
                }
            }
        }
    }
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 4);
    for (a, x, z) in actual {
        assert!(right.values().flatten().any(|value| std::ptr::eq(value, a)));
        assert!(left.values().flatten().any(|value| std::ptr::eq(value, x)));
        assert!(left.keys().any(|value| std::ptr::eq(value, z)));
    }
}

#[test]
fn caller_defined_result_provider_preserves_borrowed_fields() {
    let left = [Value(1)];
    let middle = HashMap::from([(Value(1), vec![Value(10)])]);
    let right = BTreeMap::from([(Value(10), vec![Value(100), Value(101)])]);
    let actual = nested_result::run(&left, &middle, &right);
    assert_eq!(actual.len(), 2);
    for ((a, b), c) in actual {
        assert!(std::ptr::eq(a, &left[0]));
        assert!(std::ptr::eq(b, &middle[&Value(1)][0]));
        assert!(right[&Value(10)].iter().any(|value| std::ptr::eq(value, c)));
    }
}

#[test]
fn generated_temporaries_do_not_capture_caller_or_logical_names() {
    let left = [Value(1)];
    let right = HashMap::from([(Value(1), vec![Value(10)])]);
    let actual = name_collision::run(&left, &right);
    assert_eq!(actual.len(), 1);
    let (a, b) = actual.into_iter().next().unwrap();
    assert!(std::ptr::eq(a, &left[0]));
    assert!(std::ptr::eq(b, &right[&Value(1)][0]));
}
