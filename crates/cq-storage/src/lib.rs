//! Physical access traits and concrete data structures for generated programs.
//!
//! These structures do not implement relational operators, choose access
//! paths, or couple a collection to a query-wide representation. Query access
//! is available only through the operation traits; a user type implementing
//! [`Build`] plus the access traits a plan uses is a first-class structure,
//! and the compiler names it without inspecting it.
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
//! - `α(membership) = key set`; `contains(k) ⇔ k ∈ α`.
//! - `α(ordered) = bag of (key, item)`, enumerated in key order from any
//!   bound; [`merge`] is the one co-iteration primitive; [`sorted_by`]
//!   presents an already key-ordered stream as such a cursor.
//! - [`Canonical`] marks a collection that is a set of its keys by
//!   construction.
//!
//! [`require`] holds one-line guards the compiler emits beside each
//! structure so a missing trait fails there, naming the structure.

mod hash_index;
mod host;
mod membership;
mod ordered;

pub use hash_index::{BucketCursor, Build, HashIndex, IndexRead};
pub use membership::{Canonical, ExactMembership};
pub use ordered::{
    GroupedCursor, KeyCursor, Merge, OrderedCursor, OrderedRead, Sorted, merge, sorted_by,
};

pub mod require {
    pub fn seek<T: super::IndexRead>(_: T) {}
    pub fn contains<K: ?Sized, T: super::ExactMembership<K>>(_: T) {}
    pub fn ordered<T: super::OrderedRead>(_: T) {}
    pub fn canonical<T: super::Canonical>(_: T) {}
}
