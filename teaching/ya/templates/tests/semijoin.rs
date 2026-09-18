//! This suite enters at RA, so it does not depend on the YA scheduler.
mod wiring {
    pub use ::mini_linq::{
        access_to_set, materialized_hash_program as choose_relation_lowering, pipeline_to_set,
        result_index_demands as choose_index_policy,
    };
}
use std::collections::BTreeSet;
use wiring as mini_linq;

mini_linq::choose_index_policy! {
    struct PartialKey;
    relation Left(a:i32, payload:i32); relation Right(a:i32, witness:i32);
    answer(a,p) :- Left(a,p), Right(a,w);
    relational {
        l = rename Left {a -> a, payload -> p};
        r = rename Right {a -> a, witness -> w};
        s = semijoin l with r;
        out = project s keep {a,p};
        output out as answer(a,p);
    }
}

mini_linq::choose_index_policy! {
    struct FullKey;
    relation Left(a:i32, b:i32); relation Right(a:i32, b:i32);
    answer(a,b) :- Left(a,b), Right(a,b);
    relational {
        l = rename Left {a -> a, b -> b};
        r = rename Right {a -> a, b -> b};
        s = semijoin l with r;
        out = project s keep {a,b};
        output out as answer(a,b);
    }
}

mini_linq::choose_index_policy! {
    struct EmptyKey;
    relation Left(a:i32, b:i32); relation Right(c:i32);
    answer(a,b) :- Left(a,b), Right(c);
    relational {
        l = rename Left {a -> a, b -> b};
        r = rename Right {c -> c};
        s = semijoin l with r;
        out = project s keep {a,b};
        output out as answer(a,b);
    }
}

mini_linq::choose_index_policy! {
    struct CompositeKey;
    relation Left(a:i32, b:i32, payload:String);
    relation Right(a:i32, b:i32, witness:i32);
    answer(a,b,p) :- Left(a,b,p), Right(a,b,w);
    relational {
        l = rename Left {a -> a, b -> b, payload -> p};
        r = rename Right {a -> a, b -> b, witness -> w};
        s = semijoin l with r;
        out = project s keep {a,b,p};
        output out as answer(a,b,p);
    }
}

fn binary(mask: usize) -> Vec<(i32, i32)> {
    (0..4)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| (i / 2, i % 2))
        .collect()
}

#[test]
fn semijoin_preserves_exactly_supported_left_tuples() {
    // Enumerate all pairs of binary relations over {0,1}. The oracle is a
    // direct existential predicate, independent of indexes and Access syntax.
    for left_mask in 0..16 {
        for right_mask in 0..16 {
            let mut left = binary(left_mask);
            let mut right = binary(right_mask);
            left.extend(left.clone());
            right.extend(right.clone());
            let expected: BTreeSet<_> = left
                .iter()
                .filter(|(a, _)| right.iter().any(|(b, _)| a == b))
                .copied()
                .collect();
            assert_eq!(
                PartialKey::run(left.clone(), right.clone())
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
                expected
            );
            let expected: BTreeSet<_> = left
                .iter()
                .filter(|row| right.contains(row))
                .copied()
                .collect();
            assert_eq!(
                FullKey::run(left.clone(), right.clone())
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
                expected
            );
            let expected: BTreeSet<_> =
                left.iter().filter(|_| !right.is_empty()).copied().collect();
            assert_eq!(
                EmptyKey::run(left, right.into_iter().map(|(a, _)| (a,)))
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
                expected
            );
        }
    }
}

#[test]
fn composite_keys_require_every_column_and_keep_owned_payloads() {
    let left = [
        (1, 2, "kept".to_owned()),
        (1, 3, "wrong-second".to_owned()),
        (0, 2, "wrong-first".to_owned()),
    ];
    assert_eq!(
        CompositeKey::run(left, [(1, 2, 7), (1, 2, 8)]),
        vec![(1, 2, "kept".to_owned())]
    );
}
