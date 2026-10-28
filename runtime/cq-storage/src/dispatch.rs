//! Stable autoref dispatch. Membership first tries the borrowed query key
//! through a triple-reference receiver; double-reference receivers try native
//! reads at a declared prefix or explicitly indexed column. Single-reference
//! receivers build once.
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

/// Select an explicitly indexed declaration slot without inferring it from
/// a key type. Unsupported slots remain eligible for ordinary host fallback.
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

/// A one-probe host read. Its fallback enumerates rows without building an index.
pub struct PointHost<H, K, F>(H, F, std::marker::PhantomData<fn() -> K>);
impl<H, K, F> PointHost<H, K, F> {
    pub fn new(host: H, fallback: F) -> Self {
        Self(host, fallback, std::marker::PhantomData)
    }
}

pub trait PointRead<K> {
    type Item;
    type Cursor: Iterator<Item = Self::Item>;
    fn point(self, key: &K) -> Self::Cursor;
}
impl<H, K: Eq, F: Fn() -> I, I: Iterator> PointRead<K> for &PointHost<H, K, F> {
    type Item = I::Item;
    type Cursor = I;
    fn point(self, _: &K) -> I {
        (self.1)()
    }
}
impl<H: crate::PointLookup<Key = K>, K: Eq, F> PointRead<K> for &&PointHost<H, K, F> {
    type Item = H::Row;
    type Cursor = H::Cursor;
    fn point(self, key: &K) -> Self::Cursor {
        self.0.point_lookup(key)
    }
}

/// The key and fallback types are fixed before method resolution, so an
/// unsupported key type falls back even when another keyed read is implemented.
pub struct Host<H, K, T, F>(pub H, Lazy<T, F>, std::marker::PhantomData<fn() -> K>);
impl<H, K, T, F> Host<H, K, T, F> {
    pub fn new(host: H, fallback: &Lazy<T, F>) -> Self {
        Self(host, fallback.clone(), std::marker::PhantomData)
    }
}
impl<H: Clone, K, T, F> Clone for Host<H, K, T, F> {
    fn clone(&self) -> Self {
        Self::new(self.0.clone(), &self.1)
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

pub trait Seek<Q> {
    type Item: Copy;
    type Cursor: Iterator<Item = Self::Item>;
    fn seek(self, key: &Q) -> Self::Cursor;
}
impl<H, Q, K, T, F: FnOnce() -> T> Seek<Q> for &Host<H, K, T, F>
where
    T: IndexRead<Key = Q>,
{
    type Item = T::Item;
    type Cursor = T::Cursor;
    fn seek(self, key: &Q) -> Self::Cursor {
        self.1.get().lookup(key)
    }
}
impl<H, Q: OwnKey<Owned = K>, K: Eq, T, F> Seek<Q> for &&Host<H, K, T, F>
where
    H: IndexRead<Key = K>,
    H::Item: Columns,
{
    type Item = <H::Item as Columns>::Row;
    type Cursor = std::iter::Map<H::Cursor, fn(H::Item) -> Self::Item>;
    fn seek(self, key: &Q) -> Self::Cursor {
        self.0.lookup(&key.own_key()).map(Columns::columns)
    }
}

/// Host dispatch whose fallback borrow survives the temporary dispatch wrapper.
/// Unlike `Host`, constructing this wrapper does not clone a shared handle.
pub struct BorrowedHost<'a, H, K, T, F>(
    H,
    &'a Lazy<T, F>,
    std::marker::PhantomData<fn() -> K>,
);

impl<'a, H, K, T, F> BorrowedHost<'a, H, K, T, F> {
    pub fn new(host: H, fallback: &'a Lazy<T, F>) -> Self {
        Self(host, fallback, std::marker::PhantomData)
    }
}

/// Select the actual read handle before proving its retained cursor obligations.
pub trait BorrowedRead<Q> {
    type Read: IndexRead<Key = Q>;
    fn borrowed_read(self) -> Self::Read;
}

impl<'a, H, Q, K, T, F: FnOnce() -> T> BorrowedRead<Q>
    for &BorrowedHost<'a, H, K, T, F>
where
    &'a T: IndexRead<Key = Q>,
{
    type Read = &'a T;

    fn borrowed_read(self) -> Self::Read {
        self.1.get()
    }
}

impl<H, Q: OwnKey<Owned = K>, K: Eq, T, F> BorrowedRead<Q>
    for &&BorrowedHost<'_, H, K, T, F>
where
    H: IndexRead<Key = K>,
    H::Item: Columns,
    H::Cursor: Copy,
{
    type Read = ColumnRead<H, Q>;

    fn borrowed_read(self) -> Self::Read {
        ColumnRead(self.0.clone(), std::marker::PhantomData)
    }
}

/// Expansion dispatch skips a native read that cannot supply exact cursor lengths.
pub trait ExpandedRead<Q> {
    type Read: IndexRead<Key = Q>;
    fn expanded_read(self) -> Self::Read;
}

impl<'a, H, Q, K, T, F: FnOnce() -> T> ExpandedRead<Q>
    for &BorrowedHost<'a, H, K, T, F>
where
    &'a T: IndexRead<Key = Q>,
{
    type Read = &'a T;

    fn expanded_read(self) -> Self::Read {
        self.1.get()
    }
}

impl<H, Q: OwnKey<Owned = K>, K: Eq, T, F> ExpandedRead<Q>
    for &&BorrowedHost<'_, H, K, T, F>
where
    H: IndexRead<Key = K>,
    H::Item: Columns,
    H::Cursor: Copy + ExactSizeIterator,
{
    type Read = ColumnRead<H, Q>;

    fn expanded_read(self) -> Self::Read {
        ColumnRead(self.0.clone(), std::marker::PhantomData)
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

pub trait Contains<Q> {
    fn contains(self, key: &Q) -> bool;
}
impl<H, Q, K, T: ExactMembership<Q>, F: FnOnce() -> T> Contains<Q> for &Host<H, K, T, F> {
    fn contains(self, key: &Q) -> bool {
        self.1.get().contains(key)
    }
}
impl<H, Q: OwnKey<Owned = K>, K: Eq, T, F> Contains<Q> for &&Host<H, K, T, F>
where
    H: ExactMembership<K>,
{
    fn contains(self, key: &Q) -> bool {
        self.0.contains(&key.own_key())
    }
}

// Borrowed key sets can answer the query's references directly, without
// cloning the key into the declared owned type or initializing the fallback.
impl<H, Q: OwnKey<Owned = K>, K, T, F> Contains<Q> for &&&Host<H, K, T, F>
where
    H: ExactMembership<Q>,
{
    fn contains(self, key: &Q) -> bool {
        self.0.contains(key)
    }
}

/// Membership with an owned fallback key. A computed query binding may be
/// borrowed for the native probe even though retained fallback keys must own it.
pub trait ContainsOwned<Q> {
    fn contains_owned(self, key: &Q) -> bool;
}
impl<H, Q: OwnKey<Owned = K>, K, T: ExactMembership<K>, F: FnOnce() -> T> ContainsOwned<Q>
    for &Host<H, K, T, F>
{
    fn contains_owned(self, key: &Q) -> bool {
        self.1.get().contains(&key.own_key())
    }
}
impl<H: ExactMembership<K>, Q: OwnKey<Owned = K>, K, T, F> ContainsOwned<Q> for &&Host<H, K, T, F> {
    fn contains_owned(self, key: &Q) -> bool {
        self.0.contains(&key.own_key())
    }
}
impl<H: ExactMembership<Q>, Q: OwnKey<Owned = K>, K, T, F> ContainsOwned<Q>
    for &&&Host<H, K, T, F>
{
    fn contains_owned(self, key: &Q) -> bool {
        self.0.contains(key)
    }
}

/// A constant window never builds a fallback index. Bounds contain borrowed
/// columns and can outlive the query block without copying captured values.
pub struct RangeHost<H, Q, F>(H, F, std::marker::PhantomData<fn() -> Q>);
impl<H, Q, F> RangeHost<H, Q, F> {
    pub fn new(host: H, fallback: F) -> Self {
        Self(host, fallback, std::marker::PhantomData)
    }
}

pub trait RangeRead<Q> {
    type Item;
    type Cursor: Iterator<Item = Self::Item>;
    fn range(self, low: Bound<Q>, high: Bound<Q>) -> Self::Cursor;
}
impl<H, Q, F: Fn() -> I, I: Iterator> RangeRead<Q> for &RangeHost<H, Q, F> {
    type Item = I::Item;
    type Cursor = I;
    fn range(self, _: Bound<Q>, _: Bound<Q>) -> I {
        (self.1)()
    }
}

/// A seek whose bound can be copied without invoking user Clone code.
pub trait SeekRangeLookup<Q>: OrderedRead + Copy {
    type Row: Copy;
    type RangeCursor: Iterator<Item = Self::Row>;
    fn seek_range_lookup(self, low: Bound<Q>, high: Bound<Q>) -> Self::RangeCursor;
}
impl<'a, H: OrderedRead + 'a, Q: Ord + Copy, F> RangeRead<Q> for &&'a RangeHost<H, Q, F>
where
    H::Cursor<'a>: OrderedCursor,
    <H::Cursor<'a> as OrderedCursor>::Key: Columns<Row = Q>,
    <H::Cursor<'a> as OrderedCursor>::Value: Columns,
    Q: Concat<<<H::Cursor<'a> as OrderedCursor>::Value as Columns>::Row>,
{
    type Item = <Q as Concat<<<H::Cursor<'a> as OrderedCursor>::Value as Columns>::Row>>::Row;
    type Cursor = RelationalWindow<H::Cursor<'a>, Q>;
    fn range(self, low: Bound<Q>, high: Bound<Q>) -> Self::Cursor {
        RelationalWindow {
            cursor: self.0.ordered(),
            low,
            high,
            done: false,
        }
    }
}
impl<H: SeekRangeLookup<Q>, Q, F> RangeRead<Q> for &&&RangeHost<H, Q, F> {
    type Item = H::Row;
    type Cursor = H::RangeCursor;
    fn range(self, low: Bound<Q>, high: Bound<Q>) -> Self::Cursor {
        self.0.seek_range_lookup(low, high)
    }
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

pub trait CandidateRead<Q> {
    type Item;
    type Cursor: Iterator<Item = Self::Item>;
    fn count(self, key: &Q) -> usize;
    fn keys(self, key: &Q) -> Self::Cursor;
}
impl<H, Q, K, T, F: FnOnce() -> T> CandidateRead<Q> for &Host<H, K, T, F>
where
    T: Candidates<Key = Q>,
{
    type Item = T::Item;
    type Cursor = T::Iter;
    fn count(self, key: &Q) -> usize {
        Candidates::count(self.1.get(), key)
    }
    fn keys(self, key: &Q) -> Self::Cursor {
        Candidates::keys(self.1.get(), key)
    }
}
impl<H, Q: OwnKey<Owned = K>, K, T, F> CandidateRead<Q> for &&Host<H, K, T, F>
where
    H: Candidates<Key = K>,
    H::Item: Columns,
{
    type Item = <H::Item as Columns>::Row;
    type Cursor = std::iter::Map<H::Iter, fn(H::Item) -> Self::Item>;
    fn count(self, key: &Q) -> usize {
        self.0.count(&key.own_key())
    }
    fn keys(self, key: &Q) -> Self::Cursor {
        self.0.keys(&key.own_key()).map(Columns::columns)
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
