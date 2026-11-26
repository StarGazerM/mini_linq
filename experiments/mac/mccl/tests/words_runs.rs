//! Word-run item columns, merges and `WordRunSet` publication against a host
//! oracle: stable sort of row indices by full multi-word key, grouping, the
//! directory definition over 512-bit integers, and CUDA's ratio law
//! (`DeviceRadixRun::publish`) replayed on host run lengths.

use std::rc::Rc;

use mccl::{
    Gpu, Result,
    words::{
        ItemColumn, ItemType, MAX_ITEM_COLUMNS, OffsetWidth, WordKeys, WordRun, WordRunParams,
        WordRunSet,
    },
};

/// Little-endian 512-bit value of one key, whatever its lane layout.
fn value(key: &[u64], ms_first: bool) -> [u64; 8] {
    let mut out = [0; 8];
    for (lane, slot) in out.iter_mut().enumerate().take(key.len()) {
        *slot = if ms_first {
            key[key.len() - 1 - lane]
        } else {
            key[lane]
        };
    }
    out
}

fn compare(a: &[u64; 8], b: &[u64; 8]) -> std::cmp::Ordering {
    a.iter().rev().cmp(b.iter().rev())
}

fn subtract(a: [u64; 8], b: [u64; 8]) -> [u64; 8] {
    let (mut out, mut borrow) = ([0; 8], false);
    for i in 0..8 {
        let (x, o1) = a[i].overflowing_sub(b[i]);
        let (y, o2) = x.overflowing_sub(borrow as u64);
        out[i] = y;
        borrow = o1 || o2;
    }
    out
}

fn bit_length(a: [u64; 8]) -> u32 {
    (0..8)
        .rev()
        .find(|&i| a[i] != 0)
        .map_or(0, |i| i as u32 * 64 + 64 - a[i].leading_zeros())
}

fn shift_right(a: [u64; 8], shift: u32) -> u128 {
    let mut wide = 0u128;
    for bit in 0..128u32 {
        let source = shift + bit;
        if source < 512 && (a[(source / 64) as usize] >> (source % 64)) & 1 == 1 {
            wide |= 1 << bit;
        }
    }
    wide
}

/// Host item columns of one delta: one per supported type.
#[derive(Clone, Debug, Default, PartialEq)]
struct Items {
    a: Vec<u32>,
    b: Vec<i32>,
    c: Vec<u64>,
    d: Vec<i64>,
}

const TYPES: [ItemType; 4] = [ItemType::U32, ItemType::I32, ItemType::U64, ItemType::I64];

impl Items {
    fn new(rows: usize, seed: u64) -> Self {
        let mut state = seed | 1;
        let mut items = Self::default();
        for row in 0..rows {
            let random = xorshift(&mut state);
            items.a.push(row as u32 ^ (random as u32 & 0xffff_0000));
            items.b.push(-(row as i32) ^ (random >> 40) as i32);
            items.c.push(random);
            items.d.push(i64::MIN + (random >> 3) as i64);
        }
        items
    }

    fn extend(&mut self, other: &Self) {
        self.a.extend_from_slice(&other.a);
        self.b.extend_from_slice(&other.b);
        self.c.extend_from_slice(&other.c);
        self.d.extend_from_slice(&other.d);
    }

    fn upload(&self, gpu: &Gpu) -> Result<[ItemColumn; 4]> {
        Ok([
            gpu.upload_items(&self.a)?,
            gpu.upload_items(&self.b)?,
            gpu.upload_items(&self.c)?,
            gpu.upload_items(&self.d)?,
        ])
    }

    fn read(run: &WordRun) -> Result<Self> {
        Ok(Self {
            a: run.items::<u32>(0)?.to_vec(),
            b: run.items::<i32>(1)?.to_vec(),
            c: run.items::<u64>(2)?.to_vec(),
            d: run.items::<i64>(3)?.to_vec(),
        })
    }

    fn permute(&self, order: &[u32]) -> Self {
        let pick = |i: &u32| *i as usize;
        Self {
            a: order.iter().map(|i| self.a[pick(i)]).collect(),
            b: order.iter().map(|i| self.b[pick(i)]).collect(),
            c: order.iter().map(|i| self.c[pick(i)]).collect(),
            d: order.iter().map(|i| self.d[pick(i)]).collect(),
        }
    }
}

fn refs(columns: &[ItemColumn]) -> Vec<&ItemColumn> {
    columns.iter().collect()
}

/// Every published array of a run, copied to the host.
#[derive(Debug, PartialEq)]
struct HostRun {
    keys: Vec<u64>,
    offsets: Vec<u64>,
    order: Vec<u32>,
    directory: Vec<u64>,
    items: Option<Items>,
}

impl HostRun {
    fn of(run: &WordRun) -> Result<Self> {
        let offsets = run.offsets();
        let directory = run.directory();
        Ok(Self {
            keys: run.keys().to_vec(),
            offsets: (0..offsets.len()).map(|i| offsets.get(i)).collect(),
            order: run.order().to_vec(),
            directory: (0..directory.len()).map(|i| directory.get(i)).collect(),
            items: if run.item_types().is_empty() {
                None
            } else {
                Some(Items::read(run)?)
            },
        })
    }
}

/// Oracle run of `keys` (whole keys of `params.lanes`) and optional items.
fn oracle(keys: &[u64], items: Option<&Items>, params: WordRunParams) -> HostRun {
    let lanes = params.lanes;
    let rows = keys.len() / lanes;
    let values: Vec<_> = keys
        .chunks(lanes)
        .map(|k| value(k, params.ms_first))
        .collect();
    let mut order: Vec<u32> = (0..rows as u32).collect();
    order.sort_by(|&a, &b| compare(&values[a as usize], &values[b as usize]));
    let (mut distinct, mut offsets, mut groups) = (Vec::new(), Vec::new(), Vec::new());
    for (position, &row) in order.iter().enumerate() {
        if position == 0 || values[order[position - 1] as usize] != values[row as usize] {
            distinct.extend_from_slice(&keys[row as usize * lanes..][..lanes]);
            offsets.push(position as u64);
            groups.push(values[row as usize]);
        }
    }
    offsets.push(rows as u64);
    let bits = params.radix_bits;
    let directory = if bits == 0 {
        Vec::new()
    } else {
        let (min, max) = (groups[0], groups[groups.len() - 1]);
        let shift = bit_length(subtract(max, min)).saturating_sub(bits);
        (0..(1u128 << bits) + 1)
            .map(|bucket| {
                groups.partition_point(|k| shift_right(subtract(*k, min), shift) < bucket) as u64
            })
            .collect()
    };
    HostRun {
        keys: distinct,
        offsets,
        items: items.map(|items| items.permute(&order)),
        order,
        directory,
    }
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Duplicate-heavy keys with full-domain extremes in every lane.
fn keys(rows: usize, lanes: usize, seed: u64) -> Vec<u64> {
    let mut state = seed | 1;
    (0..rows * lanes)
        .map(|i| match xorshift(&mut state) % 8 {
            0 => 0,
            1 => u64::MAX,
            2 | 3 => (i / lanes % 5) as u64,
            _ => xorshift(&mut state) >> (i % 3 * 20),
        })
        .collect()
}

fn params(lanes: usize, ms_first: bool, radix_bits: u32, offsets: OffsetWidth) -> WordRunParams {
    WordRunParams {
        lanes,
        ms_first,
        active_bits: lanes as u32 * 64,
        radix_bits,
        offsets,
    }
}

fn build(gpu: &Gpu, keys: &[u64], items: &Items, params: WordRunParams) -> Result<WordRun> {
    let resident = gpu.upload_word_keys(keys, params.lanes)?;
    let columns = items.upload(gpu)?;
    Ok(gpu
        .build_word_run_from(&resident, &refs(&columns), params)?
        .expect("nonempty build publishes a run"))
}

const LAYOUTS: [(usize, bool); 4] = [(1, false), (2, false), (3, true), (8, true)];

#[test]
fn item_columns_follow_the_stable_order() -> Result<()> {
    let gpu = Gpu::new()?;
    for (lanes, ms_first) in LAYOUTS {
        for rows in [1, 2, 255, 257, 4099, 20_001] {
            for (bits, width) in [(0, OffsetWidth::U32), (6, OffsetWidth::U64)] {
                let params = params(lanes, ms_first, bits, width);
                let seed = 0x2545_f491_4f6c_dd1d ^ (rows * 13 + lanes) as u64;
                let (keys, items) = (keys(rows, lanes, seed), Items::new(rows, seed));
                let run = build(&gpu, &keys, &items, params)?;
                assert_eq!(run.item_types(), TYPES);
                assert_eq!(
                    HostRun::of(&run)?,
                    oracle(&keys, Some(&items), params),
                    "{params:?} rows {rows}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn resident_and_host_key_builds_agree() -> Result<()> {
    let gpu = Gpu::new()?;
    let params = params(2, false, 8, OffsetWidth::U32);
    let keys = keys(10_000, 2, 77);
    let host = gpu.build_word_run(&keys, params)?.expect("run");
    let resident = gpu.upload_word_keys(&keys, 2)?;
    assert_eq!(resident.len(), 10_000);
    let built = gpu
        .build_word_run_from(&resident, &[], params)?
        .expect("run");
    // The resident keys stay usable for further builds.
    let again = gpu
        .build_word_run_from(&resident, &[], params)?
        .expect("run");
    assert!(built.item_types().is_empty());
    assert_eq!(HostRun::of(&host)?, HostRun::of(&built)?);
    assert_eq!(HostRun::of(&again)?, oracle(&keys, None, params));
    Ok(())
}

/// Keys of `rows` rows from `lane_value(row, random, lane)`, lane 0 least
/// significant, stored in the layout's memory order.
fn layout_keys(
    rows: usize,
    lanes: usize,
    ms_first: bool,
    lane_value: impl Fn(usize, u64, usize) -> u64,
) -> Vec<u64> {
    let mut state = 0x9e37_79b9_7f4a_7c15 ^ (rows * 31 + lanes) as u64;
    let mut out = Vec::with_capacity(rows * lanes);
    for row in 0..rows {
        let random = xorshift(&mut state);
        for word in 0..lanes {
            let lane = if ms_first { lanes - 1 - word } else { word };
            out.push(lane_value(row, random, lane));
        }
    }
    out
}

/// The GPU sort plan: bytes constant over all rows and constant lanes are
/// skipped, spans straddling byte boundaries sort relative to the minimum,
/// and active-bit windows bound the passes. Every case, over several tiles
/// and a single row, publishes exactly the oracle run.
#[test]
fn sort_plan_edges_match_the_oracle() -> Result<()> {
    type LaneValue = fn(usize, u64, usize) -> u64;
    let cases: [(&str, LaneValue); 7] = [
        // Bytes 0 and 5 vary; 4096 distinct values per lane.
        ("constant bytes", |_, x, _| {
            0xab00_cd00_0000_0000 | (x & 0xff) | (x >> 8 & 0xf) << 40
        }),
        ("all equal", |_, _, lane| {
            0x0123_4567_89ab_cdef ^ lane as u64
        }),
        (
            "high lane only",
            |_, x, lane| if lane == 0 { 0x5555 } else { x % 1000 },
        ),
        (
            "low lane only",
            |_, x, lane| if lane == 0 { x % 1000 } else { 7 },
        ),
        // 5 raw varying bytes, a 2-byte span above the minimum.
        ("straddles 2^32", |_, x, _| (1 << 32) - 500 + x % 1000),
        // Straddles 2^56 in the low lane only; distinct high lanes by row.
        ("straddles 2^56", |row, x, lane| match lane {
            0 => (1 << 56) - 3 + x % 7,
            _ => (row / 3) as u64,
        }),
        // Extremes around the full lane range in every byte.
        ("extremes", |_, x, _| match x % 4 {
            0 => 0,
            1 => u64::MAX,
            2 => u64::MAX - x % 300,
            _ => x % 300,
        }),
    ];
    let gpu = Gpu::new()?;
    for (lanes, ms_first) in LAYOUTS {
        for rows in [1, 3 * 2048 + 77] {
            for (name, lane_value) in cases {
                let keys = layout_keys(rows, lanes, ms_first, lane_value);
                let params = params(lanes, ms_first, 8, OffsetWidth::U32);
                let run = gpu.build_word_run(&keys, params)?.expect("run");
                assert_eq!(
                    HostRun::of(&run)?,
                    oracle(&keys, None, params),
                    "{name} {params:?} rows {rows}"
                );
            }
            // Narrow windows: the top lane keeps its low `top` bits.
            for top in [0, 12, 64] {
                let keys = layout_keys(rows, lanes, ms_first, |_, x, lane| {
                    if lane + 1 < lanes {
                        x.rotate_left(lane as u32 * 7)
                    } else if top == 64 {
                        x
                    } else {
                        x & ((1 << top) - 1)
                    }
                });
                let params = WordRunParams {
                    active_bits: (lanes as u32 - 1) * 64 + top,
                    ..params(lanes, ms_first, 8, OffsetWidth::U32)
                };
                let run = gpu.build_word_run(&keys, params)?.expect("run");
                assert_eq!(
                    HostRun::of(&run)?,
                    oracle(&keys, None, params),
                    "window {params:?} rows {rows}"
                );
            }
        }
    }
    Ok(())
}

/// Older and newer key sets with overlap, disjoint ranges and extremes.
fn merge_cases(lanes: usize) -> Vec<(Vec<u64>, Vec<u64>)> {
    let shifted = |keys: Vec<u64>, by: u64| -> Vec<u64> {
        keys.into_iter()
            .map(|k| k.saturating_add(by) | 1u64 << 63)
            .collect()
    };
    let mut cases = Vec::new();
    for (older, newer) in [(1, 1), (7, 300), (300, 7), (2048, 127), (20_001, 2000)] {
        let seed = (older * 7 + newer * 3 + lanes) as u64;
        cases.push((keys(older, lanes, seed), keys(newer, lanes, seed ^ 0xff)));
    }
    let low: Vec<u64> = (0..3000u64).flat_map(|i| vec![i / 4; lanes]).collect();
    let high = shifted(low.clone(), 10);
    cases.push((low.clone(), high.clone())); // older entirely below newer
    cases.push((high, low.clone())); // older entirely above newer
    cases.push((low.clone(), low)); // identical key sets
    cases.push((vec![u64::MAX; 64 * lanes], vec![u64::MAX; 5 * lanes]));
    cases
}

#[test]
fn merges_equal_a_fresh_build_of_the_concatenation() -> Result<()> {
    let gpu = Gpu::new()?;
    for (lanes, ms_first) in LAYOUTS {
        for (bits, width) in [
            (0, OffsetWidth::U64),
            (5, OffsetWidth::U32),
            (12, OffsetWidth::U64),
        ] {
            let params = params(lanes, ms_first, bits, width);
            for (case, (older_keys, newer_keys)) in merge_cases(lanes).into_iter().enumerate() {
                let older_items = Items::new(older_keys.len() / lanes, case as u64 + 5);
                let newer_items = Items::new(newer_keys.len() / lanes, case as u64 + 9);
                let older = build(&gpu, &older_keys, &older_items, params)?;
                let newer = build(&gpu, &newer_keys, &newer_items, params)?;
                let merged = gpu.merge_word_runs(&older, &newer)?;
                let mut keys = older_keys.clone();
                keys.extend_from_slice(&newer_keys);
                let mut items = older_items.clone();
                items.extend(&newer_items);
                let fresh = build(&gpu, &keys, &items, params)?;
                let merged = HostRun::of(&merged)?;
                assert_eq!(merged, HostRun::of(&fresh)?, "{params:?} case {case}");
                assert_eq!(merged, oracle(&keys, Some(&items), params));
            }
        }
    }
    Ok(())
}

#[test]
fn itemless_merges_keep_the_rebased_order() -> Result<()> {
    let gpu = Gpu::new()?;
    let params = params(1, false, 4, OffsetWidth::U32);
    let (a, b) = (keys(5000, 1, 3), keys(700, 1, 4));
    let older = gpu.build_word_run(&a, params)?.expect("run");
    let newer = gpu.build_word_run(&b, params)?.expect("run");
    let merged = gpu.merge_word_runs(&older, &newer)?;
    let all: Vec<u64> = a.iter().chain(&b).copied().collect();
    assert_eq!(HostRun::of(&merged)?, oracle(&all, None, params));
    Ok(())
}

/// CUDA's publication law on host run lengths: each run is the
/// concatenation of its deltas, oldest first.
struct Model {
    ratio: usize,
    runs: Vec<(Vec<u64>, Items)>,
}

impl Model {
    fn publish(&mut self, keys: &[u64], items: &Items) {
        if keys.is_empty() {
            return;
        }
        let (mut merged_keys, mut merged_items) = (keys.to_vec(), items.clone());
        while let Some((older_keys, older_items)) = self.runs.last() {
            if older_keys.len() / self.ratio >= merged_keys.len() {
                break;
            }
            let (mut k, mut i) = (older_keys.clone(), older_items.clone());
            k.extend_from_slice(&merged_keys);
            i.extend(&merged_items);
            (merged_keys, merged_items) = (k, i);
            self.runs.pop();
        }
        self.runs.push((merged_keys, merged_items));
    }

    fn lengths(&self) -> Vec<(usize, usize)> {
        self.runs
            .iter()
            .map(|(keys, _)| {
                let mut distinct = keys.clone();
                distinct.sort_unstable();
                distinct.dedup();
                (distinct.len(), keys.len())
            })
            .collect()
    }
}

/// CUDA test `u64_publication_matches_cpu_radix_run_across_ratio_transitions`.
#[test]
fn publication_follows_the_cuda_ratio_law() -> Result<()> {
    let gpu = Gpu::new()?;
    let input: Vec<u64> = (0u64..3001).map(|i| ((i * 7919) % 4099) * 3 + 5).collect();
    let mut items = Items::new(input.len(), 0);
    items.a = (0..input.len() as u32).map(|i| i ^ 0x5555_aaaa).collect();
    for (ratio, radix_bits) in [(2, 12), (4, 4), (8, 0), (4, 16)] {
        for batch in [7, 127, 2048, input.len()] {
            let params = params(1, false, radix_bits, OffsetWidth::U32);
            let mut set = WordRunSet::new(&gpu, params, ratio, &TYPES)?;
            let mut model = Model {
                ratio,
                runs: Vec::new(),
            };
            let (mut merges, mut checked_runs) = (0, 0);
            let chunks: Vec<_> = (0..input.len()).step_by(batch).collect();
            for (index, &start) in chunks.iter().enumerate() {
                let end = (start + batch).min(input.len());
                let delta = Items {
                    a: items.a[start..end].to_vec(),
                    b: items.b[start..end].to_vec(),
                    c: items.c[start..end].to_vec(),
                    d: items.d[start..end].to_vec(),
                };
                let before = set.run_count();
                let keys = gpu.upload_word_keys(&input[start..end], 1)?;
                set.publish(&keys, &refs(&delta.upload(&gpu)?))?;
                model.publish(&input[start..end], &delta);
                let lengths = set.run_lengths();
                assert_eq!(
                    lengths,
                    model.lengths(),
                    "ratio {ratio} batch {batch} publish {index}"
                );
                for pair in lengths.windows(2) {
                    assert!(pair[0].1 / ratio >= pair[1].1, "ratio law: {lengths:?}");
                }
                merges += usize::from(set.run_count() <= before);
                if index % 37 != 0 && index + 1 != chunks.len() {
                    continue;
                }
                checked_runs = checked_runs.max(set.run_count());
                for (run, (keys, items)) in set.runs().iter().zip(&model.runs) {
                    assert_eq!(HostRun::of(run)?, oracle(keys, Some(items), params));
                }
            }
            if batch <= 127 {
                assert!(merges > 0, "ratio transitions must merge runs");
            }
            if batch == 7 {
                assert!(checked_runs >= 3, "several unmerged runs must be checked");
            }
        }
    }
    Ok(())
}

#[test]
fn wide_publication_follows_the_ratio_law() -> Result<()> {
    let gpu = Gpu::new()?;
    for (lanes, ms_first) in [(2, false), (3, true)] {
        let params = params(lanes, ms_first, 7, OffsetWidth::U64);
        let mut set = WordRunSet::new(&gpu, params, 2, &[])?;
        let mut model = Model {
            ratio: 2,
            runs: Vec::new(),
        };
        for round in 0..40usize {
            let rows = 1 + round * 37 % 300;
            let delta = keys(rows, lanes, round as u64 * 31 + lanes as u64);
            set.publish(&gpu.upload_word_keys(&delta, lanes)?, &[])?;
            model.publish(&delta, &Items::default());
            assert_eq!(set.run_count(), model.runs.len());
            for (run, (keys, _)) in set.runs().iter().zip(&model.runs) {
                assert_eq!(HostRun::of(run)?, oracle(keys, None, params));
            }
        }
    }
    Ok(())
}

/// CUDA test `snapshots_retain_runs_across_later_merges`.
#[test]
fn snapshots_retain_runs_across_later_publishes_and_owner_drop() -> Result<()> {
    let gpu = Gpu::new()?;
    let params = params(1, false, 6, OffsetWidth::U32);
    let mut set = WordRunSet::new(&gpu, params, 2, &TYPES)?;
    let mut state = 0xabcdef;
    let mut retained = Vec::new();
    let mut ids = Vec::new();
    for round in 0..12usize {
        let keys: Vec<u64> = (0..(round + 1) * 40)
            .map(|_| xorshift(&mut state) % 300)
            .collect();
        let items = Items::new(keys.len(), round as u64 + 1);
        set.publish(
            &gpu.upload_word_keys(&keys, 1)?,
            &refs(&items.upload(&gpu)?),
        )?;
        let snapshot = set.snapshot();
        assert!(Rc::ptr_eq(&snapshot, &set.snapshot()));
        assert_eq!(snapshot.id(), set.snapshot().id(), "stable until publish");
        assert!(!ids.contains(&snapshot.id()), "unique ids");
        ids.push(snapshot.id());
        assert_eq!(snapshot.run_lengths(), set.run_lengths());
        let copy: Vec<HostRun> = snapshot
            .runs()
            .iter()
            .map(HostRun::of)
            .collect::<Result<_>>()?;
        retained.push((snapshot, copy));
    }
    drop(set);
    drop(gpu);
    for (snapshot, copy) in retained {
        let now: Vec<HostRun> = snapshot
            .runs()
            .iter()
            .map(HostRun::of)
            .collect::<Result<_>>()?;
        assert_eq!(now, copy, "snapshot {}", snapshot.id());
    }
    Ok(())
}

#[test]
fn empty_deltas_and_failed_builds_leave_runs_unchanged() -> Result<()> {
    let gpu = Gpu::new()?;
    let params = params(2, false, 5, OffsetWidth::U32);
    let mut set = WordRunSet::new(&gpu, params, 4, &[ItemType::U64])?;
    let empty_keys = gpu.upload_word_keys(&[], 2)?;
    let empty_items = gpu.upload_items::<u64>(&[])?;
    assert!(empty_keys.is_empty());
    let initial = set.snapshot();
    set.publish(&empty_keys, &[&empty_items])?;
    assert_eq!(set.run_count(), 0);
    assert_eq!(set.snapshot().id(), initial.id());
    assert!(
        gpu.build_word_run_from(&empty_keys, &[&empty_items], params)?
            .is_none()
    );

    let keys = gpu.upload_word_keys(&self::keys(500, 2, 1), 2)?;
    let items = gpu.upload_items(&(0..500u64).collect::<Vec<_>>())?;
    set.publish(&keys, &[&items])?;
    let published = set.snapshot();
    let lengths = set.run_lengths();
    set.publish(&empty_keys, &[&empty_items])?;
    assert_eq!(
        set.snapshot().id(),
        published.id(),
        "empty delta is a no-op"
    );
    // Failed builds: mismatched rows, types, arity and lanes.
    let short = gpu.upload_items(&[1u64, 2])?;
    let signed = gpu.upload_items(&vec![0i64; 500])?;
    let narrow = gpu.upload_word_keys(&self::keys(500, 1, 2), 1)?;
    assert!(set.publish(&keys, &[&short]).is_err());
    assert!(set.publish(&keys, &[&signed]).is_err());
    assert!(set.publish(&keys, &[]).is_err());
    assert!(set.publish(&narrow, &[&items]).is_err());
    assert_eq!(set.snapshot().id(), published.id());
    assert_eq!(set.run_lengths(), lengths);
    Ok(())
}

/// CUDA test `offset_carriers_are_type_facts_with_exact_bounds`.
#[test]
fn offset_carriers_are_exact_bounds() -> Result<()> {
    assert!(OffsetWidth::U32.fits(u32::MAX as usize));
    assert!(!OffsetWidth::U32.fits(u32::MAX as usize + 1));
    assert!(OffsetWidth::U64.fits(u32::MAX as usize + 1));
    assert!(OffsetWidth::U64.fits(usize::MAX));
    let gpu = Gpu::new()?;
    let mut state = 99;
    let keys: Vec<u64> = (0..5000).map(|_| xorshift(&mut state) % 1500).collect();
    let items = Items::new(keys.len(), 99);
    let narrow_params = params(1, false, 10, OffsetWidth::U32);
    let wide_params = params(1, false, 10, OffsetWidth::U64);
    let mut narrow = WordRunSet::new(&gpu, narrow_params, 4, &TYPES)?;
    let mut wide = WordRunSet::new(&gpu, wide_params, 4, &TYPES)?;
    for start in (0..keys.len()).step_by(611) {
        let end = (start + 611).min(keys.len());
        let delta = Items {
            a: items.a[start..end].to_vec(),
            b: items.b[start..end].to_vec(),
            c: items.c[start..end].to_vec(),
            d: items.d[start..end].to_vec(),
        };
        let resident = gpu.upload_word_keys(&keys[start..end], 1)?;
        let columns = delta.upload(&gpu)?;
        narrow.publish(&resident, &refs(&columns))?;
        wide.publish(&resident, &refs(&columns))?;
    }
    assert_eq!(narrow.run_lengths(), wide.run_lengths());
    for (n, w) in narrow.runs().iter().zip(wide.runs()) {
        assert!(matches!(n.offsets(), mccl::words::Offsets::U32(_)));
        assert!(matches!(w.offsets(), mccl::words::Offsets::U64(_)));
        assert_eq!(HostRun::of(n)?, HostRun::of(w)?);
    }
    Ok(())
}

#[test]
fn invalid_inputs_are_rejected() -> Result<()> {
    let gpu = Gpu::new()?;
    let base = params(1, false, 4, OffsetWidth::U32);
    let keys = gpu.upload_word_keys(&[3, 1, 2], 1)?;
    let wide_keys: WordKeys = gpu.upload_word_keys(&[3, 1, 2, 0], 2)?;
    assert!(gpu.upload_word_keys(&[1, 2, 3], 2).is_err(), "partial key");
    assert!(gpu.upload_word_keys(&[1; 9], 9).is_err(), "nine lanes");
    assert!(
        gpu.build_word_run_from(&wide_keys, &[], base).is_err(),
        "lanes"
    );
    let column = gpu.upload_items(&[1u32, 2, 3])?;
    let too_many: Vec<&ItemColumn> = vec![&column; MAX_ITEM_COLUMNS + 1];
    assert!(gpu.build_word_run_from(&keys, &too_many, base).is_err());
    let twelve: Vec<&ItemColumn> = vec![&column; MAX_ITEM_COLUMNS];
    let run = gpu.build_word_run_from(&keys, &twelve, base)?.expect("run");
    assert_eq!(run.items::<u32>(11)?, [2, 3, 1]);
    assert!(run.items::<i32>(0).is_err(), "typed access is exact");
    assert!(run.items::<u32>(12).is_err(), "column out of range");
    // Merges need identical params and item types.
    let plain = gpu.build_word_run_from(&keys, &[], base)?.expect("run");
    assert!(gpu.merge_word_runs(&run, &plain).is_err());
    let other = gpu
        .build_word_run_from(
            &keys,
            &[],
            WordRunParams {
                radix_bits: 5,
                ..base
            },
        )?
        .expect("run");
    assert!(gpu.merge_word_runs(&plain, &other).is_err());
    let foreign = Gpu::new()?;
    assert!(foreign.build_word_run_from(&keys, &[], base).is_err());
    assert!(foreign.merge_word_runs(&plain, &plain).is_err());
    assert!(
        WordRunSet::new(&gpu, base, 1, &[]).is_err(),
        "ratio below two"
    );
    assert!(WordRunSet::new(&gpu, base, 2, &[ItemType::U32; 13]).is_err());
    // A run merged with itself doubles every group, older copy first.
    let doubled = gpu.merge_word_runs(&plain, &plain)?;
    assert_eq!(doubled.order(), [1, 4, 2, 5, 0, 3]);
    Ok(())
}

/// SplitMix64 finalizer: spreads small ids over all 64 bits.
fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// `random`: uniform 64-bit lanes. `dup16`: rows/16 distinct keys, each lane
/// a hash of the key id (about 16 occurrences per key).
fn timing_keys(rows: usize, lanes: usize, duplicates: bool, seed: u64) -> Vec<u64> {
    let mut state = seed | 1;
    let distinct = (rows / 16).max(1) as u64;
    let mut out = Vec::with_capacity(rows * lanes);
    for _ in 0..rows {
        let id = xorshift(&mut state);
        for lane in 0..lanes {
            out.push(if duplicates {
                mix((id % distinct) * 8 + lane as u64)
            } else {
                xorshift(&mut state)
            });
        }
    }
    out
}

/// 2 untimed warmups, then the median of 7 back-to-back samples.
fn median_ms(mut run: impl FnMut() -> Result<()>) -> Result<f64> {
    for _ in 0..2 {
        run()?;
    }
    let mut samples = Vec::with_capacity(7);
    for _ in 0..7 {
        let start = std::time::Instant::now();
        run()?;
        samples.push(start.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(f64::total_cmp);
    Ok(samples[3])
}

fn report(case: &str, rows: usize, ms: f64) {
    println!(
        "{case:<36} {rows:>10} {ms:>10.2} ms {:>8.1} Mrows/s",
        rows as f64 / ms / 1e3
    );
}

/// Warm builds from resident keys (no host copy timed) and 10% merges,
/// against the u32 radix index at the same rows. Synchronous calls, so wall
/// time covers the GPU work and the count readback. Rows from
/// `WORD_RUN_ROWS` (comma list; default 1M,16M,60M; merges at <= 16M):
/// `cargo test --release -p mccl --test words_runs -- --ignored --nocapture`
#[test]
#[ignore = "timing; run explicitly"]
fn word_run_timing() -> Result<()> {
    use mccl::relation::{IndexSpec, Payload};
    let rows: Vec<usize> = std::env::var("WORD_RUN_ROWS").map_or_else(
        |_| vec![1 << 20, 16 << 20, 60 << 20],
        |list| list.split(',').map(|n| n.parse().expect("rows")).collect(),
    );
    let merges = |n: usize| n <= 16 << 20;
    let params = |lanes: usize| params(lanes, false, 16, OffsetWidth::U32);
    let gpu = Gpu::new()?;
    println!(
        "{:<36} {:>10} {:>13} {:>15}",
        "case", "rows", "median", "throughput"
    );
    for &n in &rows {
        for lanes in [1, 2] {
            for duplicates in [false, true] {
                let kind = if duplicates { "dup16" } else { "random" };
                let host = timing_keys(n, lanes, duplicates, 0x9e37_79b9 ^ n as u64);
                let resident = gpu.upload_word_keys(&host, lanes)?;
                drop(host);
                let ms = median_ms(|| {
                    gpu.build_word_run_from(&resident, &[], params(lanes))?;
                    Ok(())
                })?;
                report(&format!("word build {lanes}L {kind}"), n, ms);
                if !merges(n) {
                    continue;
                }
                let older = gpu
                    .build_word_run_from(&resident, &[], params(lanes))?
                    .expect("run");
                drop(resident);
                let delta = timing_keys(n / 10, lanes, duplicates, 0x51ed ^ n as u64);
                let newer = gpu.build_word_run(&delta, params(lanes))?.expect("run");
                let ms = median_ms(|| {
                    gpu.merge_word_runs(&older, &newer)?;
                    Ok(())
                })?;
                report(&format!("word merge +10% {lanes}L {kind}"), n + n / 10, ms);
            }
        }
        // Narrow keys: 2^24 values at 0 (3 varying bytes) and straddling
        // 2^32 (5 varying bytes, a 3-byte span above the minimum).
        for (kind, origin) in [
            ("ids<2^24", 0),
            ("2^24 span @2^32", (1u64 << 32) - (1 << 23)),
        ] {
            let mut state = 0x2545_f491 ^ n as u64;
            let host: Vec<u64> = (0..n)
                .map(|_| origin + xorshift(&mut state) % (1 << 24))
                .collect();
            let resident = gpu.upload_word_keys(&host, 1)?;
            let ms = median_ms(|| {
                gpu.build_word_run_from(&resident, &[], params(1))?;
                Ok(())
            })?;
            report(&format!("word build 1L {kind}"), n, ms);
        }
        if merges(n) {
            // One u32 and one u64 item column through build and merge.
            let host = timing_keys(n, 1, false, 7 ^ n as u64);
            let resident = gpu.upload_word_keys(&host, 1)?;
            let a = gpu.upload_items(&(0..n as u32).collect::<Vec<_>>())?;
            let b = gpu.upload_items(&host)?;
            let ms = median_ms(|| {
                gpu.build_word_run_from(&resident, &[&a, &b], params(1))?;
                Ok(())
            })?;
            report("word build 1L random +u32+u64", n, ms);
            let older = gpu
                .build_word_run_from(&resident, &[&a, &b], params(1))?
                .expect("run");
            let delta = timing_keys(n / 10, 1, false, 11 ^ n as u64);
            let delta_rows = gpu.upload_items(&(0..delta.len() as u32).collect::<Vec<_>>())?;
            let delta_values = gpu.upload_items(&delta)?;
            let newer = gpu
                .build_word_run_from(
                    &gpu.upload_word_keys(&delta, 1)?,
                    &[&delta_rows, &delta_values],
                    params(1),
                )?
                .expect("run");
            let ms = median_ms(|| {
                gpu.merge_word_runs(&older, &newer)?;
                Ok(())
            })?;
            report("word merge +10% 1L random +u32+u64", n + n / 10, ms);
        }
        // Baseline: u32 keys with row-id payloads.
        let spec = IndexSpec {
            key_column: 0,
            payload: Payload::RowId,
        };
        let mut state = 0x1234_5678 ^ n as u64;
        let relation = gpu.relation_u32::<2>(n, |row| [xorshift(&mut state) as u32, row as u32])?;
        let ms = median_ms(|| {
            gpu.build_radix_index(&relation, spec, None)?;
            Ok(())
        })?;
        report("u32 build_radix_index random", n, ms);
        if merges(n) {
            let older = gpu.build_radix_index(&relation, spec, None)?;
            drop(relation);
            let delta =
                gpu.relation_u32::<2>(n / 10, |row| [xorshift(&mut state) as u32, row as u32])?;
            let newer = gpu.build_radix_index(&delta, spec, None)?;
            let ms = median_ms(|| {
                gpu.merge_radix_indexes(&older, &newer)?;
                Ok(())
            })?;
            report("u32 merge_radix_indexes +10%", n + n / 10, ms);
        }
    }
    Ok(())
}
