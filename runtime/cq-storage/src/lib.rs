//! Physical access traits and concrete data structures for generated programs.
//!
//! Queries denote `bottom + Set`: one panic aborts evaluation. Opaque Rust
//! binds and guards keep their written chain; relational steps may move only
//! earlier across them. Their free variables determine their partial-function
//! meaning; I/O, mutation and evaluation counts are outside that meaning.
//! Relational `==` decides Leibniz identity. `Eq` and the selected storage's
//! `Hash` or `Ord` bounds are necessary checks, not a proof of admissibility.
//!
//! These structures do not implement relational operators, choose access
//! paths, or couple a collection to a query-wide representation. Query access
//! is available only through the operation traits; a user type implementing
//! [`Build`] plus the access traits a plan uses is a first-class structure,
//! and the compiler names it without inspecting it.
//!
//! [`Answers`], [`GroupedAnswers`] and allocation-free [`SingleGroup`] compose
//! query outputs as inputs. Grouped keys occupy the leading declared columns.
//! [`dispatch`] selects direct reads
//! by type, with one lazy fallback per query read. Host uniqueness is never a
//! language premise; caller-selected distinct state establishes canonical plans.
//!
//! # Abstraction functions
//!
//! Every type here is a representation of a relation and is admissible only
//! because an abstraction function `α` relates it to that relation, and each
//! operation commutes with `α`. Storage never adds meaning: `α` of a built
//! value is exactly the supplied rows. The laws, checked in
//! `tests/representation_laws.rs` and `tests/access_laws.rs`:
//!
//! - `α(index) = bag of (key, item)`; `build(I)` has `α = bag(I)` and
//!   `lookup(k)` enumerates exactly the items paired with `k`, with
//!   multiplicity and in source order.
//!   [`SortedRun`] clusters that same bag by key using a stable sort; its
//!   binary-search or presence-and-rank directory selects identical ranges.
//!   Borrowed [`RunCursor`] snapshots are Copy and retain the run's lifetime.
//! - `α(membership) = key set`; `contains(k) ⇔ k ∈ α`. Exact interval
//!   membership requires an actual represented key in the closed interval.
//! - `α(positional host)` is its ordered occurrence bag; stable base offsets
//!   distinguish duplicates. Block bounds enclose all keys, and absent bounds
//!   retain the block. Reads preserve base order and exact predicate results.
//!   [`Fetch`] reconstructs a named column from the same live occurrence.
//!   Chunked reads AND exact predicate masks and preserve scalar-read order;
//!   [`stage`] selections expose survivors before a partial operation executes.
//! - `α(selection)` is the selected occurrence bag. Narrowing is intersection;
//!   immutable snapshots and their projected builds preserve source order and
//!   multiplicity. Selection does not confer OrderedRead.
//! - `α(s.filter(c, p)) = { o ∈ α(s) | p(c(o)) }` ([`Filter`]): every read of
//!   the result agrees with it, and filtering twice is filtering by the
//!   conjunction. A semijoin is a filter by [`Membership`]. Filtered tuples
//!   are unreadable by construction (a selection's bitmap or a compact build).
//! - `α(ordered) = bag of (key, item)`, enumerated in key order from any
//!   bound; [`merge`] is the one co-iteration primitive; [`sorted_by`]
//!   presents an already key-ordered stream as such a cursor.
//! - `α(trie) = { (k, r) | r ∈ α(inner[k]) }`: a [`Trie`] is a curried
//!   index; each level is applied in turn and inner levels build lazily.
//!
//! [`require`] holds one-line guards the compiler emits beside each
//! structure so a missing trait fails there, naming the structure.
//!
//! 1. Leibniz admissibility of a column type: a type may be compared, joined,
//!    grouped, keyed or deduplicated by the relational centre only if its `==`
//!    decides Leibniz equality (Rust code cannot distinguish `==`-equal values).
//!    Holds for integers, bool, char, String/&str, the cq-load Date/Decimal,
//!    tuples and derived-Eq structs of admissible types; fails for f64/f32
//!    (0.0 == -0.0) and hand-written lenient Eq.
//! 2. Functional extensionality of embedded Rust: an embedded expression receives
//!    query variables as shared references (`&T`, never `&mut`), so its value is
//!    a function of the values of its free variables; equal (Leibniz) inputs give
//!    equal results. Interior mutability, globals and I/O are outside the meaning,
//!    as already documented (evaluation count is not preserved).
//!
//! Any surviving representative of `==`-equal values IS the value. Dedup,
//! canonical projection, and projecting away variables before an opaque Rust step
//! are sound whenever the Rust step's free variables are retained. `Eq` and the
//! selected representation's `Hash`/`Ord` bounds are necessary mechanical checks;
//! rustc cannot prove Leibniz admissibility of a hand-written implementation.

mod answers;
mod candidates;
macro_rules! count {
    ($field:ident) => {
        #[cfg(feature = "counters")]
        crate::counters::record(|counts| counts.$field += 1);
    };
}
pub(crate) use count;

#[cfg(any(feature = "counters", feature = "operator-timing"))]
pub mod counters;
pub mod dispatch;
mod hash_index;
mod hash_state;
mod join_filter;
pub use join_filter::{JoinFilterPredicate, selective_build};
mod host;
mod instrument;
mod membership;
mod ordered;
pub mod parallel;
pub mod rows;
mod radix_run;
mod radix_spline;
mod sorted_run;
pub mod sequence;
pub mod provider_contract;
pub mod sharded;
pub use sharded::Sharded;
mod trie;

pub use answers::{Answers, GroupedAnswers, SingleGroup};
pub use candidates::{Candidates, Keys};
pub use hash_index::{BucketCursor, Build, HashIndex, IndexRead};
pub use hash_state::{HashSet, HashSetConstructor, HashState};
pub use membership::{ConservativeMembership, ExactMembership};
pub use ordered::{
    GroupedCursor, KeyCursor, Merge, OrderedCursor, OrderedRead, Sorted, merge, sorted_by,
};
pub use sorted_run::{RunCursor, SortedRun};
pub use radix_run::{RadixKey, RadixRun, RadixRunReadState, RadixRunView};
pub use radix_spline::{RadixSplineMetadata, RadixSplinePoint};
pub use trie::Trie;

mod point;
pub use point::{PointLookup, PointRows};

pub mod require {
    /// Check the necessary `Eq` bound even when a provider adds no bound.
    /// Leibniz admissibility remains a caller premise; `Eq` cannot prove it.
    pub fn leibniz<T: Eq + ?Sized>(_: &T) {}
    /// Preserve a build pair while enforcing Eq on its key.
    pub fn key<K: Eq, V>(pair: (K, V)) -> (K, V) {
        pair
    }
    pub fn seek<T: super::IndexRead>(_: T) {}
    pub fn candidates<T: super::Candidates>(_: T) {}
    pub fn contains<K: ?Sized, T: super::ExactMembership<K>>(_: T) {}
    pub fn conservative<K: ?Sized, T: super::ConservativeMembership<K>>(_: &T) {}
    pub fn ordered<T: super::OrderedRead>(_: T) {}
    /// A direct ordered read must use the leading declared key type.
    pub fn ordered_key<'a, K, T: super::OrderedRead + 'a>(_: &'a T)
    where
        <T::Cursor<'a> as super::OrderedCursor>::Key: OrderedKey<K>,
    {
    }
    #[diagnostic::on_unimplemented(
        message = "ordered input key must match the leading declaration prefix"
    )]
    pub trait OrderedKey<K> {}
    impl<K> OrderedKey<K> for &K {}
    pub fn positional<T: super::PositionalRows>(_: T) {}
    pub fn block_read<H: super::BlockRead<C>, C: super::Column<H>>(_: H, _: C) {}
    /// Check a marker's schema binding without constructing or evaluating it.
    pub fn column_slot<const N: usize, H: super::PositionalRows, C: super::ColumnSlot<H, N>>(_: H) {}
    pub fn interval<K: Ord, T: super::ExactIntervalMembership<K>>(_: &T) {}
}
/// Decide Leibniz identity using the admissible column type's `==`.
/// The `Eq` bound rejects floats and PartialEq-only types; it cannot establish
/// Leibniz admissibility of a hand-written implementation.
pub fn relational_eq<T: Eq + ?Sized>(left: &T, right: &T) -> bool {
    left == right
}

/// Ord must be a total order consistent with Leibniz equality. The bound is a
/// necessary mechanical check; semantic admissibility is a caller premise.
pub fn relational_cmp<T: Ord + ?Sized>(left: &T, right: &T) -> std::cmp::Ordering {
    left.cmp(right)
}

mod string_literal {
    pub trait Sealed {}
    impl Sealed for str {}
    impl Sealed for String {}
    impl<T: super::StringValue + ?Sized> Sealed for &T {}
}

/// The standard string representations accepted beside a string literal.
/// Sealing excludes arbitrary user conversions or cross-type PartialEq code.
pub trait StringValue: string_literal::Sealed {
    fn string_value(&self) -> &str;
}
impl StringValue for str {
    fn string_value(&self) -> &str {
        self
    }
}
impl StringValue for String {
    fn string_value(&self) -> &str {
        self.as_str()
    }
}
impl<T: StringValue + ?Sized> StringValue for &T {
    fn string_value(&self) -> &str {
        T::string_value(*self)
    }
}

pub fn relational_string_eq<L: Eq + StringValue + ?Sized, R: Eq + StringValue + ?Sized>(
    left: &L,
    right: &R,
) -> bool {
    left.string_value() == right.string_value()
}
/// String literals compare the contents of standard String/str values.
pub fn relational_str_cmp<L: Ord + StringValue + ?Sized, R: Ord + StringValue + ?Sized>(
    left: &L,
    right: &R,
) -> std::cmp::Ordering {
    left.string_value().cmp(right.string_value())
}

pub fn intersect_lower<Q: Ord>(a: std::ops::Bound<Q>, b: std::ops::Bound<Q>) -> std::ops::Bound<Q> {
    use std::ops::Bound::*;
    match (&a, &b) {
        (Unbounded, _) => b,
        (_, Unbounded) => a,
        (Included(x) | Excluded(x), Included(y) | Excluded(y)) => {
            if x > y || (x == y && matches!(a, Excluded(_))) {
                a
            } else {
                b
            }
        }
    }
}
pub fn intersect_upper<Q: Ord>(a: std::ops::Bound<Q>, b: std::ops::Bound<Q>) -> std::ops::Bound<Q> {
    use std::ops::Bound::*;
    match (&a, &b) {
        (Unbounded, _) => b,
        (_, Unbounded) => a,
        (Included(x) | Excluded(x), Included(y) | Excluded(y)) => {
            if x < y || (x == y && matches!(a, Excluded(_))) {
                a
            } else {
                b
            }
        }
    }
}

mod exists;
pub use exists::Exists;

pub mod adaptive;
mod counters_memory;
pub mod fetch;
pub mod positional;
pub mod selection;
pub mod stage;
pub use adaptive::{AdaptiveKeySet, BorrowedI64TupleCodec, I64Codec, KeyCodec, NoDense};
pub use fetch::Fetch;
pub use positional::{
    BlockId, BlockRead, Column, ColumnSlot, ExactIntervalMembership, Membership, Point, PositionalRows,
    ReadPredicate, RowId, RowPredicate, Window, read_with_filter,
};
pub use selection::{Selection, SelectionRows, SelectionSummary};
pub use stage::{Chunk, for_chunks};
pub mod filter;
pub use filter::Filter;

#[cfg(doctest)]
mod misuse;

pub mod retention;
pub use retention::{Retention, residual, retain_keys, retain_rows};

/// Select printer output using the runtime's counters feature. The unselected
/// tokens are discarded before type checking; the plain branch expands verbatim.
#[cfg(not(any(feature = "counters", feature = "operator-timing")))]
#[macro_export]
macro_rules! profile_select {
    (plain { $($plain:tt)* } counters { $($profile:tt)* }) => { $($plain)* };
}
/// Select printer output using the runtime's counters feature.
#[cfg(any(feature = "counters", feature = "operator-timing"))]
#[macro_export]
macro_rules! profile_select {
    (plain { $($plain:tt)* } counters { $($profile:tt)* }) => { $($profile)* };
}
