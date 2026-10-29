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
    ($left:expr, $right:expr, $visits:ident) => {
        mini_linq::passes::sequence! {
            pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
            physical {
                relation L(a:i64) = ($left); relation R(a:i64,b:i64) = ($right);
                l = pipe rename L@0 {a -> x};
                r = pipe rename R@1 {a -> x,b -> y};
                s = pipe semi eager [x] l,r;
                p = pipe probe first [x] s,r;
                observed = pipe guard p if ({ $visits.borrow_mut().push((*x, *y)); true });
                out = pipe canon project observed keep {x,y};
                return out as answer(x,y);
            }
            structures {
                l host scan = ::core::iter::Map<_,_> [];
                r host scan = ::core::iter::Map<_,_> [];
                r host result(s) = SnapshotAlias<_,_> [x] supports [expand];
                r host result(p) = SnapshotAlias<_,_> [x] supports [expand];
                s share = lookup r [x] first s force p;
                s repr = tuples witness { r [x] at s as _ };
                p stage = batch 2 expand;
                out canon = ::std::collections::HashSet<_>;
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
    let answers: BTreeSet<_> = query!(&left, &right, visits)
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
    let answers: BTreeSet<_> = query!(&left, &right, visits)
        .map(|(key, value)| (*key, *value))
        .collect();
    assert_eq!(answers, expected.iter().copied().collect());
    assert_eq!(*visits.borrow(), expected);
}
