//! Exact trace replay; operation timers include launches and end synchronization.
use gpu_datalog_runtime::{packed_run::{PackedSet, replay_support::{packed_ranges, plain_ranges}}, Buffer, DeviceRadixRun, Packing, Pool, RadixRunParams, Result, Word};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::{Path, PathBuf}, process::Command, time::{Duration, Instant}};

#[derive(Deserialize, Serialize)]
struct Manifest { source: String, states: Vec<State> }
#[derive(Deserialize, Serialize)]
struct State { id: usize, relation: String, role: String, order: Vec<usize>, word_bytes: usize, fields: Vec<Field>, final_rows: usize, batches: Vec<Batch> }
#[derive(Deserialize, Serialize)]
struct Field { shift: u32, bits: u32 }
#[derive(Deserialize, Serialize)]
struct Batch { seq: usize, stratum: usize, phase: String, round: usize, candidates: Option<FileRows>, fresh: FileRows }
#[derive(Deserialize, Serialize)]
struct FileRows { file: String, rows: usize }

fn read_ablation() -> Result<Option<(bool, bool)>> {
    match std::env::var("PACKED_READ_ABLATION").as_deref() {
        Err(_) => Ok(None),
        Ok("geometry") => Ok(Some((false, false))),
        Ok("frames") => Ok(Some((true, true))),
        _ => Err("PACKED_READ_ABLATION must be geometry or frames".into()),
    }
}

fn tenant() -> Result<String> {
    let out = Command::new("nvidia-smi").args(["--query-compute-apps=pid,process_name", "--format=csv,noheader"]).output()?;
    if !out.status.success() { return Err("nvidia-smi tenant query failed".into()); }
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}
fn foreign(tenants: &str) -> bool {
    tenants.lines().any(|line| line.split(',').next().and_then(|pid| pid.trim().parse::<u32>().ok()) != Some(std::process::id()))
}
fn read_rows<W: Word>(root: &Path, file: &FileRows) -> Result<Vec<W>> {
    let path = root.join(&file.file);
    let bytes = if path.extension().and_then(|s| s.to_str()) == Some("zst") {
        let out = Command::new("zstd").args(["-dc"]).arg(&path).output()?;
        if !out.status.success() { return Err(format!("zstd failed: {}", path.display()).into()); }
        out.stdout
    } else { fs::read(path)? };
    if bytes.len() != file.rows.checked_mul(std::mem::size_of::<W>()).ok_or("row byte overflow")? { return Err(format!("trace length mismatch: {}", file.file).into()); }
    let lanes = (W::BITS / 64) as usize;
    let mut rows = Vec::with_capacity(file.rows);
    for bytes in bytes.chunks_exact(lanes * 8) {
        let mut word = W::zero();
        for lane in 0..lanes {
            let value = u64::from_le_bytes(bytes[lane * 8..lane * 8 + 8].try_into()?);
            word.set_lane(if W::MS_FIRST { lanes - 1 - lane } else { lane } as u32, value);
        }
        rows.push(word);
    }
    if !rows.windows(2).all(|pair| pair[0] < pair[1]) { return Err(format!("trace is not sorted unique: {}", file.file).into()); }
    Ok(rows)
}
fn ns(duration: Duration) -> u64 { duration.as_nanos() as u64 }
fn measure<T>(pool: &Pool, f: impl FnOnce() -> Result<T>) -> Result<(T, u64)> {
    pool.stream().synchronize()?;
    let start = Instant::now();
    let output = f()?;
    pool.stream().synchronize()?;
    Ok((output, ns(start.elapsed())))
}
fn plain_keys<W: Word>(state: &DeviceRadixRun<W, u64, ()>) -> Result<Vec<W>> {
    let mut rows: Vec<W> = state.to_host()?.into_iter().flat_map(|run| run.keys).collect();
    rows.sort_unstable();
    Ok(rows)
}

#[path = "packed_replay/global.rs"]
mod global;
#[path = "packed_replay/reads.rs"]
mod reads;
#[path = "packed_replay/scale.rs"]
mod scale;
#[path = "packed_replay/real.rs"]
mod real;
#[path = "packed_replay/merge_scale.rs"]
mod merge_scale;

fn write_rows<W: Word>(root: &Path, name: &str, rows: &[W]) -> Result<FileRows> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(rows));
    for word in rows {
        for physical in 0..W::BITS / 64 {
            let lane = if W::MS_FIRST { W::BITS / 64 - 1 - physical } else { physical };
            bytes.extend(word.lane(lane).to_le_bytes());
        }
    }
    fs::write(root.join(name), bytes)?;
    Ok(FileRows { file: name.to_string(), rows: rows.len() })
}
fn synthetic_state<W: Word>(root: &Path, id: usize) -> Result<State> {
    let widths = vec![32; (W::BITS / 32) as usize];
    let packing = Packing::new(&widths)?;
    let fields: Vec<_> = packing.packed_fields().map(|f| Field { shift: f.shift, bits: f.bits }).collect();
    let mut oracle = BTreeSet::new();
    let mut batches = Vec::new();
    for step in 0..6 {
        let candidates: Vec<W> = (step * 127..step * 127 + 2050).map(|i| {
            let mut word = W::zero();
            for (column, field) in fields.iter().enumerate() { word.insert(field.shift, field.bits, if column + 1 == fields.len() { i as u64 } else { (i / 37) as u64 }); }
            word
        }).collect();
        let fresh: Vec<_> = candidates.iter().filter(|key| !oracle.contains(*key)).copied().collect();
        let candidate_file = write_rows(root, &format!("s{id}-b{step}-c.bin"), &candidates)?;
        let fresh_file = write_rows(root, &format!("s{id}-b{step}-f.bin"), &fresh)?;
        oracle.extend(fresh);
        batches.push(Batch { seq: step * 4 + id, stratum: 0, phase: if step == 0 { "seed" } else { "step" }.into(), round: step, candidates: Some(candidate_file), fresh: fresh_file });
    }
    Ok(State { id, relation: format!("synthetic_{}", W::BITS), role: "canonical".into(), order: (0..fields.len()).collect(), word_bytes: std::mem::size_of::<W>(), fields, final_rows: oracle.len(), batches })
}
fn synthetic(root: &Path) -> Result<()> {
    if root.join("manifest.json").exists() { return Err("synthetic destination already has a manifest".into()); }
    fs::create_dir_all(root)?;
    let manifest = Manifest { source: "synthetic deterministic overlapping admissions".into(), states: vec![
        synthetic_state::<u64>(root, 0)?, synthetic_state::<u128>(root, 1)?, synthetic_state::<[u64; 3]>(root, 2)?, synthetic_state::<[u64; 4]>(root, 3)?] };
    fs::write(root.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut root = PathBuf::from("/dev/shm/packed-run/trace");
    let mut output = None;
    let mut create = false;
    let mut block = gpu_datalog_runtime::packed_run::DEFAULT_BLOCK_ROWS;
    let mut state_filter = None;
    let mut plain_storage = false;
    let mut small_rows = gpu_datalog_runtime::packed_run::MAX_SMALL_CANDIDATES;
    let mut varying_sort = true;
    let mut scale_rows = None;
    let mut merge_delta = None;
    let mut real_trace = None;
    let mut real_repeats = 1usize;
    let mut real_site = None;
    let mut real_launch = None;
    let mut real_reads_only = false;
    let mut admission_only = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--synthetic" => create = true,
            "--plain-storage" => plain_storage = true,
            "--small-admission" => small_rows = args.next().ok_or("missing small admission threshold")?.parse()?,
            "--generic-sort" => varying_sort = false,
            "--output" => output = Some(PathBuf::from(args.next().ok_or("missing output path")?)),
            "--block" => block = args.next().ok_or("missing block rows")?.parse()?,
            "--state" => state_filter = Some(args.next().ok_or("missing state id")?.parse::<usize>()?),
            "--scale-rows" => scale_rows = Some(args.next().ok_or("missing scale rows")?.parse::<usize>()?),
            "--merge-delta" => merge_delta = Some(args.next().ok_or("missing merge delta denominator")?.parse::<usize>()?),
            "--real-trace" => real_trace = Some(PathBuf::from(args.next().ok_or("missing real trace directory")?)),
            "--real-repeats" => real_repeats = args.next().ok_or("missing real replay repetitions")?.parse()?,
            "--real-site" => real_site = Some(args.next().ok_or("missing real site selector")?),
            "--real-launch" => real_launch = Some(args.next().ok_or("missing real launch id")?.parse::<usize>()?),
            "--real-reads-only" => real_reads_only = true,
            "--admission-only" => admission_only = true,
            "--help" => { println!("packed-replay [TRACE_DIR] [--synthetic] [--output JSON] [--block ROWS] [--state ID] [--scale-rows ROWS] [--merge-delta 16|2] [--plain-storage] [--small-admission ROWS] [--generic-sort] [--admission-only]\n  Real replay: ADMISSION_DIR --real-trace JOINT_DIR [--real-repeats N] [--real-site KERNEL/SITE] [--real-launch ID] [--real-reads-only]\n  Frame diagnostics: PACKED_MERGE_FRAMES=metadata|exact|zero|paths (paths is not a timing sample)\n  Frame variant: PACKED_LEAF_ALIGNED=1; merged reader geometry: PACKED_READ_LEAVES=1"); return Ok(()); }
            option if option.starts_with('-') => return Err(format!("unknown option {option}").into()),
            path => root = PathBuf::from(path),
        }
    }
    if small_rows > gpu_datalog_runtime::packed_run::MAX_SMALL_CANDIDATES { return Err("small admission threshold exceeds shared-memory limit".into()); }
    if admission_only {
        let manifest: Manifest = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
        let result = json!({
            "engine": global::run(&root, &manifest, block, state_filter, false, false, 0, false, false)?,
            "plain": global::run(&root, &manifest, block, state_filter, true, true, small_rows, varying_sort, false)?,
            "packed": global::run(&root, &manifest, block, state_filter, true, false, small_rows, varying_sort, false)?
        });
        if let Some(path) = output { fs::write(path, serde_json::to_vec_pretty(&result)?)?; }
        else { println!("{}", serde_json::to_string(&result)?); }
        return Ok(());
    }
    if let Some(trace) = real_trace {
        if real_repeats == 0 { return Err("real replay repetitions must be positive".into()); }
        let mut result = real::run(&trace, real_repeats, real_site.as_deref(), real_launch)?;
        if !real_reads_only {
            let manifest: Manifest = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
            result["admission"] = json!({
                "engine": global::run(&root, &manifest, block, None, false, false, 0, false, false)?,
                "plain": global::run(&root, &manifest, block, None, true, true, small_rows, varying_sort, false)?,
                "packed": global::run(&root, &manifest, block, None, true, false, small_rows, varying_sort, false)?
            });
        }
        if let Some(path) = output { fs::write(path, serde_json::to_vec_pretty(&result)?)?; }
        else { println!("{}", serde_json::to_string(&result)?); }
        return Ok(());
    }
    if create { synthetic(&root)?; }
    let manifest: Manifest = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    if let Some(rows) = scale_rows {
        let spec = manifest.states.iter().find(|s| Some(s.id) == state_filter).ok_or("--scale-rows requires a valid --state")?;
        let result = scale::run(&root, spec, rows, block, merge_delta)?;
        println!("SCALE {result}");
        if let Some(path) = output { fs::write(path, serde_json::to_vec_pretty(&result)?)?; }
        return Ok(());
    }
    let plain = global::run(&root, &manifest, block, state_filter, false, false, 0, false, true)?;
    let packed = global::run(&root, &manifest, block, state_filter, true, plain_storage, small_rows, varying_sort, true)?;
    let mut reports = Vec::new();
    println!("id,relation,role,rows,plain_bytes,packed_bytes,plain_admission_ns,packed_admission_ns");
    for (a, b) in plain["states"].as_array().unwrap().iter().zip(packed["states"].as_array().unwrap()) {
        if a["id"] != b["id"] { return Err("replay state ordering mismatch".into()); }
        let mut report = a.clone();
        report.as_object_mut().unwrap().extend(b.as_object().unwrap().clone());
        println!("{},{},{},{},{},{},{},{}", report["id"], report["relation"], report["role"], report["rows"], report["plain_bytes"], report["packed_bytes"], report["plain_admission_ns"], report["packed_admission_ns"]);
        reports.push(report);
    }
    let mut totals = serde_json::Map::new();
    for name in ["rows", "batches", "candidate_rows", "fresh_rows", "plain_bytes", "packed_bytes", "plain_runs", "packed_runs", "plain_fresh_ns", "plain_adopt_ns", "plain_merge_publication_ns", "plain_merged_rows", "plain_merges", "packed_sort_ns", "packed_subtract_ns", "packed_seal_ns", "packed_merge_ns", "packed_subtract_rows", "packed_sealed_rows", "packed_merged_rows", "packed_merges", "plain_admission_ns", "packed_admission_ns", "lookup_distinct_prefixes", "lookup_rows", "plain_lookup_ns", "packed_lookup_ns"] {
        totals.insert(name.into(), json!(reports.iter().map(|r| r[name].as_u64().unwrap()).sum::<u64>()));
    }
    totals.insert("plain_adopt_rows".into(), json!(reports.iter().map(|r| r["plain_adopt_rows"].as_u64().unwrap()).sum::<u64>()));
    for name in ["packed_partition_gpu_ns", "packed_merge_analyze_gpu_ns", "packed_merge_pack_gpu_ns", "packed_fused_small_ns", "packed_sort_digits", "packed_full_sort_digits"] {
        totals.insert(name.into(), json!(reports.iter().map(|r| r[name].as_u64().unwrap()).sum::<u64>()));
    }
    for name in ["pool_live", "pool_live_with_scratch", "pool_peak"] {
        totals.insert(format!("plain_{name}"), plain[name].clone());
        totals.insert(format!("packed_{name}"), packed[name].clone());
    }
    for prefix in ["plain", "packed"] { totals.insert(format!("{prefix}_reads"), reads::aggregate(&reports, prefix)); }
    let mut largest: Vec<_> = reports.iter().collect();
    largest.sort_by_key(|r| std::cmp::Reverse(r["rows"].as_u64().unwrap()));
    let largest: Vec<_> = largest.into_iter().take(5).map(|r| json!({"id": r["id"], "relation": r["relation"], "rows": r["rows"], "plain_reads": r["plain_reads"], "packed_reads": r["packed_reads"]})).collect();
    let rows = totals["rows"].as_u64().unwrap();
    for prefix in ["plain", "packed"] {
        let bytes = totals[&format!("{prefix}_bytes")].as_u64().unwrap();
        totals.insert(format!("{prefix}_bits_per_row"), json!(if rows == 0 { 0.0 } else { bytes as f64 * 8.0 / rows as f64 }));
    }
    let result = json!({ "source": manifest.source, "block_rows": block,
        "storage": if plain_storage { "plain" } else { "packed" },
        "largest_states": largest,
        "small_admission_rows": small_rows,
        "varying_sort": varying_sort,
        "small_run_rows": gpu_datalog_runtime::packed_run::SMALL_RUN_ROWS, "states": reports, "totals": totals,
        "plain_tenant_before": plain["tenant_before"], "plain_tenant_after": plain["tenant_after"],
        "packed_tenant_before": packed["tenant_before"], "packed_tenant_after": packed["tenant_after"],
        "timing_indicative": plain["timing_indicative"].as_bool().unwrap() || packed["timing_indicative"].as_bool().unwrap(),
        "memory_protocol": "Global seq-ordered replay, one pool with all states retained. Plain and packed run sequentially with empty-pool teardown between them. Final pool bytes after scratch release and reads include resident reader metadata; peak covers admission inputs/scratch, excluding validation/read scratch.",
        "timing_protocol": "Wall ns including launch, output allocation and final stream synchronization; input upload and oracle download excluded. Ten warmed repetitions of sorted/shuffled/mixed prefixes, shuffled range scans with suffix checksums, and shuffled sampled full-key hits/misses. Admission timers unchanged. gpu_ns fields are CUDA-event stream intervals." });
    println!("TOTAL {}", result["totals"]);
    println!("TENANTS plain_before={} plain_after={} packed_before={} packed_after={} indicative={}", result["plain_tenant_before"], result["plain_tenant_after"], result["packed_tenant_before"], result["packed_tenant_after"], result["timing_indicative"]);
    if let Some(path) = output { fs::write(path, serde_json::to_vec_pretty(&result)?)?; }
    Ok(())
}
