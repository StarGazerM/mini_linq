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
    let radix = cq_storage::RadixRun::<_, _>::build(input.into_iter());
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
fn native_and_fallback_retained_reads_borrow_stable_storage() {
    use cq_storage::dispatch::{Lazy, expanded};
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
    // A recorded native read: one direct call on the declared host.
    let direct = expanded::<_, (&i64,)>(&native);
    require_expand(&direct);
    let cursor = retain(direct, &(&key,));
    assert_eq!(cursor.map(|(v,)| *v).collect::<Vec<_>>(), [11, 11, 12]);
    assert_eq!(builds.get(), 0);
    assert_eq!(native.lookups.get(), 1);
    // A recorded fallback: built once, on first use, and shared.
    let indirect = fallback.get();
    require_expand(indirect);
    let cursor = retain(indirect, &(&key,));
    assert_eq!(cursor.map(|(v,)| *v).collect::<Vec<_>>(), [11, 11, 12]);
    assert_eq!(builds.get(), 1);
    assert_eq!(retain(fallback.get(), &(&key,)).len(), 3);
    assert_eq!(builds.get(), 1);
}

#[derive(Clone)]
struct NonLeading<H>(H);

impl<H: IndexRead> cq_storage::dispatch::IndexedColumn<1> for NonLeading<H> {
    type Read = H;

    fn indexed_column(&self) -> H {
        self.0.clone()
    }
}

#[test]
fn explicit_column_reads_retain_source_borrows() {
    use cq_storage::dispatch::{ColumnHost, Lazy, expanded};
    let key = 2i64;
    let values = [(11i64,), (11,), (12,)];
    let native = Ledger {
        rows: SortedRun::build(values.iter().map(|value| ((key,), value))),
        lookups: Cell::new(0),
    };
    let host = NonLeading(&native);
    let borrowed = &host;
    let builds = Cell::new(0);
    let fallback = Lazy::new(|| {
        builds.set(builds.get() + 1);
        SortedRun::<_, _>::build(values.iter().map(|value| ((&key,), (&value.0,))))
    });
    // The temporary slot handle is gone before the cursor is consumed.
    // Forwarding through &&host exercises generated borrowed host bindings.
    let read = expanded::<_, (&i64,)>(ColumnHost::<_, 1>::new(&borrowed));
    let cursor = retain(read, &(&key,));
    assert_eq!(cursor.len(), 3);
    assert_eq!(
        cursor.map(|(value,)| *value).collect::<Vec<_>>(),
        [11, 11, 12]
    );
    assert_eq!(
        cursor.len(),
        3,
        "a copied witness remains independently consumable"
    );
    assert_eq!((builds.get(), native.lookups.get()), (0, 1));

    // A slot declared only `exact` answers membership through its recorded fallback.
    assert!(cq_storage::ExactMembership::contains(
        fallback.get(),
        &(&key,)
    ));
    assert_eq!((builds.get(), native.lookups.get()), (1, 1));
}
