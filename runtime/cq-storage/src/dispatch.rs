//! Direct host reads and lazy fallback storage.
//!
//! Structure selection records, for every keyed read of a host, either a
//! native read the host's declaration states (IF-HostDecl) or a fallback
//! provider. Generated code makes exactly one call for that record: a native
//! function below on the host value, or a read of the fallback built once on
//! first use ([`Lazy`]). There is no capability ranking: a host whose Rust type
//! lacks the declared capability is a rustc trait error at that call.
use crate::rows::{Columns, Concat, OwnKey};
use crate::{Candidates, ExactMembership, IndexRead, OrderedCursor, OrderedRead};
use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::ops::Bound;
use std::rc::Rc;

/// An immutable native index on one explicit, zero-based declaration slot.
///
/// `Read::Key` must be a one-field tuple of the column's declared owned type.
/// Its payload must implement [`Columns`] and yield every other column in
/// original declaration order. A lookup yields every matching occurrence
/// exactly once, in source chronology, from the same immutable relation as
/// the host's row enumeration. The cursor must borrow or pin that relation,
/// not this temporary handle or the dispatch wrapper.
pub trait IndexedColumn<const COLUMN: usize>: Clone {
    type Read: IndexRead;
    fn indexed_column(&self) -> Self::Read;
}

impl<H: IndexedColumn<COLUMN>, const COLUMN: usize> IndexedColumn<COLUMN> for &H {
    type Read = H::Read;

    fn indexed_column(&self) -> Self::Read {
        H::indexed_column(*self)
    }
}

/// Read an explicitly indexed declaration slot, recorded from a host
/// declaration whose key is one non-leading column.
#[derive(Clone)]
pub struct ColumnHost<H, const COLUMN: usize>(H);

impl<H, const COLUMN: usize> ColumnHost<H, COLUMN> {
    pub fn new(host: H) -> Self {
        Self(host)
    }
}

impl<H: IndexedColumn<COLUMN>, const COLUMN: usize> IndexRead for ColumnHost<H, COLUMN> {
    type Key = <H::Read as IndexRead>::Key;
    type Item = <H::Read as IndexRead>::Item;
    type Cursor = <H::Read as IndexRead>::Cursor;

    fn lookup(&self, key: &Self::Key) -> Self::Cursor {
        self.0.indexed_column().lookup(key)
    }
}

impl<H, K: ?Sized, const COLUMN: usize> ExactMembership<K> for ColumnHost<H, COLUMN>
where
    H: IndexedColumn<COLUMN>,
    H::Read: ExactMembership<K>,
{
    fn contains(&self, key: &K) -> bool {
        self.0.indexed_column().contains(key)
    }
}

/// One fallback per query/read, initialized on first use and shared by probes.
pub struct Lazy<T, F> {
    state: Rc<LazyState<T, F>>,
}
struct LazyState<T, F> {
    value: OnceCell<T>,
    build: RefCell<Option<F>>,
}
impl<T, F: FnOnce() -> T> Lazy<T, F> {
    pub fn new(build: F) -> Self {
        Self {
            state: Rc::new(LazyState {
                value: OnceCell::new(),
                build: RefCell::new(Some(build)),
            }),
        }
    }
    /// Formal rule IF-Fallback: Initialize shared fallback storage once, on first use.
    pub fn get(&self) -> &T {
        self.state.value.get_or_init(|| {
            self.state
                .build
                .borrow_mut()
                .take()
                .expect("one initialization")()
        })
    }
}
impl<T, F> Clone for Lazy<T, F> {
    fn clone(&self) -> Self {
        Self {
            state: Rc::clone(&self.state),
        }
    }
}

/// Normalize native host columns without changing a cursor's Copy capability.
pub struct ColumnRead<H, Q>(H, std::marker::PhantomData<fn() -> Q>);

impl<H: Clone, Q> Clone for ColumnRead<H, Q> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), std::marker::PhantomData)
    }
}

impl<H, Q: OwnKey> IndexRead for ColumnRead<H, Q>
where
    H: IndexRead<Key = Q::Owned>,
    H::Item: Columns,
{
    type Key = Q;
    type Item = <H::Item as Columns>::Row;
    type Cursor = ColumnCursor<H::Cursor>;

    fn lookup(&self, key: &Q) -> Self::Cursor {
        ColumnCursor(self.0.lookup(&key.own_key()))
    }
}

/// A stateless column projection; copies only when the underlying cursor does.
#[derive(Clone, Copy)]
pub struct ColumnCursor<I>(I);

impl<I: Iterator> Iterator for ColumnCursor<I>
where
    I::Item: Columns,
{
    type Item = <I::Item as Columns>::Row;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(Columns::columns)
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.0.nth(n).map(Columns::columns)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<I: ExactSizeIterator> ExactSizeIterator for ColumnCursor<I> where I::Item: Columns {}

/// Formal rule IF-Dispatch: a recorded native `exact` read on the host's
/// declared key. The borrowed query key is owned once for the lookup and
/// native items are normalized to borrowed columns.
#[inline]
pub fn seek<H, Q>(host: H, key: &Q) -> ColumnCursor<H::Cursor>
where
    Q: OwnKey,
    H: IndexRead<Key = Q::Owned>,
    H::Item: Columns,
{
    ColumnCursor(host.lookup(&key.own_key()))
}

/// A recorded native `membership` read on the host's declared key.
#[inline]
pub fn contains<H, Q>(host: H, key: &Q) -> bool
where
    Q: OwnKey,
    H: ExactMembership<Q::Owned>,
{
    host.contains(&key.own_key())
}

/// A recorded native `membership` read on a `borrowed` host declaration:
/// the host stores borrowed key columns and takes the query key unchanged.
#[inline]
pub fn contains_borrowed<H, Q>(host: H, key: &Q) -> bool
where
    H: ExactMembership<Q>,
{
    host.contains(key)
}

/// The number of candidates under a prefix of a recorded native `candidates` read.
#[inline]
pub fn candidate_count<H, Q>(host: H, key: &Q) -> usize
where
    Q: OwnKey,
    H: Candidates<Key = Q::Owned>,
{
    host.count(&key.own_key())
}

/// The candidates under a prefix of a recorded native `candidates` read.
#[inline]
pub fn candidate_keys<H, Q>(host: H, key: &Q) -> ColumnCursor<H::Iter>
where
    Q: OwnKey,
    H: Candidates<Key = Q::Owned>,
    H::Item: Columns,
{
    ColumnCursor(host.keys(&key.own_key()))
}

/// A recorded native `exact` read retained across probes: the key-normalizing
/// handle whose cursor is `Copy` (`supports [retain]`).
#[inline]
pub fn retained<H, Q>(host: H) -> ColumnRead<H, Q>
where
    Q: OwnKey,
    H: IndexRead<Key = Q::Owned>,
    H::Item: Columns,
    H::Cursor: Copy,
{
    ColumnRead(host, std::marker::PhantomData)
}

/// A recorded native `exact` read expanded by exact cursor lengths
/// (`supports [expand]`).
#[inline]
pub fn expanded<H, Q>(host: H) -> ColumnRead<H, Q>
where
    Q: OwnKey,
    H: IndexRead<Key = Q::Owned>,
    H::Item: Columns,
    H::Cursor: Copy + ExactSizeIterator,
{
    ColumnRead(host, std::marker::PhantomData)
}

/// A recorded native constant point read (`exact` on the declared key).
#[inline]
pub fn point<H, K>(host: H, key: &K) -> H::Cursor
where
    H: crate::PointLookup<Key = K>,
{
    host.point_lookup(key)
}

/// A recorded native `range` read: a seek to the lower bound of a window on
/// the host's one-column declared prefix.
#[inline]
pub fn range<H, Q>(host: H, low: Bound<Q>, high: Bound<Q>) -> H::RangeCursor
where
    H: SeekRangeLookup<Q>,
{
    host.seek_range_lookup(low, high)
}

/// A recorded native `ordered` window: the host's key-ordered cursor from its
/// start, cut to the bounds.
#[inline]
pub fn window<'a, H, Q>(
    host: &'a H,
    low: Bound<Q>,
    high: Bound<Q>,
) -> RelationalWindow<H::Cursor<'a>, Q>
where
    H: OrderedRead + 'a,
    H::Cursor<'a>: OrderedCursor,
    <H::Cursor<'a> as OrderedCursor>::Key: Columns<Row = Q>,
    <H::Cursor<'a> as OrderedCursor>::Value: Columns,
    Q: Ord + Copy + Concat<<<H::Cursor<'a> as OrderedCursor>::Value as Columns>::Row>,
{
    RelationalWindow {
        cursor: host.ordered(),
        low,
        high,
        done: false,
    }
}

/// A seek whose bound can be copied without invoking user Clone code.
pub trait SeekRangeLookup<Q>: OrderedRead + Copy {
    type Row: Copy;
    type RangeCursor: Iterator<Item = Self::Row>;
    fn seek_range_lookup(self, low: Bound<Q>, high: Bound<Q>) -> Self::RangeCursor;
}
/// A total fused ordered window. Inverted/exclusive-equal bounds yield no rows;
/// unlike a two-sided BTreeMap::range, this never panics on such bounds.
pub struct RelationalWindow<C, Q> {
    cursor: C,
    low: Bound<Q>,
    high: Bound<Q>,
    done: bool,
}
impl<C, Q> Iterator for RelationalWindow<C, Q>
where
    C: OrderedCursor,
    C::Key: Columns<Row = Q>,
    Q: Ord + Copy,
    C::Value: Columns,
    Q: Concat<<C::Value as Columns>::Row>,
{
    type Item = <Q as Concat<<C::Value as Columns>::Row>>::Row;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        for (key, value) in self.cursor.by_ref() {
            let key = key.columns();
            let below = match self.low {
                Bound::Included(b) => key < b,
                Bound::Excluded(b) => key <= b,
                Bound::Unbounded => false,
            };
            if below {
                continue;
            }
            let above = match self.high {
                Bound::Included(b) => key > b,
                Bound::Excluded(b) => key >= b,
                Bound::Unbounded => false,
            };
            if above {
                self.done = true;
                return None;
            }
            return Some(key.concat(value.columns()));
        }
        self.done = true;
        None
    }
}
impl<C, Q> std::iter::FusedIterator for RelationalWindow<C, Q>
where
    C: OrderedCursor,
    C::Key: Columns<Row = Q>,
    Q: Ord + Copy,
    C::Value: Columns,
    Q: Concat<<C::Value as Columns>::Row>,
{
}

impl<'h, T: Ord + Copy, V> SeekRangeLookup<(&'h T,)> for &'h BTreeMap<(T,), Vec<V>>
where
    &'h V: Columns,
    (&'h T,): Concat<<&'h V as Columns>::Row>,
{
    type Row = <(&'h T,) as Concat<<&'h V as Columns>::Row>>::Row;
    type RangeCursor = RelationalWindow<crate::ordered::GroupedCursor<'h, (T,), V>, (&'h T,)>;
    fn seek_range_lookup(self, low: Bound<(&'h T,)>, high: Bound<(&'h T,)>) -> Self::RangeCursor {
        let mut cursor = self.ordered();
        if let Bound::Included((value,)) | Bound::Excluded((value,)) = low {
            cursor.seek_value(&(*value,));
        }
        RelationalWindow {
            cursor,
            low,
            high,
            done: false,
        }
    }
}

/// A deferred membership set. Construction imposes no key capability on a
/// direct host path; hashing is required only when the fallback is selected.
pub struct KeySet<I: Iterator, const EAGER: bool = true> {
    #[cfg(any(feature = "counters", feature = "operator-timing"))]
    profile: Option<crate::counters::DeferredBuild>,
    source: RefCell<Option<I>>,
    keys: OnceCell<RefCell<crate::instrument::HashSet<I::Item>>>,
}
impl<K, V, I: Iterator<Item = (K, V)>, const EAGER: bool> crate::Build<I>
    for KeySet<std::iter::Map<I, fn((K, V)) -> K>, EAGER>
{
    fn build(source: I) -> Self {
        Self {
            #[cfg(any(feature = "counters", feature = "operator-timing"))]
            profile: crate::counters::defer_build(),
            source: RefCell::new(Some(source.map(|(k, _)| k))),
            keys: OnceCell::new(),
        }
    }
}
impl<K: Eq + std::hash::Hash, I: Iterator<Item = K>, const EAGER: bool> ExactMembership<K>
    for KeySet<I, EAGER>
{
    fn contains(&self, key: &K) -> bool {
        crate::count!(lookups);
        let mut keys = self
            .keys
            .get_or_init(|| {
                crate::count!(index_builds);
                #[cfg(any(feature = "counters", feature = "operator-timing"))]
                let _profile = self.profile.as_ref().map(|p| p.start());
                let keys: crate::instrument::HashSet<_> = if EAGER {
                    let source = self.source.borrow_mut().take().expect("one initialization");
                    #[cfg(feature = "counters")]
                    let source = source.inspect(|_| {
                        crate::count!(distinct_insertions);
                    });
                    source.collect()
                } else {
                    crate::instrument::HashSet::default()
                };
                #[cfg(any(feature = "counters", feature = "operator-timing"))]
                if let Some(p) = &self.profile {
                    p.cardinality(keys.len());
                }
                RefCell::new(keys)
            })
            .borrow_mut();
        if keys.contains(key) {
            return true;
        }
        if EAGER {
            return false;
        }
        #[cfg(any(feature = "counters", feature = "operator-timing"))]
        let _profile = self.profile.as_ref().map(|p| p.start());
        let mut source = self.source.borrow_mut();
        if let Some(rows) = source.as_mut() {
            for candidate in rows.by_ref() {
                let found = &candidate == key;
                crate::count!(distinct_insertions);
                keys.insert(candidate);
                if found {
                    #[cfg(any(feature = "counters", feature = "operator-timing"))]
                    if let Some(p) = &self.profile {
                        p.cardinality(keys.len());
                    }
                    return true;
                }
            }
        }
        *source = None;
        #[cfg(any(feature = "counters", feature = "operator-timing"))]
        if let Some(p) = &self.profile {
            p.cardinality(keys.len());
        }
        false
    }
}
