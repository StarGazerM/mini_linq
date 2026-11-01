//! Runtime laws of the typed radix view (P4-GPU design §4): formal rules
//! GPU-RadixLayout, GPU-RadixCursor and GPU-RadixLaneRead against a stable
//! occurrence oracle. Fixed seeds throughout.
use std::{collections::BTreeMap, sync::Arc};

use gpu_datalog_runtime::{
    pack::Word,
    radix_run::{DeviceItemColumns, ItemColumn, ItemLane, LaneKind, RadixMatches, RadixRunParams},
    readback_count, Column, DeviceRadixRun, OffsetWord, Packing, RadixLayout,
};
use gpu_device_columns::{Buffer, Pool};

type Result<T = (), E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
type Items = (Buffer<u64>, Buffer<i32>);
type Layout = RadixLayout<(i64,), (u64, i32), u64, u32, Items>;

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

fn params(ratio: usize, radix_bits: u32) -> RadixRunParams {
    RadixRunParams {
        ratio,
        radix_bits,
        merge_tile_rows: 16,
    }
}

fn pack<W: Word>(packing: &Packing, key: &[i128]) -> W {
    packing.pack(key).expect("key fits its packing")
}

/// Publish `(key, (serial, tag))` rows with `i64` keys.
fn publish_rows(
    run: &mut DeviceRadixRun<u64, u32, Items>,
    packing: &Packing,
    rows: &[(i64, (u64, i32))],
) -> Result {
    let pool = run.pool().clone();
    let keys: Vec<u64> = rows
        .iter()
        .map(|row| pack(packing, &[row.0 as i128]))
        .collect();
    let keys = Buffer::from_host(pool.clone(), &keys)?;
    let items = (
        Buffer::from_host(
            pool.clone(),
            &rows.iter().map(|row| row.1 .0).collect::<Vec<_>>(),
        )?,
        Buffer::from_host(pool, &rows.iter().map(|row| row.1 .1).collect::<Vec<_>>())?,
    );
    run.publish(&keys, &items, rows.len())
}

/// Expand every probe's cursors through one demanded column on `pool`:
/// per probe, the items of every run in run order (the stable occurrence order).
fn expand<W: Word, O: OffsetWord, C: DeviceItemColumns, T: ItemLane>(
    pool: &Arc<Pool>,
    matches: &RadixMatches<W, O, C>,
    column: &ItemColumn<T>,
) -> Result<Vec<Vec<T>>> {
    let cursors = matches.to_host()?;
    let mut bases = Vec::with_capacity(cursors.len());
    let mut total = 0_u64;
    for cursor in &cursors {
        assert_eq!(cursor.snapshot, matches.snapshot_id());
        assert!(cursor.begin <= cursor.end);
        bases.push(total);
        total += cursor.len();
    }
    let bases_buffer = Buffer::from_host(
        pool.clone(),
        &if bases.is_empty() {
            vec![0]
        } else {
            bases.clone()
        },
    )?;
    let output = matches.gather(pool, column, &bases_buffer, total as usize)?;
    let values = if total == 0 {
        Vec::new()
    } else {
        output.to_host()?
    };
    let runs = matches.run_count();
    let mut result = Vec::with_capacity(matches.probe_count());
    for probe in 0..matches.probe_count() {
        let mut items = Vec::new();
        for run in 0..runs {
            let index = probe * runs + run;
            let begin = bases[index] as usize;
            items.extend_from_slice(&values[begin..begin + cursors[index].len() as usize]);
        }
        result.push(items);
    }
    Ok(result)
}

/// Stable occurrence oracle grouped by key, in publication order.
fn groups(rows: &[(i64, (u64, i32))]) -> BTreeMap<i64, Vec<(u64, i32)>> {
    let mut groups = BTreeMap::<i64, Vec<(u64, i32)>>::new();
    for &(key, item) in rows {
        groups.entry(key).or_default().push(item);
    }
    groups
}

/// Every distinct key and its absent neighbours, plus the full-domain extremes.
fn probe_keys(groups: &BTreeMap<i64, Vec<(u64, i32)>>) -> Vec<i64> {
    let mut probes = vec![i64::MIN, i64::MAX];
    for &key in groups.keys() {
        probes.extend([key, key.wrapping_sub(1), key.wrapping_add(1)]);
    }
    probes.sort_unstable();
    probes.dedup();
    probes
}

/// Read every probe of `layout` on `pool` through both narrow columns and
/// check exact groups, stable order and run-local cursor bounds. Returns the
/// expansion of every distinct key (the factorization's image).
fn check_layout(
    layout: &Layout,
    pool: &Arc<Pool>,
    rows: &[(i64, (u64, i32))],
    run_items: &[usize],
) -> Result<Vec<(i64, (u64, i32))>> {
    let expected = groups(rows);
    let probes = probe_keys(&expected);
    let packed: Vec<u64> = probes
        .iter()
        .map(|&key| pack(layout.packing(), &[key as i128]))
        .collect();
    let buffer = Buffer::from_host(pool.clone(), &packed)?;
    let matches = layout.probe_matches(pool, &buffer, probes.len())?;
    assert_eq!(matches.snapshot_id(), layout.snapshot_id());
    assert_eq!(matches.run_count(), run_items.len());
    for cursor in matches.to_host()? {
        // Cursors are run-local: they never exceed their own run's items.
        assert!(
            cursor.end as usize <= run_items[cursor.run as usize],
            "cursor {cursor:?} leaves its run"
        );
    }
    let serials = expand(pool, &matches, &layout.item_column::<u64>(0)?)?;
    let tags = expand(pool, &matches, &layout.item_column::<i32>(1)?)?;
    let mut image = Vec::new();
    for (index, &key) in probes.iter().enumerate() {
        let got: Vec<(u64, i32)> = serials[index]
            .iter()
            .copied()
            .zip(tags[index].iter().copied())
            .collect();
        assert_eq!(serials[index].len(), tags[index].len());
        assert_eq!(
            &got,
            expected.get(&key).unwrap_or(&Vec::new()),
            "stable occurrences of key {key}"
        );
        image.extend(got.into_iter().map(|item| (key, item)));
    }
    Ok(image)
}

#[test]
fn layout_factorizes_and_preserves_occurrence_order_across_runs() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Layout::derive_packing()?;
    let mut random = Random(0x4b1d_0000_0000_0001);
    for radix_bits in [0, 8] {
        let mut run = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(2, radix_bits))?;
        let mut rows = Vec::new();
        let mut layout = Layout::publish(&run, packing.clone())?;
        let (mut most_runs, mut spanning) = (0, false);
        for (round, size) in [700_usize, 300, 90, 40, 1, 13, 500, 7, 3, 260]
            .into_iter()
            .enumerate()
        {
            let batch: Vec<(i64, (u64, i32))> = (0..size)
                .map(|i| {
                    let key = (random.next() % 97) as i64 - 48;
                    (key, ((round * 1000 + i) as u64, -(i as i32) ^ round as i32))
                })
                .collect();
            publish_rows(&mut run, &packing, &batch)?;
            rows.extend(batch);
            layout = layout.republish(&run)?;
            assert_eq!(layout.len(), rows.len());
            assert_eq!(
                layout.logical_count()?,
                rows.len(),
                "resident count equals occurrences"
            );
            let lengths: Vec<usize> = run
                .run_lengths()
                .into_iter()
                .map(|(_, items)| items)
                .collect();
            most_runs = most_runs.max(lengths.len());
            let mut image = check_layout(&layout, &pool, &rows, &lengths)?;
            // Factorization: ⋃ {k} × V_k over distinct keys is the input multiset.
            let mut input = rows.clone();
            image.sort_unstable();
            input.sort_unstable();
            assert_eq!(
                image, input,
                "factorization preserves the occurrence multiset"
            );
            // Some key's occurrences span several runs after unmerged publications.
            let matches = layout.probe_matches(
                &pool,
                &Buffer::from_host(pool.clone(), &[pack::<u64>(&packing, &[0])])?,
                1,
            )?;
            spanning |= matches
                .to_host()?
                .iter()
                .filter(|cursor| !cursor.is_empty())
                .count()
                > 1;
        }
        assert!(most_runs >= 3, "laws must be checked over several runs");
        assert!(spanning, "a key must span several runs");
    }
    Ok(())
}

#[test]
fn snapshot_publication_constructs_without_host_readback() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Layout::derive_packing()?;
    let mut run = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(4, 4))?;
    publish_rows(
        &mut run,
        &packing,
        &[(3, (1, 1)), (3, (2, 2)), (-9, (3, 3))],
    )?;
    let before = readback_count();
    let layout = Layout::publish(&run, packing.clone())?;
    let republished = layout.republish(&run)?;
    assert_eq!(
        readback_count(),
        before,
        "constructing a snapshot downloads no count"
    );
    assert_eq!(republished.snapshot_id(), layout.snapshot_id());
    assert_eq!(layout.len(), 3);
    assert!(!layout.count_ptr().is_null());
    assert_eq!(layout.view().count, layout.count_ptr());
    assert_eq!(layout.view().field_count, 1);
    assert_eq!(layout.view().bits, 64);
    assert_eq!(layout.logical_count()?, 3);
    Ok(())
}

#[test]
fn republish_reuses_descriptors_and_snapshots_survive_publish_and_release() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Layout::derive_packing()?;
    let mut run = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(2, 6))?;
    let first: Vec<(i64, (u64, i32))> = (0..400)
        .map(|i| ((i % 37) as i64 - 18, (i as u64, i as i32)))
        .collect();
    publish_rows(&mut run, &packing, &first)?;
    let layout = Layout::publish(&run, packing.clone())?;
    let lengths: Vec<usize> = run
        .run_lengths()
        .into_iter()
        .map(|(_, items)| items)
        .collect();

    // An empty delta keeps the publication: same snapshot, same descriptors.
    publish_rows(&mut run, &packing, &[])?;
    let same = layout.republish(&run)?;
    assert_eq!(same.snapshot_id(), layout.snapshot_id());
    assert!(Arc::ptr_eq(same.snapshot(), layout.snapshot()));
    assert_eq!(same.fields_ptr(), layout.fields_ptr());
    assert!(
        Arc::ptr_eq(&run.snapshot()?, layout.snapshot()),
        "the run caches its publication"
    );

    // Retain a match set of the first publication before later publications.
    let probes: Vec<u64> = (-20..20).map(|key| pack(&packing, &[key])).collect();
    let probe_buffer = Buffer::from_host(pool.clone(), &probes)?;
    let retained = layout.probe_matches(&pool, &probe_buffer, probes.len())?;
    let retained_column = layout.item_column::<u64>(0)?;
    let expected_retained = expand(&pool, &retained, &retained_column)?;

    let mut rows = first.clone();
    let mut successor = layout.clone();
    for round in 1..8_u64 {
        let batch: Vec<(i64, (u64, i32))> = (0..round * 50)
            .map(|i| ((i % 11) as i64, (round * 10_000 + i, round as i32)))
            .collect();
        publish_rows(&mut run, &packing, &batch)?;
        rows.extend(batch);
        let next = successor.republish(&run)?;
        assert_ne!(
            next.snapshot_id(),
            successor.snapshot_id(),
            "changed runs are a new publication"
        );
        assert_eq!(
            next.fields_ptr(),
            layout.fields_ptr(),
            "successors share field descriptors"
        );
        successor = next;
    }
    let final_lengths: Vec<usize> = run
        .run_lengths()
        .into_iter()
        .map(|(_, items)| items)
        .collect();
    // Release the run and the newest layout; the retained ones stay exact.
    check_layout(&successor, &pool, &rows, &final_lengths)?;
    drop(run);
    drop(successor);
    check_layout(&layout, &pool, &first, &lengths)?;
    drop(same);
    drop(layout);
    // The retained match set alone keeps its snapshot's runs and columns.
    assert_eq!(
        expand(&pool, &retained, &retained_column)?,
        expected_retained
    );
    Ok(())
}

#[test]
fn zero_publication_reads_exact_empty_results() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Layout::derive_packing()?;
    let mut run = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(4, 12))?;
    let layout = Layout::publish(&run, packing.clone())?;
    assert!(layout.is_empty());
    assert_eq!(layout.run_count(), 0);
    assert_eq!(layout.logical_count()?, 0);
    let probes: Vec<u64> = [i64::MIN, 0, i64::MAX]
        .iter()
        .map(|&key| pack(&packing, &[key as i128]))
        .collect();
    let buffer = Buffer::from_host(pool.clone(), &probes)?;
    let keys = layout.probe_keys(&pool, &buffer, probes.len())?.to_host()?;
    assert!(keys.iter().all(|probe| probe.contains == 0
        && probe.count == 0
        && probe.begin == 0
        && probe.end == 0));
    let matches = layout.probe_matches(&pool, &buffer, probes.len())?;
    assert!(matches.to_host()?.is_empty());
    assert_eq!(
        expand(&pool, &matches, &layout.item_column::<u64>(0)?)?,
        vec![Vec::<u64>::new(); 3]
    );
    let full = layout.prefix_interval(&[])?;
    let low = Buffer::from_host(pool.clone(), &[*full.start()])?;
    let high = Buffer::from_host(pool.clone(), &[*full.end()])?;
    let interval = layout.probe_intervals(&pool, &low, &high, 1)?.to_host()?;
    assert_eq!((interval[0].groups, interval[0].items), (0, 0));
    // Empty deltas leave the zero publication itself in place.
    publish_rows(&mut run, &packing, &[])?;
    assert_eq!(layout.republish(&run)?.snapshot_id(), layout.snapshot_id());
    Ok(())
}

#[test]
fn lane_reads_follow_fork_and_join_readiness() -> Result {
    let main = Pool::new(0, 0, 0)?;
    let lane = main.lane()?;
    let foreign = Pool::new(0, 0, 0)?;
    assert!(lane.shares_memory(&main) && !foreign.shares_memory(&main));
    let packing = Layout::derive_packing()?;
    let mut run = DeviceRadixRun::<u64, u32, Items>::new(main.clone(), params(2, 4))?;
    let mut random = Random(77);
    let mut rows = Vec::new();
    for round in 0..4_u64 {
        let batch: Vec<(i64, (u64, i32))> = (0..300)
            .map(|i| {
                (
                    (random.next() % 50) as i64,
                    (round * 1000 + i, round as i32),
                )
            })
            .collect();
        publish_rows(&mut run, &packing, &batch)?;
        rows.extend(batch);
    }
    let layout = Layout::publish(&run, packing.clone())?;
    let lengths: Vec<usize> = run
        .run_lengths()
        .into_iter()
        .map(|(_, items)| items)
        .collect();
    let main_image = check_layout(&layout, &main, &rows, &lengths)?;

    // Fork readiness: the lane stream waits for the publication's uploads.
    let fork = main.context().new_event(None)?;
    fork.record(main.stream())?;
    lane.stream().wait(&fork)?;
    let lane_image = check_layout(&layout, &lane, &rows, &lengths)?;
    assert_eq!(
        lane_image, main_image,
        "lane reads equal publication-stream reads"
    );

    // Lane-produced keys become build input on main only after join readiness.
    let delta: Vec<(i64, (u64, i32))> = (0..100)
        .map(|i| ((i % 7) as i64, (90_000 + i, 9)))
        .collect();
    let keys = Buffer::from_host_async(
        lane.clone(),
        &delta
            .iter()
            .map(|row| pack::<u64>(&packing, &[row.0 as i128]))
            .collect::<Vec<_>>(),
    )?;
    let items = (
        Buffer::from_host_async(
            lane.clone(),
            &delta.iter().map(|row| row.1 .0).collect::<Vec<_>>(),
        )?,
        Buffer::from_host_async(
            lane.clone(),
            &delta.iter().map(|row| row.1 .1).collect::<Vec<_>>(),
        )?,
    );
    let join = main.context().new_event(None)?;
    join.record(lane.stream())?;
    main.stream().wait(&join)?;
    run.publish(&keys, &items, delta.len())?;
    rows.extend(delta);
    let next = layout.republish(&run)?;
    let lengths: Vec<usize> = run
        .run_lengths()
        .into_iter()
        .map(|(_, items)| items)
        .collect();
    check_layout(&next, &main, &rows, &lengths)?;

    // Pools that do not share the snapshot's memory cannot read it.
    let probe = Buffer::from_host(foreign.clone(), &[0_u64])?;
    assert!(layout.probe_keys(&foreign, &probe, 1).is_err());
    assert!(layout.probe_matches(&foreign, &probe, 1).is_err());
    let main_probe = Buffer::from_host(main.clone(), &[0_u64])?;
    assert!(
        layout.probe_keys(&foreign, &main_probe, 1).is_err(),
        "execution pool, not probe pool, qualifies the read"
    );
    assert!(
        layout.probe_keys(&main, &probe, 1).is_err(),
        "probes must share the execution pool's memory"
    );
    Ok(())
}

#[test]
fn empty_keys_and_all_equal_keys_form_one_group() -> Result {
    type Unit = RadixLayout<(), (u32,), u64, u32, Buffer<u32>>;
    let pool = Pool::new(0, 0, 0)?;
    let packing = Unit::derive_packing()?;
    assert_eq!(packing.bits(), 0);
    let mut run = DeviceRadixRun::<u64, u32, Buffer<u32>>::new(pool.clone(), params(2, 3))?;
    let mut serial = 0_u32;
    for size in [5_usize, 64, 2, 1] {
        let values: Vec<u32> = (0..size as u32).map(|i| serial + i).collect();
        serial += size as u32;
        run.publish(
            &Buffer::from_host(pool.clone(), &vec![0_u64; size])?,
            &Buffer::from_host(pool.clone(), &values)?,
            size,
        )?;
    }
    let layout = Unit::publish(&run, packing)?;
    assert!(
        run.run_lengths().iter().all(|&(keys, _)| keys == 1),
        "one distinct key per run"
    );
    let range = layout.prefix_interval(&[])?;
    assert_eq!((*range.start(), *range.end()), (0, 0));
    let probe = Buffer::from_host(pool.clone(), &[0_u64])?;
    let matches = layout.probe_matches(&pool, &probe, 1)?;
    let items = expand(&pool, &matches, &layout.item_column::<u32>(0)?)?;
    assert_eq!(
        items,
        vec![(0..serial).collect::<Vec<_>>()],
        "all-equal keys keep publication order"
    );
    Ok(())
}

#[test]
fn multiword_extrema_and_prefix_intervals_are_exact() -> Result {
    type Wide = RadixLayout<(i64, u64), (u8,), u128, u64, Buffer<u8>>;
    let pool = Pool::new(0, 0, 0)?;
    let packing = Wide::derive_packing()?;
    let firsts = [i64::MIN, -1, 0, 1, i64::MAX];
    let seconds = [0, 1, u64::MAX - 1, u64::MAX];
    let mut rows = Vec::new();
    for (index, &a) in firsts.iter().enumerate() {
        for (inner, &b) in seconds.iter().enumerate() {
            if (index + inner) % 3 != 1 {
                rows.push(((a, b), (index * 4 + inner) as u8));
                rows.push(((a, b), (index * 4 + inner + 100) as u8));
            }
        }
    }
    let mut run = DeviceRadixRun::<u128, u64, Buffer<u8>>::new(pool.clone(), params(2, 16))?;
    for chunk in rows.chunks(5) {
        let keys: Vec<u128> = chunk
            .iter()
            .map(|row| pack(&packing, &[row.0 .0 as i128, row.0 .1 as i128]))
            .collect();
        let items: Vec<u8> = chunk.iter().map(|row| row.1).collect();
        run.publish(
            &Buffer::from_host(pool.clone(), &keys)?,
            &Buffer::from_host(pool.clone(), &items)?,
            chunk.len(),
        )?;
    }
    let layout = Wide::publish(&run, packing.clone())?;
    // Every complete key, present or absent, reads its exact group.
    let mut probes = Vec::new();
    for &a in &firsts {
        for &b in &seconds {
            probes.push((a, b));
        }
    }
    let packed: Vec<u128> = probes
        .iter()
        .map(|&(a, b)| pack(&packing, &[a as i128, b as i128]))
        .collect();
    let matches = layout.probe_matches(
        &pool,
        &Buffer::from_host(pool.clone(), &packed)?,
        probes.len(),
    )?;
    let items = expand(&pool, &matches, &layout.item_column::<u8>(0)?)?;
    for (index, key) in probes.iter().enumerate() {
        let expected: Vec<u8> = rows
            .iter()
            .filter(|row| row.0 == *key)
            .map(|row| row.1)
            .collect();
        assert_eq!(items[index], expected, "key {key:?}");
    }
    // Prefix intervals: present first columns count their groups, absent ones none.
    let prefixes = [i64::MIN, -1, 0, 1, i64::MAX, 2, -2];
    let (mut low, mut high) = (Vec::new(), Vec::new());
    for &a in &prefixes {
        let range = layout.prefix_interval(&[a as i128])?;
        low.push(*range.start());
        high.push(*range.end());
    }
    let intervals = layout
        .probe_intervals(
            &pool,
            &Buffer::from_host(pool.clone(), &low)?,
            &Buffer::from_host(pool.clone(), &high)?,
            prefixes.len(),
        )?
        .to_host()?;
    for (index, &a) in prefixes.iter().enumerate() {
        let items = rows.iter().filter(|row| row.0 .0 == a).count() as u64;
        assert_eq!(intervals[index].items, items, "prefix {a}");
        assert_eq!(
            intervals[index].groups == 0,
            items == 0,
            "absent prefix {a}"
        );
    }
    Ok(())
}

#[test]
fn many_runs_keep_run_local_cursors() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Layout::derive_packing()?;
    let mut run = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(2, 2))?;
    let mut rows = Vec::new();
    for (round, size) in [1024_usize, 256, 64, 16, 4, 1].into_iter().enumerate() {
        let batch: Vec<(i64, (u64, i32))> = (0..size)
            .map(|i| ((i % 5) as i64, ((round * 10_000 + i) as u64, round as i32)))
            .collect();
        publish_rows(&mut run, &packing, &batch)?;
        rows.extend(batch);
    }
    assert_eq!(
        run.run_count(),
        6,
        "decreasing batches stay unmerged under ratio 2"
    );
    let layout = Layout::publish(&run, packing)?;
    let lengths: Vec<usize> = run
        .run_lengths()
        .into_iter()
        .map(|(_, items)| items)
        .collect();
    check_layout(&layout, &pool, &rows, &lengths)?;
    Ok(())
}

#[test]
fn partial_payload_demand_reads_only_named_columns() -> Result {
    type Triple =
        RadixLayout<(u32,), (u16, i64, u8), u64, u32, (Buffer<u16>, Buffer<i64>, Buffer<u8>)>;
    let pool = Pool::new(0, 0, 0)?;
    let packing = Triple::derive_packing()?;
    let rows: Vec<(u32, (u16, i64, u8))> = (0..900)
        .map(|i| (i % 13, (i as u16, -(i as i64) * 3, (i % 251) as u8)))
        .collect();
    let mut run = DeviceRadixRun::<u64, u32, (Buffer<u16>, Buffer<i64>, Buffer<u8>)>::new(
        pool.clone(),
        params(2, 4),
    )?;
    let mut other = DeviceRadixRun::<u64, u32, (Buffer<u16>, Buffer<i64>, Buffer<u8>)>::new(
        pool.clone(),
        params(2, 4),
    )?;
    for chunk in rows.chunks(200) {
        let keys: Vec<u64> = chunk
            .iter()
            .map(|row| pack(&packing, &[row.0 as i128]))
            .collect();
        let items = (
            Buffer::from_host(
                pool.clone(),
                &chunk.iter().map(|row| row.1 .0).collect::<Vec<_>>(),
            )?,
            Buffer::from_host(
                pool.clone(),
                &chunk.iter().map(|row| row.1 .1).collect::<Vec<_>>(),
            )?,
            Buffer::from_host(
                pool.clone(),
                &chunk.iter().map(|row| row.1 .2).collect::<Vec<_>>(),
            )?,
        );
        run.publish(
            &Buffer::from_host(pool.clone(), &keys)?,
            &items,
            chunk.len(),
        )?;
        other.publish(
            &Buffer::from_host(pool.clone(), &keys)?,
            &items,
            chunk.len(),
        )?;
    }
    let layout = Triple::publish(&run, packing.clone())?;
    let foreign = Triple::publish(&other, packing.clone())?;
    let probes: Vec<u64> = (0..14).map(|key| pack(&packing, &[key])).collect();
    let matches = layout.probe_matches(
        &pool,
        &Buffer::from_host(pool.clone(), &probes)?,
        probes.len(),
    )?;
    let third = expand(&pool, &matches, &layout.item_column::<u8>(2)?)?;
    for (key, items) in third.iter().enumerate() {
        let expected: Vec<u8> = rows
            .iter()
            .filter(|row| row.0 == key as u32)
            .map(|row| row.1 .2)
            .collect();
        assert_eq!(items, &expected);
    }
    assert!(
        layout.item_column::<u16>(1).is_err(),
        "lane type must match the demanded column"
    );
    assert!(
        layout.item_column::<u8>(3).is_err(),
        "column must lie in the value tuple"
    );
    let bases = Buffer::from_host(pool.clone(), &vec![0_u64; matches.to_host()?.len()])?;
    assert!(
        matches
            .gather(&pool, &foreign.item_column::<u8>(2)?, &bases, 1)
            .is_err(),
        "cursors read only columns of their own snapshot"
    );
    Ok(())
}

#[test]
fn layout_schemas_are_checked_against_their_carriers() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let run = DeviceRadixRun::<u64, u32, Items>::new(pool.clone(), params(4, 0))?;
    // Packing differing from the key tuple schema.
    assert!(Layout::publish(&run, Packing::with_columns(&[Column::U64])?).is_err());
    assert!(Layout::publish(&run, Packing::with_columns(&[Column::I64, Column::I64])?).is_err());
    // Value lanes that cannot hold the value schema or differ in arity.
    let narrow =
        DeviceRadixRun::<u64, u32, (Buffer<u64>, Buffer<u32>)>::new(pool.clone(), params(4, 0))?;
    type Narrow = RadixLayout<(i64,), (u64, i64), u64, u32, (Buffer<u64>, Buffer<u32>)>;
    assert!(Narrow::publish(&narrow, Narrow::derive_packing()?).is_err());
    type Arity = RadixLayout<(i64,), (u64,), u64, u32, Items>;
    assert!(Arity::publish(&run, Arity::derive_packing()?).is_err());
    // Key packing wider than the key word.
    type Overwide = RadixLayout<(i64, u64), (u64, i32), u64, u32, Items>;
    assert!(Overwide::publish(&run, Overwide::derive_packing()?).is_err());
    // Wider lanes of the same signedness, and unsigned values in wider signed lanes, hold exactly.
    type Widened = RadixLayout<(i64,), (u32, u16), u64, u32, Items>;
    assert!(Widened::publish(&run, Widened::derive_packing()?).is_ok());
    assert!(LaneKind {
        bits: 64,
        signed: true
    }
    .holds(Column::U32));
    assert!(!LaneKind {
        bits: 32,
        signed: true
    }
    .holds(Column::U32));
    assert!(!LaneKind {
        bits: 64,
        signed: false
    }
    .holds(Column::I8));
    assert_eq!(
        <[u64; 2] as ItemLane>::KIND,
        LaneKind {
            bits: 128,
            signed: false
        }
    );
    Ok(())
}
