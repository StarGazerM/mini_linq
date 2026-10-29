#[allow(dead_code)]
mod experiment {
    include!("h01.rs");
    use rayon::prelude::*;
    const CHUNK_ROWS: usize = 65_536;
    const OUTPUT_B: &str = "/data/mini_linq/experiments/storage-bench/results/cpu-h01-order-distinct-2026-10-29.json";
    const NAMES: [&str; 8] = ["H_hash_String_SipHash", "R_reserved_hash_codes", "S2_ordered_String_ahash", "S3_ordered_codes_array", "P_packed_radix", "Pprime_packed_comparison", "S1_load_order_verification", "P_load_FOR_bounds"];

    trait Prefix { fn prefix(&self) -> (i64, i64); }
    impl Prefix for Row { fn prefix(&self) -> (i64, i64) { (self.0, self.1) } }
    impl Prefix for CodeRow { fn prefix(&self) -> (i64, i64) { (self.0, self.1) } }
    // Nondecreasing, not strictly increasing: equal-prefix runs are accepted.
    fn ordered(rows: &[Row]) -> bool {
        rows.windows(2).all(|w| w[0].prefix() <= w[1].prefix())
    }
    fn range<T: Prefix>(rows: &[T], chunk: usize) -> std::ops::Range<usize> {
        let nominal_start = chunk * CHUNK_ROWS;
        let nominal_end = (nominal_start + CHUNK_ROWS).min(rows.len());
        let mut start = nominal_start;
        if start > 0 && start < rows.len() && rows[start].prefix() == rows[start - 1].prefix() {
            let prefix = rows[start].prefix();
            start += rows[start..].partition_point(|r| r.prefix() == prefix);
        }
        if start >= nominal_end { return start..start; }
        let mut end = nominal_end;
        if end < rows.len() && rows[end].prefix() == rows[end - 1].prefix() {
            let prefix = rows[end].prefix();
            end += rows[end..].partition_point(|r| r.prefix() == prefix);
        }
        start..end
    }
    fn combine(mut a: Answer, b: Answer) -> Answer {
        for g in 0..6 { for k in 0..3 { a[g][k] += b[g][k]; } }
        a
    }
    fn parallel_chunks<T: Prefix + Sync, F: Fn(&[T]) -> Answer + Sync>(rows: &[T], t: usize, sorted: bool, f: F) -> Answer {
        if t == 1 || !sorted { return f(rows); }
        (0..rows.len().div_ceil(CHUNK_ROWS)).into_par_iter()
            .map(|chunk| f(&rows[range(rows, chunk)]))
            .reduce(|| [[0; 3]; 6], combine)
    }
    fn hash_codes(rows: &[CodeRow]) -> Answer {
        let mut seen = HashSet::with_capacity_and_hasher(rows.len(), DirectState::default());
        let mut out = [[0; 3]; 6];
        for r in rows {
            if r.6 <= CUTOFF && seen.insert((&r.1, &r.3, &r.0, &r.5, r.5, &r.4, r.4, &r.2, &r.6)) {
                add(&mut out, (r.2 * 2 + r.3) as usize, r.4, r.5);
            }
        }
        out
    }
    fn run_strings(rows: &[Row]) -> Answer {
        let mut groups = HashMap::with_hasher(DirectState::default());
        let mut start = 0;
        for (i, r) in rows.iter().enumerate() {
            if i == 0 || rows[i - 1].prefix() != r.prefix() { start = i; }
            // X,Y,X within a prefix run must suppress the second X: comparing
            // only the immediately preceding full tuple would be incorrect.
            if r.6 <= CUTOFF && !rows[start..i].iter().any(|previous| previous == r) {
                let state = groups.entry((&r.2, &r.3)).or_insert([0; 3]);
                state[0] += r.4; state[1] += r.5; state[2] += 1;
            }
        }
        finish(groups)
    }
    fn run_codes(rows: &[CodeRow]) -> Answer {
        let mut out = [[0; 3]; 6];
        let mut start = 0;
        for (i, r) in rows.iter().enumerate() {
            if i == 0 || rows[i - 1].prefix() != r.prefix() { start = i; }
            if r.6 <= CUTOFF && !rows[start..i].iter().any(|previous| previous == r) {
                add(&mut out, (r.2 * 2 + r.3) as usize, r.4, r.5);
            }
        }
        out
    }
    fn values(r: &CodeRow) -> [i64; 7] { [r.0, r.1, r.2 as i64, r.3 as i64, r.4, r.5, r.6 as i64] }
    #[derive(Clone, Debug, PartialEq)]
    struct Bounds { bases: [i64; 7], widths: [u32; 7], shifts: [u32; 7], total: u32 }
    impl Bounds {
        fn scan(rows: &[CodeRow]) -> Self {
            let mut low = [i64::MAX; 7]; let mut high = [i64::MIN; 7];
            for r in rows { for (i, x) in values(r).iter().enumerate() { low[i] = low[i].min(*x); high[i] = high[i].max(*x); } }
            if rows.is_empty() { low = [0; 7]; high = [0; 7]; }
            let mut widths = [0; 7]; let mut shifts = [0; 7]; let mut total = 0;
            for i in 0..7 {
                shifts[i] = total;
                let span = (high[i] as i128 - low[i] as i128) as u128;
                widths[i] = 128 - span.leading_zeros(); total += widths[i];
            }
            assert!(total <= 128, "full source tuple cannot fit u128: {total} bits");
            Self { bases: low, widths, shifts, total }
        }
        fn pack(&self, r: &CodeRow) -> u128 {
            let mut key = 0;
            for (i, x) in values(r).iter().enumerate() {
                if self.widths[i] != 0 { key |= ((*x as i128 - self.bases[i] as i128) as u128) << self.shifts[i]; }
            }
            key
        }
        fn field(&self, key: u128, i: usize) -> i64 {
            if self.widths[i] == 0 { return self.bases[i]; }
            let mask = u128::MAX >> (128 - self.widths[i]);
            ((key >> self.shifts[i] & mask) as i128 + self.bases[i] as i128) as i64
        }
        fn group(&self, key: u128, out: &mut Answer) {
            let group = (self.field(key, 2) * 2 + self.field(key, 3)) as usize;
            add(out, group, self.field(key, 4), self.field(key, 5));
        }
    }
    // Each radix task writes only its precomputed per-bucket exclusive ranges.
    // Ranges are disjoint across tasks and buckets, and cover the output once.
    struct Scatter(*mut u128);
    unsafe impl Send for Scatter {}
    unsafe impl Sync for Scatter {}
    impl Scatter {
        fn write(&self, index: usize, value: u128) {
            // SAFETY: caller uses the exclusive histogram prefix ranges above;
            // scratch is fully allocated and no reads occur until Rayon joins.
            unsafe { self.0.add(index).write(value); }
        }
    }
    fn radix(keys: &mut Vec<u128>, total_bits: u32, t: usize) {
        if keys.len() < 2 || total_bits == 0 { return; }
        let mut scratch = vec![0u128; keys.len()];
        let chunks = keys.len().div_ceil(CHUNK_ROWS);
        let mut hist = vec![[0usize; 256]; if t == 1 { 1 } else { chunks }];
        let mut positions = hist.clone();
        for shift in (0..total_bits).step_by(8) {
            for h in &mut hist { h.fill(0); }
            if t == 1 {
                for &k in keys.iter() { hist[0][((k >> shift) & 255) as usize] += 1; }
            } else {
                hist.par_iter_mut().zip(keys.par_chunks(CHUNK_ROWS)).for_each(|(h, chunk)| {
                    for &k in chunk { h[((k >> shift) & 255) as usize] += 1; }
                });
            }
            let mut offset = 0;
            for digit in 0..256 {
                for c in 0..hist.len() { positions[c][digit] = offset; offset += hist[c][digit]; }
            }
            assert_eq!(offset, keys.len());
            if t == 1 {
                let p = &mut positions[0];
                for &k in keys.iter() { let digit = ((k >> shift) & 255) as usize; scratch[p[digit]] = k; p[digit] += 1; }
            } else {
                let scatter = Scatter(scratch.as_mut_ptr());
                positions.par_iter_mut().zip(keys.par_chunks(CHUNK_ROWS)).for_each(|(p, chunk)| {
                    for &k in chunk { let digit = ((k >> shift) & 255) as usize; scatter.write(p[digit], k); p[digit] += 1; }
                });
            }
            std::mem::swap(keys, &mut scratch);
        }
    }
    struct Measured { answer: Answer, phases: [f64; 4] }
    fn packed(rows: &[CodeRow], bounds: &Bounds, t: usize, comparison: bool) -> Measured {
        let start = Instant::now();
        let mut keys: Vec<u128> = if t == 1 {
            rows.iter().filter(|r| r.6 <= CUTOFF).map(|r| bounds.pack(r)).collect()
        } else {
            rows.par_chunks(CHUNK_ROWS).flat_map_iter(|chunk| chunk.iter().filter(|r| r.6 <= CUTOFF).map(|r| bounds.pack(r))).collect()
        };
        let encode = start.elapsed().as_nanos() as f64;
        let start = Instant::now();
        if comparison { if t == 1 { keys.sort_unstable(); } else { keys.par_sort_unstable(); } }
        else { radix(&mut keys, bounds.total, t); }
        let sort = start.elapsed().as_nanos() as f64;
        let start = Instant::now();
        // Adjacent-unique emits a selection mask, not a compacted key copy.
        let mut first = vec![0u8; keys.len()];
        if t == 1 { for i in 0..keys.len() { first[i] = (i == 0 || keys[i] != keys[i - 1]) as u8; } }
        else { first.par_chunks_mut(CHUNK_ROWS).enumerate().for_each(|(chunk, flags)| {
            for (j, flag) in flags.iter_mut().enumerate() { let i = chunk * CHUNK_ROWS + j; *flag = (i == 0 || keys[i] != keys[i - 1]) as u8; }
        }); }
        let unique = start.elapsed().as_nanos() as f64;
        let start = Instant::now();
        let group = |start: usize, end: usize| { let mut answer = [[0; 3]; 6]; for i in start..end { if first[i] != 0 { bounds.group(keys[i], &mut answer); } } answer };
        let answer = if t == 1 { group(0, keys.len()) } else {
            (0..keys.len().div_ceil(CHUNK_ROWS)).into_par_iter().map(|chunk| { let start = chunk * CHUNK_ROWS; group(start, (start + CHUNK_ROWS).min(keys.len())) }).reduce(|| [[0; 3]; 6], combine)
        };
        Measured { answer, phases: [encode, sort, unique, start.elapsed().as_nanos() as f64] }
    }
    fn execute(v: usize, rows: &[Row], codes: &[CodeRow], sorted: bool, bounds: &Bounds, t: usize) -> Measured {
        let answer = match v {
            0 => parallel_chunks(rows, t, sorted, |r| dedup9::<DirectState>(r, false)),
            1 => parallel_chunks(codes, t, sorted, hash_codes),
            2 if sorted => parallel_chunks(rows, t, true, run_strings),
            2 => dedup9::<DirectState>(rows, false),
            3 if sorted => parallel_chunks(codes, t, true, run_codes),
            3 => hash_codes(codes),
            4 | 5 => return packed(codes, bounds, t, v == 5),
            6 => { let fact = ordered(rows); black_box(fact); let mut a = [[0; 3]; 6]; a[0][0] = fact as i64; a },
            7 => { let observed = black_box(Bounds::scan(codes)); assert_eq!(&observed, bounds); let mut a = [[0; 3]; 6]; a[0][0] = observed.total as i64; a },
            _ => unreachable!(),
        };
        Measured { answer, phases: [0.; 4] }
    }
    fn encode_rows(rows: &[Row]) -> Vec<CodeRow> {
        rows.iter().map(|r| (r.0, r.1, (code(r.2.as_bytes()[0], b'F') / 2) as u8, (code(b'A', r.3.as_bytes()[0]) % 2) as u8, r.4, r.5, r.6)).collect()
    }
    fn correctness_b() {
        let x: Row = (1, 1, "A".into(), "F".into(), 100, 10000, 10000);
        let mut y = x.clone(); y.4 = 200;
        let mut z = x.clone(); z.6 = 11000;
        let mut next = x.clone(); next.1 = 2;
        // Nonadjacent full duplicates in one equal-prefix run, including a
        // run crossing task boundaries, and an excluded predicate row.
        let mut examples = vec![vec![x.clone(), y.clone(), x.clone(), z, next.clone()], vec![x.clone(); CHUNK_ROWS + 17]];
        examples[1].push(y.clone()); examples[1].push(x.clone()); examples[1].push(next);
        let mut unsorted = examples[0].clone(); unsorted.reverse(); examples.push(unsorted);
        for rows in examples {
            let codes = encode_rows(&rows); let bounds = Bounds::scan(&codes); let sorted = ordered(&rows);
            let reference = dedup9::<DirectState>(&rows, false);
            for t in [1, 24] {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap();
                pool.install(|| { for v in 0..6 { assert_eq!(execute(v, &rows, &codes, sorted, &bounds, t).answer, reference, "variant {} t{t}", NAMES[v]); } });
            }
            let mut packed: Vec<_> = codes.iter().map(|r| bounds.pack(r)).collect(); let mut expected = packed.clone(); expected.sort_unstable();
            rayon::ThreadPoolBuilder::new().num_threads(24).build().unwrap().install(|| radix(&mut packed, bounds.total, 24));
            assert_eq!(packed, expected);
            for r in &codes { let key = bounds.pack(r); for (i, value) in values(r).iter().enumerate() { assert_eq!(bounds.field(key, i), *value); } }
        }
        eprintln!("M6b duplicate/nonadjacent/prefix-boundary/unsorted fallback/packing/radix checks passed");
    }
    fn save_b(v: &Value) { fs::write(OUTPUT_B, serde_json::to_vec_pretty(v).unwrap()).unwrap(); }
    pub fn run_b() {
        let small = std::env::args().any(|x| x == "--small");
        assert!(small || std::env::args().any(|x| x == "--machine-quiet"));
        correctness_b();
        let (rows, codes, _) = load(if small { 4099 } else { 6_001_215 });
        let sorted = ordered(&rows); let bounds = Bounds::scan(&codes);
        let reference = dedup9::<DirectState>(&rows, false);
        let oracle: Value = serde_json::from_slice(&fs::read("/data/mini_linq/features/workloads/target/bench/runs/sf-1/retail_h01/default/oracle.json").unwrap()).unwrap();
        if !small {
            let names = ["A", "N", "R"]; let statuses = ["F", "O"];
            let actual: Vec<Value> = reference.iter().enumerate().filter(|(_, a)| a[2] != 0).map(|(g, a)| json!([names[g / 2], statuses[g % 2], a[0].to_string(), a[1].to_string(), a[2].to_string()])).collect();
            assert_eq!(json!(actual), oracle["rows"]);
        }
        let mut report = json!({"experiment":"M6b","metadata":{"n":rows.len(),"threads":[1,24],"warmups":5,"samples":21,"bootstrap_resamples":2000,"sorted_nondecreasing_verified":sorted,"uniqueness_assumed":false,"fallback":"If prefix order verification fails, S2/S3 and parallel H/R use a global serial hash distinct; P/Pprime always handle arbitrary order","prefix":["l_orderkey","l_linenumber"],"prefix_chunk_rows":CHUNK_ROWS,"load_pass_execution_threads":1,"for_fields":["orderkey","linenumber","returnflag_code","linestatus_code","quantity_cents","price_cents","shipdate_days"],"for_bases":bounds.bases,"for_widths":bounds.widths,"for_shifts":bounds.shifts,"packed_bits":bounds.total,"radix_digit_bits":8,"radix_passes":bounds.total.div_ceil(8),"parallel_radix":"fixed-chunk histograms, per-bucket/chunk exclusive offsets, stable parallel scatter; one scratch vector per query","unique":"adjacent-unique selection mask; no compacted-key copy","allocation":"all query working vectors/sets/masks and their destruction included; resident input/code rows and load facts excluded","H_t24":"same H kernel in order-verified prefix-aligned fixed tasks, not claimed to be the actual generated parallel query","duckdb_t1_reference_ms":819.,"duckdb_reference_is_supplied_not_remeasured":true,"oracle_exact":!small,"reference":reference,"flags":"-C target-cpu=native; release opt-level=3; lto=thin; codegen-units=1","rustc":command("rustc",&["-Vv"]),"cpu":command("lscpu",&[]),"driver":command("nvidia-smi",&["--query-gpu=name,driver_version","--format=csv"]),"small":small},"windows":[],"rows":[]});
        for t in [1, 24] {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap();
            let (samples, phase_samples) = loop {
                let before = state();
                if !small && busy(&before) { report["windows"].as_array_mut().unwrap().push(json!({"threads":t,"before":before,"after":before,"accepted":false,"reason":"busy before group"})); save_b(&report); std::thread::sleep(std::time::Duration::from_secs(15)); continue; }
                let mut samples = vec![Vec::new(); NAMES.len()];
                let mut phases = vec![vec![Vec::new(); 4]; NAMES.len()];
                let mut round_states = Vec::new(); let mut contaminated = false;
                for round in 0..if small { 1 } else { 26 } {
                    for j in 0..NAMES.len() {
                        let v = (j + round) % NAMES.len();
                        let start = Instant::now();
                        let got = pool.install(|| execute(v, &rows, &codes, sorted, &bounds, t));
                        let elapsed = start.elapsed().as_nanos() as f64;
                        let expected = if v < 6 { reference } else { let mut a = [[0; 3]; 6]; a[0][0] = if v == 6 { sorted as i64 } else { bounds.total as i64 }; a };
                        assert_eq!(black_box(got.answer), expected, "{} round {round} t{t}", NAMES[v]);
                        if !small && round >= 5 { samples[v].push(elapsed); for p in 0..4 { phases[v][p].push(got.phases[p]); } }
                    }
                    let observed = state(); contaminated = !small && busy(&observed);
                    round_states.push(json!({"round":round,"state":observed}));
                    if contaminated { break; }
                }
                let after = state(); let accepted = !contaminated && (small || !busy(&after));
                report["windows"].as_array_mut().unwrap().push(json!({"threads":t,"before":before,"after":after,"round_states":round_states,"accepted":accepted,"discarded_samples_ns":if accepted { Value::Null } else { json!(samples) }}));
                if accepted { break (samples, phases); }
                save_b(&report); eprintln!("M6b discarded contaminated t{t} group");
            };
            if !small {
                for v in 0..NAMES.len() {
                    let m = med(&samples[v]); let ci = bootstrap(&samples[v], None);
                    let mut row = json!({"variant":NAMES[v],"threads":t,"execution_threads":if v >= 6 { 1 } else { t },"sample_scope":if v >= 6 { "load-time pass" } else { "resident query" },"median_ns":m,"ci95_ns":ci,"samples_ns":samples[v],"ratio_to_H":m/med(&samples[0]),"ratio_to_H_ci95":bootstrap(&samples[v],Some(&samples[0])),"ratio_to_duckdb_t1":m/819_000_000.,"ratio_to_duckdb_t1_ci95":[ci[0]/819_000_000.,ci[1]/819_000_000.],"exact":true});
                    if v == 4 || v == 5 {
                        let phases: Vec<_> = ["encode_and_allocate", "sort", "adjacent_unique_mask", "group_packed_codes"].iter().enumerate().map(|(p,name)| json!({"phase":name,"median_ns":med(&phase_samples[v][p]),"ci95_ns":bootstrap(&phase_samples[v][p],None),"samples_ns":phase_samples[v][p]})).collect();
                        let residual: Vec<_> = samples[v].iter().enumerate().map(|(i, total)| total - (0..4).map(|p| phase_samples[v][p][i]).sum::<f64>()).collect();
                        row["phases"] = json!(phases); row["overhead_and_drop_median_ns"] = json!(med(&residual));
                    }
                    report["rows"].as_array_mut().unwrap().push(row);
                }
                save_b(&report); eprintln!("M6b completed t{t}, order verified={sorted}, packed bits={}",bounds.total);
            }
        }
        if small { println!("M6b small matrix exact t1/t24; no timing samples retained"); }
    }
}
fn main() { experiment::run_b(); }
