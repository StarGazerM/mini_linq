//! Passive counters that generated access code can update explicitly.
//! Storage methods never update these counters or choose what counts as work.

use std::cell::Cell;

/// Logical access observations, not measurements of disk I/O or cache misses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub blocks_considered: usize,
    pub blocks_skipped: usize,
    pub blocks_opened: usize,
    pub rows_visited: usize,
    pub exact_probes: usize,
    pub index_rows: usize,
}

#[derive(Default)]
pub struct Counters(Cell<Work>);

impl Counters {
    pub fn add(&self, change: impl FnOnce(&mut Work)) {
        let mut value = self.0.get();
        change(&mut value);
        self.0.set(value);
    }

    pub fn take(&self) -> Work {
        self.0.take()
    }
}
