//! Cursor operations over a relation's representation, as analysis facts.
//!
//! An operation has a law against the relation's abstraction function:
//! `seek[C](k) = σ_{C=k}`, `contains` its nonemptiness, `ordered`
//! enumeration in key order from any bound, `iter` enumeration of the
//! support. Each is one runtime trait (`IndexRead`, `ExactMembership`,
//! `OrderedRead`). No language spells them as claims: which operations a
//! program performs follows from the plan and the policy, which structure
//! answers them from its key, and whether the type implements the trait is
//! checked by rustc where the generated program uses it.

/// A required cursor operation, with its key columns as heading ordinals.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    Iter,
    Contains(Vec<usize>),
    Seek(Vec<usize>),
    Ordered(Vec<usize>),
}

impl Operation {
    /// Whether performing this access proves the demand `required`: an exact
    /// or ordered seek on the same columns witnesses membership.
    pub fn discharges(&self, required: &Operation) -> bool {
        match (self, required) {
            (
                Operation::Ordered(p) | Operation::Seek(p) | Operation::Contains(p),
                Operation::Contains(r),
            )
            | (Operation::Ordered(p) | Operation::Seek(p), Operation::Seek(r)) => p == r,
            (Operation::Ordered(p), Operation::Ordered(r)) => p == r,
            (Operation::Iter, Operation::Iter) => true,
            _ => false,
        }
    }
}
