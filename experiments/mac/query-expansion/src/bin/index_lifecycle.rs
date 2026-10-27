//! Construction matrix: completed public constructors and first CPU reads, not GPU counters.
#[cfg(target_os = "macos")]
mod mac {
    use cq_mac::{
        IndexSpec, MacContext, MacRun, Payload, Relation as SharedRelation, backend::MAX_ROWS,
        memory,
    };
    use serde_json::{Value, json};
    use std::{error::Error, fs, path::PathBuf, time::Instant};

    type Result<T> = std::result::Result<T, Box<dyn Error>>;
    const MODES: [&str; 3] = ["cpu_comparison", "cpu_radix", "gpu_mccl"];
    const PROFILES: [&str; 5] = [
        "orders_primary",
        "orders_customer",
        "lineitem_order",
        "lineitem_part",
        "partsupp_part",
    ];

    struct Options {
        sizes: Vec<usize>,
        relations: Vec<usize>,
        layout: String,
        source_columns: usize,
        distributions: Vec<String>,
        modes: Vec<usize>,
        cache_policies: Vec<String>,
        cache_bytes: usize,
        runs: usize,
        warmups: usize,
        out: Option<PathBuf>,
    }

    impl Options {
        fn parse() -> Result<Self> {
            let mut options = Self {
                sizes: vec![65_536, 1_048_576, 4_194_304, 16_777_216, 67_108_864],
                relations: vec![1],
                layout: "balanced".into(),
                source_columns: 1,
                distributions: vec!["uniform".into(), "dense".into()],
                modes: vec![0, 1, 2],
                cache_policies: vec!["pooled".into()],
                cache_bytes: 2 * 1024 * 1024 * 1024,
                runs: 5,
                warmups: 2,
                out: None,
            };
            let mut args = std::env::args().skip(1);
            while let Some(flag) = args.next() {
                let value = args
                    .next()
                    .ok_or_else(|| format!("missing value after {flag}"))?;
                match flag.as_str() {
                    "--sizes" => {
                        options.sizes = value
                            .split(',')
                            .map(str::parse)
                            .collect::<std::result::Result<_, _>>()?
                    }
                    "--relations" => {
                        options.relations = value
                            .split(',')
                            .map(str::parse)
                            .collect::<std::result::Result<_, _>>()?
                    }
                    "--layout" => options.layout = value,
                    "--source-columns" => options.source_columns = value.parse()?,
                    "--distributions" => {
                        options.distributions = value.split(',').map(str::to_owned).collect()
                    }
                    "--modes" => {
                        options.modes = value
                            .split(',')
                            .map(|name| {
                                MODES
                                    .iter()
                                    .position(|mode| *mode == name)
                                    .ok_or_else(|| format!("unknown mode {name}"))
                            })
                            .collect::<std::result::Result<_, _>>()?
                    }
                    "--cache" => {
                        options.cache_policies = value.split(',').map(str::to_owned).collect()
                    }
                    "--cache-bytes" => options.cache_bytes = value.parse()?,
                    "--runs" => options.runs = value.parse()?,
                    "--warmups" => options.warmups = value.parse()?,
                    "--out" => options.out = Some(value.into()),
                    _ => return Err(format!("unknown option {flag}").into()),
                }
            }
            if options.sizes.is_empty()
                || options.sizes.iter().any(|&n| n > MAX_ROWS)
                || options.relations.is_empty()
                || options.relations.iter().any(|&n| ![1, 4, 16].contains(&n))
                || options.runs == 0
                || options.modes.is_empty()
            {
                return Err("use nonempty --sizes <= MAX_ROWS, --relations 1,4,16, nonempty --modes and --runs >=1".into());
            }
            if !["balanced", "skewed"].contains(&options.layout.as_str()) {
                return Err("--layout must be balanced or skewed".into());
            }
            if ![1, 2].contains(&options.source_columns) {
                return Err("--source-columns must be 1 (key-only) or 2 (two indexes sharing each relation)".into());
            }
            if options.cache_policies.is_empty()
                || options
                    .cache_policies
                    .iter()
                    .any(|p| !["pooled", "cleared"].contains(&p.as_str()))
            {
                return Err("--cache must select pooled,cleared".into());
            }
            if options.distributions.is_empty()
                || options.distributions.iter().any(|s| {
                    ![
                        "uniform",
                        "dense",
                        "97_keys",
                        "all_equal",
                        "ascending",
                        "descending",
                        "clustered",
                        "skew90",
                        "mixed_relations",
                    ]
                    .contains(&s.as_str())
                        && !PROFILES.contains(&s.as_str())
                })
            {
                return Err("unknown distribution; use uniform,dense,97_keys,all_equal,ascending,descending,clustered,skew90,orders_primary,orders_customer,lineitem_order,lineitem_part,partsupp_part,mixed_relations".into());
            }
            Ok(options)
        }
    }

    struct RestoreCache(Option<usize>);
    impl RestoreCache {
        fn restore(&mut self) -> Result<()> {
            if let Some(previous) = self.0 {
                memory::set_cache_limit(previous)?;
                self.0 = None;
            }
            Ok(())
        }
    }
    impl Drop for RestoreCache {
        fn drop(&mut self) {
            if let Err(error) = self.restore() {
                eprintln!("failed to restore process-wide MLX cache limit: {error}");
            }
        }
    }

    fn stats() -> Result<Value> {
        let value = memory::stats()?;
        Ok(
            json!({"active_bytes":value.active_bytes, "cached_bytes":value.cached_bytes, "peak_bytes":value.peak_bytes}),
        )
    }

    fn faults() -> Value {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // getrusage initializes the entire Darwin rusage only on success.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return json!({"available":false, "error":std::io::Error::last_os_error().to_string()});
        }
        let usage = unsafe { usage.assume_init() };
        json!({"available":true, "minor":usage.ru_minflt, "major":usage.ru_majflt})
    }

    fn fault_delta(before: &Value, after: &Value) -> Value {
        match (
            before["minor"].as_i64(),
            before["major"].as_i64(),
            after["minor"].as_i64(),
            after["major"].as_i64(),
        ) {
            (Some(a), Some(b), Some(c), Some(d)) => {
                json!({"available":true, "minor":c-a, "major":d-b})
            }
            _ => json!({"available":false, "reason":"getrusage snapshot unavailable"}),
        }
    }

    fn phase<T>(operation: impl FnOnce() -> Result<T>) -> Result<(T, Value)> {
        let before_memory = stats()?;
        let before_faults = faults();
        let start = Instant::now();
        let value = operation()?;
        let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
        let after_faults = faults();
        let after_memory = stats()?;
        Ok((
            value,
            json!({"wall_ms":wall_ms, "memory_before":before_memory, "memory_after":after_memory,
            "faults_before":before_faults, "faults_after":after_faults, "fault_delta":fault_delta(&before_faults, &after_faults)}),
        ))
    }

    fn row_counts(total: usize, relations: usize, skewed: bool) -> Vec<usize> {
        let mut counts = vec![0; relations];
        let first = usize::from(skewed && relations > 1);
        if first != 0 {
            counts[0] = total * 4 / 5;
        }
        let remainder = total - counts[0];
        let slots = relations - first;
        for (index, count) in counts[first..].iter_mut().enumerate() {
            *count = remainder / slots + usize::from(index < remainder % slots);
        }
        counts
    }

    fn checksum(rows: impl Iterator<Item = (u32, u32)>) -> u64 {
        rows.fold(0u64, |sum, (key, payload)| {
            sum.rotate_left(7)
                .wrapping_add((u64::from(key) << 32) | u64::from(payload))
        })
    }

    struct Relation {
        source: SharedRelation,
        spec: IndexSpec,
        ingestion: Value,
        expected: Vec<(u32, u32)>,
        expected_checksum: u64,
        probes: Vec<u32>,
        metadata: Value,
    }

    fn source(
        context: &MacContext,
        n: usize,
        distribution: &str,
        relation: usize,
        columns: usize,
    ) -> Result<Vec<Relation>> {
        let seed = 0x83a6237cu32.wrapping_add((relation as u32 + 1).wrapping_mul(0x9e3779b9));
        let profile = if distribution == "mixed_relations" {
            PROFILES[relation % PROFILES.len()]
        } else {
            distribution
        };
        let synthetic = PROFILES.contains(&profile);
        let rotation = seed as usize % n.max(1);
        let parts = (n / 32).max(1);
        let partsupp_scale = n.max(2);
        // Match the H09 lineitem -> filtered, distinct (part,supplier), lexicographic partsupp projection.
        // Its natural row count differs from n, so explicitly tile/truncate that projection to n rows.
        let partsupp = if profile == "partsupp_part" {
            let mut pairs: Vec<_> = (0..partsupp_scale)
                .filter(|i| i % 11 != 0)
                .map(|i| (((i * 13) % parts) as u32 + 1, ((i * 17) % 257) as u32 + 1))
                .collect();
            pairs.sort_unstable();
            pairs.dedup();
            pairs
        } else {
            Vec::new()
        };
        let mut state = seed;
        let mut key = |i: usize| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let j = (i + rotation) % n.max(1);
            let key = match profile {
                "uniform" => state,
                "dense" => ((i * 13 + seed as usize) % parts) as u32 + 1,
                "97_keys" => state % 97,
                "all_equal" => seed,
                "ascending" => i as u32 + (seed & 0xffff),
                "descending" => (n - 1 - i) as u32 + (seed & 0xffff),
                "clustered" => ((i / 256 + seed as usize) % parts) as u32 + 1,
                "skew90" => {
                    if i % 10 == 0 {
                        state
                    } else {
                        state % 97
                    }
                }
                "orders_primary" => j as u32 + 1,
                "orders_customer" => (j % (n / 8).max(1)) as u32 + 1,
                "lineitem_order" => ((j * 17) % ((n / 4).max(1) + 7)) as u32 + 1,
                "lineitem_part" => ((j * 13) % parts) as u32 + 1,
                "partsupp_part" => partsupp[(i + rotation) % partsupp.len()].0,
                _ => unreachable!(),
            };
            key
        };
        // Populate immutable shared storage directly once. Row-ID payloads are
        // generated by each constructor, never staged as a second host column.
        let (source, ingestion) = phase(|| {
            Ok(if columns == 1 {
                context.relation_u32(n, |i| [key(i)])?
            } else {
                context.relation_u32(n, |i| {
                    [
                        key(i),
                        ((i * 17 + seed as usize) % (n / 8).max(1)) as u32 + 1,
                    ]
                })?
            })
        })?;
        let mut indexes = Vec::with_capacity(columns);
        for key_column in 0..columns {
            // CPU correctness oracles are outside ingestion and construction timing.
            let mut expected: Vec<_> = source
                .u32_values()?
                .chunks_exact(columns)
                .enumerate()
                .map(|(i, row)| (row[key_column], i as u32))
                .collect();
            expected.sort_by_key(|row| row.0);
            let expected_checksum = checksum(expected.iter().copied());
            let mut cardinality = 0usize;
            let mut varying = 0u32;
            let mut previous = None;
            let first_key = expected.first().map(|row| row.0);
            let mut probes = vec![0, u32::MAX];
            for &(key, _) in &expected {
                if previous != Some(key) {
                    cardinality += 1;
                    // Keep a representative missing key without searching the input repeatedly.
                    if probes.len() < 12 {
                        if let Some(prev) = previous {
                            if key - prev > 1 {
                                probes.push(prev + 1);
                            }
                        }
                    }
                    previous = Some(key);
                }
                if let Some(first) = first_key {
                    varying |= key ^ first;
                }
            }
            if !expected.is_empty() {
                for position in [
                    0,
                    expected.len() / 4,
                    expected.len() / 2,
                    expected.len() * 3 / 4,
                    expected.len() - 1,
                ] {
                    let key = expected[position].0;
                    probes.push(key);
                    if let Some(key) = key.checked_sub(1) {
                        probes.push(key);
                    }
                    if let Some(key) = key.checked_add(1) {
                        probes.push(key);
                    }
                }
            }
            // Exercise every radix-12 directory boundary via public lookup (CQ intentionally exposes no directory slice).
            for bucket in 0..4096u32 {
                let key = bucket << 20;
                probes.push(key);
                if key > 0 {
                    probes.push(key - 1);
                }
            }
            probes.sort_unstable();
            probes.dedup();
            let varying_key_bytes: Vec<_> = (0..4)
                .filter(|byte| (varying >> (byte * 8)) & 0xff != 0)
                .collect();
            let provenance = match profile {
                "orders_primary" => {
                    json!({"fixture":"H03 h03_scaled", "fixture_lineitem_rows":n * 4, "formula":"order key j+1; n order rows; source rotated by seed"})
                }
                "orders_customer" => {
                    json!({"fixture":"H03 h03_scaled", "fixture_lineitem_rows":n * 4, "customers":(n/8).max(1), "formula":"order customer j%max(n/8,1)+1; n order rows; source rotated by seed"})
                }
                "lineitem_order" => {
                    json!({"fixture":"H03 h03_scaled", "fixture_lineitem_rows":n, "orders":(n/4).max(1), "formula":"(j*17)%(orders+7)+1; source rotated by seed"})
                }
                "lineitem_part" => {
                    json!({"fixture":"H09 h09_scaled", "fixture_lineitem_rows":n, "parts":parts, "formula":"(j*13)%parts+1; source rotated by seed"})
                }
                "partsupp_part" => {
                    json!({"fixture":"H09 h09_scaled", "fixture_lineitem_rows":partsupp_scale, "parts":parts,
                "natural_distinct_part_supplier_rows":partsupp.len(),
                "formula":"lexicographic distinct ((i*13)%parts+1,(i*17)%257+1) for i%11!=0, project part; rotate and tile/truncate to requested rows",
                "tiling":"cyclic projection, not dbgen; source row-ID payloads assigned after tiling",
                "scale_policy":"max(requested rows,2), ensuring a nonempty filtered projection before cyclic resizing"})
                }
                _ => json!({"fixture":"generic deterministic synthetic", "formula":match profile {
                    "uniform" => "xorshift32 full-width state",
                    "dense" => "(i*13+seed)%max(n/32,1)+1",
                    "97_keys" => "xorshift32 state%97",
                    "all_equal" => "seed",
                    "ascending" => "i+(seed&65535)",
                    "descending" => "n-1-i+(seed&65535)",
                    "clustered" => "(i/256+seed)%max(n/32,1)+1; contiguous 256-row key clusters",
                    "skew90" => "i%10==0: full-width xorshift32; otherwise state%97",
                    _ => unreachable!(),
                }}),
            };
            let metadata = json!({"relation":relation, "key_column":key_column,
            "profile":if key_column == 0 { profile } else { "shared_secondary" },
            "rows":n, "source_bytes":n as u64 * columns as u64 * 4, "source_columns":columns,
            "synthetic_not_dbgen":true, "relation_shaped":synthetic && key_column == 0, "seed":seed,
            "source_rotation_rows":if synthetic && key_column == 0 { rotation } else { 0 },
            "key_cardinality":cardinality, "varying_key_bit_mask":varying, "varying_key_bytes_lsb_zero_based":varying_key_bytes,
            "varying_key_byte_count":varying_key_bytes.len(), "key_min":expected.first().map(|r|r.0), "key_max":expected.last().map(|r|r.0),
            "payload":"source row-ID local to this relation", "expected_checksum":expected_checksum,
            "lookup_probe_count":probes.len(), "provenance":if key_column == 0 { provenance } else {
                json!({"fixture":"second column of the SAME shared relation",
                    "formula":"(i*17+seed)%max(n/8,1)+1", "payload":"original row-ID; not a partition or separate relation"})
            }});
            indexes.push(Relation {
                source: source.clone(),
                spec: IndexSpec {
                    key_column,
                    payload: Payload::RowId,
                },
                ingestion: ingestion.clone(),
                expected,
                expected_checksum,
                probes,
                metadata,
            });
        }
        Ok(indexes)
    }

    fn check(run: &MacRun, relation: &Relation) -> Result<()> {
        if run.len() != relation.expected.len() || !run.rows().eq(relation.expected.iter().copied())
        {
            return Err("full stable sorted key/payload equality failed".into());
        }
        for key in &relation.probes {
            let begin = relation.expected.partition_point(|row| row.0 < *key);
            let end = relation.expected.partition_point(|row| row.0 <= *key);
            if run.contains(key) != (begin != end)
                || !run
                    .lookup(key)
                    .eq(relation.expected[begin..end].iter().map(|row| row.1))
            {
                return Err(
                    format!("directory-backed lookup/contains mismatch for key {key}").into(),
                );
            }
        }
        Ok(())
    }

    fn sample(
        context: &MacContext,
        relations: &[Relation],
        mode: usize,
        cache: &str,
    ) -> Result<Value> {
        let (_, boundary) = phase(|| {
            if cache == "cleared" {
                memory::clear_cache()?;
            }
            Ok(())
        })?;
        let mut live = Vec::with_capacity(relations.len());
        let mut per_relation = Vec::with_capacity(relations.len());
        let before_memory = stats()?;
        let before_faults = faults();
        let start = Instant::now();
        let mut wall_ms = 0.0;
        for relation in relations {
            let (run, build) = phase(|| {
                Ok(match mode {
                    0 => context.build_on_cpu(&relation.source, relation.spec, None)?,
                    1 => context.build_on_cpu_radix(&relation.source, relation.spec, None)?,
                    2 => context.build(&relation.source, relation.spec, None)?,
                    _ => unreachable!(),
                })
            })?;
            wall_ms += build["wall_ms"].as_f64().expect("recorded wall time");
            live.push(run);
            per_relation.push(json!({"relation":relation.metadata["relation"], "key_column":relation.spec.key_column,
                "profile":relation.metadata["profile"], "rows":relation.source.len(), "construction":build}));
        }
        let batch_wall_ms = start.elapsed().as_secs_f64() * 1000.0;
        let after_faults = faults();
        let after_memory = stats()?;
        let construction = json!({"wall_ms":wall_ms, "batch_wall_ms_including_diagnostics":batch_wall_ms,
            "memory_before":before_memory, "memory_after":after_memory,
            "faults_before":before_faults, "faults_after":after_faults, "fault_delta":fault_delta(&before_faults, &after_faults)});
        let mut scan_ms = 0.0;
        let scan_before_memory = stats()?;
        let scan_before_faults = faults();
        for ((run, relation), record) in live.iter().zip(relations).zip(&mut per_relation) {
            let (actual, scan) = phase(|| Ok(checksum(run.rows())))?;
            if actual != relation.expected_checksum {
                return Err("first CPU scan checksum mismatch".into());
            }
            scan_ms += scan["wall_ms"].as_f64().expect("recorded wall time");
            record["first_cpu_scan"] = scan;
            record["first_cpu_scan_checksum"] = json!(actual);
        }
        let scan_after_faults = faults();
        let scan_after_memory = stats()?;
        let scans = json!({"wall_ms":scan_ms, "memory_before":scan_before_memory, "memory_after":scan_after_memory,
            "faults_before":scan_before_faults, "faults_after":scan_after_faults,
            "fault_delta":fault_delta(&scan_before_faults, &scan_after_faults)});
        let validation_start = Instant::now();
        // Every output remains simultaneously live through construction, all first reads, and all checks.
        for (run, relation) in live.iter().zip(relations) {
            check(run, relation)?;
        }
        let validation_ms = validation_start.elapsed().as_secs_f64() * 1000.0;
        let release_before_memory = stats()?;
        let release_before_faults = faults();
        let mut release_ms = 0.0;
        for (run, record) in live.into_iter().zip(&mut per_relation) {
            let (_, release) = phase(|| {
                drop(run);
                Ok(())
            })?;
            release_ms += release["wall_ms"].as_f64().expect("recorded wall time");
            record["release"] = release;
            record["validation"] = json!(
                "passed full stable rows and representative directory-backed lookup/contains"
            );
        }
        let release_after_faults = faults();
        let release_after_memory = stats()?;
        Ok(
            json!({"wall_ms":wall_ms, "first_cpu_scan_ms":scan_ms, "release_ms":release_ms,
            "cache_boundary":boundary, "cache_boundary_operation":if cache == "cleared" {"clear inactive MLX cache"} else {"retain inactive MLX cache (no operation)"},
            "construction":construction, "first_cpu_scan":scans, "release":{"wall_ms":release_ms,
                "memory_before":release_before_memory, "memory_after":release_after_memory,
                "faults_before":release_before_faults, "faults_after":release_after_faults,
                "fault_delta":fault_delta(&release_before_faults, &release_after_faults)},
            "validation_ms_excluded":validation_ms, "validation":"passed", "relations":per_relation}),
        )
    }

    fn statistics(samples: &[Value], field: &str) -> Value {
        let mut values: Vec<_> = samples
            .iter()
            .filter_map(|value| value[field].as_f64())
            .collect();
        if values.is_empty() {
            return json!({"available":false});
        }
        values.sort_by(f64::total_cmp);
        let middle = values.len() / 2;
        let median = if values.len() % 2 == 0 {
            (values[middle - 1] + values[middle]) / 2.0
        } else {
            values[middle]
        };
        json!({"min":values[0], "median":median, "max":values[values.len()-1], "count":values.len()})
    }

    fn persist(options: &Options, report: &Value) -> Result<()> {
        if let Some(path) = &options.out {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)?;
            }
            let temporary = path.with_file_name(format!(
                "{}.{}.tmp",
                path.file_name()
                    .ok_or("output needs a filename")?
                    .to_string_lossy(),
                std::process::id()
            ));
            let mut writer = std::io::BufWriter::new(fs::File::create(&temporary)?);
            serde_json::to_writer_pretty(&mut writer, report)?;
            std::io::Write::flush(&mut writer)?;
            fs::rename(temporary, path)?;
        }
        Ok(())
    }

    fn configurations(options: &Options, context: &MacContext, report: &mut Value) -> Result<()> {
        let mut invoked = [false; 3];
        let mut configuration_index = 0usize;
        println!(
            "distribution\ttotal_rows\trelations\tsource_columns\tlayout\tcache\tmode\tingestion_ms\tsource_bytes\twall_ms\tfirst_cpu_scan_ms\trelease_ms"
        );
        for distribution in &options.distributions {
            for &total in &options.sizes {
                for &relation_count in &options.relations {
                    let counts = row_counts(total, relation_count, options.layout == "skewed");
                    report["current_configuration"] = json!({"distribution":distribution, "total_rows":total,
                        "relation_count":relation_count, "layout":options.layout, "stage":"generating source/reference"});
                    persist(options, report)?;
                    let mut relations = Vec::with_capacity(relation_count * options.source_columns);
                    for (index, &n) in counts.iter().enumerate() {
                        relations.extend(source(
                            context,
                            n,
                            distribution,
                            index,
                            options.source_columns,
                        )?);
                    }
                    let ingestions: Vec<_> = relations.iter().filter(|r| r.spec.key_column == 0)
                        .map(|r| json!({"relation":r.metadata["relation"], "source_bytes":r.metadata["source_bytes"],
                            "ingestion":r.ingestion})).collect();
                    let ingestion_ms: f64 = ingestions
                        .iter()
                        .map(|r| {
                            r["ingestion"]["wall_ms"]
                                .as_f64()
                                .expect("ingestion wall time")
                        })
                        .sum();
                    for cache in &options.cache_policies {
                        report["current_configuration"] = json!({"distribution":distribution, "total_rows":total,
                            "relation_count":relation_count, "layout":options.layout, "cache":cache, "stage":"sampling"});
                        persist(options, report)?;
                        let mut records: Vec<Value> = options
                            .modes
                            .iter()
                            .map(|&mode| {
                                json!({"mode":MODES[mode],
                            "first_invocation":[], "warmup_samples":[], "samples":[]})
                            })
                            .collect();
                        // First observation is per configuration and separate from warmups/steady-state statistics.
                        for round in 0..(1 + options.warmups + options.runs) {
                            for step in 0..options.modes.len() {
                                let slot =
                                    (configuration_index + round + step) % options.modes.len();
                                let mode = options.modes[slot];
                                let mut value = sample(context, &relations, mode, cache)?;
                                value["order"] = json!(step);
                                value["round_including_first_and_warmups"] = json!(round);
                                value["process_first_invocation_of_mode"] = json!(!invoked[mode]);
                                invoked[mode] = true;
                                let field = if round == 0 {
                                    "first_invocation"
                                } else if round <= options.warmups {
                                    "warmup_samples"
                                } else {
                                    "samples"
                                };
                                value["phase"] = json!(field);
                                if round > options.warmups {
                                    value["round"] = json!(round - options.warmups - 1);
                                }
                                records[slot][field]
                                    .as_array_mut()
                                    .expect("sample array")
                                    .push(value);
                            }
                        }
                        for record in &mut records {
                            let values = record["samples"].as_array().expect("sample array");
                            let mut summary = serde_json::Map::new();
                            print!(
                                "{distribution}\t{total}\t{relation_count}\t{}\t{}\t{cache}\t{}\t{ingestion_ms:.3}\t{}",
                                options.source_columns,
                                options.layout,
                                record["mode"].as_str().expect("mode"),
                                total as u64 * options.source_columns as u64 * 4
                            );
                            for field in ["wall_ms", "first_cpu_scan_ms", "release_ms"] {
                                let stat = statistics(values, field);
                                print!(
                                    "\t{:.3}",
                                    stat["median"].as_f64().expect("nonempty measured samples")
                                );
                                summary.insert(field.into(), stat);
                            }
                            println!();
                            let mut relation_summaries = Vec::with_capacity(relations.len());
                            for index in 0..relations.len() {
                                let relation_values: Vec<_> = values.iter().map(|sample| json!({
                                    "wall_ms":sample["relations"][index]["construction"]["wall_ms"],
                                    "first_cpu_scan_ms":sample["relations"][index]["first_cpu_scan"]["wall_ms"],
                                    "release_ms":sample["relations"][index]["release"]["wall_ms"]})).collect();
                                relation_summaries.push(json!({"relation":relations[index].metadata["relation"],
                                    "key_column":relations[index].spec.key_column, "wall_ms":statistics(&relation_values, "wall_ms"),
                                    "first_cpu_scan_ms":statistics(&relation_values, "first_cpu_scan_ms"),
                                    "release_ms":statistics(&relation_values, "release_ms")}));
                            }
                            record["summary"] = json!(summary);
                            record["relation_summaries"] = json!(relation_summaries);
                        }
                        report["results"].as_array_mut().expect("results array").push(json!({"status":"complete",
                            "configuration_index":configuration_index, "distribution":distribution, "total_rows":total,
                            "source_bytes":total as u64 * options.source_columns as u64 * 4,
                            "source_columns":options.source_columns, "index_count":relations.len(),
                            "ingestion_ms":ingestion_ms, "source_ingestions":ingestions,
                            "relation_count":relation_count, "layout":options.layout, "actual_relation_rows":counts, "cache":cache,
                            "relations":relations.iter().map(|r| &r.metadata).collect::<Vec<_>>(), "modes":records}));
                        configuration_index += 1;
                        report["completed_configurations"] = json!(configuration_index);
                        report["current_configuration"] = json!({"stage":"between configurations"});
                        persist(options, report)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn main() -> Result<()> {
        let options = Options::parse()?;
        let mut report = json!({"schema":"mccl.index-lifecycle.v3", "status":"incomplete", "completed_configurations":0,
            "configuration_count":options.distributions.len() * options.sizes.len() * options.relations.len() * options.cache_policies.len(),
            "configuration":{"sizes_total_rows":options.sizes, "relations":options.relations, "layout":options.layout,
                "source_columns":options.source_columns,
                "distributions":options.distributions, "modes":options.modes.iter().map(|&i| MODES[i]).collect::<Vec<_>>(),
                "cache_policies":options.cache_policies, "cache_limit_bytes":options.cache_bytes, "runs":options.runs, "warmups":options.warmups},
            "row_semantics":"--sizes is TOTAL rows across independent indexed relations, not rows per relation; balanced quotient/remainder or floor(80% total) in relation zero and balanced remainder",
            "source_semantics":"each relation is ingested once directly into immutable shared u32 storage and retained across all indexes, modes and cache policies; --source-columns 1 stores keys only, 2 builds indexes of both independently distributed key columns from the SAME relation; GPU generates original local row-ID payloads; synthetic formulas, not dbgen or TPC-H",
            "allocation":"bounded process-wide MLX inactive-buffer cache; cache boundary separately timed before construction; no implicit wired-budget changes; pooled is not guaranteed residency and cleared does not guarantee physically cold pages",
            "memory":"MLX process-wide active/cached/peak byte snapshots, peak is cumulative (never reset), not physical residency or DRAM traffic counters",
            "faults":"getrusage(RUSAGE_SELF) process-wide cumulative minor/major faults and interval deltas; unavailable reported explicitly",
            "order":"rotate mode slots by configuration index plus round; first observation per configuration separate, process first invocation marked; warmups recorded but excluded from summaries; predecessors/idle gaps not balanced",
            "liveness":"build every index of every relation, scan all indexes, validate all indexes, then release outputs; shared sources remain live across samples and cache clears",
            "timing":"ingestion wall and memory/fault snapshots reported once per source; CPU wall times around real public completed constructors start after ingestion; aggregate wall_ms sums per-index constructor timers; construction batch wall includes diagnostic/loop overhead; scan/release aggregates sum individual timers; fault spans can include diagnostic overhead; no GPU timestamps",
            "first_cpu_scan":"first harness full-row read after all constructions; rotate-left-7 and wrapping-add packed key:u32/payload:u32 checksum, includes iteration/checksum, excludes validation; not guaranteed cold CPU caches",
            "validation":"linear full stable sorted row equality; exact cardinality and varying bytes derived once from reference; binary-search oracle for representative present/missing probes and all radix-12 bucket boundaries via public lookup/contains; CQ does not expose a raw directory slice",
            "source_bytes":"total_rows*source_columns*4 logical retained shared-source bytes, counted once per relation even with two indexes; not physical residency or measured memory traffic; oracle vectors are separate untimed CPU memory",
            "results":[]});
        persist(&options, &report)?;
        let mut restore = RestoreCache(None);
        let outcome = (|| -> Result<()> {
            let setup_start = Instant::now();
            let context = MacContext::new()?;
            report["context_setup_ms_excluded"] =
                json!(setup_start.elapsed().as_secs_f64() * 1000.0);
            let previous = memory::set_cache_limit(options.cache_bytes)?;
            restore.0 = Some(previous);
            report["previous_cache_limit_bytes"] = json!(previous);
            // Enforce the configured starting cache bound: lowering the limit alone does not empty old cache.
            let (_, initialization) = phase(|| {
                memory::clear_cache()?;
                Ok(())
            })?;
            report["initial_cache_clear_excluded"] = initialization;
            persist(&options, &report)?;
            configurations(&options, &context, &mut report)
        })();
        let restoration = restore.restore();
        report["cache_limit_restored"] = json!(restoration.is_ok());
        if let Err(error) = &restoration {
            report["restoration_error"] = json!(error.to_string());
        }
        if let Err(error) = &outcome {
            report["error"] = json!(error.to_string());
        }
        report["status"] = json!(if outcome.is_ok() && restoration.is_ok() {
            "complete"
        } else {
            "failed"
        });
        persist(&options, &report)?;
        outcome?;
        restoration?;
        if let Some(path) = options.out {
            println!("Raw lifecycle evidence: {}", path.display());
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    mac::main()
}

#[cfg(not(target_os = "macos"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("index_lifecycle requires macOS/MCCL".into())
}
