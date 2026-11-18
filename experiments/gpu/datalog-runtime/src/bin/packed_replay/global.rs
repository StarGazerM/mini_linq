use super::*;
use std::sync::Arc;

pub(super) fn frame_policy() -> Result<gpu_datalog_runtime::packed_run::MergeFrames> {
    use gpu_datalog_runtime::packed_run::MergeFrames;
    Ok(match std::env::var("PACKED_MERGE_FRAMES").as_deref().unwrap_or("metadata") {
        "metadata" => MergeFrames::Metadata,
        "exact" => MergeFrames::ExactCodes,
        "zero" => MergeFrames::ZeroPlain,
        "paths" => MergeFrames::LeafPaths,
        _ => return Err("PACKED_MERGE_FRAMES must be metadata, exact, zero or paths".into()),
    })
}

trait ReplayState {
    fn admit(&mut self, root: &Path, batch: &Batch) -> Result<()>;
    fn report(&self, run_reads: bool) -> Result<Value>;
}
struct Replay<'a, W: Word> {
    spec: &'a State,
    pool: Arc<Pool>,
    plain: DeviceRadixRun<W, u64, ()>,
    packed: Option<PackedSet<W>>,
    oracle: BTreeSet<W>,
    prefixes: BTreeSet<u64>,
    candidate_rows: usize,
    fresh_rows: usize,
    fresh_ns: u64,
    adopt_ns: u64,
    adopt_rows: usize,
    merge_ns: u64,
    merged_rows: usize,
    merges: usize,
    admissions: Vec<Value>,
    small_rows: usize,
    varying_sort: bool,
    sort_digits: u64,
    full_sort_digits: u64,
}
impl<'a, W: Word> Replay<'a, W> {
    fn new(spec: &'a State, pool: Arc<Pool>, compressed: bool, block: u32, plain_storage: bool, small_rows: usize, varying_sort: bool) -> Result<Self> {
        let packing = Packing::new(&spec.fields.iter().map(|f| f.bits).collect::<Vec<_>>())?;
        if spec.word_bytes != std::mem::size_of::<W>() || !packing.packed_fields().zip(&spec.fields).all(|(a, b)| a.shift == b.shift && a.bits == b.bits) { return Err("trace packing mismatch".into()); }
        let params = RadixRunParams { ratio: 4, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 };
        let plain = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), params, packing.bits())?;
        let packed = if compressed {
            let mut set = if plain_storage { PackedSet::new_plain(pool.clone(), packing, block, 4)? }
                else { PackedSet::new(pool.clone(), packing, block, 4)? };
            set.profile(true);
            set.merge_frames(frame_policy()?);
            Some(set)
        } else { None };
        Ok(Self { spec, pool, plain, packed, oracle: BTreeSet::new(), prefixes: BTreeSet::new(), candidate_rows: 0, fresh_rows: 0, fresh_ns: 0, adopt_ns: 0, adopt_rows: 0, merge_ns: 0, merged_rows: 0, merges: 0, admissions: Vec::new(), small_rows, varying_sort, sort_digits: 0, full_sort_digits: 0 })
    }
}
impl<W: Word> ReplayState for Replay<'_, W> {
    fn admit(&mut self, root: &Path, batch: &Batch) -> Result<()> {
        let _admission = cq_gpu_runtime::nvtx::range(c"admission");
        if self.spec.role == "canonical" && batch.candidates.is_none() { return Err("canonical batch missing candidates".into()); }
        let candidates = read_rows::<W>(root, batch.candidates.as_ref().unwrap_or(&batch.fresh))?;
        let expected = read_rows::<W>(root, &batch.fresh)?;
        let host_fresh: Vec<_> = candidates.iter().filter(|key| !self.oracle.contains(*key)).copied().collect();
        if host_fresh != expected { return Err(format!("host subtract invariant failed state={} seq={}", self.spec.id, batch.seq).into()); }
        if let Some(field) = self.spec.fields.first() {
            for key in &candidates { self.prefixes.insert(key.extract(field.shift, field.bits)); }
        }
        let input = Buffer::from_host(self.pool.clone(), &candidates)?;
        let previous = (self.fresh_ns, self.adopt_ns, self.merge_ns, self.merged_rows, self.merges);
        let before = self.packed.as_ref().map(|set| set.times()).unwrap_or_default();
        let previous_digits = (self.sort_digits, self.full_sort_digits);
        if let Some(packed) = &mut self.packed {
            let fresh = if candidates.len() <= self.small_rows {
                unsafe { packed.fresh_small(&input, candidates.len())? }
            } else {
                let full_digits = self.spec.fields.iter().map(|f| f.bits).sum::<u32>().div_ceil(8);
                let ((sorted, digits), elapsed) = measure(&self.pool, || unsafe {
                    let _sort = cq_gpu_runtime::nvtx::range(c"packed_sort");
                    if self.varying_sort { packed.sort_varying(&input, candidates.len()) }
                    else { Ok((self.plain.sort_unique(&input, candidates.len())?, full_digits)) }
                })?;
                self.sort_digits += digits as u64;
                self.full_sort_digits += full_digits as u64;
                self.fresh_ns += elapsed;
                unsafe { packed.subtract(sorted)? }
            };
            let actual = fresh.buffer().to_host_prefix(fresh.len())?;
            if actual != expected {
                let mismatch = actual.iter().zip(&expected).position(|(a, b)| a != b).unwrap_or(actual.len().min(expected.len()));
                return Err(format!("packed subtract invariant failed state={} seq={} rows={}/{} first_mismatch={} actual={:?} expected={:?}",
                    self.spec.id, batch.seq, actual.len(), expected.len(), mismatch, actual.get(mismatch), expected.get(mismatch)).into());
            }
            unsafe { packed.append_sorted(&fresh)?; }
        } else {
            let (fresh, elapsed) = measure(&self.pool, || unsafe {
                let _fresh = cq_gpu_runtime::nvtx::range(c"engine_sort_subtract");
                self.plain.fresh(&input, candidates.len())
            })?;
            self.fresh_ns += elapsed;
            if fresh.buffer().to_host_prefix(fresh.len())? != expected { return Err(format!("plain subtract invariant failed state={} seq={}", self.spec.id, batch.seq).into()); }
            let mut merged = expected.len();
            let mut merges = 0;
            if merged != 0 {
                for (len, _) in self.plain.run_lengths().into_iter().rev() {
                    if len / 4 >= merged { break; }
                    merged += len; self.merged_rows += merged; merges += 1;
                }
            }
            let (_, elapsed) = measure(&self.pool, || unsafe {
                let _publish = cq_gpu_runtime::nvtx::range(if merges == 0 { c"engine_adopt" } else { c"engine_merge_publication" });
                self.plain.append_sorted(fresh)
            })?;
            if merges == 0 { self.adopt_ns += elapsed; self.adopt_rows += expected.len(); } else { self.merge_ns += elapsed; }
            self.merges += merges;
        }
        let after = self.packed.as_ref().map(|set| set.times()).unwrap_or_default();
        self.admissions.push(json!({
            "seq": batch.seq, "candidate_rows": candidates.len(), "fresh_rows": expected.len(),
            "fresh_ns": self.fresh_ns - previous.0,
            "subtract_ns": ns(after.subtract - before.subtract),
            "seal_ns": ns(after.seal - before.seal) + self.adopt_ns - previous.1,
            "merge_ns": ns(after.merge - before.merge) + self.merge_ns - previous.2,
            "merged_rows": after.merged_rows - before.merged_rows + self.merged_rows - previous.3,
            "merges": after.merges - before.merges + self.merges - previous.4,
            "partition_gpu_ns": ns(after.partition_gpu - before.partition_gpu),
            "analyze_gpu_ns": ns(after.merge_analyze_gpu - before.merge_analyze_gpu),
            "pack_gpu_ns": ns(after.merge_pack_gpu - before.merge_pack_gpu),
            "fused_small_ns": ns(after.fused_small - before.fused_small),
            "sort_digits": self.sort_digits - previous_digits.0,
            "full_sort_digits": self.full_sort_digits - previous_digits.1
        }));
        self.candidate_rows += candidates.len(); self.fresh_rows += expected.len();
        self.oracle.extend(expected);
        Ok(())
    }
    fn report(&self, run_reads: bool) -> Result<Value> {
        let (bytes, runs, rows) = if let Some(packed) = &self.packed { (packed.allocated_bytes(), packed.runs().len(), packed.len()) } else { (self.plain.allocated_bytes(), self.plain.run_lengths().len(), self.plain.len()) };
        if self.oracle.len() != self.spec.final_rows || rows != self.spec.final_rows { return Err(format!("final row count mismatch state={}", self.spec.id).into()); }
        let expected: Vec<_> = self.oracle.iter().copied().collect();
        let actual = if let Some(packed) = &self.packed { packed.to_host()? } else { plain_keys(&self.plain)? };
        if actual != expected { return Err(format!("final contents mismatch state={}", self.spec.id).into()); }
        let mut result = json!({ "id": self.spec.id, "relation": self.spec.relation, "role": self.spec.role, "order": self.spec.order, "word_bytes": self.spec.word_bytes,
            "rows": self.spec.final_rows, "batches": self.spec.batches.len(), "candidate_rows": self.candidate_rows, "fresh_rows": self.fresh_rows });
        let map = result.as_object_mut().unwrap();
        let prefix = if self.packed.is_some() { "packed" } else { "plain" };
        if run_reads {
            let reads = reads::run(self.spec, &self.pool, &self.plain, self.packed.as_ref(), &self.prefixes, &expected)?;
            map.insert("lookup_distinct_prefixes".into(), json!(self.prefixes.len()));
            map.insert("lookup_rows".into(), reads["prefix_sorted"]["probes"].clone());
            map.insert(format!("{prefix}_lookup_ns"), reads["prefix_sorted"]["ns"].clone());
            map.insert(format!("{prefix}_reads"), reads);
        }
        map.insert(format!("{prefix}_admissions"), json!(self.admissions));
        let bytes = self.packed.as_ref().map_or(bytes, |packed| packed.allocated_bytes());
        map.insert(format!("{prefix}_bytes"), json!(bytes));
        map.insert(format!("{prefix}_runs"), json!(runs));
        map.insert(format!("{prefix}_bits_per_row"), json!(if rows == 0 { 0.0 } else { bytes as f64 * 8.0 / rows as f64 }));
        if let Some(packed) = &self.packed {
            map.insert("packed_payload_bytes".into(), json!(packed.runs().iter().map(|r| r.payload_bytes()).sum::<usize>()));
            map.insert("packed_header_bytes".into(), json!(packed.runs().iter().map(|r| r.header_bytes()).sum::<usize>()));
            map.insert("packed_fence_bytes".into(), json!(packed.runs().iter().map(|r| r.fence_bytes()).sum::<usize>()));
            map.insert("packed_tail_rows".into(), json!(packed.runs().iter().map(|r| r.len() % r.block_rows() as usize).sum::<usize>()));
            let mut widths = [0u64; 65];
            for run in packed.runs() {
                for (count, value) in widths.iter_mut().zip(run.frame_width_histogram()?) { *count += value; }
            }
            map.insert("packed_frame_width_histogram".into(), json!(widths.to_vec()));
            let t = packed.times();
            for (key, value) in [("packed_sort_ns", self.fresh_ns), ("packed_subtract_ns", ns(t.subtract)), ("packed_seal_ns", ns(t.seal)), ("packed_merge_ns", ns(t.merge)), ("packed_subtract_rows", t.subtract_rows as u64), ("packed_sealed_rows", t.sealed_rows as u64), ("packed_merged_rows", t.merged_rows as u64), ("packed_merges", t.merges as u64), ("packed_admission_ns", self.fresh_ns + ns(t.subtract + t.seal + t.merge))] { map.insert(key.into(), json!(value)); }
            map.insert("packed_leaf_merge_ns".into(), json!(ns(t.leaf_merge)));
            map.insert("packed_leaf_merges".into(), json!(t.leaf_merges));
            map.insert("packed_fused_small_ns".into(), json!(ns(t.fused_small)));
            map.insert("packed_sort_digits".into(), json!(self.sort_digits));
            map.insert("packed_full_sort_digits".into(), json!(self.full_sort_digits));
            map.insert("packed_partition_gpu_ns".into(), json!(ns(t.partition_gpu)));
            map.insert("packed_merge_analyze_gpu_ns".into(), json!(ns(t.merge_analyze_gpu)));
            map.insert("packed_merge_pack_gpu_ns".into(), json!(ns(t.merge_pack_gpu)));
        } else {
            for (key, value) in [("plain_fresh_ns", self.fresh_ns), ("plain_adopt_ns", self.adopt_ns), ("plain_merge_publication_ns", self.merge_ns), ("plain_merged_rows", self.merged_rows as u64), ("plain_merges", self.merges as u64), ("plain_admission_ns", self.fresh_ns + self.adopt_ns + self.merge_ns)] { map.insert(key.into(), json!(value)); }
            map.insert("plain_adopt_rows".into(), json!(self.adopt_rows));
        }
        Ok(result)
    }
}

pub(super) fn run(root: &Path, manifest: &Manifest, block: u32, state_filter: Option<usize>, compressed: bool, plain_storage: bool, small_rows: usize, varying_sort: bool, run_reads: bool) -> Result<Value> {
    let _configuration = cq_gpu_runtime::nvtx::range(if !compressed { c"engine" } else if plain_storage { c"new_plain" } else { c"new_packed" });
    let pool = Pool::new(0, 0, 0)?;
    let specs: Vec<_> = manifest.states.iter().filter(|state| state_filter.is_none_or(|id| id == state.id)).collect();
    let mut states: Vec<Box<dyn ReplayState + '_>> = Vec::new();
    for spec in &specs {
        let state: Box<dyn ReplayState> = match spec.word_bytes {
            8 => Box::new(Replay::<u64>::new(spec, pool.clone(), compressed, block, plain_storage, small_rows, varying_sort)?),
            16 => Box::new(Replay::<u128>::new(spec, pool.clone(), compressed, block, plain_storage, small_rows, varying_sort)?),
            24 => Box::new(Replay::<[u64; 3]>::new(spec, pool.clone(), compressed, block, plain_storage, small_rows, varying_sort)?),
            32 => Box::new(Replay::<[u64; 4]>::new(spec, pool.clone(), compressed, block, plain_storage, small_rows, varying_sort)?),
            width => return Err(format!("unsupported trace word_bytes={width}").into()),
        };
        states.push(state);
    }
    let mut jobs = Vec::new();
    for (state, spec) in specs.iter().enumerate() {
        for (batch, admission) in spec.batches.iter().enumerate() { jobs.push((admission.seq, state, batch)); }
    }
    jobs.sort_unstable();
    if !jobs.windows(2).all(|pair| pair[0].0 < pair[1].0) { return Err("duplicate global admission sequence".into()); }
    let before = tenant()?;
    for &(_, state, batch) in &jobs { states[state].admit(root, &specs[state].batches[batch])?; }
    pool.stream().synchronize()?;
    let with_scratch = pool.memory_stats();
    gpu_datalog_runtime::radix_run::release_set_scratch();
    gpu_datalog_runtime::packed_run::release_sort_scratch();
    pool.stream().synchronize()?;
    let mut reports = Vec::new();
    for state in &states { reports.push(state.report(run_reads)?); }
    let final_memory = pool.memory_stats();
    let after = tenant()?;
    // The pool must be empty before the other representation starts replaying.
    drop(states);
    gpu_datalog_runtime::radix_run::release_set_scratch();
    gpu_datalog_runtime::packed_run::release_sort_scratch();
    pool.stream().synchronize()?;
    if pool.memory_stats().current_bytes != 0 { return Err(format!("replay retained {} bytes after teardown", pool.memory_stats().current_bytes).into()); }
    Ok(json!({ "states": reports, "pool_live": final_memory.current_bytes, "pool_live_with_scratch": with_scratch.current_bytes,
        "merge_frames": std::env::var("PACKED_MERGE_FRAMES").unwrap_or_else(|_| "metadata".into()),
        "pool_peak": with_scratch.peak_bytes, "tenant_before": before, "tenant_after": after,
        "timing_indicative": foreign(&before) || foreign(&after) }))
}
