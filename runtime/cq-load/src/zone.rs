//! Typed summaries over immutable decoded tuples. No Parquet statistics enter
//! this path: block boundaries are independent of source batches/row groups.
use crate::{Date, Decimal};
use cq_storage::rows::{Columns, Rows};
use cq_storage::{BlockId, BlockRead, Column, PositionalRows, RowId};
use std::marker::PhantomData;
use std::num::NonZeroUsize;

/// Owned immutable rows and one typed metadata tuple per positional block.
/// Construction examines each row once. Reads skip in-memory rows, not decoding
/// or file I/O. The last block may be shorter than `block_rows`.
pub struct ZoneRows<T, M> {
    rows: Vec<T>,
    metadata: Vec<M>,
    block_rows: NonZeroUsize,
}

/// Metadata allocation accounting, excluding the inline value itself.
/// Report owned heap capacities, excluding allocator overhead and borrowed data.
pub trait ZoneMetadata {
    fn heap_bytes(&self) -> usize;
}

/// Construct one enclosing summary per column of a decoded block.
///
/// Implementations must enclose every key, using the column's exact relational
/// equality and order, or return unavailable bounds. Tuple schemas of `MinMax`
/// and `NoBounds` fields are supplied through arity 12 (the storage tuple limit).
/// They visit the rows once, updating all column summaries together.
pub trait ZoneSchema<T> {
    type Metadata: ZoneMetadata;
    fn summarize(rows: &[T]) -> Self::Metadata;
}

/// A column summary and its matching borrowed or copied relational key.
/// `bounds` must enclose every value passed to `observe`; unavailable bounds
/// must return `None`. Neither key conversion nor summarization may substitute
/// a different equality/order (for example float total ordering or collation).
pub trait ZoneField<T>: Default + ZoneMetadata {
    type Key<'a>
    where
        T: 'a,
        Self: 'a;
    fn observe(&mut self, value: &T);
    fn key(value: &T) -> Self::Key<'_>;
    fn bounds(&self) -> Option<(Self::Key<'_>, Self::Key<'_>)>;
}

/// Optional typed extrema. Supports integers, bool, Date, Decimal, and text.
/// Text extrema are owned Strings and keys borrow `str` with Rust's exact order.
/// For unsupported/null/unordered fields choose `NoBounds<T>` explicitly.
pub struct MinMax<T> {
    bounds: Option<(T, T)>,
}
impl<T> Default for MinMax<T> {
    fn default() -> Self {
        Self { bounds: None }
    }
}
impl<T: Ord + Clone> MinMax<T> {
    fn observe_value(&mut self, value: &T) {
        match &mut self.bounds {
            None => self.bounds = Some((value.clone(), value.clone())),
            Some((lo, hi)) => {
                if value < lo {
                    lo.clone_from(value);
                }
                if value > hi {
                    hi.clone_from(value);
                }
            }
        }
    }
}
macro_rules! scalar {
    ($([$($generic:tt)*] $ty:ty),* $(,)?) => {$ (
        impl<$($generic)*> ZoneMetadata for MinMax<$ty> {
            fn heap_bytes(&self) -> usize { 0 }
        }
        impl<$($generic)*> ZoneField<$ty> for MinMax<$ty> {
            type Key<'a> = $ty;
            fn observe(&mut self, value: &$ty) { self.observe_value(value); }
            fn key(value: &$ty) -> $ty { *value }
            fn bounds(&self) -> Option<($ty, $ty)> { self.bounds }
        }
    )* };
}
scalar!([] i8, [] i16, [] i32, [] i64, [] i128, [] isize,
        [] u8, [] u16, [] u32, [] u64, [] u128, [] usize,
        [] bool, [] Date, [const SCALE: u32] Decimal<SCALE>);

impl ZoneMetadata for MinMax<String> {
    fn heap_bytes(&self) -> usize {
        self.bounds
            .as_ref()
            .map_or(0, |(lo, hi)| lo.capacity() + hi.capacity())
    }
}
impl MinMax<String> {
    fn observe_text(&mut self, value: &str) {
        match &mut self.bounds {
            None => self.bounds = Some((value.to_owned(), value.to_owned())),
            Some((lo, hi)) => {
                if value < lo.as_str() {
                    lo.clear();
                    lo.push_str(value);
                }
                if value > hi.as_str() {
                    hi.clear();
                    hi.push_str(value);
                }
            }
        }
    }
    fn text_bounds(&self) -> Option<(&str, &str)> {
        self.bounds
            .as_ref()
            .map(|(lo, hi)| (lo.as_str(), hi.as_str()))
    }
}
impl ZoneField<String> for MinMax<String> {
    type Key<'a> = &'a str;
    fn observe(&mut self, value: &String) {
        self.observe_text(value);
    }
    fn key(value: &String) -> &str {
        value
    }
    fn bounds(&self) -> Option<(&str, &str)> {
        self.text_bounds()
    }
}
impl<'s> ZoneField<&'s str> for MinMax<String> {
    type Key<'a>
        = &'a str
    where
        's: 'a;
    fn observe(&mut self, value: &&'s str) {
        self.observe_text(value);
    }
    fn key<'a>(value: &'a &'s str) -> &'a str {
        value
    }
    fn bounds(&self) -> Option<(&str, &str)> {
        self.text_bounds()
    }
}

/// Unavailable statistics for any field type; owns no data and never prunes.
/// Ordered keys are borrowed and tested per row. Unordered fields (e.g. f64)
/// remain available through `Rows`, but cannot implement storage's `Ord` column
/// contract. This also supports opaque user types and optional/null fields.
pub struct NoBounds<T>(PhantomData<fn() -> T>);
impl<T> Default for NoBounds<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<T> ZoneMetadata for NoBounds<T> {
    fn heap_bytes(&self) -> usize {
        0
    }
}
impl<T> ZoneField<T> for NoBounds<T> {
    type Key<'a>
        = &'a T
    where
        T: 'a;
    fn observe(&mut self, _: &T) {}
    fn key(value: &T) -> &T {
        value
    }
    fn bounds(&self) -> Option<(&T, &T)> {
        None
    }
}

impl<T, M: ZoneMetadata> ZoneRows<T, M> {
    /// Move an existing vector into the host; summarize each row exactly once.
    pub fn from_rows<S: ZoneSchema<T, Metadata = M>>(
        rows: Vec<T>,
        block_rows: NonZeroUsize,
    ) -> Self {
        let metadata = rows.chunks(block_rows.get()).map(S::summarize).collect();
        Self {
            rows,
            metadata,
            block_rows,
        }
    }

    /// Decode/consume batches once, preserving order and duplicates. A partial
    /// block continues into the next batch; empty batches do not split blocks.
    /// On an error, release partial rows/metadata and stop consuming the source.
    pub fn from_batches<S, E, I>(batches: I, block_rows: NonZeroUsize) -> Result<Self, E>
    where
        S: ZoneSchema<T, Metadata = M>,
        I: IntoIterator<Item = Result<Vec<T>, E>>,
    {
        let mut host = Self {
            rows: Vec::new(),
            metadata: Vec::new(),
            block_rows,
        };
        let mut summarized = 0;
        for batch in batches {
            host.rows.extend(batch?);
            while host.rows.len() - summarized >= block_rows.get() {
                let end = summarized + block_rows.get();
                host.metadata
                    .push(S::summarize(&host.rows[summarized..end]));
                summarized = end;
            }
        }
        if summarized < host.rows.len() {
            host.metadata.push(S::summarize(&host.rows[summarized..]));
        }
        Ok(host)
    }

    pub fn block_rows(&self) -> NonZeroUsize {
        self.block_rows
    }

    /// Retained metadata capacity in bytes, including owned variable-width
    /// extrema. Excludes base rows, inline host, allocator overhead and source
    /// decoder buffers; this is not a process peak-allocation measurement.
    pub fn metadata_bytes(&self) -> usize {
        self.metadata.capacity() * std::mem::size_of::<M>()
            + self
                .metadata
                .iter()
                .map(ZoneMetadata::heap_bytes)
                .sum::<usize>()
    }
}

impl<'a, T, M> Rows for &'a ZoneRows<T, M>
where
    &'a T: Columns,
{
    type Row = <&'a T as Columns>::Row;
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.rows.iter().map(Columns::columns)
    }
}
impl<'a, T, M> PositionalRows for &'a ZoneRows<T, M>
where
    &'a T: Columns,
{
    fn len(self) -> usize {
        self.rows.len()
    }
    fn row_at(self, id: RowId) -> Option<Self::Row> {
        self.rows.get(id.offset()).map(Columns::columns)
    }
    fn block_size(self) -> NonZeroUsize {
        self.block_rows
    }
}

/// Zero-based declaration-order column. Invalid indices or unordered key types
/// do not implement `cq_storage::Column`. Slice, Vec and array hosts use the
/// same MinMax key conversion with scalar BlockRead; they acquire no metadata.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColumnAt<const N: usize>;
pub fn column<const N: usize>() -> ColumnAt<N> {
    ColumnAt
}

impl ZoneMetadata for () {
    fn heap_bytes(&self) -> usize {
        0
    }
}
impl ZoneSchema<()> for () {
    type Metadata = ();
    fn summarize(_: &[()]) {}
}
macro_rules! columns {
    ([$($t:ident:$m:ident:$i:tt),+];) => {};
    ([$($t:ident:$m:ident:$i:tt),+]; $selected:ident:$summary:ident:$n:tt $(,$rest_t:ident:$rest_m:ident:$rest_i:tt)*) => {
        impl<'a, $($t: 'a, $m: 'a),+> Column<&'a ZoneRows<($($t,)+), ($($m,)+)>> for ColumnAt<$n>
        where $summary: ZoneField<$selected>, <$summary as ZoneField<$selected>>::Key<'a>: Ord {
            type Key = <$summary as ZoneField<$selected>>::Key<'a>;
            fn key(self, host: &'a ZoneRows<($($t,)+), ($($m,)+)>, id: RowId) -> Self::Key {
                $summary::key(&host.rows[id.offset()].$n)
            }
        }
        impl<'a, $($t: 'a, $m: 'a),+> BlockRead<ColumnAt<$n>> for &'a ZoneRows<($($t,)+), ($($m,)+)>
        where $summary: ZoneField<$selected>, <$summary as ZoneField<$selected>>::Key<'a>: Ord {
            // Reuse Phase 1's read loop: it counts considered/visited/skipped and
            // tests only surviving rows. Selection::narrow uses this hook too.
            fn bounds(self, _: ColumnAt<$n>, block: BlockId) -> Option<(
                <$summary as ZoneField<$selected>>::Key<'a>, <$summary as ZoneField<$selected>>::Key<'a>
            )> {
                self.metadata.get(block.offset())?.$n.bounds()
            }
        }
        impl<'a, $($t: 'a),+> Column<&'a [($($t,)+)]> for ColumnAt<$n>
        where MinMax<$selected>: ZoneField<$selected>, <MinMax<$selected> as ZoneField<$selected>>::Key<'a>: Ord {
            type Key = <MinMax<$selected> as ZoneField<$selected>>::Key<'a>;
            fn key(self, host: &'a [($($t,)+)], id: RowId) -> Self::Key {
                <MinMax<$selected> as ZoneField<$selected>>::key(&host[id.offset()].$n)
            }
        }
        impl<'a, $($t: 'a),+> Column<&'a Vec<($($t,)+)>> for ColumnAt<$n>
        where Self: Column<&'a [($($t,)+)]> {
            type Key = <Self as Column<&'a [($($t,)+)]>>::Key;
            fn key(self, host: &'a Vec<($($t,)+)>, id: RowId) -> Self::Key { self.key(host.as_slice(), id) }
        }
        impl<'a, $($t: 'a),+, const SIZE: usize> Column<&'a [($($t,)+); SIZE]> for ColumnAt<$n>
        where Self: Column<&'a [($($t,)+)]> {
            type Key = <Self as Column<&'a [($($t,)+)]>>::Key;
            fn key(self, host: &'a [($($t,)+); SIZE], id: RowId) -> Self::Key { self.key(host.as_slice(), id) }
        }
        columns!([$($t:$m:$i),+]; $($rest_t:$rest_m:$rest_i),*);
    };
}
macro_rules! tuple {
    ($($t:ident:$m:ident:$i:tt),+) => {
        impl<$($m: ZoneMetadata),+> ZoneMetadata for ($($m,)+) {
            fn heap_bytes(&self) -> usize { 0 $(+ self.$i.heap_bytes())+ }
        }
        impl<$($t, $m: ZoneField<$t>),+> ZoneSchema<($($t,)+)> for ($($m,)+) {
            type Metadata = Self;
            fn summarize(rows: &[($($t,)+)]) -> Self {
                let mut result = Self::default();
                for row in rows { $(result.$i.observe(&row.$i);)+ }
                result
            }
        }
        columns!([$($t:$m:$i),+]; $($t:$m:$i),+);
    };
}
tuple!(T0:M0:0);
tuple!(T0:M0:0, T1:M1:1);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2, T3:M3:3);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2, T3:M3:3, T4:M4:4);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2, T3:M3:3, T4:M4:4, T5:M5:5);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2, T3:M3:3, T4:M4:4, T5:M5:5, T6:M6:6);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2, T3:M3:3, T4:M4:4, T5:M5:5, T6:M6:6, T7:M7:7);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2, T3:M3:3, T4:M4:4, T5:M5:5, T6:M6:6, T7:M7:7, T8:M8:8);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2, T3:M3:3, T4:M4:4, T5:M5:5, T6:M6:6, T7:M7:7, T8:M8:8, T9:M9:9);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2, T3:M3:3, T4:M4:4, T5:M5:5, T6:M6:6, T7:M7:7, T8:M8:8, T9:M9:9, T10:M10:10);
tuple!(T0:M0:0, T1:M1:1, T2:M2:2, T3:M3:3, T4:M4:4, T5:M5:5, T6:M6:6, T7:M7:7, T8:M8:8, T9:M9:9, T10:M10:10, T11:M11:11);
