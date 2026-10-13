//! Feature-off aliases are the selected fast-hash collections and cursors, with no counter state
//! or counter calls. Compiler-emitted tokens never depend on this feature.
#[cfg(feature = "counters")]
pub(crate) use crate::counters::{HashMap, HashSet};
#[cfg(not(feature = "counters"))]
pub(crate) type HashMap<K, V> = std::collections::HashMap<K, V, crate::HashState>;
#[cfg(not(feature = "counters"))]
pub(crate) use crate::HashSet;

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

#[cfg(not(feature = "counters"))]
pub(crate) type RetainedVec<V> = Vec<V>;
#[cfg(feature = "counters")]
pub struct RetainedVec<V> {
    values: Vec<V>,
    allocation: crate::counters_memory::Allocation,
}
#[cfg(feature = "counters")]
impl<V> Default for RetainedVec<V> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            allocation: crate::counters_memory::Allocation::default(),
        }
    }
}
#[cfg(feature = "counters")]
impl<V> RetainedVec<V> {
    pub(crate) fn account(&mut self) {
        self.allocation = crate::counters_memory::Allocation::new(
            self.values.capacity() * std::mem::size_of::<V>(),
        );
    }
}
#[cfg(feature = "counters")]
impl<V> std::ops::Deref for RetainedVec<V> {
    type Target = Vec<V>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
#[cfg(feature = "counters")]
impl<V> std::ops::DerefMut for RetainedVec<V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}
