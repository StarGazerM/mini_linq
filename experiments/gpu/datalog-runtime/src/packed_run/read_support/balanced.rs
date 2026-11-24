//! Actual-length scheduling without a device-to-host work-count readback.
//! Each warp owns 1024 atoms in the concatenation of all run/probe ranges.
use super::*;

pub(super) fn read<W: Word>(state: &PackedSet<W>, low: &Buffer<W>, high: &Buffer<W>, suffix_bits: u32, plain_storage: bool) -> Result<Buffer<[u64; 3]>> {
    let output = Buffer::zeroed(low.pool().clone(), low.len())?;
    if !low.is_empty() && !state.runs().is_empty() {
        if plain_storage { enqueue::<W, true, false, false>(state, low, high, suffix_bits, &output)?; }
        else {
            match checksum_layout(state, suffix_bits) {
                (true, true) => enqueue::<W, false, true, true>(state, low, high, suffix_bits, &output)?,
                (true, false) => enqueue::<W, false, true, false>(state, low, high, suffix_bits, &output)?,
                _ => enqueue::<W, false, false, false>(state, low, high, suffix_bits, &output)?,
            }
        }
    }
    Ok(output)
}

fn enqueue<W: Word, const PLAIN: bool, const LINEAR: bool, const NARROW: bool>(state: &PackedSet<W>, low: &Buffer<W>, high: &Buffer<W>, suffix_bits: u32, output: &Buffer<[u64; 3]>) -> Result<()> {
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU64};
    let pool = low.pool();
    let views = state.read_views()?;
    let probes = low.len();
    let entries = probes * state.runs().len();
    let launch = state.modules.rccl.on(pool.stream())?;
    let mut workspace = rccl_cub::expand_ranges_workspace_in(pool.resource(), &launch, entries)?;
    let ranges = Ranges::<W, PLAIN> { views: views.ptr(), low: low.ptr(), high: high.ptr(), probes };
    let target = output.ptr();
    rccl_cub::device_prepare_ranges_in(&launch, ranges, |r: [u64; 2]| (r[0], r[1].max(r[0])),
        move |i, r: [u64; 2]| unsafe {
            let result = target.add(i % probes).cast::<u64>();
            DeviceAtomicU64::from_ptr(result).fetch_add(r[0], AtomicOrdering::Relaxed);
            DeviceAtomicU64::from_ptr(result.add(1)).fetch_add(r[1], AtomicOrdering::Relaxed);
        }, entries, u64::MAX, &mut workspace)?;
    let sources = Sources::<W, PLAIN, LINEAR, NARROW> { views: views.ptr(), probes, suffix_bits };
    rccl_cub::device_reduce_ranges_in(&launch, sources, move |i, sum| unsafe {
        DeviceAtomicU64::from_ptr(target.add(i % probes).cast::<u64>().add(2)).fetch_add(sum, AtomicOrdering::Relaxed);
    }, entries, &mut workspace)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct Ranges<W: Word, const PLAIN: bool> {
    views: *const PackedView<W>,
    low: *const W,
    high: *const W,
    probes: usize,
}
impl<W: Word, const PLAIN: bool> rccl_cub::DeviceReadAt for Ranges<W, PLAIN> {
    type Item = [u64; 2];
    #[inline(always)]
    unsafe fn read_at(self, index: usize) -> Self::Item {
        let probe = index % self.probes;
        let (a, b) = device::read_range::<W, PLAIN>(*self.views.add(index / self.probes), *self.low.add(probe), *self.high.add(probe));
        [a, b]
    }
}

#[derive(Clone, Copy)]
struct Sources<W: Word, const PLAIN: bool, const LINEAR: bool, const NARROW: bool> {
    views: *const PackedView<W>,
    probes: usize,
    suffix_bits: u32,
}
impl<W: Word, const PLAIN: bool, const LINEAR: bool, const NARROW: bool> rccl_cub::DeviceReadAt for Sources<W, PLAIN, LINEAR, NARROW> {
    type Item = ChecksumRange<W, PLAIN, LINEAR, NARROW>;
    #[inline(always)]
    unsafe fn read_at(self, index: usize) -> Self::Item {
        ChecksumRange { view: *self.views.add(index / self.probes), suffix_bits: self.suffix_bits }
    }
}

#[derive(Clone, Copy)]
struct ChecksumRange<W: Word, const PLAIN: bool, const LINEAR: bool, const NARROW: bool> {
    view: PackedView<W>,
    suffix_bits: u32,
}
impl<W: Word, const PLAIN: bool, const LINEAR: bool, const NARROW: bool> rccl_cub::DeviceReadAt for ChecksumRange<W, PLAIN, LINEAR, NARROW> {
    type Item = u64;
    #[inline(always)]
    unsafe fn read_at(self, index: usize) -> u64 {
        if PLAIN { checksum(*self.view.plain.add(index), self.suffix_bits) }
        else if LINEAR { checksum_linear_row(self.view, index as u64, self.suffix_bits) }
        else { checksum_row(self.view, index as u64, self.suffix_bits) }
    }
    #[inline(always)]
    unsafe fn sum_cooperative(self, begin: usize, end: usize, lane: usize, lanes: usize) -> u64 {
        if end - begin <= lane { return 0; }
        if lanes.is_power_of_two() && lanes <= u32::MAX as usize {
            return super::read_kernels::scan_sum::<W, PLAIN, LINEAR, NARROW>(self.view, begin as u64, end as u64, lane as u32, lanes.trailing_zeros(), self.suffix_bits);
        }
        self.sum_strided(begin + lane, end, lanes)
    }
}

