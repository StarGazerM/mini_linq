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
pub mod phases;
mod stats;
pub use stats::{readback_count, record_readback, trace_kernels, trace_memory, MemoryUse};
pub use gpu_device_columns::{Buffer, Pool};
pub use pack::{Column, Packing, Word};
pub use runs::{AdmissionTimings, DeviceRuns, Run, RunView};
pub use arena::{concatenate_batches, CandidateArena, CandidateBatch, CandidateLease};
pub use layout::{device, DeviceLayout, PackedField, PackedRunView, RunsView, Scalar, ScalarTuple};
pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

/// Host-side bijection between logical symbols and their device integer codes.
///
/// Codes must be stable for an execution, preserve equality, and decode back to
/// the original logical value. Device kernels never receive `T` or invoke this
/// codec: uploads and captured constants call `code`, downloads call `decode`.
pub trait SymbolCodec<T> {
    fn code(value: &T) -> u32;
    fn decode(code: u32) -> T;
}

/// Type-only layout metadata for a logical column stored as a `u32` code.
pub struct Symbol<T, C>(std::marker::PhantomData<fn() -> (T, C)>);

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

#[cfg(test)]
mod symbol_tests {
    use super::SymbolCodec;

    struct Text;
    impl SymbolCodec<String> for Text {
        fn code(value: &String) -> u32 {
            match value.as_str() {
                "" => 0,
                "first" => 1,
                "last" => u32::MAX,
                _ => panic!("unknown test symbol"),
            }
        }

        fn decode(code: u32) -> String {
            match code {
                0 => String::new(),
                1 => "first".to_owned(),
                u32::MAX => "last".to_owned(),
                _ => panic!("unknown test code"),
            }
        }
    }

    #[test]
    fn codecs_restore_owned_logical_values_across_the_full_code_domain() {
        for value in ["", "first", "last"] {
            let logical = value.to_owned();
            assert_eq!(Text::decode(Text::code(&logical)), logical);
        }
        assert_eq!(Text::code(&"last".to_owned()), u32::MAX);
    }
}
