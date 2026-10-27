use cq_storage::provider_contract::{require_expand, require_keys, require_retain, retain};
use cq_storage::{Build, IndexRead, Keys, RunCursor, SortedRun};
use std::cell::Cell;

// Neither provider nor cursor uses a runtime-recognized spelling.
struct Ledger<K, V> {
    rows: SortedRun<K, V>,
    lookups: Cell<usize>,
}

#[derive(Clone, Copy)]
struct Bookmark<'a, V: Copy>(RunCursor<'a, V>);

impl<V: Copy> Iterator for Bookmark<'_, V> {
    type Item = V;
    fn next(&mut self) -> Option<V> {
        self.0.next()
    }
    fn nth(&mut self, n: usize) -> Option<V> {
        self.0.nth(n)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
impl<V: Copy> ExactSizeIterator for Bookmark<'_, V> {}

impl<'a, K: Copy + Ord, V: Copy> IndexRead for &'a Ledger<K, V> {
    type Key = K;
    type Item = V;
    type Cursor = Bookmark<'a, V>;
    fn lookup(&self, key: &K) -> Self::Cursor {
        self.lookups.set(self.lookups.get() + 1);
        Bookmark((&self.rows).lookup(key))
    }
}

impl<K, V> Keys for Ledger<K, V> {
    type Key = K;
    type Iter<'a>
        = <SortedRun<K, V> as Keys>::Iter<'a>
    where
        Self: 'a;
    fn key_count(&self) -> usize {
        self.rows.key_count()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.rows.distinct_keys()
    }
}

fn expansion_law<R: IndexRead<Key = i64, Item = char> + Copy>(provider: R)
where
    R::Cursor: Copy + ExactSizeIterator,
{
    let outer = [2, 9, 1, 2];
    let ordinary: Vec<_> = outer
        .into_iter()
        .flat_map(|key| provider.lookup(&key).map(move |value| (key, value)))
        .collect();
    let witnesses: Vec<_> = outer
        .into_iter()
        .filter_map(|key| {
            let cursor = retain(provider, &key);
            let mut first = cursor;
            first.next().map(|_| (key, cursor))
        })
        .collect();
    let retained: Vec<_> = witnesses
        .iter()
        .flat_map(|&(key, cursor)| cursor.map(move |value| (key, value)))
        .collect();
    let staged: Vec<_> =
        cq_storage::stage::segmented::<2, _, _, _>(outer.into_iter(), |key| provider.lookup(key))
            .collect();
    let combined: Vec<_> =
        cq_storage::stage::segmented::<2, _, _, _>(witnesses.into_iter(), |&(_, cursor)| cursor)
            .map(|((key, _), value)| (key, value))
            .collect();
    let expected = vec![
        (2, 'b'),
        (2, 'b'),
        (2, 'c'),
        (1, 'a'),
        (2, 'b'),
        (2, 'b'),
        (2, 'c'),
    ];
    assert_eq!(ordinary, expected);
    assert_eq!(retained, expected);
    assert_eq!(staged, expected);
    assert_eq!(combined, expected);
}

#[test]
fn generic_proofs_do_no_lookup_and_cursor_types_preserve_occurrences() {
    let input = [(2, 'b'), (1, 'a'), (2, 'b'), (2, 'c')];
    let ledger = Ledger {
        rows: SortedRun::build(input.into_iter()),
        lookups: Cell::new(0),
    };
    require_retain(&ledger);
    require_expand(&ledger);
    require_keys(&ledger);
    assert_eq!(ledger.lookups.get(), 0);
    let cursor: Bookmark<'_, char> = retain(&ledger, &2);
    assert_eq!(ledger.lookups.get(), 1);
    assert_eq!(cursor.collect::<Vec<_>>(), ['b', 'b', 'c']);
    expansion_law(&ledger);
    type OrderedLedger<K, V> = SortedRun<K, V>;
    let alias = OrderedLedger::build(input.into_iter());
    expansion_law(&alias);
    let radix = cq_storage::RadixRun::build(input.into_iter());
    expansion_law(&radix);
    for keys in [
        ledger.distinct_keys().copied().collect::<Vec<_>>(),
        alias.distinct_keys().copied().collect(),
        radix.distinct_keys().copied().collect(),
    ] {
        assert_eq!(keys, [1, 2]);
    }
    assert_eq!(ledger.key_count(), 2);
    assert_eq!(alias.key_count(), 2);
    assert_eq!(radix.key_count(), 2);
}

#[test]
fn borrowed_host_dispatch_preserves_native_and_fallback_cursor_lifetimes() {
    use cq_storage::dispatch::{BorrowedHost, BorrowedRead, ExpandedRead, Lazy};
    let key = 2i64;
    let values = [(11i64,), (11,), (12,)];
    let native = Ledger {
        rows: SortedRun::build(values.iter().map(|value| ((key,), value))),
        lookups: Cell::new(0),
    };
    let builds = Cell::new(0);
    let fallback = Lazy::new(|| {
        builds.set(builds.get() + 1);
        SortedRun::<_, _>::build(values.iter().map(|value| ((&key,), (&value.0,))))
    });
    // The wrapper is temporary; both returned handles borrow stable storage.
    let direct = (&&BorrowedHost::<_, (i64,), _, _>::new(&native, &fallback)).expanded_read();
    require_expand(&direct);
    let cursor = retain(direct, &(&key,));
    assert_eq!(cursor.map(|(v,)| *v).collect::<Vec<_>>(), [11, 11, 12]);
    assert_eq!(builds.get(), 0);
    assert_eq!(native.lookups.get(), 1);
    let indirect = (&BorrowedHost::<_, (i64,), _, _>::new((), &fallback)).borrowed_read();
    require_expand(indirect);
    let cursor = retain(indirect, &(&key,));
    assert_eq!(cursor.map(|(v,)| *v).collect::<Vec<_>>(), [11, 11, 12]);
    assert_eq!(builds.get(), 1);
    assert_eq!(retain(indirect, &(&key,)).len(), 3);
    assert_eq!(builds.get(), 1);
}

#[test]
fn a_native_noncopy_cursor_uses_the_declared_retained_fallback() {
    use cq_storage::dispatch::{BorrowedHost, BorrowedRead, ExpandedRead, Lazy};
    let key = 2i64;
    let values = [(11i64,), (11,), (12,)];
    let native = std::collections::BTreeMap::from([((key,), values.to_vec())]);
    let builds = Cell::new(0);
    let fallback = Lazy::new(|| {
        builds.set(builds.get() + 1);
        SortedRun::<_, _>::build(values.iter().map(|value| ((&key,), (&value.0,))))
    });
    let read = (&&BorrowedHost::<_, (i64,), _, _>::new(&native, &fallback)).borrowed_read();
    let cursor = retain(read, &(&key,));
    assert_eq!(cursor.map(|(v,)| *v).collect::<Vec<_>>(), [11, 11, 12]);
    assert_eq!(builds.get(), 1);
    let read = (&&BorrowedHost::<_, (i64,), _, _>::new(&native, &fallback)).expanded_read();
    require_expand(read);
    assert_eq!(retain(read, &(&key,)).len(), 3);
    assert_eq!(builds.get(), 1);
}
