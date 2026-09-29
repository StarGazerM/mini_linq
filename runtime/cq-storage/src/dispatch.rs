//! Stable autoref dispatch. The double-reference receiver tries a native read
//! at the declared leading key type. The single-reference receiver builds once.
use crate::rows::{Columns, Concat, OwnKey};
use crate::{Candidates, ExactMembership, IndexRead, OrderedCursor, OrderedRead};
use std::cell::{OnceCell, RefCell};

/// The key and fallback types are fixed before method resolution, so an
/// unsupported key type falls back even when another keyed read is implemented.
pub struct Host<'a, H, K, T, F>(pub H, &'a Lazy<T, F>, std::marker::PhantomData<fn() -> K>);
impl<'a, H, K, T, F> Host<'a, H, K, T, F> {
    pub fn new(host: H, fallback: &'a Lazy<T, F>) -> Self {
        Self(host, fallback, std::marker::PhantomData)
    }
}

/// One fallback per query/read, initialized on first use and shared by probes.
pub struct Lazy<T, F> {
    value: OnceCell<T>,
    build: RefCell<Option<F>>,
}
impl<T, F: FnOnce() -> T> Lazy<T, F> {
    pub fn new(build: F) -> Self {
        Self {
            value: OnceCell::new(),
            build: RefCell::new(Some(build)),
        }
    }
    /// Formal rule IF-Fallback: Initialize shared fallback storage once, on first use.
    pub fn get(&self) -> &T {
        self.value
            .get_or_init(|| self.build.borrow_mut().take().expect("one initialization")())
    }
}

pub trait Seek<Q> {
    type Item: Copy;
    type Cursor: Iterator<Item = Self::Item>;
    fn seek(self, key: &Q) -> Self::Cursor;
}
impl<'a, H, Q, K, T, F: FnOnce() -> T> Seek<Q> for &Host<'a, H, K, T, F>
where
    &'a T: IndexRead<Key = Q>,
{
    type Item = <&'a T as IndexRead>::Item;
    type Cursor = <&'a T as IndexRead>::Cursor;
    fn seek(self, key: &Q) -> Self::Cursor {
        self.1.get().lookup(key)
    }
}
impl<H, Q: OwnKey<Owned = K>, K, T, F> Seek<Q> for &&Host<'_, H, K, T, F>
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
impl<H, Q, K, T: ExactMembership<Q>, F: FnOnce() -> T> Contains<Q> for &Host<'_, H, K, T, F> {
    fn contains(self, key: &Q) -> bool {
        self.1.get().contains(key)
    }
}
impl<H, Q: OwnKey<Owned = K>, K, T, F> Contains<Q> for &&Host<'_, H, K, T, F>
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
type Ordered<K, R> = std::collections::BTreeMap<K, Vec<R>>;
impl<'a, H, K: Ord + 'static, R: Copy + 'a, F: FnOnce() -> Ordered<K, R>> Range
    for &Host<'a, H, K, Ordered<K, R>, F>
{
    type Key = K;
    type Row = R;
    type Cursor =
        std::iter::Map<Window<crate::GroupedCursor<'a, K, R>, K>, fn((&'a K, &'a R)) -> R>;
    fn range(self, low: Option<&'static K>, high: Option<&'static K>) -> Self::Cursor {
        let mut cursor = self.1.get().ordered();
        if let Some(low) = low {
            cursor.seek(&low);
        }
        Window {
            cursor,
            high,
            done: false,
        }
        .map(|(_, v)| *v)
    }
}
impl<'a, 'h, H: OrderedRead + 'a, K: Ord + 'static, T, F> Range for &&'a Host<'_, H, K, T, F>
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

pub trait CandidateRead<Q> {
    type Item;
    type Cursor: Iterator<Item = Self::Item>;
    fn count(self, key: &Q) -> usize;
    fn keys(self, key: &Q) -> Self::Cursor;
}
impl<'a, H, Q, K, T, F: FnOnce() -> T> CandidateRead<Q> for &Host<'a, H, K, T, F>
where
    &'a T: Candidates<Key = Q>,
{
    type Item = <&'a T as Candidates>::Item;
    type Cursor = <&'a T as Candidates>::Iter;
    fn count(self, key: &Q) -> usize {
        Candidates::count(&self.1.get(), key)
    }
    fn keys(self, key: &Q) -> Self::Cursor {
        Candidates::keys(&self.1.get(), key)
    }
}
impl<H, Q: OwnKey<Owned = K>, K, T, F> CandidateRead<Q> for &&Host<'_, H, K, T, F>
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
pub struct KeySet<I: Iterator> {
    source: RefCell<Option<I>>,
    keys: OnceCell<std::collections::HashSet<I::Item>>,
}
impl<K, V, I: Iterator<Item = (K, V)>> crate::Build<I>
    for KeySet<std::iter::Map<I, fn((K, V)) -> K>>
{
    fn build(source: I) -> Self {
        Self {
            source: RefCell::new(Some(source.map(|(k, _)| k))),
            keys: OnceCell::new(),
        }
    }
}
impl<K: Eq + std::hash::Hash, I: Iterator<Item = K>> ExactMembership<K> for KeySet<I> {
    fn contains(&self, key: &K) -> bool {
        self.keys
            .get_or_init(|| {
                self.source
                    .borrow_mut()
                    .take()
                    .expect("one initialization")
                    .collect()
            })
            .contains(key)
    }
}
