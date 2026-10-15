//! Filtering as a representation operation.
//!
//! `α(s.filter(column, p)) = { o ∈ α(s) | p(column(o)) }`: every read of the
//! result (rows, positions, contains, narrow, build) agrees with that relation,
//! with multiplicity and in source order, and filtering twice equals filtering
//! once by the conjunction. A semijoin is a filter whose predicate is
//! [`Membership`](crate::Membership) over the support's key set.
//!
//! Each realization makes a filtered tuple unreadable by construction, not by
//! skipping it in one cursor: a positional host filters to a [`Selection`],
//! whose only state is its bitmap and whose every read consults it; a selection
//! narrows to a smaller one; [`Selection::build`] compacts survivors into a new
//! structure that never contains the filtered tuples.
use crate::{BlockRead, Column, PositionalRows, ReadPredicate, Selection, SelectionSummary};

/// Narrow a representation by a key predicate; see the module law.
pub trait Filter<C, P> {
    type Filtered;
    fn filter(self, column: C, predicate: &P) -> Self::Filtered;
}

macro_rules! filter_host {
    ([$($g:tt)*] $ty:ty) => {
        impl<'a, $($g)* C, P> Filter<C, P> for &'a $ty
        where
            &'a $ty: BlockRead<C>,
            C: Column<&'a $ty>,
            P: ReadPredicate<C::Key>,
        {
            type Filtered = Selection<&'a $ty>;
            fn filter(self, column: C, predicate: &P) -> Self::Filtered {
                Selection::select(self, column, predicate, SelectionSummary::None)
            }
        }
    };
}
filter_host!([T,][T]);
filter_host!([T,] Vec<T>);
filter_host!([T, const N: usize,] [T; N]);

impl<H, C, P> Filter<C, P> for &Selection<H>
where
    H: PositionalRows + BlockRead<C>,
    C: Column<H>,
    P: ReadPredicate<C::Key>,
{
    type Filtered = Selection<H>;
    fn filter(self, column: C, predicate: &P) -> Self::Filtered {
        self.narrow(column, predicate, SelectionSummary::None)
    }
}
