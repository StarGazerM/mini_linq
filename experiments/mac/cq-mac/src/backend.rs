//! Thin CQ adapter over MCCL's completed immutable MLX pair indexes.

use std::fmt;

#[cfg(target_os = "macos")]
#[path = "radix.rs"]
mod radix;
#[cfg(target_os = "macos")]
pub use radix::{IndexedI64Relation, MacRadixRun};

/// Conservative bound for padded radix/scan/merge grids and u32 offsets.
/// Individual allocations must also fit the selected device's buffer limit.
pub const MAX_ROWS: usize = 1 << 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacError {
    UnsupportedPlatform,
    CapacityExceeded,
    ContextMismatch,
    AllocationFailed(&'static str),
    CommandFailed(String),
}

impl fmt::Display for MacError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                f.write_str("cq-mac requires macOS; no CPU fallback is provided")
            }
            Self::CapacityExceeded => {
                f.write_str("row count or buffer size exceeds cq-mac/device capacity")
            }
            Self::ContextMismatch => f.write_str("runs belong to a different MCCL context"),
            Self::AllocationFailed(resource) => write!(f, "MLX/host allocation failed: {resource}"),
            Self::CommandFailed(message) => write!(f, "MCCL operation failed: {message}"),
        }
    }
}

impl std::error::Error for MacError {}

/// A column projection over an immutable shared source relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexSpec {
    pub key_column: usize,
    pub payload: Payload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    RowId,
    Column(usize),
}

/// Immutable, cheaply cloned source storage retained independently of indexes.
#[derive(Clone)]
pub struct Relation {
    inner: platform::Source,
}

impl Relation {
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn columns(&self) -> usize {
        self.inner.columns()
    }
    pub fn u32_values(&self) -> Result<&[u32], MacError> {
        platform::u32_values(&self.inner)
    }
    pub fn i64_values(&self) -> Result<&[i64], MacError> {
        platform::i64_values(&self.inner)
    }
}

/// Thread-affine MCCL context. MLX owns allocation pooling and GPU completion
/// lifetimes; published immutable runs retain their context and may outlive it.
pub struct MacContext {
    inner: platform::Context,
}

impl MacContext {
    pub fn new() -> Result<Self, MacError> {
        platform::Context::new().map(|inner| Self { inner })
    }

    pub(crate) fn owns_run(&self, run: &MacRun) -> bool {
        self.inner.owns(&run.inner)
    }

    /// Ingest full numeric rows directly into shared storage exactly once.
    pub fn relation_u32<const C: usize>(
        &self,
        rows: usize,
        initialize: impl FnMut(usize) -> [u32; C],
    ) -> Result<Relation, MacError> {
        self.inner
            .relation_u32(rows, initialize)
            .map(|inner| Relation { inner })
    }

    /// Retain wide signed values without narrowing during ingestion.
    pub fn relation_i64<const C: usize>(
        &self,
        rows: usize,
        initialize: impl FnMut(usize) -> [i64; C],
    ) -> Result<Relation, MacError> {
        self.inner
            .relation_i64(rows, initialize)
            .map(|inner| Relation { inner })
    }

    /// GPU projection, optional selection, stable sort and directory construction.
    /// Input storage already exists; this operation never ingests host rows.
    pub fn build(
        &self,
        source: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<MacRun, MacError> {
        self.inner
            .build(&source.inner, spec, selection.map(|s| &s.inner))
            .map(|inner| MacRun { inner })
    }

    /// Explicit stable CPU comparison-sort baseline over the same shared source.
    pub fn build_on_cpu(
        &self,
        source: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<MacRun, MacError> {
        self.inner
            .build_on_cpu(&source.inner, spec, selection.map(|s| &s.inner))
            .map(|inner| MacRun { inner })
    }

    /// Explicit stable CPU byte-radix baseline over the same shared source.
    pub fn build_on_cpu_radix(
        &self,
        source: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<MacRun, MacError> {
        self.inner
            .build_on_cpu_radix(&source.inner, spec, selection.map(|s| &s.inner))
            .map(|inner| MacRun { inner })
    }

    /// Completed stable GPU merge; older equal keys precede newer equal keys.
    /// Both inputs must belong to this context. Independently created contexts
    /// are rejected with ContextMismatch even when using the same physical GPU;
    /// neither input is modified and rejection submits no GPU work.
    pub fn merge(&self, older: &MacRun, newer: &MacRun) -> Result<MacRun, MacError> {
        self.inner
            .merge(&older.inner, &newer.inner)
            .map(|inner| MacRun { inner })
    }
}

/// Cheaply cloned ownership of completed immutable MLX rows and directory.
/// Reads never evaluate a graph, call FFI, allocate, or retain another owner.
#[derive(Clone)]
pub struct MacRun {
    inner: platform::Completed,
}

impl fmt::Debug for MacRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MacRun")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl MacRun {
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn rows(&self) -> impl ExactSizeIterator<Item = (u32, u32)> + DoubleEndedIterator + '_ {
        self.inner.rows().iter().map(|row| (row.key, row.value))
    }

    /// Exact radix-12 bucket lookup, retaining duplicate payload chronology.
    pub fn lookup(&self, key: &u32) -> RunCursor<'_> {
        self.inner.lookup(key)
    }

    pub fn contains(&self, key: &u32) -> bool {
        self.inner.contains(key)
    }
}

pub use platform::Cursor as RunCursor;

impl<'a> cq_storage::IndexRead for &'a MacRun {
    type Key = u32;
    type Item = u32;
    type Cursor = RunCursor<'a>;

    fn lookup(&self, key: &u32) -> Self::Cursor {
        MacRun::lookup(self, key)
    }
}

impl cq_storage::ExactMembership<u32> for MacRun {
    fn contains(&self, key: &u32) -> bool {
        MacRun::contains(self, key)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{IndexSpec, MAX_ROWS, MacError, Payload};
    pub use mccl::index::Cursor;
    pub(super) type Completed = mccl::index::Index;
    pub(super) type Source = mccl::relation::Relation;

    pub(super) struct Context(pub(super) mccl::Gpu);

    pub(super) fn error(error: mccl::Error) -> MacError {
        MacError::CommandFailed(error.to_string())
    }

    pub(super) fn projection(spec: IndexSpec) -> mccl::relation::IndexSpec {
        mccl::relation::IndexSpec {
            key_column: spec.key_column,
            payload: match spec.payload {
                Payload::RowId => mccl::relation::Payload::RowId,
                Payload::Column(column) => mccl::relation::Payload::Column(column),
            },
        }
    }

    pub(super) fn u32_values(source: &Source) -> Result<&[u32], MacError> {
        source.u32_values().map_err(error)
    }
    pub(super) fn i64_values(source: &Source) -> Result<&[i64], MacError> {
        source.i64_values().map_err(error)
    }

    impl Context {
        pub(super) fn new() -> Result<Self, MacError> {
            mccl::Gpu::new().map(Self).map_err(error)
        }

        pub(super) fn owns(&self, run: &Completed) -> bool {
            run.belongs_to(&self.0)
        }

        pub(super) fn relation_u32<const C: usize>(
            &self,
            rows: usize,
            initialize: impl FnMut(usize) -> [u32; C],
        ) -> Result<Source, MacError> {
            if rows > MAX_ROWS {
                return Err(MacError::CapacityExceeded);
            }
            self.0.relation_u32(rows, initialize).map_err(error)
        }

        pub(super) fn relation_i64<const C: usize>(
            &self,
            rows: usize,
            initialize: impl FnMut(usize) -> [i64; C],
        ) -> Result<Source, MacError> {
            if rows > MAX_ROWS {
                return Err(MacError::CapacityExceeded);
            }
            self.0.relation_i64(rows, initialize).map_err(error)
        }

        fn check_source(
            &self,
            source: &Source,
            selection: Option<&Source>,
        ) -> Result<(), MacError> {
            if !source.belongs_to(&self.0) || selection.is_some_and(|s| !s.belongs_to(&self.0)) {
                return Err(MacError::ContextMismatch);
            }
            if source.len() > MAX_ROWS {
                return Err(MacError::CapacityExceeded);
            }
            Ok(())
        }

        pub(super) fn build(
            &self,
            source: &Source,
            spec: IndexSpec,
            selection: Option<&Source>,
        ) -> Result<Completed, MacError> {
            self.check_source(source, selection)?;
            self.0
                .build_index(source, projection(spec), selection)
                .map_err(error)
        }

        pub(super) fn build_on_cpu(
            &self,
            source: &Source,
            spec: IndexSpec,
            selection: Option<&Source>,
        ) -> Result<Completed, MacError> {
            self.check_source(source, selection)?;
            self.0
                .build_index_on_cpu(source, projection(spec), selection)
                .map_err(error)
        }

        pub(super) fn build_on_cpu_radix(
            &self,
            source: &Source,
            spec: IndexSpec,
            selection: Option<&Source>,
        ) -> Result<Completed, MacError> {
            self.check_source(source, selection)?;
            self.0
                .build_index_on_cpu_radix(source, projection(spec), selection)
                .map_err(error)
        }

        pub(super) fn merge(
            &self,
            older: &Completed,
            newer: &Completed,
        ) -> Result<Completed, MacError> {
            if !older.belongs_to(&self.0) || !newer.belongs_to(&self.0) {
                return Err(MacError::ContextMismatch);
            }
            older
                .len()
                .checked_add(newer.len())
                .filter(|&n| n <= MAX_ROWS)
                .ok_or(MacError::CapacityExceeded)?;
            self.0.merge_indexes(older, newer).map_err(error)
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{IndexSpec, MacError};
    use std::{convert::Infallible, iter::FusedIterator};

    pub(super) struct Context;

    // No host representation/fallback can be constructed on unsupported targets.
    #[derive(Clone)]
    pub(super) struct Completed(Infallible);

    #[derive(Clone)]
    pub(super) struct Source(Infallible);
    impl Source {
        pub(super) fn len(&self) -> usize {
            match self.0 {}
        }
        pub(super) fn columns(&self) -> usize {
            match self.0 {}
        }
    }
    pub(super) fn u32_values(source: &Source) -> Result<&[u32], MacError> {
        match source.0 {}
    }
    pub(super) fn i64_values(source: &Source) -> Result<&[i64], MacError> {
        match source.0 {}
    }

    pub(super) struct Row {
        pub key: u32,
        pub value: u32,
    }

    #[derive(Clone, Copy, Debug)]
    pub struct Cursor<'a> {
        never: &'a Infallible,
    }

    impl Iterator for Cursor<'_> {
        type Item = u32;
        fn next(&mut self) -> Option<u32> {
            match *self.never {}
        }
        fn size_hint(&self) -> (usize, Option<usize>) {
            match *self.never {}
        }
    }
    impl ExactSizeIterator for Cursor<'_> {}
    impl FusedIterator for Cursor<'_> {}

    impl Context {
        pub(super) fn new() -> Result<Self, MacError> {
            Err(MacError::UnsupportedPlatform)
        }
        pub(super) fn relation_u32<const C: usize>(
            &self,
            _: usize,
            _: impl FnMut(usize) -> [u32; C],
        ) -> Result<Source, MacError> {
            Err(MacError::UnsupportedPlatform)
        }
        pub(super) fn relation_i64<const C: usize>(
            &self,
            _: usize,
            _: impl FnMut(usize) -> [i64; C],
        ) -> Result<Source, MacError> {
            Err(MacError::UnsupportedPlatform)
        }
        pub(super) fn build(
            &self,
            _: &Source,
            _: IndexSpec,
            _: Option<&Source>,
        ) -> Result<Completed, MacError> {
            Err(MacError::UnsupportedPlatform)
        }
        pub(super) fn build_on_cpu(
            &self,
            _: &Source,
            _: IndexSpec,
            _: Option<&Source>,
        ) -> Result<Completed, MacError> {
            Err(MacError::UnsupportedPlatform)
        }
        pub(super) fn build_on_cpu_radix(
            &self,
            _: &Source,
            _: IndexSpec,
            _: Option<&Source>,
        ) -> Result<Completed, MacError> {
            Err(MacError::UnsupportedPlatform)
        }
        pub(super) fn merge(&self, _: &Completed, _: &Completed) -> Result<Completed, MacError> {
            Err(MacError::UnsupportedPlatform)
        }
        pub(super) fn owns(&self, run: &Completed) -> bool {
            match run.0 {}
        }
    }

    impl Completed {
        pub(super) fn len(&self) -> usize {
            match self.0 {}
        }
        pub(super) fn rows(&self) -> &[Row] {
            match self.0 {}
        }
        pub(super) fn lookup(&self, _: &u32) -> Cursor<'_> {
            match self.0 {}
        }
        pub(super) fn contains(&self, _: &u32) -> bool {
            match self.0 {}
        }
    }
}
