//! GPU-resident packed Datalog tuples and immutable, disjoint accepted runs.
//! All device work uses the owning [`Pool`]'s legacy default stream.
//! [`DeviceLayout`] retains typed, immutable run inputs and uploads only their
//! erased descriptors. [`CandidateArena`] leases packed output lanes to Sequence
//! terminals; [`CandidateBatch`] retains those lanes and input owners without a
//! candidate-row download. Signed packing preserves numeric lexicographic order.
pub mod arena;
pub mod layout;
pub mod pack;
pub mod runs;
pub use gpu_device_columns::{Buffer, Pool};
pub use pack::{Column, Packing, Word};
pub use runs::{AdmissionTimings, DeviceRuns, Run, RunView};
pub use arena::{concatenate_batches, CandidateArena, CandidateBatch, CandidateLease};
pub use layout::{device, DeviceLayout, PackedField, PackedRunView, RunsView, Scalar, ScalarTuple};
pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

thread_local! {
    static THREAD_POOL: std::cell::RefCell<Option<std::sync::Arc<Pool>>> =
        const { std::cell::RefCell::new(None) };
}

/// The calling thread's device-0 pool, created on first use and reused by
/// every later program on this thread. The runtime's thread-local scratch and
/// module caches retain their pool; one pool per program would keep each
/// finished program's peak reservation alive in those caches.
pub fn thread_pool() -> Result<std::sync::Arc<Pool>> {
    THREAD_POOL.with(|slot| {
        if let Some(pool) = slot.borrow().as_ref() {
            return Ok(pool.clone());
        }
        let pool = Pool::new(0, 0, 0)?;
        *slot.borrow_mut() = Some(pool.clone());
        Ok(pool)
    })
}
