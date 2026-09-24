#[allow(unused_imports)]
mod execution {
    pub use ::mini_linq::*;
    pub use cq_state_providers::{hash_distinct as distinct_state, hash_set_sink as set_sink};
}
use execution as mini_linq;

use std::collections::HashSet;

#[test]
fn set_access_evaluates_all_candidates_and_deduplicates_projected_values() {
    let calls = std::cell::Cell::new(0);
    let observed = &calls;
    let rows = mini_linq::access_to_set! {
        out(x) => (x % 2) :-
        R(x) => bind (x,) <- ([(1,), (3,), (2,), (4,), (4,)]),
        Keep(x) => if ({ observed.set(observed.get() + 1); x < 4 });
    };
    assert_eq!(calls.get(), 5);
    assert_eq!(rows, HashSet::from([0, 1]));
    assert_eq!(
        mini_linq::access_to_set! { out() => (()) :- Unit() => if (true); },
        HashSet::from([()]),
    );
    assert!(mini_linq::access_to_set! { out() => (()) :- Unit() => if (false); }.is_empty());
}

#[test]
fn set_pipeline_owns_projected_values_without_a_duplicate_clone_set() {
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct Value(i32);

    let rows = mini_linq::pipeline_to_set! {
        iter0 = scan (value,) in ([(Value(1),), (Value(1),), (Value(2),)]) yield (value,);
        iter1 = project iter0 as (value,) yield (value);
        iter2 = distinct iter1;
        return iter2;
    };
    assert_eq!(rows, HashSet::from([Value(1), Value(2)]));
}

mod composed {
    pub(super) static CALLS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    macro_rules! observed_set {
        ($($plan:tt)*) => {{
            CALLS.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed);
            ::mini_linq::pipeline_to_set! { $($plan)* }
        }};
    }
    pub(crate) use observed_set;
    mod wiring {
        pub(crate) use super::observed_set as pipeline_to_set;
        pub use ::mini_linq::*;
        #[allow(unused_imports)]
        pub(crate) use cq_state_providers::{
            hash_distinct as distinct_state, hash_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    pub(super) fn rows() -> std::collections::HashSet<i32> {
        mini_linq::access_to_set! {
            out(x) => (x) :- R(x) => bind (x,) <- ([(1,), (1,), (2,)]);
        }
    }
}

#[test]
fn set_continuation_is_selected_by_an_ordinary_module_alias() {
    assert_eq!(composed::rows(), HashSet::from([1, 2]));
    assert_eq!(
        composed::CALLS.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[cfg(feature = "pipeline-support-iter")]
#[test]
fn streaming_access_preserves_support_without_allocating_a_distinct_set() {
    let stream: Vec<_> = mini_linq::access_to_support_iter! {
        out(k) => ([k]) :- R(k,_v) => bind (k,_v) <- ([(1_i64,10), (1,11), (2,20)]);
    }
    .collect();
    let support: HashSet<_> = mini_linq::access_to_set! {
        out(k) => ([k]) :- R(k,_v) => bind (k,_v) <- ([(1_i64,10), (1,11), (2,20)]);
    };
    assert_eq!(stream, vec![[1], [1], [2]]);
    assert_eq!(stream.into_iter().collect::<HashSet<_>>(), support);
}
