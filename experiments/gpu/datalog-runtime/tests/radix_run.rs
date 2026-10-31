//! Formal rules GPU-RadixRun, GPU-RadixPublish and GPU-RadixItemColumns
//! against a CPU oracle: `cq_storage::RadixRun` for u64 keys and a stable
//! occurrence list for every key width. Fixed seeds throughout.
use std::{collections::BTreeMap, sync::Arc};

use cq_storage::{ExactMembership, IndexRead, Publish, RadixRun};
use gpu_datalog_runtime::{
    pack::Word,
    radix_run::{
        device, offset_fits, DeviceItemColumns, DeviceRadixRun, HostRadixRun, IntervalProbe, ItemRange,
        KeyProbe, OffsetWord, RadixRunParams, RadixSnapshot,
    },
};
use gpu_device_columns::{Buffer, Pool};

type Result<T = (), E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
type Item = (u64, u32);
type Items = (Buffer<u64>, Buffer<u32>);

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

fn pool() -> Result<Arc<Pool>> {
    Ok(Pool::new(0, 0, 0)?)
}

fn params(ratio: usize, radix_bits: u32, merge_tile_rows: u32) -> RadixRunParams {
    RadixRunParams { ratio, radix_bits, merge_tile_rows }
}

fn ones<W: Word>() -> W {
    let mut word = W::zero();
    for lane in 0..W::BITS / 64 {
        word.set_lane(lane, u64::MAX);
    }
    word
}

fn upload_items(pool: &Arc<Pool>, rows: &[(impl Copy, Item)]) -> Result<Items> {
    Ok((
        Buffer::from_host(pool.clone(), &rows.iter().map(|row| row.1 .0).collect::<Vec<_>>())?,
        Buffer::from_host(pool.clone(), &rows.iter().map(|row| row.1 .1).collect::<Vec<_>>())?,
    ))
}

fn publish<W: Word, O: OffsetWord, C: DeviceItemColumns>(
    run: &mut DeviceRadixRun<W, O, C>,
    keys: &[W],
    items: &C,
) -> Result {
    let buffer = Buffer::from_host(run.pool().clone(), keys)?;
    run.publish(&buffer, items, keys.len())
}

fn publish_rows<W: Word, O: OffsetWord>(run: &mut DeviceRadixRun<W, O, Items>, rows: &[(W, Item)]) -> Result {
    let items = upload_items(run.pool(), rows)?;
    publish(run, &rows.iter().map(|row| row.0).collect::<Vec<_>>(), &items)
}

/// Stable occurrence oracle: every published `(key, item)` in publication order.
struct Oracle<W> {
    rows: Vec<(W, Item)>,
}

impl<W> Default for Oracle<W> {
    fn default() -> Self {
        Self { rows: Vec::new() }
    }
}

/// Grouped view of the oracle: each key's items in publication order and the
/// number of occurrences with smaller keys.
struct Index<W> {
    groups: BTreeMap<W, (u64, Vec<Item>)>,
    total: u64,
}

impl<W: Word> Oracle<W> {
    fn index(&self) -> Index<W> {
        let mut groups = BTreeMap::<W, (u64, Vec<Item>)>::new();
        for &(key, item) in &self.rows {
            groups.entry(key).or_default().1.push(item);
        }
        let mut before = 0;
        for (start, items) in groups.values_mut() {
            *start = before;
            before += items.len() as u64;
        }
        Index { groups, total: before }
    }
}

impl<W: Word> Index<W> {
    fn lookup(&self, key: W) -> Vec<Item> {
        self.groups.get(&key).map_or_else(Vec::new, |group| group.1.clone())
    }
    fn range(&self, key: W) -> (u64, u64) {
        self.groups.get(&key).map_or((self.total, self.total), |(start, items)| (*start, start + items.len() as u64))
    }
    /// Occurrences with keys `<= key` (or `< key` when `inclusive` is false).
    fn rank(&self, key: W, inclusive: bool) -> u64 {
        let end = if inclusive { std::ops::Bound::Included(key) } else { std::ops::Bound::Excluded(key) };
        self.groups.range((std::ops::Bound::Unbounded, end)).next_back()
            .map_or(0, |(_, (start, items))| start + items.len() as u64)
    }
    fn interval_items(&self, low: W, high: W) -> u64 {
        self.rank(high, true).saturating_sub(self.rank(low, false))
    }
}

/// Distinct keys, extremes, and neighbours (often absent) of every key.
fn probes<W: Word>(oracle: &Oracle<W>, random: &mut Random) -> Vec<W> {
    let mut probes = vec![W::zero(), ones::<W>()];
    for &(key, _) in &oracle.rows {
        probes.push(key);
        let mut next = key;
        next.set_lane(0, key.lane(0).wrapping_add(1));
        probes.push(next);
        let mut previous = key;
        previous.set_lane(0, key.lane(0).wrapping_sub(1));
        probes.push(previous);
    }
    for _ in 0..64 {
        let mut word = W::zero();
        for lane in 0..W::BITS / 64 {
            word.set_lane(lane, random.next());
        }
        probes.push(word);
    }
    probes.sort_unstable();
    probes.dedup();
    probes
}

fn download<T: Copy + cuda_core::DeviceCopy>(buffer: Buffer<T>) -> Result<Vec<T>> {
    if buffer.is_empty() { Ok(Vec::new()) } else { Ok(buffer.to_host()?) }
}

/// Storage laws of the downloaded runs: distinct increasing keys, exact
/// offsets, the radix directory's lower-bound law, and the ratio law.
fn check_structure<W: Word, O: OffsetWord, H>(
    host: &[HostRadixRun<W, O, H>],
    lengths: &[(usize, usize)],
    parameters: RadixRunParams,
) {
    assert_eq!(host.len(), lengths.len());
    for pair in lengths.windows(2) {
        assert!(pair[0].1 / parameters.ratio >= pair[1].1, "ratio law violated: {lengths:?}");
    }
    for (run, &(keys, items)) in host.iter().zip(lengths) {
        assert!(keys > 0 && items >= keys, "published runs are nonempty groups");
        assert_eq!(run.keys.len(), keys);
        assert!(run.keys.windows(2).all(|pair| pair[0] < pair[1]), "keys must be distinct and increasing");
        assert_eq!(run.offsets.len(), keys + 1);
        assert_eq!(run.offsets[0].to_u64(), 0);
        assert!(run.offsets.windows(2).all(|pair| pair[0] < pair[1]), "groups must be nonempty");
        assert_eq!(run.offsets[keys].to_u64(), items as u64);
        if parameters.radix_bits == 0 {
            assert!(run.directory.is_empty());
            continue;
        }
        let (min, max) = (run.keys[0], run.keys[keys - 1]);
        assert_eq!(run.directory.len(), (1 << parameters.radix_bits) + 1);
        for (bucket, entry) in run.directory.iter().enumerate() {
            let lower = run.keys
                .partition_point(|&key| device::prefix(key, min, max, parameters.radix_bits) < bucket as u64);
            assert_eq!(entry.to_u64(), lower as u64, "directory bucket {bucket}");
        }
    }
}

struct DeviceReads {
    keys: Vec<KeyProbe>,
    groups: Vec<ItemRange>,
    intervals: Vec<IntervalProbe>,
    pairs: Vec<(usize, usize)>,
}

fn read<W: Word, O: OffsetWord, C: DeviceItemColumns>(
    snapshot: &RadixSnapshot<W, O, C>,
    probes: &[W],
) -> Result<DeviceReads> {
    let pool = snapshot.pool().clone();
    let buffer = Buffer::from_host(pool.clone(), probes)?;
    // Deterministic interval endpoints over the sorted probe set.
    let pairs: Vec<_> = (0..probes.len()).map(|i| (i, (i * 7 + 3) % probes.len())).collect();
    let low = Buffer::from_host(pool.clone(), &pairs.iter().map(|&(a, _)| probes[a]).collect::<Vec<_>>())?;
    let high = Buffer::from_host(pool, &pairs.iter().map(|&(_, b)| probes[b]).collect::<Vec<_>>())?;
    Ok(DeviceReads {
        keys: download(snapshot.probe_keys(&buffer, probes.len())?)?,
        groups: download(snapshot.probe_groups(&buffer, probes.len())?)?,
        intervals: download(snapshot.probe_intervals(&low, &high, probes.len())?)?,
        pairs,
    })
}

/// Every device read against the oracle; returns the reads and oracle index.
fn check<W: Word, O: OffsetWord>(
    run: &DeviceRadixRun<W, O, Items>,
    oracle: &Oracle<W>,
    probes: &[W],
) -> Result<(DeviceReads, Index<W>)> {
    let snapshot = run.snapshot()?;
    assert_eq!(snapshot.run_count(), run.run_count());
    assert_eq!(snapshot.item_count(), oracle.rows.len());
    let reads = read(&snapshot, probes)?;
    let host = run.to_host()?;
    check_structure(&host, &run.run_lengths(), run.params());
    let oracle = oracle.index();
    let runs = host.len();
    for (index, &key) in probes.iter().enumerate() {
        let expected = oracle.lookup(key);
        let probe = reads.keys[index];
        assert_eq!(probe.contains != 0, !expected.is_empty(), "contains {key:?}");
        assert_eq!(probe.count, expected.len() as u64, "count {key:?}");
        assert_eq!((probe.begin, probe.end), oracle.range(key), "range {key:?}");
        let mut items = Vec::new();
        for (position, stored) in host.iter().enumerate() {
            let group = reads.groups[index * runs + position];
            if group.begin == group.end {
                // Absent groups use the run's exact terminal position.
                assert_eq!(group.begin, stored.offsets.last().unwrap().to_u64());
            }
            for item in group.begin..group.end {
                items.push((stored.items.0[item as usize], stored.items.1[item as usize]));
            }
        }
        assert_eq!(items, expected, "lookup {key:?}");
    }
    for (index, &(a, b)) in reads.pairs.iter().enumerate() {
        let (low, high) = (probes[a], probes[b]);
        let groups = host.iter()
            .map(|run| {
                let first = run.keys.partition_point(|&key| key < low);
                let last = run.keys.partition_point(|&key| key <= high);
                last.saturating_sub(first) as u64
            })
            .sum::<u64>();
        assert_eq!(reads.intervals[index], IntervalProbe { groups, items: oracle.interval_items(low, high) });
    }
    Ok((reads, oracle))
}

#[test]
fn u64_publication_matches_cpu_radix_run_across_ratio_transitions() -> Result {
    let pool = pool()?;
    let input: Vec<(u64, Item)> =
        (0_u64..3001).map(|i| (((i * 7919) % 4099) * 3 + 5, (i, (i as u32) ^ 0x5555_aaaa))).collect();
    let mut random = Random(0x51ed_270b_7c3a_9e44);
    for (ratio, radix_bits, tile) in [(2, 12, 1), (4, 4, 32), (8, 0, 7), (4, 16, 1000)] {
        for batch in [7, 127, 2048, input.len()] {
            let parameters = params(ratio, radix_bits, tile);
            let mut device = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), parameters)?;
            let mut cpu = RadixRun::<u64, Item>::with_ratio([], ratio);
            let mut oracle = Oracle::default();
            let chunks: Vec<_> = input.chunks(batch).collect();
            let (mut merges, mut checked_runs) = (0, 0);
            for (index, chunk) in chunks.iter().enumerate() {
                let before = device.run_count();
                publish_rows(&mut device, chunk)?;
                cpu.publish(chunk.iter().copied());
                oracle.rows.extend_from_slice(chunk);
                assert_eq!(device.run_count(), cpu.run_count(), "ratio {ratio} batch {batch} publish {index}");
                merges += usize::from(device.run_count() <= before);
                if index % 37 != 0 && index + 1 != chunks.len() {
                    continue;
                }
                checked_runs = checked_runs.max(device.run_count());
                let probes = probes(&oracle, &mut random);
                let (reads, expected) = check(&device, &oracle, &probes)?;
                for (probe, &key) in reads.keys.iter().zip(&probes) {
                    assert_eq!(probe.contains != 0, cpu.contains(&key));
                    assert_eq!(probe.count as usize, cpu.count(&key));
                    assert_eq!((probe.begin as usize, probe.end as usize), cpu.range(&key));
                    assert_eq!(expected.lookup(key), (&cpu).lookup(&key).collect::<Vec<_>>());
                }
            }
            if batch <= 127 {
                assert!(merges > 0, "ratio transitions must merge runs");
            }
            if batch == 7 {
                assert!(checked_runs >= 3, "reads must be checked across several unmerged runs");
            }
        }
    }
    Ok(())
}

#[test]
fn full_domain_u64_keys_have_exact_absent_positions() -> Result {
    let pool = pool()?;
    let keys = [0, 1, 2, 1 << 32, 1 << 63, (1 << 63) + 1, u64::MAX - 1, u64::MAX];
    let mut random = Random(0x0dd1_ce5e_ed00_0001);
    for radix_bits in [0, 1, 12, 20] {
        let mut device = DeviceRadixRun::<u64, u64, Items>::new(pool.clone(), params(4, radix_bits, 3))?;
        let mut cpu = RadixRun::<u64, Item>::with_ratio([], 4);
        let mut oracle = Oracle::default();
        for round in 0..6_u64 {
            let rows: Vec<_> = (0..(round * 5 + 1))
                .map(|i| (keys[(random.next() % keys.len() as u64) as usize], (round * 100 + i, i as u32)))
                .collect();
            publish_rows(&mut device, &rows)?;
            cpu.publish(rows.iter().copied());
            oracle.rows.extend(rows);
            assert_eq!(device.run_count(), cpu.run_count());
            let probes = probes(&oracle, &mut random);
            let (reads, _) = check(&device, &oracle, &probes)?;
            for (probe, key) in reads.keys.iter().zip(&probes) {
                assert_eq!((probe.begin as usize, probe.end as usize), cpu.range(key));
            }
        }
    }
    Ok(())
}

fn wide_keys<W: Word>(seed: u64) -> Result {
    let pool = pool()?;
    let mut random = Random(seed);
    // Distinct words that differ in every lane, including only in the lowest.
    let mut words = vec![W::zero(), ones::<W>()];
    for _ in 0..300 {
        let mut word = W::zero();
        for lane in 0..W::BITS / 64 {
            word.set_lane(lane, if random.next() % 4 == 0 { random.next() % 3 } else { random.next() });
        }
        let mut neighbour = word;
        neighbour.set_lane(0, word.lane(0) ^ 1);
        words.extend([word, neighbour]);
    }
    for (ratio, radix_bits, tile) in [(2, 8, 5), (4, 0, 64), (3, 20, 1)] {
        let mut device = DeviceRadixRun::<W, u32, Items>::new(pool.clone(), params(ratio, radix_bits, tile))?;
        let mut oracle = Oracle::default();
        let mut serial = 0_u64;
        for size in [1_usize, 17, 0, 257, 3, 900, 64, 2] {
            let rows: Vec<_> = (0..size).map(|_| {
                serial += 1;
                (words[(random.next() % words.len() as u64) as usize], (serial, serial as u32 * 3))
            }).collect();
            publish_rows(&mut device, &rows)?;
            oracle.rows.extend(rows);
            let probes = probes(&oracle, &mut random);
            check(&device, &oracle, &probes)?;
        }
    }
    Ok(())
}

#[test]
fn wide_u128_keys_preserve_every_lane() -> Result {
    wide_keys::<u128>(0x1234_5678_9abc_def0)
}

#[test]
fn wide_array_keys_preserve_every_lane() -> Result {
    wide_keys::<[u64; 3]>(0x0fed_cba9_8765_4321)
}

#[test]
fn sorted_reversed_and_single_key_inputs_keep_occurrence_order() -> Result {
    let pool = pool()?;
    let ascending: Vec<(u64, Item)> = (0..1025).map(|i| (i / 3, (i, i as u32))).collect();
    let reversed: Vec<_> = ascending.iter().rev().copied().collect();
    let single: Vec<_> = (0..777).map(|i| (42_u64, (i, i as u32))).collect();
    let mut random = Random(7);
    for rows in [ascending, reversed, single] {
        let items = upload_items(&pool, &rows)?;
        let keys = Buffer::from_host(pool.clone(), &rows.iter().map(|row| row.0).collect::<Vec<_>>())?;
        let device = DeviceRadixRun::<u64, u32, Items>::build(pool.clone(), params(4, 6, 16), &keys, &items, rows.len())?;
        assert_eq!(device.run_count(), 1);
        let oracle = Oracle { rows };
        check(&device, &oracle, &probes(&oracle, &mut random))?;
    }
    Ok(())
}

#[test]
fn empty_providers_and_batches_read_exact_empty_results() -> Result {
    let pool = pool()?;
    let empty_items = upload_items(&pool, &[] as &[(u64, Item)])?;
    let keys = Buffer::<u64>::from_host(pool.clone(), &[])?;
    let mut device = DeviceRadixRun::<u64, u32, Items>::build(pool.clone(), params(4, 12, 32), &keys, &empty_items, 0)?;
    assert!(device.is_empty());
    assert_eq!(device.run_count(), 0);
    let probes = [0, 7, u64::MAX];
    let snapshot = device.snapshot()?;
    let reads = read(&snapshot, &probes)?;
    assert!(reads.keys.iter().all(|probe| *probe == KeyProbe::default()));
    assert!(reads.groups.is_empty());
    assert!(reads.intervals.iter().all(|probe| *probe == IntervalProbe::default()));
    publish(&mut device, &[], &empty_items)?;
    assert_eq!(device.run_count(), 0);
    let rows = [(7_u64, (1, 1)), (7, (2, 2))];
    publish_rows(&mut device, &rows)?;
    publish(&mut device, &[], &empty_items)?;
    assert_eq!(device.run_lengths(), [(1, 2)]);
    check(&device, &Oracle { rows: rows.to_vec() }, &probes)?;
    Ok(())
}

#[test]
fn offset_carriers_are_type_facts_with_exact_bounds() -> Result {
    assert!(offset_fits::<u32>(u32::MAX as usize));
    assert!(!offset_fits::<u32>(u32::MAX as usize + 1));
    assert!(offset_fits::<u64>(u32::MAX as usize + 1));
    assert!(offset_fits::<u64>(usize::MAX));
    let pool = pool()?;
    let mut random = Random(99);
    let rows: Vec<(u64, Item)> = (0..5000).map(|i| (random.next() % 1500, (i, i as u32))).collect();
    let mut narrow = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(4, 10, 32))?;
    let mut wide = DeviceRadixRun::<u64, u64, Items>::new(pool.clone(), params(4, 10, 32))?;
    let mut oracle = Oracle::default();
    for chunk in rows.chunks(611) {
        publish_rows(&mut narrow, chunk)?;
        publish_rows(&mut wide, chunk)?;
        oracle.rows.extend_from_slice(chunk);
    }
    assert_eq!(narrow.run_lengths(), wide.run_lengths());
    let probes = probes(&oracle, &mut random);
    let (narrow_reads, _) = check(&narrow, &oracle, &probes)?;
    let (wide_reads, _) = check(&wide, &oracle, &probes)?;
    assert_eq!(narrow_reads.keys, wide_reads.keys);
    assert_eq!(narrow_reads.groups, wide_reads.groups);
    assert_eq!(narrow_reads.intervals, wide_reads.intervals);
    for (n, w) in narrow.to_host()?.into_iter().zip(wide.to_host()?) {
        assert_eq!(n.keys, w.keys);
        assert_eq!(n.items, w.items);
        assert!(n.offsets.iter().map(|o| *o as u64).eq(w.offsets.iter().copied()));
        assert!(n.directory.iter().map(|o| *o as u64).eq(w.directory.iter().copied()));
    }
    Ok(())
}

#[test]
fn key_reads_are_independent_of_item_columns() -> Result {
    let pool = pool()?;
    let mut random = Random(0xfeed);
    let rows: Vec<(u64, Item)> = (0..4000).map(|i| (random.next() % 900, (i, i as u32))).collect();
    let mut keyed = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(2, 8, 32))?;
    let mut key_only = DeviceRadixRun::<u64, u32, ()>::new(pool.clone(), params(2, 8, 32))?;
    let mut oracle = Oracle::default();
    for chunk in rows.chunks(333) {
        publish_rows(&mut keyed, chunk)?;
        publish(&mut key_only, &chunk.iter().map(|row| row.0).collect::<Vec<_>>(), &())?;
        oracle.rows.extend_from_slice(chunk);
    }
    assert_eq!(keyed.run_lengths(), key_only.run_lengths());
    let probes = probes(&oracle, &mut random);
    let keyed_snapshot = keyed.snapshot()?;
    let key_only_snapshot = key_only.snapshot()?;
    assert_eq!(keyed_snapshot.items_view().column_count, 2);
    assert_eq!(key_only_snapshot.items_view().column_count, 0);
    let items_before: Vec<_> = keyed.to_host()?.into_iter().map(|run| run.items).collect();
    // Key reads take only the keys view: no item pointer reaches these kernels.
    let with_items = read(&keyed_snapshot, &probes)?;
    let without_items = read(&key_only_snapshot, &probes)?;
    assert_eq!(with_items.keys, without_items.keys);
    assert_eq!(with_items.groups, without_items.groups);
    assert_eq!(with_items.intervals, without_items.intervals);
    let items_after: Vec<_> = keyed.to_host()?.into_iter().map(|run| run.items).collect();
    assert_eq!(items_before, items_after);
    check(&keyed, &oracle, &probes)?;
    Ok(())
}

#[test]
fn snapshots_retain_runs_across_later_merges() -> Result {
    let pool = pool()?;
    let mut random = Random(0xabcdef);
    let mut device = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(2, 6, 8))?;
    let mut oracle = Oracle::default();
    let mut retained = Vec::new();
    for round in 0..12_u64 {
        let rows: Vec<(u64, Item)> =
            (0..(round + 1) * 40).map(|i| (random.next() % 300, (round * 1000 + i, round as u32))).collect();
        publish_rows(&mut device, &rows)?;
        oracle.rows.extend(rows);
        let probes = probes(&oracle, &mut random);
        let snapshot = device.snapshot()?;
        let expected = read(&snapshot, &probes)?;
        retained.push((snapshot, probes, expected.keys));
    }
    drop(device);
    for (snapshot, probes, expected) in retained {
        assert_eq!(read(&snapshot, &probes)?.keys, expected);
    }
    Ok(())
}

#[test]
fn invalid_parameters_and_inputs_are_rejected() -> Result {
    let pool = pool()?;
    for bad in [params(1, 4, 32), params(4, 33, 32), params(4, 4, 0)] {
        assert!(DeviceRadixRun::<u64, u32, ()>::new(pool.clone(), bad).is_err());
    }
    let mut device = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(4, 4, 32))?;
    let keys = Buffer::from_host(pool.clone(), &[1_u64, 2, 3])?;
    let short = upload_items(&pool, &[(1_u64, (1, 1))])?;
    assert!(device.publish(&keys, &short, 3).is_err(), "item columns shorter than keys");
    assert!(device.publish(&keys, &short, 4).is_err(), "rows exceed key capacity");
    assert_eq!(device.run_count(), 0, "failed publication leaves state unchanged");
    Ok(())
}
