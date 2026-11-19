use super::*;
use gpu_datalog_runtime::packed_run::read_support::{checksum, packed_read, plain_read};

pub(super) const WORKLOADS: [&str; 5] = ["prefix_sorted", "prefix_shuffled", "prefix_mixed", "range_scan", "membership"];

fn workload_range(name: &str) -> cq_gpu_runtime::nvtx::Range {
    cq_gpu_runtime::nvtx::range(match name {
        "prefix_sorted" => c"read_prefix_sorted",
        "prefix_shuffled" => c"read_prefix_shuffled",
        "prefix_mixed" => c"read_prefix_mixed",
        "range_scan" => c"read_range_scan",
        "membership" => c"read_membership",
        _ => unreachable!(),
    })
}

fn state_range(id: usize) -> cq_gpu_runtime::nvtx::Range {
    cq_gpu_runtime::nvtx::range(match id {
        90 => c"state_90",
        112 => c"state_112",
        137 => c"state_137",
        147 => c"state_147",
        _ => c"state_other",
    })
}

fn shuffle<T>(values: &mut [T]) {
    let mut seed = 0xd1b54a32d192ed03u64;
    for i in (1..values.len()).rev() {
        seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
        values.swap(i, seed as usize % (i + 1));
    }
}

fn interval<W: Word>(field: &Field, prefix: u64) -> (W, W) {
    let mut low = W::zero(); low.insert(field.shift, field.bits, prefix);
    let mut high = low;
    for lane in 0..W::BITS / 64 {
        let bits = field.shift.saturating_sub(lane * 64).min(64);
        if bits != 0 { high.set_lane(lane, high.lane(lane) | (u64::MAX >> (64 - bits))); }
    }
    (low, high)
}

pub(super) fn run<W: Word>(spec: &State, pool: &std::sync::Arc<Pool>, plain: &DeviceRadixRun<W, u64, ()>, packed: Option<&PackedSet<W>>, prefixes: &BTreeSet<u64>, expected: &[W]) -> Result<Value> {
    let mut sorted = Vec::new();
    let mut mixed = Vec::new();
    let mut absent_below = 0;
    let mut absent_between = 0;
    let mut absent_above = 0;
    if let Some(field) = spec.fields.first() {
        sorted.extend(prefixes.iter().map(|&p| interval::<W>(field, p)));
        mixed.clone_from(&sorted);
        if let Some(&first) = prefixes.first() {
            if first > 0 { mixed.push(interval(field, first - 1)); absent_below += 1; }
        }
        let mut previous = None;
        for &p in prefixes {
            if let Some(prev) = previous { if p > prev + 1 { mixed.push(interval(field, prev + 1)); absent_between += 1; } }
            previous = Some(p);
        }
        let maximum = u64::MAX >> (64 - field.bits);
        if let Some(&last) = prefixes.last() {
            if last < maximum { mixed.push(interval(field, last + 1)); absent_above += 1; }
        }
    }
    let mut shuffled = sorted.clone(); shuffle(&mut shuffled); shuffle(&mut mixed);
    // Deterministically sample up to 65,536 present keys and one successor miss per sampled key.
    let mut members = Vec::new();
    let stride = expected.len().div_ceil(65536).max(1);
    for &key in expected.iter().step_by(stride) {
        members.push((key, key));
        let mut next = key;
        let mut carry = true;
        for lane in 0..W::BITS / 64 {
            if carry { let (value, overflow) = next.lane(lane).overflowing_add(1); next.set_lane(lane, value); carry = overflow; }
        }
        if !carry && expected.binary_search(&next).is_err() { members.push((next, next)); }
    }
    shuffle(&mut members);
    let mut result = serde_json::Map::new();
    let floor = || gpu_datalog_runtime::packed_run::read_diagnostics::launch_floor(pool);
    floor()?;
    let mut floor_ns = 0;
    for _ in 0..10 { floor_ns += measure(pool, floor)?.1; }
    result.insert("launch_floor_ns_per_repeat".into(), json!(floor_ns as f64 / 10.0));
    if std::env::var_os("PACKED_READ_STAGES").is_some() {
        if let Some(packed) = packed {
            let probes = Buffer::from_host(pool.clone(), &shuffled.iter().map(|p| p.0).collect::<Vec<_>>())?;
            result.insert("stage_gpu_ns".into(), json!(gpu_datalog_runtime::packed_run::read_diagnostics::stages(packed, &probes)?));
        }
    }
    for (name, probes) in [(WORKLOADS[0], &sorted), (WORKLOADS[1], &shuffled), (WORKLOADS[2], &mixed), (WORKLOADS[3], &shuffled), (WORKLOADS[4], &members)] {
        let low = Buffer::from_host(pool.clone(), &probes.iter().map(|p| p.0).collect::<Vec<_>>())?;
        let high = Buffer::from_host(pool.clone(), &probes.iter().map(|p| p.1).collect::<Vec<_>>())?;
        let membership = name == "membership";
        let scan = name == "range_scan";
        let suffix_bits = spec.fields.first().map_or(W::BITS, |field| if field.shift == 0 { W::BITS } else { field.shift });
        let mut scanned = 0u64;
        let mut misses = 0u64;
        let mut oracle = Vec::with_capacity(probes.len());
        for &(lo, hi) in probes {
            if membership {
                let found = expected.binary_search(&lo).is_ok(); misses += !found as u64;
                oracle.push([found as u64, 0, 0]);
            } else {
                let a = expected.partition_point(|key| *key < lo);
                let b = expected.partition_point(|key| *key <= hi);
                misses += (a == b) as u64;
                let sum = if scan { scanned += (b - a) as u64; expected[a..b].iter().fold(0u64, |sum, &key| sum.wrapping_add(checksum(key, suffix_bits))) } else { 0 };
                oracle.push([a as u64, b as u64, sum]);
            }
        }
        let repeats = if probes.is_empty() { 0 } else { 10u64 };
        let mut elapsed = 0;
        if scan || membership {
            let launch = || if let Some(packed) = packed { packed_read(packed, &low, &high, suffix_bits, membership) } else { plain_read(plain, &low, &high, suffix_bits, membership) };
            if launch()?.to_host()? != oracle { return Err(format!("read oracle mismatch state={} workload={name}", spec.id).into()); }
            let _workload = workload_range(name);
            let _state = state_range(spec.id);
            for _ in 0..repeats { elapsed += measure(pool, launch)?.1; }
        } else {
            let launch = || if let Some(packed) = packed { packed_ranges(packed, &low, &high) } else { plain_ranges(plain, &low, &high) };
            let actual = launch()?.to_host()?;
            if actual.iter().zip(&oracle).any(|(a, b)| *a != [b[0], b[1]]) || actual.len() != oracle.len() { return Err(format!("read oracle mismatch state={} workload={name}", spec.id).into()); }
            let _workload = workload_range(name);
            let _state = state_range(spec.id);
            for _ in 0..repeats { elapsed += measure(pool, launch)?.1; }
        }
        result.insert(name.into(), json!({ "ns": elapsed, "probes": probes.len() as u64 * repeats, "rows_scanned": scanned * repeats,
            "misses_per_repeat": misses, "repeats": repeats,
            "ns_per_repeat": if repeats == 0 { 0.0 } else { elapsed as f64 / repeats as f64 },
            "ns_per_probe": if probes.is_empty() { 0.0 } else { elapsed as f64 / (probes.len() as u64 * repeats) as f64 },
            "above_30000_probes": probes.len() >= 30000,
            "launch_floor_adjusted_ns_per_probe": if probes.is_empty() { 0.0 } else { (elapsed as f64 / repeats as f64 - floor_ns as f64 / 10.0).max(0.0) / probes.len() as f64 },
            "lookups_per_second": if elapsed == 0 { 0.0 } else { probes.len() as f64 * repeats as f64 * 1e9 / elapsed as f64 },
            "rows_per_second": if elapsed == 0 { 0.0 } else { scanned as f64 * repeats as f64 * 1e9 / elapsed as f64 } }));
    }
    result.insert("absent_prefixes".into(), json!({"below": absent_below, "between": absent_between, "above": absent_above}));
    Ok(Value::Object(result))
}

pub(super) fn aggregate(reports: &[Value], prefix: &str) -> Value {
    let mut result = serde_json::Map::new();
    for name in WORKLOADS {
        let key = format!("{prefix}_reads");
        let sum = |field: &str| reports.iter().map(|r| r[&key][name][field].as_u64().unwrap()).sum::<u64>();
        let ns = sum("ns"); let probes = sum("probes"); let rows = sum("rows_scanned");
        let steady: Vec<_> = reports.iter().map(|r| &r[&key][name]).filter(|r| r["above_30000_probes"] == true).collect();
        let steady_ns = steady.iter().map(|r| r["ns"].as_u64().unwrap()).sum::<u64>();
        let steady_probes = steady.iter().map(|r| r["probes"].as_u64().unwrap()).sum::<u64>();
        result.insert(name.into(), json!({"ns": ns, "probes": probes, "rows_scanned": rows,
            "steady_states": steady.len(), "steady_ns": steady_ns, "steady_probes": steady_probes,
            "steady_ns_per_probe": if steady_probes == 0 { 0.0 } else { steady_ns as f64 / steady_probes as f64 },
            "lookups_per_second": if ns == 0 { 0.0 } else { probes as f64 * 1e9 / ns as f64 },
            "rows_per_second": if ns == 0 { 0.0 } else { rows as f64 * 1e9 / ns as f64 }}));
    }
    Value::Object(result)
}
