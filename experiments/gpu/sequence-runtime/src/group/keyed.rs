//! Host protocol and device monoids for the two realizations of a keyed fold.
use std::mem::size_of;
use super::*;
use cuda_device::{atomic::{AtomicOrdering, BlockAtomicI64}, device};
use gpu_device_columns::Bounds;

pub struct DenseFrame {
    domains: Vec<GroupKeyDomain>,
    bins: usize,
    aggregates: usize,
}

impl DenseFrame {
    pub(super) fn from_bounds(bounds: &[Bounds], aggregates: usize, geometry: Geometry) -> Option<Self> {
        let measured = |b: &Bounds| match *b {
            Bounds::Inclusive { min, max } if min <= max => Some(domain(min, max)),
            _ => None,
        };
        let bins = Self::bins(bounds.iter().map(|b| measured(b).map(|d| d.bound)), aggregates, geometry)?;
        Some(Self { domains: bounds.iter().filter_map(measured).collect(), bins, aggregates })
    }

    pub(super) fn new(domains: &[GroupKeyDomain], aggregates: usize, geometry: Geometry) -> Option<Self> {
        let bins = Self::bins(domains.iter().map(|d| Some(d.bound)), aggregates, geometry)?;
        Some(Self { domains: domains.to_vec(), bins, aggregates })
    }

    fn bins(mut ranges: impl ExactSizeIterator<Item=Option<u128>>, aggregates: usize, geometry: Geometry) -> Option<usize> {
        if ranges.len() == 0 { return None; }
        let limit = (geometry.shared_bytes as usize / size_of::<i64>()) / aggregates.checked_add(1)?;
        ranges.try_fold(1usize, |n, range| {
            let product = n.checked_mul(usize::try_from(range?).ok()?)?;
            (product != 0 && product <= limit).then_some(product)
        })
    }
}

fn domain(minimum: i64, maximum: i64) -> GroupKeyDomain {
    let range = maximum.wrapping_sub(minimum) as u64;
    GroupKeyDomain { minimum, bound: u128::from(range) + 1, bits: (64 - range.leading_zeros()).max(1) }
}

#[inline(always)]
#[device]
pub(super) fn keyed_identity(kind: u32) -> i64 {
    match kind { 2 => i64::MAX, 3 => i64::MIN, _ => 0 }
}

#[inline(always)]
#[device]
pub(super) unsafe fn keyed_combine(kind: u32, a: i64, b: i64, error: *mut u32) -> i64 {
    match kind {
        2 => a.min(b), 3 => a.max(b), _ => {
            let (sum, overflow) = a.overflowing_add(b);
            if overflow { unsafe { crate::device::flag_overflow(error) }; }
            sum
        }
    }
}

/// Combine a live argument into its block's aligned shared cell. The caller
/// initializes the monoid identity before any lane updates it and supplies a
/// stream-local sticky error word. Count is bounded by the u32 input extent.
#[inline(always)]
#[device]
pub unsafe fn dense_add(cell: *mut i64, kind: u32, x: i64, error: *mut u32) {
    let cell = unsafe { BlockAtomicI64::from_ptr(cell) };
    match kind {
        2 => { cell.fetch_min(x, AtomicOrdering::Relaxed); },
        3 => { cell.fetch_max(x, AtomicOrdering::Relaxed); },
        _ => {
            let old = cell.fetch_add(x, AtomicOrdering::Relaxed);
            if kind != 0 && old.checked_add(x).is_none() { unsafe { crate::device::flag_overflow(error) }; }
        }
    }
}

pub struct DenseFold {
    pool: Arc<Pool>,
    frame: DenseFrame,
    axes: Buffer<i64>, // (minimum, cardinality) per key, most significant first
    specs: Buffer<Aggregate>,
    partials: Buffer<i64>, // block-major [bin][occupancy, aggregate ...]
    metadata: Arc<Buffer<OutputCount>>,
    blocks: u32,
}

fn metadata(pool: &Arc<Pool>, groups: usize) -> Result<Arc<Buffer<OutputCount>>> {
    Ok(Arc::new(Buffer::from_host(pool.clone(), &[OutputCount { rows: groups as u64, error: 0, padding: 0 }])?))
}
fn error(meta: &Buffer<OutputCount>) -> *mut u32 { meta.ptr().cast::<u32>().wrapping_add(2) }

impl DenseFold {
    pub fn new(pool: Arc<Pool>, frame: DenseFrame, blocks: u32, aggregates: &[Aggregate]) -> Result<Self> {
        if frame.aggregates != aggregates.len() || blocks == 0 {
            return Err("inconsistent dense accumulator frame".into());
        }
        let stride = aggregates.len() + 1;
        let axes = frame.domains.iter().flat_map(|d| [d.minimum, d.bound as i64]).collect::<Vec<_>>();
        let cells = frame.bins.checked_mul(stride).and_then(|n| n.checked_mul(blocks as usize)).ok_or("dense partial extent overflow")?;
        Ok(Self {
            axes: Buffer::from_host(pool.clone(), &axes)?, specs: Buffer::from_host(pool.clone(), aggregates)?,
            partials: unsafe { Buffer::uninitialized(pool.clone(), cells)? },
            metadata: metadata(&pool, 0)?, pool, blocks, frame,
        })
    }
    pub fn axes(&self) -> *mut i64 { self.axes.ptr() }
    pub fn specs(&self) -> *mut Aggregate { self.specs.ptr() }
    pub fn bins(&self) -> usize { self.frame.bins }
    pub fn partials(&self) -> *mut i64 { self.partials.ptr() }
    pub fn error(&self) -> *mut u32 { error(&self.metadata) }

    /// # Safety
    /// All block histograms must have been completely written on this stream.
    pub unsafe fn finish(self, statuses: Vec<Arc<Buffer<OutputCount>>>, aux: &Auxiliary) -> Result<GroupedOutput> {
        let p = programs(&self.pool)?;
        let m = &p.module;
        let stream = self.pool.stream();
        let bins = self.bins();
        let keys = self.frame.domains.len();
        let width = keys + self.specs.len();
        let stride = self.specs.len() + 1;
        let sums = unsafe { Buffer::<i64>::uninitialized(self.pool.clone(), bins * stride)? };
        let occupied = Buffer::<u32>::zeroed(self.pool.clone(), bins.div_ceil(32))?;
        m.dense_combine(stream, &m.prepare_dense_combine(aux.group_reduce.config((bins * stride) as u32))?,
            self.partials(), self.specs(), self.blocks as usize, bins, stride, sums.ptr(), occupied.ptr(), self.error())?;
        // Only occupancy bits cross to the host for the exact allocation and
        // live extrema. The same device bitmap supplies lexicographic ordinals.
        let bits = occupied.to_host()?;
        let groups: usize = bits.iter().map(|w| w.count_ones() as usize).sum();
        let mut extrema = vec![(i64::MAX, i64::MIN); keys];
        for bin in 0..bins {
            if bits[bin/32] & (1 << (bin%32)) == 0 { continue; }
            let mut id = bin;
            for (d, (min, max)) in self.frame.domains.iter().zip(&mut extrema).rev() {
                let radix = d.bound as usize;
                let x = d.minimum + (id % radix) as i64;
                *min = (*min).min(x); *max = (*max).max(x);
                id /= radix;
            }
        }
        let values = unsafe { Buffer::<i64>::uninitialized(self.pool.clone(), groups * width)? };
        let valid = unsafe { Buffer::<u8>::uninitialized(self.pool.clone(), groups * width)? };
        m.dense_publish(stream, &m.prepare_dense_publish(aux.group.config(bins.div_ceil(256) as u32))?,
            sums.ptr(), occupied.ptr(), self.axes(), bins, keys, width, values.ptr(), valid.ptr(),
            self.metadata.ptr().cast::<u64>(), groups as u64, self.specs(), self.error())?;
        let key_domains = extrema.into_iter().map(|(min, max)| if groups != 0 { domain(min, max) } else {
            GroupKeyDomain { minimum: 0, bound: 0, bits: 0 }
        }).collect();
        Ok(output(values, valid, self.metadata, statuses, width, key_domains))
    }
}

fn output(values: Buffer<i64>, valid: Buffer<u8>, metadata: Arc<Buffer<OutputCount>>,
    statuses: Vec<Arc<Buffer<OutputCount>>>, width: usize, key_domains: Vec<GroupKeyDomain>) -> GroupedOutput {
    if !key_domains.is_empty() && crate::index_bytes::enabled() {
        let bytes = values.bytes() + valid.bytes() + metadata.bytes();
        crate::index_bytes::record("group", bytes, bytes);
    }
    GroupedOutput { values, valid, metadata, statuses, width, key_domains }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn segmented(pool: &Arc<Pool>, p: &GroupPrograms, input: &Buffer<i64>, order: *const u32,
    rows: usize, domains: Vec<GroupKeyDomain>, aggregates: &[Aggregate], statuses: Vec<Arc<Buffer<OutputCount>>>,
    aux: &Auxiliary) -> Result<GroupedOutput> {
    let keys = domains.len();
    let width = keys + aggregates.len();
    if rows == 0 {
        let groups = usize::from(keys == 0);
        let values = Buffer::zeroed(pool.clone(), groups * width)?;
        let valid = if groups == 0 { Buffer::zeroed(pool.clone(), 0)? } else {
            Buffer::from_host(pool.clone(), &aggregates.iter().map(|a| u8::from(a.kind < 2)).collect::<Vec<_>>())?
        };
        return Ok(output(values, valid, metadata(pool, groups)?, statuses, width, domains));
    }
    let m = &p.module;
    let stream = pool.stream();
    let warps = rows.div_ceil(32);
    let masks = unsafe { Buffer::<u32>::uninitialized(pool.clone(), warps)? };
    let mut ends = temporary::<u32>(pool, warps)?;
    let geom = aux.group;
    let grid = geom.grid(m.prepare_run_heads(geom.config(1))?.function(), rows)?;
    m.run_heads(stream, &m.prepare_run_heads(geom.config(grid))?, input.ptr(), order, rows, width, keys,
        masks.ptr(), pointer(&ends))?;
    let launch = p.cub.on(stream)?;
    let mut workspace = rccl_cub::device_scan_workspace_in::<_, u32>(pool.resource(), &launch, warps)?;
    rccl_cub::device_inclusive_sum_in_place(&launch, &mut ends, &mut workspace)?;
    let mut groups = 0u32;
    unsafe { cuda_core::simt::memory::memcpy_dtoh_async(&mut groups, ends.cu_deviceptr() + ((warps - 1) * 4) as u64,
        size_of::<u32>(), stream.cu_stream())?; }
    stream.synchronize()?;
    let groups = groups as usize;
    let meta = metadata(pool, groups)?;
    let specs = Buffer::from_host(pool.clone(), aggregates)?;
    let values = unsafe { Buffer::<i64>::uninitialized(pool.clone(), groups * width)? };
    let valid = unsafe { Buffer::<u8>::uninitialized(pool.clone(), groups * width)? };
    let carries = unsafe { Buffer::<i64>::uninitialized(pool.clone(), warps * aggregates.len())? };
    let grid = geom.grid(m.prepare_run_fragments(geom.config(1))?.function(), rows)?;
    m.run_fragments(stream, &m.prepare_run_fragments(geom.config(grid))?, input.ptr(), order, masks.ptr(),
        pointer(&ends), rows, keys, width, specs.ptr(), values.ptr(), valid.ptr(), carries.ptr(), error(&meta))?;
    if !aggregates.is_empty() {
        let grid = geom.grid(m.prepare_run_carries(geom.config(1))?.function(), warps)?;
        m.run_carries(stream, &m.prepare_run_carries(geom.config(grid))?, masks.ptr(), pointer(&ends),
            carries.ptr(), warps, keys, width, specs.ptr(), values.ptr(), error(&meta))?;
        if aggregates.iter().any(|a| a.min > i64::MIN || a.max < i64::MAX) {
            let grid = geom.grid(m.prepare_run_check(geom.config(1))?.function(), groups)?;
            m.run_check(stream, &m.prepare_run_check(geom.config(grid))?, groups, keys, width,
                specs.ptr(), values.ptr(), error(&meta))?;
        }
    }
    release(stream, ends)?;
    Ok(output(values, valid, meta, statuses, width, domains))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_frame_must_fit_including_occupancy_and_every_aggregate() {
        let geom = Auxiliary::SUPPORTED.group_reduce;
        let bins = |bounds: &[Bounds]| DenseFrame::from_bounds(bounds, 3, geom).map(|f| f.bins);
        assert_eq!(bins(&[Bounds::Inclusive { min: -64, max: 63 }]), Some(128));
        assert_eq!(bins(&[Bounds::Inclusive { min: -64, max: 64 }]), None);
        assert_eq!(bins(&[Bounds::Empty]), None);
        let full = Bounds::Inclusive { min: i64::MIN, max: i64::MAX };
        assert_eq!(bins(&[full, full]), None);
    }

    #[test]
    fn sparse_bins_and_warp_continuations_check_only_completed_carriers() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        for gap in [3, 65, 8192] {
            // The first key spans several warps and has out-of-carrier
            // partials but an in-carrier result. The other key is a singleton.
            let mut input = Vec::new();
            for i in 0..65 {
                let x = if i == 0 { 1000 } else if i == 64 { -999 } else { 0 };
                input.extend_from_slice(&[-10, x, x, x, 1]);
            }
            input.extend_from_slice(&[gap-10, -3, -3, -3, 1]);
            let mut specs = [
                Aggregate { kind: 1, min: -10, max: 10 },
                aggregate::<Option<i64>, true>(2)?, aggregate::<Option<i64>, true>(3)?,
                aggregate::<usize, false>(0)?,
            ];
            let run = |input: &[i64], specs: &[Aggregate], mask: Option<&Buffer<u32>>| -> Result<_> {
                let input = Buffer::from_host(pool.clone(), input)?;
                unsafe { SegmentedGroup::fold(pool.clone(), &input, mask, input.len()/5, &[true],
                    specs, Vec::new(), &Auxiliary::SUPPORTED) }
            };
            let rows = run(&input, &specs, None)?.with_decoder(|v: &[i64], p: &[u8]| -> Result<_> {
                Ok((v.to_vec(), p.to_vec()))
            }).to_host()?;
            assert_eq!(rows, [(vec![-10, 1, -999, 1000, 65], vec![1;5]),
                               (vec![gap-10, -3, -3, -3, 1], vec![1;5])]);
            specs[0].max = 0;
            assert!(run(&input, &specs, None)?.checked_len().is_err());
            specs[0] = aggregate::<i64, false>(1)?;
            input[1] = i64::MAX;
            input[64*5+1] = 1;
            assert!(run(&input, &specs, None)?.checked_len().is_err());
            let none = Buffer::<u32>::zeroed(pool.clone(), 66)?;
            assert_eq!(run(&input, &specs, Some(&none))?.checked_len()?, 0);
        }
        Ok(())
    }
}
