//! Feature-off aliases are the original collections and cursors, with no state
//! or counter calls. Compiler-emitted tokens never depend on this feature.
#[cfg(feature = "counters")]
pub(crate) use crate::counters::{HashMap, HashSet};
#[cfg(not(feature = "counters"))]
pub(crate) use std::collections::{HashMap, HashSet};

#[cfg(feature = "counters")]
pub struct Cursor<I> {
    pub(crate) source: I,
}
#[cfg(not(feature = "counters"))]
pub type Cursor<I> = I;

#[cfg(feature = "counters")]
macro_rules! cursor {
    ($source:expr $(,)?) => {
        $crate::instrument::Cursor { source: $source }
    };
}
#[cfg(not(feature = "counters"))]
macro_rules! cursor {
    ($source:expr $(,)?) => {
        $source
    };
}
pub(crate) use cursor;
#[cfg(feature = "counters")]
impl<I: Iterator> Iterator for Cursor<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        crate::count!(cursor_steps);
        let item = self.source.next();
        if item.is_some() {
            crate::count!(rows_enumerated);
        }
        item
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.size_hint()
    }
}
#[cfg(feature = "counters")]
impl<I: std::iter::FusedIterator> std::iter::FusedIterator for Cursor<I> {}
