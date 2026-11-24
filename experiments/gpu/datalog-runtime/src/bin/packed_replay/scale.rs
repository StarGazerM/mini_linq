use super::*;
use gpu_datalog_runtime::SortedBatch;

pub(super) fn run(root: &Path, spec: &State, rows: usize, block: u32, merge_delta: Option<usize>) -> Result<Value> {
    match spec.word_bytes {
        8 => run_word::<u64>(root, spec, rows, block, merge_delta),
        16 => run_word::<u128>(root, spec, rows, block, merge_delta),
        24 => run_word::<[u64; 3]>(root, spec, rows, block, merge_delta),
        32 => run_word::<[u64; 4]>(root, spec, rows, block, merge_delta),
        _ => Err("unsupported scale word width".into()),
    }
}

fn run_word<W: Word>(root: &Path, spec: &State, rows: usize, block: u32, merge_delta: Option<usize>) -> Result<Value> {
    if rows == 0 || rows > 1 << 28 { return Err("scale rows must be in 1..=2^28".into()); }
    let field = spec.fields.first().ok_or("scale requires a leading field")?;
    let mut source = Vec::with_capacity(spec.final_rows);
    for batch in &spec.batches { source.extend(read_rows::<W>(root, &batch.fresh)?); }
    source.sort_unstable();
    if source.len() != spec.final_rows || source.is_empty() || !source.windows(2).all(|w| w[0] < w[1]) { return Err("scale source must be sorted unique".into()); }
    let first = source[0].extract(field.shift, field.bits);
    let last = source[source.len() - 1].extract(field.shift, field.bits);
    let span = last.checked_sub(first).and_then(|v| v.checked_add(1)).ok_or("scale prefix span overflow")?;
    let maximum = u64::MAX >> (64 - field.bits);
    let replicas = rows.div_ceil(source.len());
    if (replicas as u64 - 1).checked_mul(span).and_then(|n| n.checked_add(last)).is_none_or(|n| n > maximum) { return Err("scale offsets exceed the original leading field width".into()); }
    let mut expected = Vec::with_capacity(rows);
    let mut prefixes = BTreeSet::new();
    for replica in 0..replicas {
        for &original in source.iter().take(rows - expected.len()) {
            let mut word = original;
            // Word::insert ORs bits; clear the original leading field first.
            for lane in 0..W::BITS / 64 {
                let bits = field.shift.saturating_sub(lane * 64).min(64);
                word.set_lane(lane, if bits == 0 { 0 } else { original.lane(lane) & (u64::MAX >> (64 - bits)) });
            }
            let prefix = original.extract(field.shift, field.bits) + replica as u64 * span;
            word.insert(field.shift, field.bits, prefix);
            prefixes.insert(prefix);
            expected.push(word);
        }
    }
    if !expected.windows(2).all(|w| w[0] < w[1]) { return Err("scaled keys are not sorted unique".into()); }
    drop(source);
    if let Some(denominator) = merge_delta {
        return super::merge_scale::run(spec, &expected, block, denominator);
    }
    let before = tenant()?;
    let mut configurations = serde_json::Map::new();
    for name in ["engine", "new_plain", "new_packed"] {
        let _configuration = cq_gpu_runtime::nvtx::range(match name {
            "engine" => c"engine",
            "new_plain" => c"new_plain",
            _ => c"new_packed",
        });
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&spec.fields.iter().map(|f| f.bits).collect::<Vec<_>>())?;
        let params = RadixRunParams { ratio: 4, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 };
        let mut engine = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), params, packing.bits())?;
        let mut packed = match name {
            "new_plain" => Some(PackedSet::new_plain(pool.clone(), packing, block, 4)?),
            "new_packed" => Some(PackedSet::new(pool.clone(), packing, block, 4)?),
            _ => None,
        };
        if name == "new_packed" && std::env::var_os("PACKED_READ_LEAVES").is_some() {
            for side in 0..2 {
                let keys: Vec<_> = expected.iter().enumerate().filter_map(|(i, &key)|
                    (((i % 3 == 2) as usize) == side).then_some(key)).collect();
                let batch = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &keys)?, keys.len())? };
                unsafe { packed.as_mut().unwrap().append_sorted(&batch)?; }
            }
            if let Some((fixed, inherit)) = super::read_ablation()? {
                let input = Buffer::from_host(pool.clone(), &expected)?;
                unsafe { packed.as_mut().unwrap().replay_reframe(&input, fixed, inherit)?; }
            }
        } else {
            let batch = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &expected)?, rows)? };
            if let Some(set) = &mut packed { unsafe { set.append_sorted(&batch)?; } drop(batch); }
            else { unsafe { engine.append_sorted(batch)?; } }
        }
        pool.stream().synchronize()?;
        let workloads = reads::run(spec, &pool, &engine, packed.as_ref(), &prefixes, &expected)?;
        let bytes = packed.as_ref().map_or_else(|| engine.allocated_bytes(), |set| set.allocated_bytes());
        let memory = pool.memory_stats();
        if memory.peak_bytes >= 12 * 1024 * 1024 * 1024 { return Err("scale exceeded 12 GiB pool budget".into()); }
        configurations.insert(name.into(), json!({"reads": workloads, "bytes": bytes, "pool_live": memory.current_bytes, "pool_peak": memory.peak_bytes}));
        drop(packed); drop(engine);
        pool.stream().synchronize()?;
        if pool.memory_stats().current_bytes != 0 { return Err("scale retained device memory".into()); }
    }
    let after = tenant()?;
    Ok(json!({"state": spec.id, "rows": rows, "source_rows": spec.final_rows, "word_bytes": spec.word_bytes,
        "replicas": replicas, "prefix_offset": span, "prefixes": prefixes.len(), "block_rows": block,
        "configurations": configurations, "tenant_before": before, "tenant_after": after,
        "timing_indicative": foreign(&before) || foreign(&after),
        "protocol": "One sorted run per configuration, same process, original field widths, repeated real keys with disjoint leading-field offsets; final replica truncated. All prefix ranges and scanned checksums checked against generated host keys. Input upload and construction excluded."}))
}
