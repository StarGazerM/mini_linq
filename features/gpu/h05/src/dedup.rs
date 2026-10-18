//! Exact GPU deduplication of domain-validated (order, supplier, nation) triples.
//!
//! Every occupied table slot contains `((order << 22) | (supplier << 5) |
//! nation) + 1`; zero is reserved for an empty slot. Hashes select probe starts
//! only: the complete encoded key determines equality. No shared memory or
//! RCCL device code is used.

use cuda_core::{CudaContext, CudaStream, LaunchConfig1D};
use gpu_blocked_run::{Buffer, MemoryResource};
use std::{error::Error, sync::Arc};

/// Enqueue deduplication of the first `count_device[0]` triples on the GPU.
///
/// The caller must validate `0 <= order < 2^26`, `0 <= supplier < 2^17`, and
/// `0 <= nation < 2^5` before uploading inputs. These bounds make the encoding
/// injective, with occupied words in `1..=2^48`, including the all-zero triple.
/// Inputs must belong to `ctx` and their writes must precede this call on
/// `stream`. `count_device` must contain the same `count` used to size the
/// launch, and `capacity` must be a power of two and at least `4 * count`.
///
/// Returns the hash table (including zero-valued empty slots) and two device
/// metadata words: `[unique_count, full_status]`. Only successful insertions
/// increment the count. The caller must read metadata after stream completion
/// and reject nonzero full status before accepting the result. This function
/// performs no host readback or synchronization; all buffers must remain alive
/// until their stream-ordered work completes.
/// Output ownership borrows the shared pool and stream; dropping these buffers
/// returns their storage to the pool without a synchronous CUDA device free.
pub fn dedup<'a, B: cuda_host::KernelSliceArg<Elem = i64>>(
    ctx: &Arc<CudaContext>,
    stream: &'a CudaStream,
    memory: &'a MemoryResource,
    orders: &B,
    suppliers: &B,
    nations: &B,
    count_device: &Buffer<'_, u32>,
    count: usize,
    capacity: usize,
) -> Result<(Buffer<'a, u64>, Buffer<'a, u32>), Box<dyn Error>> {
    if count_device.is_empty() {
        return Err("dedup device row count is missing".into());
    }
    if count > orders.len() || count > suppliers.len() || count > nations.len() {
        return Err("dedup input count exceeds a column length".into());
    }
    if count > u32::MAX as usize {
        return Err("dedup input count exceeds the u32 counter domain".into());
    }
    let minimum_capacity = count.checked_mul(4).ok_or("dedup capacity overflow")?;
    if !capacity.is_power_of_two() || capacity < minimum_capacity {
        return Err("dedup capacity must be a power of two and at least four times the input count".into());
    }

    let table = Buffer::<u64>::zeroed_in(memory, stream, capacity)?;
    let metadata = Buffer::<u32>::zeroed_in(memory, stream, 2)?;
    if count != 0 {
        let module = unsafe { dedup_kernels::load(ctx)? };
        let config = LaunchConfig1D::new(count.div_ceil(256) as u32, 256, 0);
        let launch = module.prepare_dedup_insert(config)?;
        // SAFETY: the buffers have the declared capacities and the caller
        // keeps them alive through stream completion. All launches and
        // initialization use this same stream; table accesses are atomic.
        unsafe {
            module.dedup_insert(
                stream,
                &launch,
                orders,
                suppliers,
                nations,
                count_device.cu_deviceptr() as *const u32,
                table.cu_deviceptr() as *mut u64,
                capacity,
                metadata.cu_deviceptr() as *mut u32,
            )?;
        }
    }
    Ok((table, metadata))
}

/// Decode an occupied table word. Skip empty (`0`) slots before calling.
pub fn decode(word: u64) -> (i64, i64, i64) {
    let packed = word - 1;
    (
        (packed >> 22) as i64,
        ((packed >> 5) & ((1u64 << 17) - 1)) as i64,
        (packed & 31) as i64,
    )
}

#[cuda_device::cuda_module]
mod dedup_kernels {
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32, DeviceAtomicU64};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn dedup_insert(
        orders: &[i64],
        suppliers: &[i64],
        nations: &[i64],
        count_device: *const u32,
        table: *mut u64,
        capacity: usize,
        metadata: *mut u32,
    ) {
        let row = thread::index_1d().get();
        if row >= unsafe { count_device.read() } as usize {
            return;
        }
        let word = ((orders[row] as u64) << 22
            | (suppliers[row] as u64) << 5
            | nations[row] as u64)
            + 1;

        // SplitMix64's finalizer spreads all three exact key fields over the
        // table index; no hash bits are used for duplicate equality.
        let mut hash = word;
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        hash ^= hash >> 31;
        let mask = capacity - 1;
        let mut slot = hash as usize & mask;
        let mut probes = 0;
        while probes < capacity {
            let entry = unsafe { DeviceAtomicU64::from_ptr(table.add(slot)) };
            match entry.compare_exchange_relaxed(0, word) {
                Ok(_) => {
                    unsafe { DeviceAtomicU32::from_ptr(metadata) }
                        .fetch_add(1, AtomicOrdering::Relaxed);
                    return;
                }
                Err(existing) => {
                    if existing == word {
                        return;
                    }
                }
            }
            slot = (slot + 1) & mask;
            probes += 1;
        }
        // A full traversal is finite even if the host's capacity invariant is
        // breached. No occupied slot is ever overwritten or treated as equal
        // merely because its hash collides.
        unsafe { DeviceAtomicU32::from_ptr(metadata.add(1)) }
            .store(1, AtomicOrdering::Relaxed);
    }
}
