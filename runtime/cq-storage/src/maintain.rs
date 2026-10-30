//! Ordinary index maintenance (P3d, `doc/PLANNING-REDESIGN.md`).
//!
//! Distinct and Datalog admission are one construct: each candidate batch is
//! anti-joined against an index of the output relation, and the fresh rows are
//! published into every index of that relation (`doc/formal/incremental.tex`,
//! ADM-Batch). Every index is a complete column permutation; the
//! index-selection pass records which ordinary member is canonical. Nothing
//! here names a canonical, `seen` or `all` store: these are the operations any
//! index provider offers, together with [`crate::ExactMembership`].
//!
//! How a provider deduplicates a batch (hashing, sorted-run merge, radix sort)
//! and when it physically combines storage (eagerly, or lazily as leveled runs)
//! are its own choices under the laws below. Rust bounds enforce capabilities,
//! not these laws.

// DESIGN index-protocol
// PLANNING P3d: canonical is an ordinary index chosen by index selection; no
// separate membership store, provider kind or printer branch may exist.

/// Formal rule IX-Fresh: the distinct rows of a batch absent from the represented relation.
///
/// For an index representing relation `O`, `fresh(batch)` returns exactly
/// `supp(batch) \ O`, without duplicates, in a provider-chosen order. It reads
/// the index only: the represented relation is unchanged. The batch is taken by
/// value so a provider may reuse its allocation.
pub trait Fresh {
    type Row;
    fn fresh(&self, batch: Vec<Self::Row>) -> Vec<Self::Row>;
}

/// Formal rule IX-Publish: extend the represented relation by disjoint, duplicate-free rows.
///
/// Precondition: the items are the index's projection of rows that are
/// pairwise distinct and absent from the represented relation (for example the
/// output of [`Fresh::fresh`] on the canonical index). Afterwards the index
/// represents the old relation plus exactly those rows. Reads between two
/// publications observe the same relation (IX-Stable); a provider may defer
/// physical merging as long as every read observes the published relation.
pub trait Publish {
    type Item;
    fn publish<I: IntoIterator<Item = Self::Item>>(&mut self, items: I);
}
