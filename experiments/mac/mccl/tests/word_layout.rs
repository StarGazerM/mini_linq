//! Laws of the typed word-run layout (CUDA tests `radix_layout.rs` and
//! `radix_candidates.rs`), read on the CPU over GPU-built runs against
//! stable occurrence oracles. Fixed seeds throughout.

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::RangeInclusive,
    rc::Rc,
};

use mccl::{
    Gpu, Result,
    word_layout::{Column, ItemColumnView, LaneKind, PackedKey, Packing, WordLayout},
    word_read::{WordRunView, WordRunsView},
    words::{ItemLane, ItemType, OffsetWidth, WordRunParams, WordRunSet},
};

type Row = (i32, (u64, i32));
type Layout = WordLayout<(i32,), (u64, i32)>;
const ITEMS: [ItemType; 2] = [ItemType::U64, ItemType::I32];

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

fn params(lanes: usize, ms_first: bool, active_bits: u32, radix_bits: u32) -> WordRunParams {
    WordRunParams {
        lanes,
        ms_first,
        active_bits,
        radix_bits,
        offsets: OffsetWidth::U32,
    }
}

/// One-lane i32 keys: the packing's 32 bits are the sorted window.
fn key_params(radix_bits: u32) -> WordRunParams {
    params(1, false, 32, radix_bits)
}

/// Publish `(key, (serial, tag))` rows as one delta.
fn publish_rows(gpu: &Gpu, set: &mut WordRunSet, packing: &Packing, rows: &[Row]) -> Result<()> {
    let keys = rows
        .iter()
        .map(|row| packing.pack(&[row.0.into()], 1, false).map(|key| key[0]))
        .collect::<Result<Vec<_>>>()?;
    let serials: Vec<u64> = rows.iter().map(|row| row.1.0).collect();
    let tags: Vec<i32> = rows.iter().map(|row| row.1.1).collect();
    set.publish(
        &gpu.upload_word_keys(&keys, 1)?,
        &[&gpu.upload_items(&serials)?, &gpu.upload_items(&tags)?],
    )
}

fn run_items(set: &WordRunSet) -> Vec<usize> {
    set.run_lengths()
        .into_iter()
        .map(|(_, items)| items)
        .collect()
}

/// Every run's local cursor of `key`, in run order, expanded through
/// `column`: the stable occurrence order.
fn expand<T: ItemLane>(
    views: &[WordRunView<'_>],
    column: &ItemColumnView<'_, T>,
    key: &[u64],
) -> Vec<T> {
    let mut items = Vec::new();
    for (run, view) in views.iter().enumerate() {
        let (begin, end) = view.range(key);
        items.extend_from_slice(&column.run(run)[begin..end]);
    }
    items
}

/// Stable occurrence oracle grouped by key, in publication order.
fn groups(rows: &[Row]) -> BTreeMap<i32, Vec<(u64, i32)>> {
    let mut groups = BTreeMap::<i32, Vec<(u64, i32)>>::new();
    for &(key, item) in rows {
        groups.entry(key).or_default().push(item);
    }
    groups
}

/// Every distinct key and its absent neighbours, plus the domain extremes.
fn probe_keys(groups: &BTreeMap<i32, Vec<(u64, i32)>>) -> Vec<i32> {
    let mut probes = vec![i32::MIN, i32::MAX];
    for &key in groups.keys() {
        probes.extend([key, key.wrapping_sub(1), key.wrapping_add(1)]);
    }
    probes.sort_unstable();
    probes.dedup();
    probes
}

/// Read every probe through both narrow columns and check exact groups,
/// stable order, run-local cursors and merged positions. Returns the
/// expansion of every distinct key (the factorization's image).
fn check_layout(layout: &Layout, rows: &[Row], run_items: &[usize]) -> Result<Vec<Row>> {
    let expected = groups(rows);
    let views = layout.views();
    assert_eq!(views.len(), run_items.len());
    assert_eq!(layout.run_count(), run_items.len());
    assert_eq!(layout.len(), rows.len());
    let reads = WordRunsView::new(&views)?;
    let serials = layout.item_column::<u64>(0)?;
    let tags = layout.item_column::<i32>(1)?;
    assert_eq!(serials.snapshot_id(), layout.snapshot_id());
    let mut image = Vec::new();
    for key in probe_keys(&expected) {
        let packed = layout.pack(&[key.into()])?;
        assert_eq!(layout.unpack(&packed)?, [i128::from(key)]);
        for (run, view) in views.iter().enumerate() {
            // Cursors are run-local: they never leave their own run's items.
            assert_eq!(view.len(), run_items[run]);
            let (begin, end) = view.range(&packed);
            assert!(
                begin <= end && end <= run_items[run],
                "cursor leaves run {run}"
            );
        }
        let got: Vec<(u64, i32)> = expand(&views, &serials, &packed)
            .into_iter()
            .zip(expand(&views, &tags, &packed))
            .collect();
        let want = expected.get(&key).map_or(&[][..], Vec::as_slice);
        assert_eq!(got, want, "stable occurrences of key {key}");
        assert_eq!(reads.count(&packed), want.len());
        let below: usize = expected.range(..key).map(|(_, items)| items.len()).sum();
        let merged = if want.is_empty() {
            (rows.len(), rows.len())
        } else {
            (below, below + want.len())
        };
        assert_eq!(reads.range(&packed), merged, "merged positions of {key}");
        image.extend(got.into_iter().map(|item| (key, item)));
    }
    Ok(image)
}

#[test]
fn layout_factorizes_and_preserves_occurrence_order_across_runs() -> Result<()> {
    let gpu = Gpu::new()?;
    let packing = Layout::derive_packing()?;
    let mut random = Random(0x4b1d_0000_0000_0001);
    for radix_bits in [0, 8] {
        let mut set = WordRunSet::new(&gpu, key_params(radix_bits), 2, &ITEMS)?;
        let mut rows = Vec::new();
        let mut layout = Layout::publish(&set, packing.clone())?;
        let (mut most_runs, mut spanning) = (0, false);
        for (round, size) in [700_usize, 300, 90, 40, 1, 13, 500, 7, 3, 260]
            .into_iter()
            .enumerate()
        {
            let batch: Vec<Row> = (0..size)
                .map(|i| {
                    let key = (random.next() % 97) as i32 - 48;
                    (key, ((round * 1000 + i) as u64, -(i as i32) ^ round as i32))
                })
                .collect();
            publish_rows(&gpu, &mut set, &packing, &batch)?;
            rows.extend(batch);
            layout = layout.republish(&set)?;
            let lengths = run_items(&set);
            most_runs = most_runs.max(lengths.len());
            let mut image = check_layout(&layout, &rows, &lengths)?;
            // Factorization: the union of {k} x V_k is the input multiset.
            let mut input = rows.clone();
            image.sort_unstable();
            input.sort_unstable();
            assert_eq!(image, input, "factorization preserves the multiset");
            let zero = layout.pack(&[0])?;
            spanning |= layout
                .views()
                .iter()
                .filter(|view| view.contains(&zero))
                .count()
                > 1;
        }
        assert!(most_runs >= 3, "laws must be checked over several runs");
        assert!(spanning, "a key must span several runs");
    }
    Ok(())
}

#[test]
fn snapshot_publication_and_republish_identity() -> Result<()> {
    let gpu = Gpu::new()?;
    let packing = Layout::derive_packing()?;
    let mut set = WordRunSet::new(&gpu, key_params(4), 4, &ITEMS)?;
    publish_rows(
        &gpu,
        &mut set,
        &packing,
        &[(3, (1, 1)), (3, (2, 2)), (-9, (3, 3))],
    )?;
    let layout = Layout::publish(&set, packing)?;
    let republished = layout.republish(&set)?;
    assert_eq!(republished.snapshot_id(), layout.snapshot_id());
    assert!(Rc::ptr_eq(republished.snapshot(), layout.snapshot()));
    assert_eq!(layout.len(), 3);
    assert_eq!(layout.packing().len(), 1);
    assert_eq!(layout.packing().bits(), 32);
    // A set of other params cannot republish this layout.
    let other = WordRunSet::new(&gpu, key_params(5), 4, &ITEMS)?;
    assert!(layout.republish(&other).is_err());
    let lanes = WordRunSet::new(&gpu, key_params(4), 4, &[ItemType::U64, ItemType::I64])?;
    assert!(layout.republish(&lanes).is_err());
    Ok(())
}

#[test]
fn republish_shares_packing_and_snapshots_survive_publish_and_release() -> Result<()> {
    let gpu = Gpu::new()?;
    let packing = Layout::derive_packing()?;
    let mut set = WordRunSet::new(&gpu, key_params(6), 2, &ITEMS)?;
    let first: Vec<Row> = (0..400).map(|i| ((i % 37) - 18, (i as u64, i))).collect();
    publish_rows(&gpu, &mut set, &packing, &first)?;
    let layout = Layout::publish(&set, packing.clone())?;
    let lengths = run_items(&set);

    // An empty delta keeps the publication: same snapshot, same packing.
    publish_rows(&gpu, &mut set, &packing, &[])?;
    let same = layout.republish(&set)?;
    assert_eq!(same.snapshot_id(), layout.snapshot_id());
    assert!(Rc::ptr_eq(same.snapshot(), layout.snapshot()));
    assert!(std::ptr::eq(same.packing(), layout.packing()));
    assert!(
        Rc::ptr_eq(&set.snapshot(), layout.snapshot()),
        "the set caches its publication"
    );

    // Retain the first publication's expansions before later publications.
    let probes: Vec<PackedKey> = (-20..20)
        .map(|key| layout.pack(&[key]))
        .collect::<Result<_>>()?;
    let expected_retained: Vec<Vec<u64>> = {
        let views = layout.views();
        let column = layout.item_column::<u64>(0)?;
        probes
            .iter()
            .map(|key| expand(&views, &column, key))
            .collect()
    };
    let retained = Rc::clone(layout.snapshot());

    let mut rows = first.clone();
    let mut successor = layout.clone();
    for round in 1..8_u64 {
        let batch: Vec<Row> = (0..round * 50)
            .map(|i| ((i % 11) as i32, (round * 10_000 + i, round as i32)))
            .collect();
        publish_rows(&gpu, &mut set, &packing, &batch)?;
        rows.extend(batch);
        let next = successor.republish(&set)?;
        assert_ne!(
            next.snapshot_id(),
            successor.snapshot_id(),
            "changed runs are a new publication"
        );
        assert!(
            std::ptr::eq(next.packing(), layout.packing()),
            "successors share the packing"
        );
        successor = next;
    }
    let final_lengths = run_items(&set);
    // Release the set and the newest layout; the retained ones stay exact.
    check_layout(&successor, &rows, &final_lengths)?;
    drop(set);
    drop(successor);
    check_layout(&layout, &first, &lengths)?;
    drop(same);
    drop(layout);
    // The retained snapshot alone keeps its runs and columns.
    let views: Vec<_> = retained.runs().iter().map(WordRunView::from).collect();
    let columns = retained
        .runs()
        .iter()
        .map(|run| run.items::<u64>(0))
        .collect::<Result<Vec<_>>>()?;
    let reread: Vec<Vec<u64>> = probes
        .iter()
        .map(|key| {
            views
                .iter()
                .zip(&columns)
                .flat_map(|(view, column)| {
                    let (begin, end) = view.range(key);
                    column[begin..end].iter().copied()
                })
                .collect()
        })
        .collect();
    assert_eq!(reread, expected_retained);
    Ok(())
}

#[test]
fn zero_publication_reads_exact_empty_results() -> Result<()> {
    let gpu = Gpu::new()?;
    let packing = Layout::derive_packing()?;
    let mut set = WordRunSet::new(&gpu, key_params(12), 4, &ITEMS)?;
    let layout = Layout::publish(&set, packing.clone())?;
    assert!(layout.is_empty());
    assert_eq!(layout.run_count(), 0);
    let views = layout.views();
    let reads = WordRunsView::new(&views)?;
    let column = layout.item_column::<u64>(0)?;
    assert_eq!(column.run_count(), 0);
    for key in [i32::MIN, 0, i32::MAX] {
        let packed = layout.pack(&[key.into()])?;
        assert!(!reads.contains(&packed));
        assert_eq!((reads.count(&packed), reads.range(&packed)), (0, (0, 0)));
        assert!(expand(&views, &column, &packed).is_empty());
    }
    let full = layout.prefix_interval(&[])?;
    assert_eq!(
        (full.start().as_slice(), full.end().as_slice()),
        (&[0][..], &[0xffff_ffff][..])
    );
    assert_eq!(reads.interval_counts(full.start(), full.end()), (0, 0));
    assert_eq!(reads.distinct(full.start(), full.end()), 0);
    assert_eq!(reads.distinct_at(full.start(), full.end(), 0), None);
    // Empty deltas leave the zero publication itself in place.
    publish_rows(&gpu, &mut set, &packing, &[])?;
    assert_eq!(layout.republish(&set)?.snapshot_id(), layout.snapshot_id());
    Ok(())
}

#[test]
fn empty_keys_and_all_equal_keys_form_one_group() -> Result<()> {
    type Unit = WordLayout<(), (u32,)>;
    let gpu = Gpu::new()?;
    let packing = Unit::derive_packing()?;
    assert_eq!(packing.bits(), 0);
    let mut set = WordRunSet::new(&gpu, params(1, false, 0, 3), 2, &[ItemType::U32])?;
    let mut serial = 0_u32;
    for size in [5_u32, 64, 2, 1] {
        let values: Vec<u32> = (serial..serial + size).collect();
        serial += size;
        set.publish(
            &gpu.upload_word_keys(&vec![0; size as usize], 1)?,
            &[&gpu.upload_items(&values)?],
        )?;
    }
    let layout = Unit::publish(&set, packing)?;
    assert!(layout.run_count() > 1);
    assert!(
        set.run_lengths().iter().all(|&(keys, _)| keys == 1),
        "one distinct key per run"
    );
    let range = layout.prefix_interval(&[])?;
    assert_eq!(
        (range.start().as_slice(), range.end().as_slice()),
        (&[0][..], &[0][..])
    );
    assert_eq!(layout.pack(&[])?.as_slice(), [0]);
    let views = layout.views();
    assert_eq!(
        expand(&views, &layout.item_column::<u32>(0)?, &[0]),
        (0..serial).collect::<Vec<_>>(),
        "all-equal keys keep publication order"
    );
    let reads = WordRunsView::new(&views)?;
    assert_eq!(reads.distinct(range.start(), range.end()), 1);
    assert_eq!(
        reads.interval_counts(range.start(), range.end()),
        (layout.run_count(), serial as usize)
    );
    Ok(())
}

#[test]
fn multiword_extrema_and_prefix_intervals_are_exact() -> Result<()> {
    type Wide = WordLayout<(i64, u64), (u32,)>;
    let gpu = Gpu::new()?;
    let packing = Wide::derive_packing()?;
    assert_eq!(packing.bits(), 128);
    let firsts = [i64::MIN, -1, 0, 1, i64::MAX];
    let seconds = [0, 1, u64::MAX - 1, u64::MAX];
    let mut rows = Vec::new();
    for (index, &a) in firsts.iter().enumerate() {
        for (inner, &b) in seconds.iter().enumerate() {
            if (index + inner) % 3 != 1 {
                rows.push(((a, b), (index * 4 + inner) as u32));
                rows.push(((a, b), (index * 4 + inner + 100) as u32));
            }
        }
    }
    // u128 (least-significant lane first) and [u64; 2] (most-significant first).
    for ms_first in [false, true] {
        let word = WordRunParams {
            offsets: OffsetWidth::U64,
            ..params(2, ms_first, 128, 16)
        };
        let mut set = WordRunSet::new(&gpu, word, 2, &[ItemType::U32])?;
        for chunk in rows.chunks(5) {
            let mut keys = Vec::new();
            for &((a, b), _) in chunk {
                keys.extend_from_slice(&packing.pack(&[a.into(), b.into()], 2, ms_first)?);
            }
            let items: Vec<u32> = chunk.iter().map(|row| row.1).collect();
            set.publish(
                &gpu.upload_word_keys(&keys, 2)?,
                &[&gpu.upload_items(&items)?],
            )?;
        }
        let layout = Wide::publish(&set, packing.clone())?;
        assert!(layout.run_count() > 1);
        let views = layout.views();
        let reads = WordRunsView::new(&views)?;
        let column = layout.item_column::<u32>(0)?;
        // Every complete key, present or absent, reads its exact group; keys
        // order as their tuples.
        let mut previous: Option<PackedKey> = None;
        for &a in &firsts {
            for &b in &seconds {
                let key = layout.pack(&[a.into(), b.into()])?;
                assert!(previous < Some(key), "packing preserves tuple order");
                previous = Some(key);
                assert_eq!(layout.unpack(&key)?, [i128::from(a), i128::from(b)]);
                let expected: Vec<u32> = rows
                    .iter()
                    .filter(|row| row.0 == (a, b))
                    .map(|row| row.1)
                    .collect();
                assert_eq!(expand(&views, &column, &key), expected, "key {a} {b}");
            }
        }
        // Prefix intervals: present first fields count their groups, absent ones none.
        for a in [i64::MIN, -1, 0, 1, i64::MAX, 2, -2] {
            let range = layout.prefix_interval(&[a.into()])?;
            assert_eq!(Some(range.clone()), layout.encode_prefix(&[a]));
            let items = rows.iter().filter(|row| row.0.0 == a).count();
            let (groups, found) = reads.interval_counts(range.start(), range.end());
            assert_eq!(found, items, "prefix {a}");
            assert_eq!(groups == 0, items == 0, "absent prefix {a}");
            let distinct: BTreeSet<_> = rows
                .iter()
                .filter(|row| row.0.0 == a)
                .map(|row| row.0)
                .collect();
            assert_eq!(reads.distinct(range.start(), range.end()), distinct.len());
        }
    }
    Ok(())
}

#[test]
fn runtime_key_widths_preserve_extrema_prefixes_and_native_payloads() -> Result<()> {
    type Narrow = WordLayout<(i32, u32), (u64, i64)>;
    let gpu = Gpu::new()?;
    for bits in [1, 7, 19, 31, 32] {
        let packing = Narrow::derive_packing_with_fields(&[(bits, true), (bits, false)])?;
        assert_eq!(packing.bits(), bits * 2);
        let minimum = -(1_i128 << (bits - 1));
        let maximum = (1_i128 << (bits - 1)) - 1;
        let unsigned_maximum = (1_i128 << bits) - 1;
        let rows = [
            [minimum, 0],
            [minimum, unsigned_maximum],
            [maximum, 0],
            [maximum, unsigned_maximum],
        ];
        let keys = rows
            .iter()
            .map(|row| packing.pack(row, 1, false))
            .collect::<Result<Vec<_>>>()?;
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(packing.pack(&[maximum + 1, 0], 1, false).is_err());
        assert!(packing.pack(&[0, unsigned_maximum + 1], 1, false).is_err());
        let serials = [u64::MAX, 1 << 63, 0, 7];
        let tags = [i64::MIN, i64::MAX, -1, 0];
        let mut set = WordRunSet::new(
            &gpu,
            params(1, false, 64, 0),
            4,
            &[ItemType::U64, ItemType::I64],
        )?;
        // Reversed publication order makes the GPU sort all four extreme keys.
        let reversed: Vec<u64> = keys.iter().rev().map(|key| key[0]).collect();
        let rev_serials: Vec<u64> = serials.iter().rev().copied().collect();
        let rev_tags: Vec<i64> = tags.iter().rev().copied().collect();
        set.publish(
            &gpu.upload_word_keys(&reversed, 1)?,
            &[
                &gpu.upload_items(&rev_serials)?,
                &gpu.upload_items(&rev_tags)?,
            ],
        )?;
        let layout = Narrow::publish(&set, packing)?;
        let views = layout.views();
        let (serial, tag) = (layout.item_column::<u64>(0)?, layout.item_column::<i64>(1)?);
        for (index, key) in keys.iter().enumerate() {
            assert_eq!(expand(&views, &serial, key), [serials[index]]);
            assert_eq!(expand(&views, &tag, key), [tags[index]]);
        }
        let reads = WordRunsView::new(&views)?;
        for prefix in [minimum, maximum] {
            let range = layout.prefix_interval(&[prefix])?;
            assert_eq!(reads.interval_counts(range.start(), range.end()), (2, 2));
        }
        assert!(layout.prefix_interval(&[maximum + 1]).is_err());
        assert_eq!(layout.encode_prefix(&[(maximum + 1) as i64]), None);
    }
    // Loaded widths stay within the schema's columns and signedness.
    assert!(Narrow::derive_packing_with_fields(&[(33, true), (1, false)]).is_err());
    assert!(Narrow::derive_packing_with_fields(&[(8, false), (8, false)]).is_err());
    assert!(Narrow::derive_packing_with_fields(&[(8, true)]).is_err());
    Ok(())
}

#[test]
fn six_unmerged_runs_keep_run_local_cursors() -> Result<()> {
    let gpu = Gpu::new()?;
    let packing = Layout::derive_packing()?;
    let mut set = WordRunSet::new(&gpu, key_params(2), 2, &ITEMS)?;
    let mut rows = Vec::new();
    for (round, size) in [1024_usize, 256, 64, 16, 4, 1].into_iter().enumerate() {
        let batch: Vec<Row> = (0..size)
            .map(|i| ((i % 5) as i32, ((round * 10_000 + i) as u64, round as i32)))
            .collect();
        publish_rows(&gpu, &mut set, &packing, &batch)?;
        rows.extend(batch);
    }
    assert_eq!(
        set.run_count(),
        6,
        "decreasing batches stay unmerged under ratio 2"
    );
    let layout = Layout::publish(&set, packing)?;
    check_layout(&layout, &rows, &run_items(&set))?;
    Ok(())
}

#[test]
fn partial_payload_demand_reads_only_named_columns() -> Result<()> {
    type Triple = WordLayout<(u32,), (u16, i64, u8)>;
    const LANES: [ItemType; 3] = [ItemType::U32, ItemType::I64, ItemType::U32];
    let gpu = Gpu::new()?;
    let packing = Triple::derive_packing()?;
    let rows: Vec<(u32, (u16, i64, u8))> = (0..900)
        .map(|i| (i % 13, (i as u16, -(i as i64) * 3, (i % 251) as u8)))
        .collect();
    let mut set = WordRunSet::new(&gpu, key_params(4), 2, &LANES)?;
    let mut other = WordRunSet::new(&gpu, key_params(4), 2, &LANES)?;
    for chunk in rows.chunks(200) {
        let keys: Vec<u64> = chunk.iter().map(|row| row.0.into()).collect();
        let first: Vec<u32> = chunk.iter().map(|row| row.1.0.into()).collect();
        let second: Vec<i64> = chunk.iter().map(|row| row.1.1).collect();
        let third: Vec<u32> = chunk.iter().map(|row| row.1.2.into()).collect();
        let items = [
            gpu.upload_items(&first)?,
            gpu.upload_items(&second)?,
            gpu.upload_items(&third)?,
        ];
        let items: Vec<_> = items.iter().collect();
        set.publish(&gpu.upload_word_keys(&keys, 1)?, &items)?;
        other.publish(&gpu.upload_word_keys(&keys, 1)?, &items)?;
    }
    let layout = Triple::publish(&set, packing.clone())?;
    let foreign = Triple::publish(&other, packing)?;
    let views = layout.views();
    let third = layout.item_column::<u32>(2)?;
    let second = layout.item_column::<i64>(1)?;
    for key in 0..14_u32 {
        let packed = layout.pack(&[key.into()])?;
        let matching = rows.iter().filter(|row| row.0 == key);
        assert_eq!(
            expand(&views, &third, &packed),
            matching
                .clone()
                .map(|row| u32::from(row.1.2))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            expand(&views, &second, &packed),
            matching.map(|row| row.1.1).collect::<Vec<_>>()
        );
    }
    assert!(
        layout.item_column::<u32>(1).is_err(),
        "lane type must match the column"
    );
    assert!(
        layout.item_column::<i32>(0).is_err(),
        "signedness is part of the lane"
    );
    assert!(
        layout.item_column::<u32>(3).is_err(),
        "column must lie in the value tuple"
    );
    assert_ne!(
        foreign.item_column::<u32>(2)?.snapshot_id(),
        third.snapshot_id(),
        "columns name their own snapshot"
    );
    Ok(())
}

#[test]
fn layout_schemas_are_checked_against_their_carriers() -> Result<()> {
    let gpu = Gpu::new()?;
    let set = WordRunSet::new(&gpu, key_params(0), 4, &ITEMS)?;
    // Packing differing from the key tuple schema.
    assert!(Layout::publish(&set, Packing::with_columns(&[Column::U32])?).is_err());
    assert!(Layout::publish(&set, Packing::with_columns(&[Column::I32, Column::I32])?).is_err());
    assert!(Layout::publish(&set, Packing::with_fields(&[(33, true)])?).is_err());
    // Value lanes that cannot hold the value schema or differ in arity.
    let narrow = WordRunSet::new(&gpu, key_params(0), 4, &[ItemType::U64, ItemType::U32])?;
    type Narrow = WordLayout<(i32,), (u64, i64)>;
    assert!(Narrow::publish(&narrow, Narrow::derive_packing()?).is_err());
    type Arity = WordLayout<(i32,), (u64,)>;
    assert!(Arity::publish(&set, Arity::derive_packing()?).is_err());
    // Key packing wider than the key word, or than its sorted bits.
    type Overwide = WordLayout<(i32, u32, u32), (u64, i32)>;
    assert!(Overwide::publish(&set, Overwide::derive_packing()?).is_err());
    type Pair = WordLayout<(i32, u32), (u64, i32)>;
    assert!(Pair::publish(&set, Pair::derive_packing()?).is_err());
    let window = WordRunSet::new(&gpu, params(1, false, 64, 0), 4, &ITEMS)?;
    assert!(Pair::publish(&window, Pair::derive_packing()?).is_ok());
    // Wider lanes of the same signedness, and unsigned values in wider
    // signed lanes, hold exactly.
    type Widened = WordLayout<(i32,), (u32, u16)>;
    assert!(Widened::publish(&set, Widened::derive_packing()?).is_ok());
    assert!(
        LaneKind {
            bits: 64,
            signed: true
        }
        .holds(Column::U32)
    );
    assert!(
        !LaneKind {
            bits: 32,
            signed: true
        }
        .holds(Column::U32)
    );
    assert!(
        !LaneKind {
            bits: 64,
            signed: false
        }
        .holds(Column::I8)
    );
    assert!(LaneKind::of(<i32 as ItemLane>::TYPE).holds(Column::I16));
    assert!(!LaneKind::of(<u64 as ItemLane>::TYPE).holds(Column::I64));
    assert_eq!(
        LaneKind::of(ItemType::U64),
        LaneKind {
            bits: 64,
            signed: false
        }
    );
    Ok(())
}

#[test]
fn packing_orders_tuples_across_lane_boundaries() -> Result<()> {
    // 40-bit fields straddle both lane boundaries of a 120-bit key.
    let packing = Packing::with_fields(&[(40, true), (40, false), (40, true)])?;
    let half = 1_i128 << 39;
    let values = [-half, -half + 1, -1, 0, 1, half - 2, half - 1];
    let unsigned = [0, 1, half, 2 * half - 2, 2 * half - 1];
    let mut tuples = Vec::new();
    for &a in &values {
        for &b in &unsigned {
            for &c in &values {
                tuples.push([a, b, c]);
            }
        }
    }
    for ms_first in [false, true] {
        let keys = tuples
            .iter()
            .map(|tuple| packing.pack(tuple, 2, ms_first))
            .collect::<Result<Vec<_>>>()?;
        assert!(
            keys.windows(2).all(|pair| pair[0] < pair[1]),
            "lexicographic order"
        );
        for (tuple, key) in tuples.iter().zip(&keys) {
            assert_eq!(packing.unpack(key, ms_first)?, tuple);
            assert_eq!(packing.field(key, ms_first, 2), tuple[2]);
        }
        // A prefix's interval holds exactly the tuples with that prefix.
        for prefix in [&tuples[17][..1], &tuples[40][..2], &tuples[3][..3], &[][..]] {
            let range = packing.prefix_range(prefix, 2, ms_first)?;
            for (tuple, key) in tuples.iter().zip(&keys) {
                assert_eq!(range.contains(key), tuple.starts_with(prefix));
            }
        }
        let mut stray = packing.pack(&tuples[0], 2, ms_first)?.to_vec();
        stray[if ms_first { 0 } else { 1 }] |= 1 << 60;
        assert!(
            packing.unpack(&stray, ms_first).is_err(),
            "bits outside the schema"
        );
    }
    assert!(
        packing.pack(&[0, 0, 0], 1, false).is_err(),
        "word too narrow"
    );
    assert!(packing.pack(&[0, 0], 2, false).is_err(), "arity");
    assert!(packing.prefix_range(&[0, 0, 0, 0], 2, false).is_err());
    assert!(Packing::new(&[0]).is_err() && Packing::new(&[65]).is_err());
    assert!(Packing::new(&[64; 9]).is_err(), "wider than the widest key");
    assert_eq!(Column::I8.encode(-128)?, 0);
    assert_eq!(Column::I8.encode(127)?, 255);
    assert_eq!(
        Column::I64.decode(Column::I64.encode(i64::MIN.into())?)?,
        i128::from(i64::MIN)
    );
    assert!(Column::U16.encode(-1).is_err() && Column::U16.decode(1 << 16).is_err());
    // The device probe ABI reads a 64-bit unsigned field's i64 bit pattern.
    let wide = Packing::with_columns(&[Column::U64, Column::I32])?;
    let range = wide.encode_prefix(&[-1], 2, false).expect("u64::MAX fits");
    // 96 key bits, least-significant lane first: u64::MAX occupies bits 32..96.
    assert_eq!(
        range.start().as_slice(),
        [u64::MAX << 32, u64::from(u32::MAX)]
    );
    assert_eq!(range.end().as_slice(), [u64::MAX, u64::from(u32::MAX)]);
    assert_eq!(wide.encode_prefix(&[0, 1 << 31], 2, false), None);
    Ok(())
}

// ---- Candidate reads (CUDA tests `radix_candidates.rs`) on the CPU ----

type Lookup = WordLayout<(i32,), (i64, u32)>;
type Side = WordLayout<(i32, i32), (i64,)>;

/// Full-domain i32 keys in the i64 probe ABI.
const DOMAIN: [i64; 12] = [
    i32::MIN as i64,
    i32::MIN as i64 + 1,
    -(1 << 30),
    -5,
    -1,
    0,
    1,
    2,
    7,
    1 << 30,
    i32::MAX as i64 - 1,
    i32::MAX as i64,
];
/// Values in no relation: absent prefixes and candidates.
const ABSENT: [i64; 3] = [3, -2, (1 << 30) + 1];

/// One two-level side: occurrences `(prefix, candidate)` in publication
/// order, one run per batch.
struct SideRows {
    batches: Vec<Vec<(i64, i64)>>,
}

impl SideRows {
    /// Distinct candidates of `prefix` in the read enumeration order: runs
    /// oldest first, ascending keys within a run, each key in its oldest run.
    fn candidates(&self, packing: &Packing, prefix: i64) -> Vec<i64> {
        let mut seen = BTreeSet::new();
        let mut order = Vec::new();
        for batch in &self.batches {
            let mut keys: Vec<(PackedKey, i64)> = batch
                .iter()
                .filter(|row| row.0 == prefix)
                .map(|row| {
                    let key = packing
                        .pack(&[row.0.into(), row.1.into()], 2, false)
                        .expect("key fits its packing");
                    (key, row.1)
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
        self.batches
            .iter()
            .flatten()
            .any(|&row| row == (prefix, candidate))
    }
}

fn publish_side(gpu: &Gpu, side: &SideRows) -> Result<(Side, usize)> {
    let packing = Side::derive_packing()?;
    let mut set = WordRunSet::new(&gpu, params(2, false, 128, 4), 2, &[ItemType::I64])?;
    for batch in &side.batches {
        let mut keys = Vec::new();
        for &(prefix, candidate) in batch {
            keys.extend_from_slice(&packing.pack(&[prefix.into(), candidate.into()], 2, false)?);
        }
        let items: Vec<i64> = batch.iter().map(|row| row.1).collect();
        set.publish(
            &gpu.upload_word_keys(&keys, 2)?,
            &[&gpu.upload_items(&items)?],
        )?;
    }
    let runs = set.run_count();
    Ok((Side::publish(&set, packing)?, runs))
}

struct Smallest {
    counts: Vec<usize>,
    choices: Vec<usize>,
    answers: Vec<Vec<i64>>,
}

fn bounds(range: &RangeInclusive<PackedKey>) -> (&[u64], &[u64]) {
    (range.start(), range.end())
}

/// The emitted smallest read over `sides <= 3`: exact distinct candidate
/// counts per side, the first minimum side, then each of its candidates
/// kept iff every other side holds it.
fn run_smallest(layouts: &[&Side], prefixes: &[[i64; 3]]) -> Result<Smallest> {
    let views: Vec<Vec<WordRunView<'_>>> = layouts.iter().map(|layout| layout.views()).collect();
    let reads = views
        .iter()
        .map(|views| WordRunsView::new(views))
        .collect::<Result<Vec<_>>>()?;
    let columns = layouts
        .iter()
        .map(|layout| layout.item_column::<i64>(0))
        .collect::<Result<Vec<_>>>()?;
    let mut result = Smallest {
        counts: Vec::new(),
        choices: Vec::new(),
        answers: Vec::new(),
    };
    for prefix in prefixes {
        let ranges: Vec<_> = (0..layouts.len())
            .map(|side| layouts[side].encode_prefix(&[prefix[side]]))
            .collect();
        let (mut choice, mut minimum) = (0, 0);
        for (side, range) in ranges.iter().enumerate() {
            let total = range.as_ref().map_or(0, |range| {
                let (low, high) = bounds(range);
                reads[side].distinct(low, high)
            });
            result.counts.push(total);
            if side == 0 || total < minimum {
                (choice, minimum) = (side, total);
            }
        }
        result.choices.push(choice);
        let mut kept = Vec::new();
        if let Some(range) = &ranges[choice] {
            let (low, high) = bounds(range);
            let chosen = &reads[choice];
            for ordinal in 0..minimum {
                let (run, slot) = chosen
                    .distinct_at(low, high, ordinal)
                    .expect("ordinal below the distinct count");
                let first = chosen.runs()[run].offset(slot);
                let candidate = columns[choice].get(run, first);
                let live = (0..layouts.len()).all(|other| {
                    other == choice
                        || layouts[other]
                            .encode_prefix(&[prefix[other], candidate])
                            .is_some_and(|key| reads[other].contains(key.start()))
                });
                if live {
                    kept.push(candidate);
                }
            }
            assert_eq!(chosen.distinct_at(low, high, minimum), None);
        }
        result.answers.push(kept);
    }
    Ok(result)
}

/// Compare one smallest read with the candidate-set oracle: exact counts per
/// side, first-in-order minimum, chosen side's candidates filtered by every
/// other side, in the chosen side's enumeration order.
fn check_smallest(sides: &[&SideRows], result: &Smallest, prefixes: &[[i64; 3]]) -> Result<()> {
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
                result.counts[probe * sides.len() + index],
                list.len(),
                "exact distinct count, probe {probe} side {index}"
            );
            if list.len() < candidates[choice].len() {
                choice = index;
            }
        }
        assert_eq!(
            result.choices[probe], choice,
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
        assert_eq!(
            result.answers[probe], expected,
            "smallest answers, probe {probe}"
        );
    }
    Ok(())
}

fn random_side(random: &mut Random, batches: &[usize], prefixes: usize) -> SideRows {
    // Few prefixes and duplicated rows: heavy candidate sharing and repeated
    // (prefix, candidate) keys within and across runs.
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
fn distinct_counts_and_enumerates_each_candidate_once_across_runs() -> Result<()> {
    let gpu = Gpu::new()?;
    let mut random = Random(0x5eed_0001);
    // Decreasing batches stay unmerged under ratio 2: a key repeated in a
    // newer run counts once, in its oldest run, never a per-run sum.
    let side = random_side(&mut random, &[96, 24, 8, 2], 4);
    let (layout, runs) = publish_side(&gpu, &side)?;
    assert_eq!(runs, 4, "the multirun case keeps every publication");
    let mut prefixes: Vec<[i64; 3]> = DOMAIN.iter().map(|&p| [p, 0, 0]).collect();
    prefixes.extend(ABSENT.iter().map(|&p| [p, 0, 0]));
    let result = run_smallest(&[&layout], &prefixes)?;
    check_smallest(&[&side], &result, &prefixes)?;
    let views = layout.views();
    let reads = WordRunsView::new(&views)?;
    let mut repeated = false;
    for (probe, listed) in result.answers.iter().enumerate() {
        assert_eq!(
            listed.iter().collect::<BTreeSet<_>>().len(),
            listed.len(),
            "distinct_at repeats a candidate, probe {probe}"
        );
        if let Some(range) = layout.encode_prefix(&[prefixes[probe][0]]) {
            let (groups, _) = reads.interval_counts(range.start(), range.end());
            repeated |= groups > listed.len();
        }
    }
    assert!(repeated, "some candidate must repeat across runs");
    Ok(())
}

#[test]
fn random_k_way_smallest_matches_candidate_set_oracle() -> Result<()> {
    let gpu = Gpu::new()?;
    for seed in 0..6_u64 {
        let mut random = Random(0x5eed_1000 + seed);
        let k = 2 + (seed % 2) as usize;
        let shapes: [&[usize]; 3] = [&[64, 16, 4], &[40], &[30, 10, 3, 1]];
        let sides: Vec<SideRows> = (0..k)
            .map(|index| random_side(&mut random, shapes[(index + seed as usize) % 3], 3))
            .collect();
        let mut layouts = Vec::new();
        for side in &sides {
            let (layout, runs) = publish_side(&gpu, side)?;
            assert_eq!(runs, side.batches.len(), "every batch stays its own run");
            layouts.push(layout);
        }
        let prefixes = probe_prefixes(&mut random, 300);
        let result = run_smallest(&layouts.iter().collect::<Vec<_>>(), &prefixes)?;
        check_smallest(&sides.iter().collect::<Vec<_>>(), &result, &prefixes)?;
    }
    Ok(())
}

#[test]
fn smallest_counters_cover_ties_empty_and_absent_sides() -> Result<()> {
    let gpu = Gpu::new()?;
    let tie = SideRows {
        batches: vec![vec![(1, 5), (1, 6), (1, 5), (1, 5)], vec![(1, 6), (2, 9)]],
    };
    let same = SideRows {
        batches: vec![vec![(1, 6), (1, 7)], vec![(1, 7), (1, 6)]],
    };
    let empty = SideRows {
        batches: Vec::new(),
    };
    let (tie_layout, tie_runs) = publish_side(&gpu, &tie)?;
    let (same_layout, _) = publish_side(&gpu, &same)?;
    let (empty_layout, empty_runs) = publish_side(&gpu, &empty)?;
    assert_eq!(tie_runs, 2);
    assert_eq!(empty_runs, 0);
    let prefixes = [[1, 1, 1], [2, 1, 1], [3, 3, 3], [1, 1, 0]];
    // Ties (2 vs 2 distinct candidates despite 5 and 4 occurrences) choose
    // the first side; an absent prefix counts zero and ends the probe.
    let result = run_smallest(&[&tie_layout, &same_layout], &prefixes)?;
    assert_eq!(result.counts[0..2], [2, 2]);
    assert_eq!(result.choices[0], 0);
    assert_eq!(result.answers[0], [6]);
    assert_eq!(result.counts[2..4], [1, 2]);
    assert_eq!(result.counts[4..6], [0, 0]);
    assert!(result.answers[2].is_empty());
    check_smallest(&[&tie, &same], &result, &prefixes)?;
    // An empty side is the minimum of every probe (the first, where all
    // three are empty): no answers.
    let result = run_smallest(&[&tie_layout, &same_layout, &empty_layout], &prefixes)?;
    assert_eq!(result.choices, [2, 2, 0, 2]);
    assert!(result.answers.iter().all(Vec::is_empty));
    check_smallest(&[&tie, &same, &empty], &result, &prefixes)?;
    Ok(())
}

#[test]
fn duplicate_heavy_prefix_lookups_equal_stable_groups() -> Result<()> {
    let gpu = Gpu::new()?;
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
            let payload = match serial % 3 {
                0 => i64::MIN,
                1 => i64::MAX,
                _ => random.next() as i64,
            };
            (key, (payload, serial))
        })
        .collect();
    let mut set = WordRunSet::new(&gpu, key_params(3), 2, &[ItemType::I64, ItemType::U32])?;
    let mut start = 0;
    for size in [2000, 600, 300, 100] {
        let batch = &rows[start..start + size];
        start += size;
        let keys = batch
            .iter()
            .map(|row| packing.pack(&[row.0.into()], 1, false).map(|key| key[0]))
            .collect::<Result<Vec<_>>>()?;
        let payloads: Vec<i64> = batch.iter().map(|row| row.1.0).collect();
        let serials: Vec<u32> = batch.iter().map(|row| row.1.1).collect();
        set.publish(
            &gpu.upload_word_keys(&keys, 1)?,
            &[&gpu.upload_items(&payloads)?, &gpu.upload_items(&serials)?],
        )?;
    }
    assert!(set.run_count() > 1);
    let layout = Lookup::publish(&set, packing)?;
    let views = layout.views();
    let reads = WordRunsView::new(&views)?;
    let (payloads, serials) = (layout.item_column::<i64>(0)?, layout.item_column::<u32>(1)?);
    for probe in DOMAIN.into_iter().chain(ABSENT) {
        // Encode the bound key once, then read every run's local interval.
        let range = layout.encode_prefix(&[probe]).expect("probe fits i32");
        let (low, high) = bounds(&range);
        let mut got = Vec::new();
        for (run, view) in views.iter().enumerate() {
            let (begin, end) = view.interval_items(low, high);
            got.extend(
                (begin..end)
                    .map(|position| (payloads.get(run, position), serials.get(run, position))),
            );
        }
        let expected: Vec<(i64, u32)> = rows
            .iter()
            .filter(|row| row.0 == probe)
            .map(|row| row.1)
            .collect();
        assert_eq!(got, expected, "stable occurrences of {probe}");
        assert_eq!(reads.count(low), expected.len());
        assert_eq!(reads.interval_counts(low, high).1, expected.len());
    }
    Ok(())
}
