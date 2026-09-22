//! Execution storage is caller-selected; equality and ownership are provider laws.

mod ordered {
    mod wiring {
        pub use ::mini_linq::*;
        pub use cq_example_providers::{
            ordered_distinct as distinct_state, ordered_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    #[test]
    fn ordered_distinct_streams_first_occurrences_without_hashing() {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
        struct Row(i32); // Deliberately no Hash.
        let mut rows = mini_linq::pipeline_to_distinct_iter! {
            a = scan row in ([Row(2), Row(2), Row(1), Row(3)]) yield (row,);
            b = project a as (row,) yield (row);
            unique = distinct b;
            return unique;
        };
        assert_eq!(rows.next(), Some(Row(2)));
        assert_eq!(rows.collect::<Vec<_>>(), [Row(1), Row(3)]);
    }

    #[test]
    fn both_set_endpoints_consume_non_clone_non_hash_values() {
        #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
        struct Row(i32);
        let pulled = mini_linq::pipeline_to_set! {
            a = scan row in ([Row(2), Row(1), Row(2)]) yield (row,);
            b = project a as (row,) yield (row);
            unique = distinct b;
            return unique;
        };
        let pushed = mini_linq::access_to_set_loops! {
            answer(row) => (row) :- R(row) => bind row <- ([Row(2), Row(1), Row(2)]);
        };
        let expected = std::collections::BTreeSet::from([Row(1), Row(2)]);
        assert_eq!(pulled, expected);
        assert_eq!(pushed, expected);
    }
}

mod custom {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static STARTS: AtomicUsize = AtomicUsize::new(0);
    static INSERTS: AtomicUsize = AtomicUsize::new(0);

    // An injective integer key represents equality for Row. Keeping the key
    // requires neither cloning the payload nor an Eq/Hash impl on that payload.
    macro_rules! select_distinct {
        (operator custom_unique; new) => {{
            STARTS.fetch_add(1, Ordering::SeqCst);
            Vec::<i32>::new()
        }};
        (operator custom_unique; insert($state:expr, $row:expr)) => {{
            INSERTS.fetch_add(1, Ordering::SeqCst);
            let key = $row.0;
            if $state.contains(&key) { false } else { $state.push(key); true }
        }};
        (operator ordered_unique; $($task:tt)*) => {
            ::cq_example_providers::ordered_distinct! { operator ordered_unique; $($task)* }
        };
    }
    pub(crate) use select_distinct;
    mod wiring {
        pub(crate) use super::select_distinct as distinct_state;
        pub use ::mini_linq::pipeline_to_distinct_iter;
    }
    use wiring as mini_linq;

    #[test]
    fn policy_dispatches_by_operator_and_preserves_lazy_demand() {
        struct Row(i32);
        let mut rows = mini_linq::pipeline_to_distinct_iter! {
            a = scan row in ([Row(2), Row(2), Row(1)].into_iter()
                .chain(std::iter::once_with(|| panic!("polled past demand")))) yield (row,);
            b = project a as (row,) yield (row);
            custom_unique = distinct b;
            return custom_unique;
        };
        assert_eq!(STARTS.load(Ordering::SeqCst), 0);
        assert_eq!(INSERTS.load(Ordering::SeqCst), 0);
        assert_eq!(rows.next().unwrap().0, 2);
        assert_eq!(STARTS.load(Ordering::SeqCst), 1);
        assert_eq!(INSERTS.load(Ordering::SeqCst), 1);
        assert_eq!(rows.next().unwrap().0, 1);
        assert_eq!(INSERTS.load(Ordering::SeqCst), 3);
        drop(rows);
        let ordered = mini_linq::pipeline_to_distinct_iter! {
            a = scan x in ([3, 1, 3]) yield (x,);
            b = project a as (x,) yield (x);
            ordered_unique = distinct b;
            return ordered_unique;
        };
        assert_eq!(ordered.collect::<Vec<_>>(), [3, 1]);
    }
}

mod no_state {
    // No distinct_state or set_sink binding: these endpoints do not retain one.
    use ::mini_linq;

    #[test]
    fn support_and_exists_need_no_state_provider() {
        struct Row;
        let rows = mini_linq::pipeline_to_support_iter! {
            a = scan row in ([Row, Row]) yield (row,);
            b = project a as (row,) yield (row);
            unique = distinct b;
            return unique;
        };
        assert_eq!(rows.count(), 2);
        assert!(mini_linq::pipeline_to_exists! {
            a = scan row in ([Row]) yield (row,);
            b = project a as (row,) yield (row);
            unique = distinct b;
            return unique;
        });
    }
}

mod custom_sink {
    #[derive(Debug, PartialEq, Eq)]
    struct Row(i32); // Neither Clone, Hash nor Ord.
    struct Rows(Box<[Row]>);

    macro_rules! sink {
        (operator $name:ident; new) => {
            Vec::<Row>::new()
        };
        (operator $name:ident; insert($state:expr, $row:expr)) => {{
            let row = $row;
            if !$state.contains(&row) {
                $state.push(row);
            }
        }};
        (operator $name:ident; finish($state:expr)) => {
            Rows($state.into_boxed_slice())
        };
    }
    pub(crate) use sink;
    mod wiring {
        pub(crate) use super::sink as set_sink;
        pub use ::mini_linq::{access_to_set_loops, pipeline_to_set};
    }
    use wiring as mini_linq;

    #[test]
    fn finish_controls_the_output_representation_for_both_endpoints() {
        let pulled: Rows = mini_linq::pipeline_to_set! {
            a = scan row in ([Row(2), Row(1), Row(2)]) yield (row,);
            b = project a as (row,) yield (row);
            unique = distinct b;
            return unique;
        };
        let pushed: Rows = mini_linq::access_to_set_loops! {
            answer(row) => (row) :- R(row) => bind row <- ([Row(2), Row(1), Row(2)]);
        };
        assert_eq!(&*pulled.0, &[Row(2), Row(1)]);
        assert_eq!(pushed.0, pulled.0);
    }
}
