use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

type SnapshotAlias<K, V> = cq_storage::RadixRun<K, V>;

struct NativeHost {
    source: Vec<(i64, i64)>,
    index: BTreeMap<(i64,), Vec<(i64,)>>,
    lookups: Cell<usize>,
    scans: Cell<usize>,
}
impl NativeHost {
    fn new(source: Vec<(i64, i64)>) -> Self {
        let mut index: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for &(key, value) in &source {
            index.entry((key,)).or_default().push((value,));
        }
        Self {
            source,
            index,
            lookups: Cell::new(0),
            scans: Cell::new(0),
        }
    }
}
impl<'a> cq_storage::rows::Rows for &'a NativeHost {
    type Row = (&'a i64, &'a i64);
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.scans.set(self.scans.get() + 1);
        self.source.iter().map(|(key, value)| (key, value))
    }
}
#[derive(Clone, Copy)]
struct NativeCursor<'a>(&'a [(i64,)]);
impl<'a> Iterator for NativeCursor<'a> {
    type Item = &'a (i64,);
    fn next(&mut self) -> Option<Self::Item> {
        let (first, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(first)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.0.len(), Some(self.0.len()))
    }
}
impl ExactSizeIterator for NativeCursor<'_> {}
impl<'a> cq_storage::IndexRead for &'a NativeHost {
    type Key = (i64,);
    type Item = &'a (i64,);
    type Cursor = NativeCursor<'a>;
    fn lookup(&self, key: &Self::Key) -> Self::Cursor {
        self.lookups.set(self.lookups.get() + 1);
        NativeCursor(self.index.get(key).map_or(&[], Vec::as_slice))
    }
}

macro_rules! query {
    ($left:expr, $right:expr, $visits:ident, [$($host:tt)*], [$($providers:tt)*], [$($reads:tt)*]) => {
        mini_linq::passes::sequence! {
            pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
            physical {
                relation L(a:i64) = ($left); relation R(a:i64,b:i64) = ($right) $($host)*;
                l = pipe rename L@0 {a -> x};
                r = pipe rename R@1 {a -> x,b -> y};
                s = pipe semi eager [x] l,r;
                p = pipe probe first [x] s,r;
                observed = pipe guard p if ({ $visits.borrow_mut().push((*x, *y)); true });
                out = let canon project observed keep {x,y};
                return out as answer(x,y);
            }
            structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; $($providers)* sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
                l host scan = @host_scan as ::core::iter::Map<_,_> [];
                r host scan = @host_scan as ::core::iter::Map<_,_> [];
                $($reads)*
                s share = lookup r [x] first s force p;
                s repr = tuples witness { r [x] at s as _ };
                p stage = batch 2 expand;
                out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
            }
        }
    };
}

#[test]
fn retained_host_alias_uses_native_copy_cursor_without_rebuilding_or_reprobing() {
    let left = [(2_i64,), (1,), (4,), (1,)];
    let right = NativeHost::new(vec![(1, 10), (2, 20), (1, 11), (1, 10)]);
    let expected = vec![
        (2, 20),
        (1, 10),
        (1, 11),
        (1, 10),
        (1, 10),
        (1, 11),
        (1, 10),
    ];
    let visits = &RefCell::new(Vec::new());
    let answers: BTreeSet<_> = query!(&left, &right, visits,
        [host [a] reads [exact] supports [expand]], [],
        [r native result(s) = exact [x] supports [expand];
         r native result(p) = exact [x] supports [expand];])
    .map(|(key, value)| (*key, *value))
    .collect();
    assert_eq!(answers, expected.iter().copied().collect());
    assert_eq!(*visits.borrow(), expected);
    assert_eq!(right.lookups.get(), left.len());
    assert_eq!(right.scans.get(), 0);
}

#[test]
fn noncopy_native_cursor_uses_declared_snapshot_fallback_and_preserves_duplicates() {
    let left = [(2_i64,), (1,), (4,), (1,)];
    let right = BTreeMap::from([
        ((1_i64,), vec![(10_i64,), (11,), (10,)]),
        ((2,), vec![(20,)]),
    ]);
    let expected = vec![
        (2, 20),
        (1, 10),
        (1, 11),
        (1, 10),
        (1, 10),
        (1, 11),
        (1, 10),
    ];
    let visits = &RefCell::new(Vec::new());
    let answers: BTreeSet<_> = query!(&left, &right, visits, [],
        [snapshot_alias = SnapshotAlias<_,_> { reads [exact, range, membership], supports [retain, expand, keys], build typed };],
        [r host result(s) = @snapshot_alias as SnapshotAlias<_,_> [x] supports [expand];
         r host result(p) = @snapshot_alias as SnapshotAlias<_,_> [x] supports [expand];])
        .map(|(key, value)| (*key, *value))
        .collect();
    assert_eq!(answers, expected.iter().copied().collect());
    assert_eq!(*visits.borrow(), expected);
}

#[test]
fn retained_host_cursor_crosses_membership_without_a_second_lookup() {
    for (left, payload, members) in [
        (vec![], vec![], vec![]),
        (vec![(1, 9)], vec![], vec![(9,)]),
        (vec![(1, 9)], vec![(1, 10)], vec![]),
        (
            vec![(2, 9), (1, 9), (4, 9), (1, 8), (1, 9)],
            vec![(1, 10), (2, 20), (1, 11), (1, 10)],
            vec![(9,), (9,)],
        ),
    ] {
        let expected: Vec<_> = left
            .iter()
            .filter(|&&(_, tag)| members.contains(&(tag,)))
            .flat_map(|&(key, _)| {
                payload
                    .iter()
                    .filter_map(move |&(other, value)| (key == other).then_some((key, value)))
            })
            .collect();
        let right = NativeHost::new(payload);
        let visits = &RefCell::new(Vec::new());
        // Select the same native exact cursor at both demands. Automatic
        // membership selection intentionally uses a fallback when the host
        // advertises exact reads but not native membership.
        let answers: BTreeSet<_> = cq_rewrites::retain_lookups! {
            pipeline {
                mini_linq::passes::sequence,
                mini_linq::passes::sequential,
                mini_linq::passes::translate_pull
            };
            physical {
                relation L(a:i64,c:i64) = (&left);
                relation R(a:i64,b:i64) = (&right) host [a] reads [exact] supports [retain, expand];
                relation M(c:i64) = (&members);
                l = pipe rename L@0 {a -> x,c -> z};
                r = pipe rename R@1 {a -> x,b -> y};
                m = let canon rename M@2 {c -> z};
                s = pipe semi eager [x] l,r;
                checked = pipe probe first [z] s,m;
                projected = pipe project checked keep {x};
                p = pipe probe first [x] projected,r;
                observed = pipe guard p if ({ visits.borrow_mut().push((*x, *y)); true });
                out = let canon project observed keep {x,y};
                return out as answer(x,y);
            }
            structures {
                providers {
                    host_scan = ::core::iter::Map<_, _> { reads [], build typed };
                    hash_set = ::cq_storage::HashSet<_> { reads [membership], build typed, shared };
                    sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
                }
                l host scan = @host_scan as ::core::iter::Map<_,_> [];
                r host scan = @host_scan as ::core::iter::Map<_,_> [];
                m host scan = @host_scan as ::core::iter::Map<_,_> [];
                r native result(s) = exact [x] supports [retain, expand];
                r native result(p) = exact [x] supports [retain, expand];
                m = @hash_set as ::cq_storage::HashSet<_> [z] membership;
                m canon = [z];
                out rows = @sort as ::cq_storage::dedup::Sort;
                out canon = rows;
            }
        }
        .map(|(key, value)| (*key, *value))
        .collect();
        assert_eq!(answers, expected.iter().copied().collect());
        assert_eq!(*visits.borrow(), expected);
        // Includes rejected membership rows and missing payload keys: one first
        // lookup per input row, never a re-probe at the forcing expansion.
        assert_eq!(right.lookups.get(), left.len());
        assert_eq!(right.scans.get(), 0);
    }
}
