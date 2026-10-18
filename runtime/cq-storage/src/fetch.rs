//! Reconstruct a represented column from a live base occurrence.
use crate::rows::Columns;
use crate::{Column, PositionalRows, RowId};

/// Fetching commutes with the host's row abstraction: the result is the named
/// column of `row_at(id)`. The ID must be live and belong to this host.
pub trait Fetch<C: Column<Self>>: PositionalRows {
    fn fetch(self, column: C, id: RowId) -> C::Key;
}

macro_rules! fetch {
    ([$($g:tt)*] $ty:ty) => {
        impl<'a, $($g)* C> Fetch<C> for &'a $ty
        where &'a T: Columns, C: Column<Self> {
            fn fetch(self, column: C, id: RowId) -> C::Key {
                column.key(self, id)
            }
        }
    };
}
fetch!([T,][T]);
fetch!([T,] Vec<T>);
fetch!([T, const N: usize,] [T; N]);
