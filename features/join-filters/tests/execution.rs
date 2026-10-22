use cq_join_filters::{AlwaysTrue, BloomFilter};
use cq_load::{MinMax, ZoneRows};
use std::{collections::BTreeSet, num::NonZeroUsize};

type FilterAlias = BloomFilter<2>;
struct ConservativeOnly;
impl<I: Iterator<Item = (i64, ())>> cq_storage::Build<I> for ConservativeOnly {
    fn build(source: I) -> Self {
        source.for_each(drop);
        Self
    }
}
impl cq_storage::ConservativeMembership<i64> for ConservativeOnly {
    fn test(&self, _: &i64) -> bool {
        true
    }
}

macro_rules! run {
    ($left:ident, $right:ident, $filter:ty, $($passes:path),+) => {
        mini_linq::passes::select! {
            pipeline {
                cq_rewrites::join_filter(l), cq_rewrites::retype(l join_filter => $filter), $($passes),+
            };
            physical {
                relation L(a:i64,b:i64) = (&$left);
                relation R(c:i64,d:i64) = (&$right);
                l = pipe rename L@0 {a -> x,b -> y};
                r = pipe rename R@1 {c -> x,d -> z};
                s = pipe range select r if (z > 1);
                b = let project s keep {x,z};
                j = pipe probe first [x] l,b;
                out = pipe canon project j keep {x,y,z}; return out as answer(x,y,z);
            }

        }
        .into_iter()
        .map(|(x, y, z)| (*x, *y, *z))
        .collect::<BTreeSet<_>>()
    };
}

#[test]
fn filter_corpus_agrees_with_independent_exact_join() {
    // Deterministic corpus: empty builds, all matches, no matches, duplicates,
    // saturated tiny filters, shuffled positions and extreme scalar keys.
    for seed in 0..80_i64 {
        let mut left = (0..seed)
            .map(|i| ((i * 7 + seed) % 23 - 11, i % 9))
            .collect::<Vec<_>>();
        let mut right = (0..seed)
            .map(|i| ((i * 11) % 29 - 14, i % 5))
            .collect::<Vec<_>>();
        if seed % 3 == 0 {
            left.extend([(i64::MIN, 7), (i64::MAX, 8), (0, 9), (0, 9)]);
            right.extend([(i64::MIN, 2), (i64::MAX, 3), (0, 4), (0, 4)]);
        }
        let expected = left
            .iter()
            .flat_map(|&(x, y)| {
                right
                    .iter()
                    .filter(move |&&(k, z)| k == x && z > 1)
                    .map(move |&(_, z)| (x, y, z))
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            run!(
                left,
                right,
                AlwaysTrue,
                mini_linq::passes::sequence,
                mini_linq::passes::sequential,
                mini_linq::passes::translate_pull
            ),
            expected
        );
        assert_eq!(
            run!(
                left,
                right,
                FilterAlias,
                mini_linq::passes::sequence,
                mini_linq::passes::sequential,
                mini_linq::passes::translate_pull
            ),
            expected
        );
        assert_eq!(run!(left, right, AlwaysTrue, own_push::push), expected);
        assert_eq!(
            run!(
                left,
                right,
                ConservativeOnly,
                mini_linq::passes::sequence,
                mini_linq::passes::sequential,
                mini_linq::passes::translate_pull
            ),
            expected
        );
        let left =
            ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(left, NonZeroUsize::new(3).unwrap());
        assert_eq!(
            run!(
                left,
                right,
                BloomFilter<2>,
                mini_linq::passes::sequence,
                mini_linq::passes::sequential,
                mini_linq::passes::translate_pull
            ),
            expected
        );
        assert_eq!(run!(left, right, AlwaysTrue, own_push::push), expected);
    }
}
