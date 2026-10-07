//! Phase 1 API misuse fixtures, compiled by rustdoc against the built library.
//! Missing positions:
//! ```compile_fail
//! use cq_storage::PositionalRows;
//! let host = std::collections::HashSet::from([(1i64,)]);
//! let _ = (&host).positions();
//! ```
//! Missing block read:
//! ```compile_fail
//! use cq_storage::{BlockRead, Column, PositionalRows};
//! fn read<H: PositionalRows, C: Column<H>>(h: H, c: C) {
//!     let _ = h.read(c, &cq_storage::Point(0));
//! }
//! ```
//! Membership alone cannot prove exact interval existence:
//! ```compile_fail
//! use cq_storage::{Membership, ReadPredicate};
//! let set = std::collections::HashSet::from([1i64]);
//! let _ = Membership(&set).test(&1);
//! ```
//! Marker host mismatch:
//! ```compile_fail
//! use cq_storage::*;
//! #[derive(Clone, Copy)] struct Col;
//! impl<'a> Column<&'a [(i64,)]> for Col {
//!     type Key = i64;
//!     fn key(self, h: &'a [(i64,)], id: RowId) -> i64 { *h.row_at(id).unwrap().0 }
//! }
//! let host = [(String::from("x"),)];
//! let _ = Selection::select(&host[..], Col, &Point(1), SelectionSummary::None);
//! ```
//! Predicate key mismatch:
//! ```compile_fail
//! use cq_storage::{Point, ReadPredicate};
//! let _ = Point(1i32).test(&"x");
//! ```
//! Codec key mismatch:
//! ```compile_fail
//! use cq_storage::{AdaptiveKeySet, I64Codec};
//! let _ = AdaptiveKeySet::<_,_,64>::from_keys([String::from("x")], I64Codec);
//! ```
//! Borrowed selection lifetime:
//! ```compile_fail
//! use cq_storage::*;
//! #[derive(Clone, Copy)] struct Col;
//! impl<'a> Column<&'a [(i64,)]> for Col {
//!     type Key = &'a i64;
//!     fn key(self, h: &'a [(i64,)], id: RowId) -> Self::Key { h.row_at(id).unwrap().0 }
//! }
//! let selected;
//! {
//!     let host = [(1i64,)];
//!     selected = Selection::select(&host[..], Col, &Point(&1), SelectionSummary::None);
//! }
//! let _ = selected.rows().next();
//! ```
//! Selection does not confer Canonical:
//! ```compile_fail
//! use cq_storage::{Selection, PositionalRows};
//! fn misuse<H: PositionalRows>(selection: &Selection<H>) {
//!     cq_storage::require::canonical(selection);
//! }
//! ```
//! Selection does not confer OrderedRead:
//! ```compile_fail
//! use cq_storage::{Selection, PositionalRows};
//! fn misuse<H: PositionalRows>(selection: &Selection<H>) {
//!     cq_storage::require::ordered(selection);
//! }
//! ```
//! IDs cannot be constructed unchecked:
//! ```compile_fail
//! let _ = cq_storage::RowId(999);
//! ```
//! ```compile_fail
//! let _ = cq_storage::BlockId(999);
//! ```
//! No raw selection collection protocol:
//! ```compile_fail
//! use cq_storage::{Selection, RowId};
//! fn misuse(ids: Vec<RowId>) -> Selection<&'static [(i64,)]> {
//!     ids.into_iter().collect()
//! }
//! ```
//! Borrowed column keys, borrowed tuple codecs, and host lifetimes compose:
//! ```
//! use cq_storage::*;
//! #[derive(Clone, Copy)] struct Col;
//! impl<'a> Column<&'a [(i64, String)]> for Col {
//!     type Key = &'a i64;
//!     fn key(self, h: &'a [(i64, String)], id: RowId) -> Self::Key {
//!         h.row_at(id).unwrap().0
//!     }
//! }
//! let host = [(1i64, String::from("a")), (1, String::from("b"))];
//! let selection = Selection::select(&host[..], Col, &Point(&1), SelectionSummary::PerBlock);
//! let keys: AdaptiveKeySet<(&i64,), BorrowedI64TupleCodec, 64> =
//!     selection.build(|(k, _)| ((k,), ()));
//! assert!(keys.contains(&(&1,)));
//! assert_eq!(selection.rows().count(), 2);
//! require::positional(&host[..]);
//! require::block_read(&host[..], Col);
//! require::interval::<(&i64,), _>(&keys);
//! ```
