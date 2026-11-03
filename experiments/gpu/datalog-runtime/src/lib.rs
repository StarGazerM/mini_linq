//! GPU-resident packed Datalog tuples and immutable full-key radix sets.
//! All device work uses an explicitly ordered [`Pool`] stream.
//! [`RadixLayout`] retains typed immutable radix snapshots and field metadata.
//! [`CandidateArena`] leases packed output lanes to Sequence
//! terminals; [`CandidateBatch`] retains those lanes and input owners without a
//! candidate-row download. Signed packing preserves numeric lexicographic order.
//! [`DeviceRadixRun`] is the occurrence-preserving grouped index engine;
//! [`RadixLayout`] is its typed relational view over one immutable publication.
pub mod arena;
pub mod lanes;
pub mod layout;
pub mod pack;
pub mod phases;
pub mod radix_run;
mod runs;
mod stats;
pub use arena::{concatenate_batches, CandidateArena, CandidateBatch, CandidateLease};
pub use gpu_device_columns::{Buffer, Pool};
pub use lanes::{Job, Lane, Lanes};
pub use layout::{encode_scalar, PackedField, Scalar, ScalarTuple};
pub use pack::{Column, Packing, Word};
pub use radix_run::{
    DeviceItemColumns, DeviceRadixRun, ItemColumn, OffsetWord, RadixItemsView, RadixKeysView,
    RadixLayout, RadixLayoutView, RadixMatch, RadixMatches, RadixRunParams, RadixSnapshot,
    SourceItemColumns, SourceLane,
};
pub use runs::SortedBatch;
pub use stats::{
    readback_count, record_readback, reset_work_counters, set_work_counters_enabled,
    trace_kernels, trace_memory, trace_work, work_counters, MemoryUse, SortWork, WorkCounters,
};
pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

// DL-WaveLanes Sync audit: lane workers receive cloned input layouts, run holes
// into their own arenas on lane pools, and return batches to the main thread.
const _: () = {
    const fn send_sync<T: Send + Sync>() {}
    const fn send<T: Send>() {}
    send_sync::<Pool>();
    send_sync::<Buffer<u64>>();
    send_sync::<Packing>();
    send_sync::<RadixLayout<(u32, i64), (), u128, u64, ()>>();
    send_sync::<CandidateBatch>();
    send::<CandidateArena>();
    send::<Job>();
};

/// Host-side bijection between logical symbols and their device integer codes.
///
/// Codes must be stable for an execution, preserve equality, and decode back to
/// the original logical value. Device kernels never receive `T` or invoke this
/// codec: uploads and captured constants call `code`, downloads call `decode`.
pub trait SymbolCodec<T> {
    fn code(value: &T) -> u32;
    // DECODE P: datalog print output reconstruction invokes this inverse for final host answer rows.
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

        // DECODE P: diagnostic Text codec exposes logical strings for round-trip assertions.
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
            // DECODE P: diagnostic direct symbol round trip publishes its reconstructed String.
            assert_eq!(Text::decode(Text::code(&logical)), logical);
        }
        assert_eq!(Text::code(&"last".to_owned()), u32::MAX);
        type TextScalar = super::Symbol<String, Text>;
        for value in ["", "first", "last"] {
            let logical = value.to_owned();
            let code = <TextScalar as super::Scalar>::encode(&logical);
            assert_eq!(code, Text::code(&logical) as i64);
            // DECODE P: diagnostic Scalar symbol round trip publishes its reconstructed logical value.
            assert_eq!(<TextScalar as super::Scalar>::decode(code as i128), logical);
        }
    }
}
