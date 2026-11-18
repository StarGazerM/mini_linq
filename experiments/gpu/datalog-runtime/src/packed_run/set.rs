use super::*;
use crate::{runs::{read_count, Native}, SortedBatch};
use rccl_cub::{DeviceSliceRead, DeviceSliceWrite};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default, Debug)]
pub struct OperationTimes {
    pub subtract: Duration,
    /// Subset of `subtract`: fused sorting, deduplication and subtraction.
    pub fused_small: Duration,
    pub seal: Duration,
    pub merge: Duration,
    /// Subset of merge wall time which published packed rather than plain storage.
    pub leaf_merge: Duration,
    pub leaf_merges: usize,
    /// CUDA-event stream intervals; unlike the fields above, not host wall time.
    pub partition_gpu: Duration,
    pub merge_analyze_gpu: Duration,
    pub merge_pack_gpu: Duration,
    pub subtractions: usize,
    pub seals: usize,
    pub merges: usize,
    pub subtract_rows: usize,
    pub sealed_rows: usize,
    pub merged_rows: usize,
}

/// Geometrically maintained immutable packed runs. Admissions are transactional:
/// existing runs survive a failed seal/merge and returned fresh batches stay valid.
pub struct PackedSet<W: Word> {
    pub(super) pool: Arc<Pool>,
    pub(super) modules: Arc<Modules>,
    packing: Packing,
    block_rows: u32,
    ratio: usize,
    plain_limit: usize,
    merge_frames: super::MergeFrames,
    pub(super) runs: Vec<Arc<PackedRun<W>>>,
    rows: usize,
    pub(super) profile: bool,
    pub(super) times: OperationTimes,
    merge_events: Vec<merge::MergeEvents>,
    // Populated only by readers; admission never stages descriptors for this cache.
    read_views: std::sync::Mutex<Option<Arc<Buffer<PackedView<W>>>>>,
}
impl<W: Word> PackedSet<W> {
    pub fn new(pool: Arc<Pool>, packing: Packing, block_rows: u32, ratio: usize) -> Result<Self> {
        if ratio < 2 || block_rows == 0 || block_rows > 4096 || block_rows as usize * std::mem::size_of::<W>() > 48 * 1024 || packing.bits() > W::BITS {
            return Err("invalid packed set dimensions".into());
        }
        Ok(Self { modules: Modules::load(&pool)?, pool, packing, block_rows, ratio, plain_limit: SMALL_RUN_ROWS, merge_frames: super::MergeFrames::Metadata, runs: Vec::new(), rows: 0, profile: false, times: OperationTimes::default(), merge_events: Vec::new(), read_views: std::sync::Mutex::new(None) })
    }
    /// Control configuration: identical admission and merge code, no packing.
    pub fn new_plain(pool: Arc<Pool>, packing: Packing, block_rows: u32, ratio: usize) -> Result<Self> {
        let mut set = Self::new(pool, packing, block_rows, ratio)?;
        set.plain_limit = usize::MAX;
        Ok(set)
    }
    /// Synchronize operation boundaries and collect end-to-end wall times.
    pub fn profile(&mut self, enabled: bool) { self.profile = enabled; }
    pub fn times(&self) -> OperationTimes { self.times }
    /// Diagnostic alternatives retain the same format, cutoff and merge schedule.
    pub fn merge_frames(&mut self, policy: super::MergeFrames) { self.merge_frames = policy; }
    /// Untimed reader ablation for a single merged replay run.
    ///
    /// # Safety
    /// Input contains exactly the set's sorted unique keys.
    pub unsafe fn replay_reframe(&mut self, input: &Buffer<W>, fixed: bool, inherit: bool) -> Result<()> {
        if self.runs.len() != 1 { return Err("reader ablation requires one merged run".into()); }
        let replacement = self.runs[0].replay_reframe(input, fixed, inherit)?;
        self.read_views.get_mut().expect("packed view cache").take();
        self.runs[0] = Arc::new(replacement);
        Ok(())
    }
    /// Adopt a reordered run in a newly constructed provider. The bijection
    /// preserves disjointness; its requested ratio can differ from the source.
    pub(super) unsafe fn append_reordered(&mut self, mut run: Arc<PackedRun<W>>) -> Result<()> {
        let rows = run.len();
        let mut keep = self.runs.len();
        while keep > 0 && self.runs[keep - 1].len() / self.ratio < run.len() {
            run = Arc::new(self.runs[keep - 1].merge(&run)?);
            keep -= 1;
        }
        self.runs.truncate(keep);
        self.runs.push(run);
        self.rows += rows;
        Ok(())
    }
    pub fn len(&self) -> usize { self.rows }
    pub fn is_empty(&self) -> bool { self.rows == 0 }
    pub fn runs(&self) -> &[Arc<PackedRun<W>>] { &self.runs }
    pub fn allocated_bytes(&self) -> usize {
        self.runs.iter().map(|run| run.allocated_bytes()).sum::<usize>()
            + self.read_views.lock().expect("packed view cache").as_ref().map_or(0, |views| views.allocated_bytes())
    }
    pub(crate) fn read_views(&self) -> Result<Arc<Buffer<PackedView<W>>>> {
        let mut cache = self.read_views.lock().expect("packed view cache");
        if cache.is_none() {
            crate::phases::admission_scope!(c"admit/view-upload");
            let views: Vec<_> = self.runs.iter().map(|run| run.view()).collect();
            *cache = Some(Arc::new(Buffer::from_host_async(self.pool.clone(), &views)?));
        }
        Ok(cache.as_ref().unwrap().clone())
    }
    pub fn absent(&self, candidates: &Buffer<W>, rows: usize) -> Result<Buffer<u32>> {
        membership_with_module(&self.runs.iter().map(|run| run.as_ref()).collect::<Vec<_>>(), candidates, rows, true, &self.modules.codec)
    }
    /// # Safety
    /// Candidates are an initialized sorted unique prefix on this pool's stream.
    pub unsafe fn subtract(&mut self, candidates: SortedBatch<W>) -> Result<SortedBatch<W>> {
        let _subtract = cq_gpu_runtime::nvtx::range(c"packed_subtract");
        let rows = candidates.len();
        if self.profile { self.pool.stream().synchronize()?; }
        let start = Instant::now();
        let batch = self.subtract_batch(candidates, None)?;
        self.times.subtract_rows += rows;
        if self.profile { self.pool.stream().synchronize()?; self.times.subtract += start.elapsed(); }
        self.times.subtractions += 1;
        Ok(batch)
    }

    /// `resident`, when supplied, describes this exact run generation and is
    /// ordered and retained on the set stream through the selection below.
    pub(crate) unsafe fn subtract_batch(&self, candidates: SortedBatch<W>, resident: Option<*const PackedView<W>>) -> Result<SortedBatch<W>> {
        let rows = candidates.len();
        crate::phases::admission_scope!(c"admit/packed-subtract", rows);
        if !self.pool.shares_memory(candidates.buffer().pool()) {
            return Err("packed admission input pool mismatch".into());
        }
        let batch = if rows == 0 || self.runs.is_empty() { candidates } else {
            let output = Buffer::<W>::uninitialized(self.pool.clone(), rows)?;
            let flags = if let Some(views) = resident {
                let flags = Buffer::uninitialized(self.pool.clone(), rows)?;
                launch_membership(&self.modules.codec, &self.pool, views, self.runs.len(),
                    candidates.buffer(), rows, true, &flags)?;
                flags
            } else {
                self.absent(candidates.buffer(), rows)?
            };
            let cub = rccl_cub::cuda::Program::load(self.pool.context())?;
            let launch = cub.on(self.pool.stream())?;
            let mut count = Native::<u64>::new(&self.pool, 1)?;
            let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(self.pool.resource(), &launch, rows)?;
            rccl_cub::thrust_integration::device_select_flagged(&launch,
                DeviceSliceRead::new(candidates.buffer().ptr()), DeviceSliceRead::new(flags.ptr()),
                DeviceSliceWrite::new(output.ptr()), count.get_mut(), rows, &mut workspace)?;
            let fresh = read_count(&self.pool, count.get())?;
            SortedBatch::from_sorted(output, fresh)?
        };
        Ok(batch)
    }
    /// # Safety
    /// Fresh is sorted, unique and disjoint from every existing run, and uses
    /// this set's packing with zero unused bits.
    pub unsafe fn append_sorted(&mut self, fresh: &SortedBatch<W>) -> Result<()> {
        self.append_with_merge_limit(fresh, self.plain_limit)
    }

    /// Maintained engine tails churn; keep a small coalesced result in words
    /// when both inputs are already plain. Never unpack a leaf to do so.
    pub(crate) unsafe fn append_maintained(&mut self, fresh: &SortedBatch<W>) -> Result<()> {
        let limit = self.plain_limit.max((8 << 20) / size_of::<W>());
        self.append_with_merge_limit(fresh, limit)
    }

    unsafe fn append_with_merge_limit(&mut self, fresh: &SortedBatch<W>, merge_limit: usize) -> Result<()> {
        crate::phases::admission_scope!(c"admit/packed-append", fresh.len());
        if !self.pool.shares_memory(fresh.buffer().pool()) { return Err("packed append pool mismatch".into()); }
        if fresh.is_empty() { return Ok(()); }
        let seal_range = cq_gpu_runtime::nvtx::range(c"packed_seal_adopt");
        if self.profile { self.pool.stream().synchronize()?; }
        let start = Instant::now();
        // A batch consumed by this admission's first merge is already a valid
        // plain merge input. Packing it first only creates disposable payload.
        let merges_immediately = self.runs.last().is_some_and(|run| run.len() / self.ratio < fresh.len());
        let mut merged = Arc::new(if fresh.len() <= self.plain_limit || merges_immediately {
            let batch = if !merges_immediately && fresh.allocated_bytes() > fresh.live_bytes() {
                // Only a retained plain run needs its spare candidate capacity
                // trimmed. Transient merge inputs keep the immutable prefix.
                let output = Buffer::uninitialized(self.pool.clone(), fresh.len())?;
                cuda_core::simt::memory::memcpy_dtod_async(output.cu_deviceptr(),
                    fresh.buffer().cu_deviceptr(), fresh.live_bytes(), self.pool.stream().cu_stream())?;
                SortedBatch::from_sorted(output, fresh.len())?
            } else { fresh.clone() };
            PackedRun::from_plain(batch, self.packing.clone(), self.block_rows, self.modules.clone())?
        } else {
            PackedRun::seal_with_modules(fresh.buffer(), fresh.len(), &self.packing, self.block_rows, self.modules.clone())?
        });
        if self.profile { self.pool.stream().synchronize()?; self.times.seal += start.elapsed(); }
        self.times.seals += 1;
        let mut keep = self.runs.len();
        self.times.sealed_rows += fresh.len();
        drop(seal_range);
        let mut pending = 0;
        let mut group_start = None;
        while keep > 0 && self.runs[keep - 1].len() / self.ratio < merged.len() {
            let events = if self.profile {
                if pending == self.merge_events.len() { self.merge_events.push(merge::MergeEvents::new(&self.pool)?); }
                group_start.get_or_insert_with(Instant::now);
                Some(&self.merge_events[pending])
            } else { None };
            merged = Arc::new(self.runs[keep - 1].merge_with_events(&merged, events, merge_limit, self.merge_frames)?);
            self.times.merges += 1;
            keep -= 1;
            self.times.merged_rows += merged.len();
            if self.profile {
                pending += 1;
                // A plain intermediate is consumed only on this stream. A
                // distinct pair of events preserves each device measurement.
                if merged.plain.is_some() && keep > 0 {
                    let next = &self.runs[keep - 1];
                    if next.len() / self.ratio < merged.len() && next.plain.is_some()
                        && next.len() <= merge_limit - merged.len() { continue; }
                }
                self.pool.stream().synchronize()?;
                let elapsed = group_start.take().unwrap().elapsed();
                self.times.merge += elapsed;
                if merged.plain.is_none() {
                    self.times.leaf_merge += elapsed;
                    self.times.leaf_merges += 1;
                }
                for e in &self.merge_events[..pending] {
                    let (partition, analyze, pack) = e.elapsed(merged.plain.is_some())?;
                    self.times.partition_gpu += partition;
                    self.times.merge_analyze_gpu += analyze;
                    self.times.merge_pack_gpu += pack;
                }
                pending = 0;
            }
        }
        let publish = (self.profile && keep < self.runs.len()).then(Instant::now);
        let packed = merged.plain.is_none();
        self.read_views.get_mut().expect("packed view cache").take();
        self.runs.truncate(keep);
        self.runs.push(merged);
        self.rows += fresh.len();
        if let Some(start) = publish {
            let elapsed = start.elapsed();
            self.times.merge += elapsed;
            if packed { self.times.leaf_merge += elapsed; }
        }
        Ok(())
    }
    /// # Safety
    /// Candidates obey the sorted unique prefix and packing invariants of `seal`.
    pub unsafe fn admit(&mut self, candidates: SortedBatch<W>) -> Result<SortedBatch<W>> {
        let fresh = self.subtract(candidates)?;
        self.append_sorted(&fresh)?;
        Ok(fresh)
    }
    /// Explicit diagnostic download, never used by admission or merge.
    // DECODE P: diagnostic set download publishes exported run keys to host verification outside admission and merge.
    pub fn to_host(&self) -> Result<Vec<W>> {
        let mut output = Vec::with_capacity(self.rows);
        for run in &self.runs { output.extend(run.unpack()?.to_host()?); }
        output.sort_unstable();
        Ok(output)
    }
}
