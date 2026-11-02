//! Runtime laws of the generated radix reads (P4-GPU slice B): formal rules
//! GPU-RadixEncodedProbe, GPU-RadixCandidates, GPU-RadixSourceBuild and
//! GPU-Smallest, executed by test kernels that mirror the emitted reads
//! (encode once, per-run item intervals, exact distinct counts, first-minimum
//! choice, membership in every other side). Fixed seeds throughout.
use std::collections::BTreeSet;

use cq_storage::RadixRun;
use gpu_datalog_runtime::{radix_run::RadixRunParams, DeviceRadixRun, Packing, RadixLayout};
use gpu_device_columns::{Buffer, Codec, ColumnView, Pool, SourceView};

type Result<T = (), E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
type Lookup = RadixLayout<(i64,), (i64, u32), u64, u32, (Buffer<i64>, Buffer<u32>)>;
type Side = RadixLayout<(i64, i64), (i64,), u128, u32, Buffer<i64>>;

/// Full-domain signed values: extremes, both signs and wide magnitudes.
const DOMAIN: [i64; 12] = [
    i64::MIN,
    i64::MIN + 1,
    -(1 << 40),
    -5,
    -1,
    0,
    1,
    2,
    7,
    1 << 40,
    i64::MAX - 1,
    i64::MAX,
];
/// Values in no relation: absent prefixes and candidates.
const ABSENT: [i64; 3] = [3, -2, (1 << 40) + 1];

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
    fn value(&mut self) -> i64 {
        DOMAIN[self.below(DOMAIN.len())]
    }
}

fn params(ratio: usize, radix_bits: u32) -> RadixRunParams {
    RadixRunParams {
        ratio,
        radix_bits,
        merge_tile_rows: 16,
    }
}

/// A native Plain column of i64 bit patterns: the resident source ABI.
fn plain(buffer: &Buffer<u64>, rows: usize) -> SourceView {
    SourceView::from(ColumnView {
        headers: std::ptr::null(),
        data: buffer.ptr(),
        len: rows as u64,
        count: std::ptr::null(),
        codec: Codec::Plain as u32,
    })
}

fn bits(values: &[i64]) -> Vec<u64> {
    values.iter().map(|&value| value as u64).collect()
}

#[cuda_host::cuda_module]
mod reads {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    use gpu_datalog_runtime::radix_run::{device, RadixItemsView, RadixLayoutView};

    /// The emitted ordinary lookup: encode the bound key once, then read
    /// every run's local item interval in run order.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn radix_test_lookup(
        view: RadixLayoutView<u64, u32>,
        items: RadixItemsView,
        probes: *const i64,
        count: u32,
        bases: *const u64,
        lengths: *mut u64,
        serials: *mut i64,
        tags: *mut u32,
    ) {
        let probe = thread::index_1d().get() as u32;
        if probe >= count {
            return;
        }
        unsafe {
            let (low, high, fits) = device::encode_prefix(view, probes.add(probe as usize), 1);
            let base = *bases.add(probe as usize);
            let capacity = *bases.add(probe as usize + 1) - base;
            let mut length = 0_u64;
            if fits {
                let mut run = 0_u32;
                while run < view.keys.run_count {
                    let (begin, end) =
                        device::interval_items(device::run(view.keys, run), low, high);
                    let mut position = begin;
                    while position < end {
                        if length < capacity {
                            let index = (base + length) as usize;
                            *serials.add(index) = device::item::<i64>(items, run, 0, position);
                            *tags.add(index) = device::item::<u32>(items, run, 1, position);
                        }
                        length += 1;
                        position += 1;
                    }
                    run += 1;
                }
            }
            *lengths.add(probe as usize) = length;
        }
    }

    /// The emitted smallest probe over `sides <= 3` two-level layouts: exact
    /// distinct candidate counts per side, the first minimum side, then each
    /// of its candidates kept iff every other side holds it.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn radix_test_smallest(
        a: RadixLayoutView<u128, u32>,
        b: RadixLayoutView<u128, u32>,
        c: RadixLayoutView<u128, u32>,
        items_a: RadixItemsView,
        items_b: RadixItemsView,
        items_c: RadixItemsView,
        sides: u32,
        prefixes: *const i64,
        count: u32,
        counts: *mut u64,
        choices: *mut u32,
        found: *mut u64,
        output: *mut i64,
        stride: u64,
    ) {
        let probe = thread::index_1d().get() as u32;
        if probe >= count {
            return;
        }
        unsafe {
            let views = [a, b, c];
            let items = [items_a, items_b, items_c];
            let mut lows = [0_u128; 3];
            let mut highs = [0_u128; 3];
            let mut choice = 0_u32;
            let mut minimum = 0_u64;
            let mut side = 0_u32;
            while side < sides {
                let prefix = prefixes.add((probe * 3 + side) as usize);
                let (low, high, fits) = device::encode_prefix(views[side as usize], prefix, 1);
                let total = if fits {
                    device::distinct(views[side as usize].keys, low, high)
                } else {
                    0
                };
                lows[side as usize] = low;
                highs[side as usize] = high;
                *counts.add((probe * 3 + side) as usize) = total;
                if side == 0 || total < minimum {
                    choice = side;
                    minimum = total;
                }
                side += 1;
            }
            *choices.add(probe as usize) = choice;
            let chosen = views[choice as usize];
            let mut kept = 0_u64;
            let mut ordinal = 0_u64;
            while ordinal < minimum {
                let (run, slot) = device::distinct_at(
                    chosen.keys,
                    lows[choice as usize],
                    highs[choice as usize],
                    ordinal,
                );
                let first = device::offset(device::run(chosen.keys, run), slot);
                let candidate = device::item::<i64>(items[choice as usize], run, 0, first);
                let mut live = true;
                let mut other = 0_u32;
                while other < sides {
                    if live && other != choice {
                        let key = [*prefixes.add((probe * 3 + other) as usize), candidate];
                        let (low, _, fits) =
                            device::encode_prefix(views[other as usize], key.as_ptr(), 2);
                        live = fits && device::contains(views[other as usize].keys, low);
                    }
                    other += 1;
                }
                if live {
                    if kept < stride {
                        *output.add((probe as u64 * stride + kept) as usize) = candidate;
                    }
                    kept += 1;
                }
                ordinal += 1;
            }
            *found.add(probe as usize) = kept;
        }
    }
}

/// One two-level side: its occurrences `(prefix, candidate)` in publication
/// order, published as one run per batch.
struct SideRows {
    batches: Vec<Vec<(i64, i64)>>,
}

impl SideRows {
    fn rows(&self) -> impl Iterator<Item = (i64, i64)> + '_ {
        self.batches.iter().flatten().copied()
    }
    /// Distinct candidates of `prefix`, in the device enumeration order:
    /// runs oldest first, ascending keys within a run, each key once in
    /// its oldest run.
    fn candidates(&self, packing: &Packing, prefix: i64) -> Vec<i64> {
        let mut seen = BTreeSet::new();
        let mut order = Vec::new();
        for batch in &self.batches {
            let mut keys: Vec<(u128, i64)> = batch
                .iter()
                .filter(|row| row.0 == prefix)
                .map(|row| {
                    let word: u128 = packing
                        .pack(&[row.0 as i128, row.1 as i128])
                        .expect("key fits its packing");
                    (word, row.1)
                })
                .collect();
            keys.sort_unstable();
            keys.dedup();
            for (_, candidate) in keys {
                if seen.insert(candidate) {
                    order.push(candidate);
                }
            }
        }
        order
    }
    fn holds(&self, prefix: i64, candidate: i64) -> bool {
        self.rows().any(|row| row == (prefix, candidate))
    }
}

fn publish_side(pool: &std::sync::Arc<Pool>, side: &SideRows) -> Result<(Side, usize)> {
    let packing = Side::derive_packing()?;
    let mut run = DeviceRadixRun::<u128, u32, Buffer<i64>>::new(pool.clone(), params(2, 4))?;
    for batch in &side.batches {
        let keys: Vec<u128> = batch
            .iter()
            .map(|row| packing.pack(&[row.0 as i128, row.1 as i128]))
            .collect::<gpu_datalog_runtime::Result<_>>()?;
        let items: Vec<i64> = batch.iter().map(|row| row.1).collect();
        run.publish(
            &Buffer::from_host(pool.clone(), &keys)?,
            &Buffer::from_host(pool.clone(), &items)?,
            batch.len(),
        )?;
    }
    let runs = run.run_count();
    Ok((Side::publish(&run, packing)?, runs))
}

struct Smallest {
    counts: Vec<u64>,
    choices: Vec<u32>,
    found: Vec<u64>,
    output: Vec<i64>,
}

fn run_smallest(
    pool: &std::sync::Arc<Pool>,
    layouts: &[&Side],
    prefixes: &[[i64; 3]],
    stride: usize,
) -> Result<Smallest> {
    let n = prefixes.len();
    let flat: Vec<i64> = prefixes.iter().flatten().copied().collect();
    let prefixes = Buffer::from_host(pool.clone(), &flat)?;
    let counts = Buffer::<u64>::zeroed(pool.clone(), n * 3)?;
    let choices = Buffer::<u32>::zeroed(pool.clone(), n)?;
    let found = Buffer::<u64>::zeroed(pool.clone(), n)?;
    let output = Buffer::<i64>::zeroed(pool.clone(), (n * stride).max(1))?;
    let at = |index: usize| layouts[index.min(layouts.len() - 1)];
    let module = unsafe { reads::load(pool.context())? };
    module.radix_test_smallest(
        pool.stream(),
        &module.prepare_radix_test_smallest(cuda_core::LaunchConfig1D::new(
            n.div_ceil(256).max(1) as u32,
            256,
            0,
        ))?,
        at(0).view(),
        at(1).view(),
        at(2).view(),
        at(0).items_view(),
        at(1).items_view(),
        at(2).items_view(),
        layouts.len() as u32,
        prefixes.ptr(),
        n as u32,
        counts.ptr(),
        choices.ptr(),
        found.ptr(),
        output.ptr(),
        stride as u64,
    )?;
    Ok(Smallest {
        counts: counts.to_host()?,
        choices: choices.to_host()?,
        found: found.to_host()?,
        output: output.to_host()?,
    })
}

/// Compare one smallest launch with the candidate-set oracle: exact counts
/// per side, first-in-order minimum, chosen side's candidates filtered by
/// every other side, in the chosen side's enumeration order.
fn check_smallest(
    sides: &[&SideRows],
    result: &Smallest,
    prefixes: &[[i64; 3]],
    stride: usize,
) -> Result {
    let packing = Side::derive_packing()?;
    for (probe, prefix) in prefixes.iter().enumerate() {
        let candidates: Vec<Vec<i64>> = sides
            .iter()
            .enumerate()
            .map(|(index, side)| side.candidates(&packing, prefix[index]))
            .collect();
        let mut choice = 0;
        for (index, list) in candidates.iter().enumerate() {
            assert_eq!(
                result.counts[probe * 3 + index],
                list.len() as u64,
                "exact distinct count, probe {probe} side {index} prefix {}",
                prefix[index]
            );
            if list.len() < candidates[choice].len() {
                choice = index;
            }
        }
        assert_eq!(
            result.choices[probe], choice as u32,
            "first minimum side, probe {probe}"
        );
        let expected: Vec<i64> = candidates[choice]
            .iter()
            .copied()
            .filter(|&candidate| {
                sides
                    .iter()
                    .enumerate()
                    .all(|(index, side)| index == choice || side.holds(prefix[index], candidate))
            })
            .collect();
        assert!(expected.len() <= stride);
        assert_eq!(result.found[probe], expected.len() as u64, "probe {probe}");
        assert_eq!(
            &result.output[probe * stride..probe * stride + expected.len()],
            expected.as_slice(),
            "smallest answers, probe {probe}"
        );
    }
    Ok(())
}

fn random_side(random: &mut Random, batches: &[usize], prefixes: usize) -> SideRows {
    // Few prefixes and duplicated rows: heavy candidate sharing and
    // repeated (prefix, candidate) keys within and across runs.
    let heads: Vec<i64> = (0..prefixes).map(|_| random.value()).collect();
    let mut previous: Vec<(i64, i64)> = Vec::new();
    let batches = batches
        .iter()
        .map(|&size| {
            let mut batch = Vec::with_capacity(size);
            for _ in 0..size {
                let row = if !previous.is_empty() && random.below(3) == 0 {
                    previous[random.below(previous.len())]
                } else {
                    (heads[random.below(heads.len())], random.value())
                };
                batch.push(row);
                previous.push(row);
            }
            batch
        })
        .collect();
    SideRows { batches }
}

fn probe_prefixes(random: &mut Random, count: usize) -> Vec<[i64; 3]> {
    let pick = |random: &mut Random| {
        if random.below(5) == 0 {
            ABSENT[random.below(ABSENT.len())]
        } else {
            random.value()
        }
    };
    (0..count)
        .map(|_| [pick(random), pick(random), pick(random)])
        .collect()
}

#[test]
fn distinct_counts_and_enumerates_each_candidate_once_across_runs() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let mut random = Random(0x5eed_0001);
    // Decreasing batches stay unmerged under ratio 2: a key repeated in a
    // newer run counts once, in its oldest run, never a per-run sum.
    let side = random_side(&mut random, &[96, 24, 8, 2], 4);
    let (layout, runs) = publish_side(&pool, &side)?;
    assert_eq!(runs, 4, "the multirun case keeps every publication");
    let mut prefixes: Vec<[i64; 3]> = DOMAIN.iter().map(|&p| [p, 0, 0]).collect();
    prefixes.extend(ABSENT.iter().map(|&p| [p, 0, 0]));
    let stride = DOMAIN.len();
    let result = run_smallest(&pool, &[&layout], &prefixes, stride)?;
    check_smallest(&[&side], &result, &prefixes, stride)?;
    // Each probe's enumeration is duplicate-free.
    for (probe, &found) in result.found.iter().enumerate() {
        let listed = &result.output[probe * stride..probe * stride + found as usize];
        assert_eq!(
            listed.iter().collect::<BTreeSet<_>>().len(),
            listed.len(),
            "distinct_at repeats a candidate, probe {probe}"
        );
    }
    Ok(())
}

#[test]
fn random_k_way_smallest_matches_candidate_set_oracle() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    for seed in 0..6_u64 {
        let mut random = Random(0x5eed_1000 + seed);
        let k = 2 + (seed % 2) as usize;
        let shapes: [&[usize]; 3] = [&[64, 16, 4], &[40], &[30, 10, 3, 1]];
        let sides: Vec<SideRows> = (0..k)
            .map(|index| random_side(&mut random, shapes[(index + seed as usize) % 3], 3))
            .collect();
        let layouts = sides
            .iter()
            .map(|side| publish_side(&pool, side).map(|(layout, _)| layout))
            .collect::<Result<Vec<_>>>()?;
        let prefixes = probe_prefixes(&mut random, 300);
        let stride = DOMAIN.len();
        let result = run_smallest(
            &pool,
            &layouts.iter().collect::<Vec<_>>(),
            &prefixes,
            stride,
        )?;
        check_smallest(
            &sides.iter().collect::<Vec<_>>(),
            &result,
            &prefixes,
            stride,
        )?;
    }
    Ok(())
}

#[test]
fn smallest_counters_cover_ties_empty_and_absent_sides() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let tie = SideRows {
        batches: vec![vec![(1, 5), (1, 6), (1, 5), (1, 5)], vec![(1, 6), (2, 9)]],
    };
    let same = SideRows {
        batches: vec![vec![(1, 6), (1, 7)], vec![(1, 7), (1, 6)]],
    };
    let empty = SideRows {
        batches: Vec::new(),
    };
    let (tie_layout, tie_runs) = publish_side(&pool, &tie)?;
    let (same_layout, _) = publish_side(&pool, &same)?;
    let (empty_layout, empty_runs) = publish_side(&pool, &empty)?;
    assert_eq!(tie_runs, 2);
    assert_eq!(empty_runs, 0);
    let prefixes = [[1, 1, 1], [2, 1, 1], [3, 3, 3], [1, 1, 0]];
    let stride = 4;
    // Ties (2 vs 2 distinct candidates despite 5 and 4 occurrences) choose
    // the first side; an absent prefix counts zero and ends the probe.
    let result = run_smallest(&pool, &[&tie_layout, &same_layout], &prefixes, stride)?;
    assert_eq!(result.counts[0..2], [2, 2]);
    assert_eq!(result.choices[0], 0);
    assert_eq!(result.found[0], 1);
    assert_eq!(result.output[0], 6);
    assert_eq!(result.counts[3..5], [1, 2]);
    assert_eq!(result.counts[6..8], [0, 0]);
    assert_eq!(result.found[2], 0);
    check_smallest(&[&tie, &same], &result, &prefixes, stride)?;
    // An empty side is the minimum of every probe (the first, where all
    // three are empty): no answers.
    let result = run_smallest(
        &pool,
        &[&tie_layout, &same_layout, &empty_layout],
        &prefixes,
        stride,
    )?;
    assert_eq!(result.choices, [2, 2, 0, 2]);
    assert!(result.found.iter().all(|&found| found == 0));
    check_smallest(&[&tie, &same, &empty], &result, &prefixes, stride)?;
    Ok(())
}

/// Duplicate-heavy occurrence lookups through the emitted encoded probe and
/// run-local intervals equal the CPU RadixRun's stable groups.
fn check_lookup(
    pool: &std::sync::Arc<Pool>,
    layout: &Lookup,
    cpu: &RadixRun<i64, (i64, u32)>,
) -> Result {
    let mut probes: Vec<i64> = DOMAIN.to_vec();
    probes.extend(ABSENT);
    let mut bases = vec![0_u64];
    for probe in &probes {
        bases.push(bases.last().unwrap() + cpu.count(probe) as u64);
    }
    let total = *bases.last().unwrap() as usize;
    let lengths = Buffer::<u64>::zeroed(pool.clone(), probes.len())?;
    let serials = Buffer::<i64>::zeroed(pool.clone(), total.max(1))?;
    let tags = Buffer::<u32>::zeroed(pool.clone(), total.max(1))?;
    let module = unsafe { reads::load(pool.context())? };
    module.radix_test_lookup(
        pool.stream(),
        &module.prepare_radix_test_lookup(cuda_core::LaunchConfig1D::new(1, 256, 0))?,
        layout.view(),
        layout.items_view(),
        Buffer::from_host(pool.clone(), &probes)?.ptr(),
        probes.len() as u32,
        Buffer::from_host(pool.clone(), &bases)?.ptr(),
        lengths.ptr(),
        serials.ptr(),
        tags.ptr(),
    )?;
    let lengths = lengths.to_host()?;
    let serials = serials.to_host()?;
    let tags = tags.to_host()?;
    let items = cpu.items();
    for (index, probe) in probes.iter().enumerate() {
        let (begin, end) = cpu.range(probe);
        assert_eq!(lengths[index], (end - begin) as u64, "count of {probe}");
        let base = bases[index] as usize;
        let got: Vec<(i64, u32)> = (base..base + (end - begin))
            .map(|position| (serials[position], tags[position]))
            .collect();
        assert_eq!(got, items[begin..end], "stable occurrences of {probe}");
    }
    Ok(())
}

#[test]
fn duplicate_heavy_lookup_matches_cpu_radix_run() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let mut random = Random(0x5eed_2000);
    let packing = Lookup::derive_packing()?;
    let rows: Vec<(i64, (i64, u32))> = (0..3000)
        .map(|serial| {
            // Three hot keys hold most occurrences.
            let key = if random.below(4) != 0 {
                DOMAIN[random.below(3) * 5]
            } else {
                random.value()
            };
            (key, (random.value(), serial))
        })
        .collect();
    // Multi-run: decreasing batches, unmerged under ratio 2.
    let mut run =
        DeviceRadixRun::<u64, u32, (Buffer<i64>, Buffer<u32>)>::new(pool.clone(), params(2, 3))?;
    let mut cpu = RadixRun::<i64, (i64, u32)>::with_ratio([], 2);
    let mut start = 0;
    for size in [2000, 600, 300, 100] {
        let batch = &rows[start..start + size];
        start += size;
        let keys: Vec<u64> = batch
            .iter()
            .map(|row| packing.pack(&[row.0 as i128]))
            .collect::<gpu_datalog_runtime::Result<_>>()?;
        run.publish(
            &Buffer::from_host(pool.clone(), &keys)?,
            &(
                Buffer::from_host(
                    pool.clone(),
                    &batch.iter().map(|r| r.1 .0).collect::<Vec<_>>(),
                )?,
                Buffer::from_host(
                    pool.clone(),
                    &batch.iter().map(|r| r.1 .1).collect::<Vec<_>>(),
                )?,
            ),
            size,
        )?;
        cpu.merge(batch.iter().copied());
    }
    assert!(run.run_count() > 1);
    check_lookup(&pool, &Lookup::publish(&run, packing)?, &cpu)
}

#[test]
fn source_build_selects_and_groups_resident_columns() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let mut random = Random(0x5eed_3000);
    let rows = 2500;
    let keys: Vec<i64> = (0..rows).map(|_| DOMAIN[random.below(4) * 3]).collect();
    let serials: Vec<i64> = (0..rows).map(|_| random.value()).collect();
    let tags: Vec<i64> = (0..rows as i64).collect();
    let key_column = Buffer::from_host(pool.clone(), &bits(&keys))?;
    let serial_column = Buffer::from_host(pool.clone(), &bits(&serials))?;
    let tag_column = Buffer::from_host(pool.clone(), &bits(&tags))?;
    let sources = [plain(&key_column, rows)];
    let values = [plain(&serial_column, rows), plain(&tag_column, rows)];
    // Whole sources.
    let layout = unsafe {
        Lookup::from_sources(
            pool.clone(),
            params(4, 4),
            &sources,
            &values,
            std::ptr::null(),
            rows,
        )?
    };
    assert_eq!(layout.len(), rows);
    let cpu = RadixRun::<i64, (i64, u32)>::from_pairs(
        (0..rows).map(|row| (keys[row], (serials[row], tags[row] as u32))),
    );
    check_lookup(&pool, &layout, &cpu)?;
    // A selection reads only its positions, in selection order.
    let selected: Vec<u32> = (0..rows as u32).filter(|row| row % 7 != 3).rev().collect();
    let positions = Buffer::from_host(pool.clone(), &selected)?;
    let layout = unsafe {
        Lookup::from_sources(
            pool.clone(),
            params(4, 0),
            &sources,
            &values,
            positions.ptr(),
            selected.len(),
        )?
    };
    assert_eq!(layout.len(), selected.len());
    let cpu = RadixRun::<i64, (i64, u32)>::from_pairs(selected.iter().map(|&row| {
        let row = row as usize;
        (keys[row], (serials[row], tags[row] as u32))
    }));
    check_lookup(&pool, &layout, &cpu)?;
    // Zero rows publish an empty layout; schema arity is checked.
    let empty = unsafe {
        Lookup::from_sources(
            pool.clone(),
            params(4, 4),
            &sources,
            &values,
            std::ptr::null(),
            0,
        )?
    };
    assert_eq!(empty.run_count(), 0);
    assert!(unsafe {
        Lookup::from_sources(
            pool.clone(),
            params(4, 4),
            &sources,
            &values[..1],
            std::ptr::null(),
            rows,
        )
    }
    .is_err());
    Ok(())
}
