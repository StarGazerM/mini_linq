use cq_join_filters::{AlwaysTrue, BloomFilter};
use cq_load::{MinMax, ZoneRows};
use std::{collections::BTreeSet, num::NonZeroUsize};

type FilterAlias = BloomFilter<2>;
struct ConservativeOnly;
impl cq_storage::BuildSidecar<i64> for ConservativeOnly {
    type Builder = Self;

    fn sidecar_builder() -> Self::Builder {
        Self
    }
}
impl cq_storage::SidecarBuilder<i64> for ConservativeOnly {
    type Output = Self;

    fn observe(&mut self, _: &i64) {}
    fn finish(self) -> Self::Output {
        self
    }
}
impl cq_storage::ConservativeMembership<i64> for ConservativeOnly {
    fn test(&self, _: &i64) -> bool {
        true
    }
}

#[test]
fn selected_run_filter_borrows_nonclone_keys_and_ignores_unneeded_columns() {
    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct Key(i64);
    struct Unused;
    type Summary = (cq_load::NoBounds<Key>, MinMax<i64>, cq_load::NoBounds<Unused>);
    let left = ZoneRows::from_rows::<Summary>(
        vec![(Key(1), 7_i64, Unused), (Key(3), 8, Unused), (Key(9), 9, Unused)],
        NonZeroUsize::new(2).unwrap(),
    );
    let right = ZoneRows::from_rows::<Summary>(
        vec![(Key(1), 2_i64, Unused), (Key(1), 3, Unused), (Key(9), 0, Unused)],
        NonZeroUsize::new(2).unwrap(),
    );
    let actual: BTreeSet<_> = mini_linq::passes::select! {
        pipeline {
            cq_rewrites::join_filter(l),
            cq_rewrites::retype(b => @sorted_run as ::cq_storage::SortedRun<_, _> supports [keys]),
            cq_rewrites::retype(l join_filter => @borrowed_run_filter as ::cq_storage::RunFilterView<'_, _, _>),
            mini_linq::passes::sequence,
            mini_linq::passes::sequential,
            mini_linq::passes::translate_pull
        };
        physical {
            relation L(a:Key,b:i64,unused:Unused) = (&left);
            relation R(c:Key,d:i64,unused:Unused) = (&right);
            l = pipe rename L {a -> x,b -> y,unused -> ignored_left};
            r = pipe rename R {c -> x,d -> z,unused -> ignored_right};
            s = pipe range select r if (z > 1);
            b = let project s keep {x,z};
            j = pipe probe first [x] l,b;
            out = let canon project j keep {y,z}; return out as answer(y,z);
        }
    }.into_iter().map(|(y,z)| (*y,*z)).collect();
    assert_eq!(actual, BTreeSet::from([(7,2),(7,3)]));
}

macro_rules! run {
    ($left:ident, $right:ident, $token:ident = $filter:ty, $build:ident, $($passes:path),+) => {
        mini_linq::passes::select! {
            pipeline {
                cq_rewrites::join_filter(l),
                cq_rewrites::retype(
                    providers { $token = $filter { reads [membership], build $build, attached_filter }; }
                    l join_filter => @$token as $filter
                ),
                $($passes),+
            };
            physical {
                relation L(a:i64,b:i64) = (&$left);
                relation R(c:i64,d:i64) = (&$right);
                l = pipe rename L {a -> x,b -> y};
                r = pipe rename R {c -> x,d -> z};
                s = pipe range select r if (z > 1);
                b = let project s keep {x,z};
                j = pipe probe first [x] l,b;
                out = let canon project j keep {x,y,z}; return out as answer(x,y,z);
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
                always_true = AlwaysTrue,
                inferred,
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
                filter_alias = FilterAlias,
                inferred,
                mini_linq::passes::sequence,
                mini_linq::passes::sequential,
                mini_linq::passes::translate_pull
            ),
            expected
        );
        assert_eq!(
            run!(left, right, always_true = AlwaysTrue, inferred, own_push::push),
            expected
        );
        assert_eq!(
            run!(
                left,
                right,
                conservative_only = ConservativeOnly,
                inferred,
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
                bloom_filter = BloomFilter<2>,
                typed,
                mini_linq::passes::sequence,
                mini_linq::passes::sequential,
                mini_linq::passes::translate_pull
            ),
            expected
        );
        assert_eq!(
            run!(left, right, always_true = AlwaysTrue, inferred, own_push::push),
            expected
        );
    }
}

thread_local! {
    static FILTER_WORK: std::cell::Cell<(usize, usize, usize)> = const {
        std::cell::Cell::new((0, 0, 0))
    };
}

struct CountedFilter;
impl cq_storage::BuildSidecar<i64> for CountedFilter {
    type Builder = Self;

    fn sidecar_builder() -> Self::Builder {
        FILTER_WORK.with(|work| {
            let (builds, inserted, probes) = work.get();
            work.set((builds + 1, inserted, probes));
        });
        Self
    }
}
impl cq_storage::SidecarBuilder<i64> for CountedFilter {
    type Output = Self;

    fn observe(&mut self, _: &i64) {
        FILTER_WORK.with(|work| {
            let (builds, inserted, probes) = work.get();
            work.set((builds, inserted + 1, probes));
        });
    }

    fn finish(self) -> Self::Output {
        self
    }
}
impl cq_storage::ConservativeMembership<i64> for CountedFilter {
    fn test(&self, _: &i64) -> bool {
        FILTER_WORK.with(|work| {
            let (builds, inserted, probes) = work.get();
            work.set((builds, inserted, probes + 1));
        });
        // Deliberate false positives: the exact operators must still decide.
        true
    }
}

macro_rules! counted_plan {
    ($left:ident, $right:ident, [$($scan:ident),+], $($plan:tt)*) => {{
        FILTER_WORK.with(|work| work.set((0, 0, 0)));
        let rows = mini_linq::passes::select! {
            pipeline {
                $(cq_rewrites::join_filter($scan),)+
                cq_rewrites::retype(
                    providers { counted_filter = CountedFilter { reads [membership], build inferred, attached_filter }; }
                    join_filter => @counted_filter as CountedFilter
                ),
                mini_linq::passes::sequence,
                mini_linq::passes::sequential,
                mini_linq::passes::translate_pull
            };
            physical {
                relation L(a:i64,b:i64) = (&$left);
                relation R(c:i64,d:i64) = (&$right);
                l = pipe rename L {a -> x,b -> y};
                r = pipe rename R {c -> x,d -> z};
                s = pipe range select r if (z > 1);
                b = let project s keep {x,z};
                $($plan)*
                return out as answer(x,y,z);
            }
        }.map(|(x, y, z)| (*x, *y, *z)).collect::<BTreeSet<_>>();
        (rows, FILTER_WORK.with(std::cell::Cell::get))
    }};
}

#[test]
fn runtime_filter_identity_shares_only_equivalent_work() {
    let left = [(0_i64, 10_i64), (1, 11), (7, 17), (2, 12)];
    let right = (0..20_i64)
        .map(|x| (x, if x < 3 { x + 2 } else { 0 }))
        .collect::<Vec<_>>();
    let expected = BTreeSet::from([(0, 10, 2), (1, 11, 3), (2, 12, 4)]);

    let (rows, work) = counted_plan!(left, right, [l],
        q = pipe semi eager [x] l,b;
        j = pipe probe first [x] q,b;
        out = let canon project j keep {x,y,z};
    );
    assert_eq!(rows, expected);
    assert_eq!(
        work,
        (1, 3, left.len()),
        "one construction and one scan test per row"
    );

    let (rows, work) = counted_plan!(left, right, [l],
        c = let project b keep {x,z};
        q = pipe probe first [x] l,b;
        p = pipe project q keep {x,y};
        j = pipe probe first [x] p,c;
        out = let canon project j keep {x,y,z};
    );
    assert_eq!(rows, expected);
    assert_eq!(work, (2, 6, 2 * left.len()), "different build versions");

    let (rows, work) = counted_plan!(left, right, [l],
        q = pipe probe first [x] l,b;
        j = pipe probe first [x,z] q,b;
        out = let canon project j keep {x,y,z};
    );
    assert_eq!(rows, expected);
    assert_eq!(
        work,
        (2, 6, 2 * left.len()),
        "different source index layouts"
    );

    let (rows, work) = counted_plan!(left, right, [l,m],
        m = pipe rename L {a -> x,b -> y};
        q = pipe probe first [x] l,b;
        j = pipe probe first [x] m,b;
        out = let canon union q,j;
    );
    assert_eq!(rows, expected);
    assert_eq!(
        work,
        (2, 6, 2 * left.len()),
        "different occurrences of one host"
    );
}
