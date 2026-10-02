//! Stable autoref dispatch. The double-reference receiver tries a native read
//! at the declared leading key type. The single-reference receiver builds once.
use crate::rows::{Columns, Concat, OwnKey};
use crate::{Candidates, ExactMembership, IndexRead, OrderedCursor, OrderedRead};
use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::ops::Bound;
use std::rc::Rc;

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

/// Bounded windows retain native lower-bound seeking. Fallback payloads retain
/// the full borrowed row, so output references never point into copied keys.
pub trait Range {
    type Key: 'static;
    type Row: Copy;
    type Cursor: Iterator<Item = Self::Row>;
    fn range(
        self,
        low: Option<&'static Self::Key>,
        high: Option<&'static Self::Key>,
    ) -> Self::Cursor;
}
/// An ordered fallback whose cursors retain shared buckets, just like HashIndex.
/// Keys and borrowed rows have exactly the same layout as a built BTreeMap.
pub struct RangeIndex<K, R> {
    buckets: Rc<BTreeMap<K, Rc<Vec<R>>>>,
}
impl<K: Ord, R, I: Iterator<Item = (K, R)>> crate::Build<I> for RangeIndex<K, R> {
    fn build(source: I) -> Self {
        let map = <BTreeMap<K, Vec<R>> as crate::Build<I>>::build(source);
        Self {
            buckets: Rc::new(map.into_iter().map(|(k, v)| (k, Rc::new(v))).collect()),
        }
    }
}
pub struct RangeCursor<K: 'static, R> {
    buckets: Rc<BTreeMap<K, Rc<Vec<R>>>>,
    low: Option<&'static K>,
    high: Option<&'static K>,
    previous: Option<K>,
    current: Option<Rc<Vec<R>>>,
    position: usize,
    done: bool,
}
impl<K: Ord + Clone + 'static, R: Copy> Iterator for RangeCursor<K, R> {
    type Item = R;
    fn next(&mut self) -> Option<R> {
        crate::count!(cursor_steps);
        if self.done {
            return None;
        }
        loop {
            if let Some(row) = self
                .current
                .as_ref()
                .and_then(|v| v.get(self.position))
                .copied()
            {
                self.position += 1;
                crate::count!(rows_enumerated);
                return Some(row);
            }
            let low = self.previous.as_ref().map_or_else(
                || self.low.map_or(Bound::Unbounded, Bound::Included),
                Bound::Excluded,
            );
            match self.buckets.range((low, Bound::Unbounded)).next() {
                Some((key, bucket)) if self.high.is_none_or(|h| key <= h) => {
                    self.previous = Some(key.clone());
                    self.current = Some(Rc::clone(bucket));
                    self.position = 0;
                }
                _ => {
                    self.done = true;
                    return None;
                }
            }
        }
    }
}
impl<K: Ord + Clone + 'static, R: Copy> std::iter::FusedIterator for RangeCursor<K, R> {}
impl<H, K: Ord + Clone + 'static, R: Copy, F: FnOnce() -> RangeIndex<K, R>> Range
    for &Host<H, K, RangeIndex<K, R>, F>
{
    type Key = K;
    type Row = R;
    type Cursor = RangeCursor<K, R>;
    fn range(self, low: Option<&'static K>, high: Option<&'static K>) -> Self::Cursor {
        crate::count!(lookups);
        RangeCursor {
            buckets: Rc::clone(&self.1.get().buckets),
            low,
            high,
            previous: None,
            current: None,
            position: 0,
            done: false,
        }
    }
}
impl<'a, 'h, H: OrderedRead + 'a, K: Ord + 'static, T, F> Range for &&'a Host<H, K, T, F>
where
    H::Cursor<'a>: OrderedCursor<Key = &'h K>,
    &'h K: Columns,
    <H::Cursor<'a> as OrderedCursor>::Value: Columns,
    <&'h K as Columns>::Row: Concat<<<H::Cursor<'a> as OrderedCursor>::Value as Columns>::Row>,
{
    type Key = K;
    type Row = <<&'h K as Columns>::Row as Concat<
        <<H::Cursor<'a> as OrderedCursor>::Value as Columns>::Row,
    >>::Row;
    type Cursor = std::iter::Map<
        Window<H::Cursor<'a>, K>,
        fn(
            (
                <H::Cursor<'a> as OrderedCursor>::Key,
                <H::Cursor<'a> as OrderedCursor>::Value,
            ),
        ) -> Self::Row,
    >;
    fn range(self, low: Option<&'static K>, high: Option<&'static K>) -> Self::Cursor {
        let mut cursor = self.0.ordered();
        if let Some(low) = low {
            cursor.seek(&low);
        }
        Window {
            cursor,
            high,
            done: false,
        }
        .map(|(k, v)| k.columns().concat(v.columns()))
    }
}
/// A fused upper-bounded view of an ordered cursor.
pub struct Window<C, K: 'static> {
    cursor: C,
    high: Option<&'static K>,
    done: bool,
}
impl<'a, K: Ord + 'static, C: OrderedCursor<Key = &'a K>> Iterator for Window<C, K> {
    type Item = C::Item;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.cursor.next() {
            Some((k, v)) if self.high.is_none_or(|h| k <= h) => Some((k, v)),
            _ => {
                self.done = true;
                None
            }
        }
    }
}
impl<'a, K: Ord + 'static, C: OrderedCursor<Key = &'a K>> std::iter::FusedIterator
    for Window<C, K>
{
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

/// Borrowed ordered handles expose a range cursor independent of the handle's
/// own lifetime, just as PointLookup does. The cursor borrows the host rows.
pub trait RangeLookup<Q>: OrderedRead + Copy {
    type Row: Copy;
    type RangeCursor: Iterator<Item = Self::Row>;
    fn range_lookup(self, low: Bound<Q>, high: Bound<Q>) -> Self::RangeCursor;
}
/// A seek whose bound can be copied without invoking user Clone code.
pub trait SeekRangeLookup<Q>: RangeLookup<Q> {
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

impl<'h, T: Ord, V> RangeLookup<(&'h T,)> for &'h BTreeMap<(T,), Vec<V>>
where
    &'h V: Columns,
    (&'h T,): Concat<<&'h V as Columns>::Row>,
{
    type Row = <(&'h T,) as Concat<<&'h V as Columns>::Row>>::Row;
    type RangeCursor = RelationalWindow<crate::ordered::GroupedCursor<'h, (T,), V>, (&'h T,)>;
    fn range_lookup(self, low: Bound<(&'h T,)>, high: Bound<(&'h T,)>) -> Self::RangeCursor {
        RelationalWindow {
            cursor: self.ordered(),
            low,
            high,
            done: false,
        }
    }
}
impl<'h, T: Ord + Copy, V> SeekRangeLookup<(&'h T,)> for &'h BTreeMap<(T,), Vec<V>>
where
    &'h V: Columns,
    (&'h T,): Concat<<&'h V as Columns>::Row>,
{
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
    source: RefCell<Option<I>>,
    keys: OnceCell<RefCell<crate::instrument::HashSet<I::Item>>>,
}
impl<K, V, I: Iterator<Item = (K, V)>, const EAGER: bool> crate::Build<I>
    for KeySet<std::iter::Map<I, fn((K, V)) -> K>, EAGER>
{
    fn build(source: I) -> Self {
        Self {
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
                let keys = if EAGER {
                    let source = self.source.borrow_mut().take().expect("one initialization");
                    #[cfg(feature = "counters")]
                    let source = source.inspect(|_| {
                        crate::count!(distinct_insertions);
                    });
                    source.collect()
                } else {
                    crate::instrument::HashSet::default()
                };
                RefCell::new(keys)
            })
            .borrow_mut();
        if keys.contains(key) {
            return true;
        }
        if EAGER {
            return false;
        }
        let mut source = self.source.borrow_mut();
        if let Some(rows) = source.as_mut() {
            for candidate in rows.by_ref() {
                let found = &candidate == key;
                crate::count!(distinct_insertions);
                keys.insert(candidate);
                if found {
                    return true;
                }
            }
        }
        *source = None;
        false
    }
}
