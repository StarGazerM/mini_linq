//! CPU reads of GPU-built word runs against an independent oracle: a
//! `BTreeMap` from each full key (most-significant lane first, so `Vec`
//! order is numeric order) to its source rows in input order.

use std::{collections::BTreeMap, ops::Bound, time::Instant};

use mccl::{
    Gpu, Result,
    word_read::{WordRunView, WordRunsView},
    words::{OffsetWidth, Offsets, WordRunParams},
};

type Oracle = BTreeMap<Vec<u64>, Vec<u32>>;

/// Most-significant-first numeric form of a key in either layout.
fn numeric(key: &[u64], ms_first: bool) -> Vec<u64> {
    let mut out = key.to_vec();
    if !ms_first {
        out.reverse();
    }
    out
}

/// The layout form of a numeric key (the conversion is an involution).
fn layout(key: &[u64], ms_first: bool) -> Vec<u64> {
    numeric(key, ms_first)
}

fn successor(key: &[u64]) -> Option<Vec<u64>> {
    let mut out = key.to_vec();
    for word in out.iter_mut().rev() {
        let (value, carry) = word.overflowing_add(1);
        *word = value;
        if !carry {
            return Some(out);
        }
    }
    None
}

fn predecessor(key: &[u64]) -> Option<Vec<u64>> {
    let mut out = key.to_vec();
    for word in out.iter_mut().rev() {
        let (value, borrow) = word.overflowing_sub(1);
        *word = value;
        if !borrow {
            return Some(out);
        }
    }
    None
}

fn oracle(keys: &[u64], lanes: usize, ms_first: bool) -> Oracle {
    let mut map = Oracle::new();
    for (row, key) in keys.chunks(lanes).enumerate() {
        map.entry(numeric(key, ms_first))
            .or_default()
            .push(row as u32);
    }
    map
}

/// Distinct keys and items strictly below `key`.
fn below(map: &Oracle, key: &[u64]) -> (usize, usize) {
    map.range::<[u64], _>((Bound::Unbounded, Bound::Excluded(key)))
        .fold((0, 0), |(keys, items), (_, rows)| {
            (keys + 1, items + rows.len())
        })
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Probes: every present key, its neighbours, both domain extremes and
/// random keys; numeric form.
fn probes(map: &Oracle, lanes: usize, seed: u64) -> Vec<Vec<u64>> {
    let mut out = vec![vec![0; lanes], vec![u64::MAX; lanes]];
    for key in map.keys() {
        out.push(key.clone());
        out.extend(predecessor(key));
        out.extend(successor(key));
    }
    let mut state = seed | 1;
    for _ in 0..64 {
        out.push((0..lanes).map(|_| xorshift(&mut state)).collect());
    }
    out
}

fn check_view(view: &WordRunView<'_>, map: &Oracle, rows: usize, seed: u64) {
    let params = view.params();
    let (lanes, ms_first) = (params.lanes, params.ms_first);
    assert_eq!(view.len(), rows);
    assert_eq!(view.key_count(), map.len());
    let probes = probes(map, lanes, seed);
    for probe in &probes {
        let key = layout(probe, ms_first);
        let (slot, found) = view.search(&key);
        assert_eq!(
            (slot, found),
            view.binary_search(&key),
            "directory-bounded search {params:?} {probe:x?}"
        );
        let (keys_below, items_below) = below(map, probe);
        let expected = map.get(probe);
        assert_eq!(
            (slot, found),
            (keys_below, expected.is_some()),
            "search {params:?} {probe:x?}"
        );
        assert_eq!(view.contains(&key), expected.is_some());
        assert_eq!(view.count(&key), expected.map_or(0, Vec::len));
        match expected {
            Some(source) => {
                assert_eq!(view.key(slot), key.as_slice());
                assert_eq!(view.range(&key), (items_below, items_below + source.len()));
                assert_eq!(view.group(slot), view.range(&key));
                assert_eq!(view.rows(slot), source.as_slice(), "stable rows");
            }
            None => assert_eq!(view.range(&key), (rows, rows)),
        }
    }
    let mut state = seed ^ 0x5555;
    for _ in 0..256 {
        let low = &probes[xorshift(&mut state) as usize % probes.len()];
        let high = &probes[xorshift(&mut state) as usize % probes.len()];
        let (first, first_items) = below(map, low);
        let (last, last_items) = match successor(high) {
            Some(next) => below(map, &next),
            None => (map.len(), rows),
        };
        let expected = if last < first {
            ((first, first), (first_items, first_items))
        } else {
            ((first, last), (first_items, last_items))
        };
        let (low, high) = (layout(low, ms_first), layout(high, ms_first));
        assert_eq!(
            view.interval(&low, &high),
            expected.0,
            "interval {params:?}"
        );
        assert_eq!(view.interval_items(&low, &high), expected.1);
    }
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

fn check(gpu: &Gpu, keys: &[u64], params: WordRunParams, seed: u64) -> Result<()> {
    let run = gpu
        .build_word_run(keys, params)?
        .expect("nonempty input publishes a run");
    let view = WordRunView::from(&run);
    check_view(
        &view,
        &oracle(keys, params.lanes, params.ms_first),
        keys.len() / params.lanes,
        seed,
    );
    // The same arrays through the raw constructor read identically.
    let raw = WordRunView::from_parts(
        run.keys(),
        run.offsets(),
        run.order(),
        run.directory(),
        params,
    )?;
    assert_eq!(raw.keys().as_ptr(), run.keys().as_ptr(), "no copy");
    Ok(())
}

/// Duplicate-heavy keys with full-domain extremes in every lane.
fn mixed(rows: usize, lanes: usize, seed: u64) -> Vec<u64> {
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

/// A narrow span far from zero in the least-significant lane only, so the
/// directory resolves low bits under equal high lanes.
fn narrow(rows: usize, lanes: usize, ms_first: bool, seed: u64) -> Vec<u64> {
    let mut state = seed | 1;
    let mut out = Vec::with_capacity(rows * lanes);
    for _ in 0..rows {
        let mut key = vec![0xdead_beef; lanes];
        key[lanes - 1] = (u64::MAX - 4096) + xorshift(&mut state) % 1000 * 3;
        out.extend(layout(&key, ms_first));
    }
    out
}

const LAYOUTS: [(usize, bool); 5] = [(1, false), (2, false), (2, true), (3, false), (3, true)];
const DIRECTORIES: [(u32, OffsetWidth); 5] = [
    (0, OffsetWidth::U32),
    (1, OffsetWidth::U64),
    (8, OffsetWidth::U32),
    (8, OffsetWidth::U64),
    (12, OffsetWidth::U32),
];

#[test]
fn reads_match_the_oracle() -> Result<()> {
    let gpu = Gpu::new()?;
    for (lanes, ms_first) in LAYOUTS {
        for (bits, width) in DIRECTORIES {
            for rows in [1, 2, 7, 300, 5000] {
                let seed = 0x9e37_79b9_7f4a_7c15 ^ (rows * 131 + lanes * 7 + bits as usize) as u64;
                let p = params(lanes, ms_first, bits, width);
                check(&gpu, &mixed(rows, lanes, seed), p, seed)?;
                check(&gpu, &narrow(rows, lanes, ms_first, seed), p, seed)?;
            }
        }
    }
    Ok(())
}

#[test]
fn degenerate_inputs() -> Result<()> {
    let gpu = Gpu::new()?;
    for (lanes, ms_first) in LAYOUTS {
        for (bits, width) in DIRECTORIES {
            let p = params(lanes, ms_first, bits, width);
            for value in [0, 7, u64::MAX] {
                check(&gpu, &vec![value; lanes], p, 1)?; // single key
                check(&gpu, &vec![value; 1000 * lanes], p, 2)?; // all equal
            }
            // Only the extremes, interleaved.
            let extremes: Vec<u64> = (0..999)
                .flat_map(|i| vec![if i % 3 == 0 { 0 } else { u64::MAX }; lanes])
                .collect();
            check(&gpu, &extremes, p, 3)?;
            // Adjacent keys across a lane carry.
            let carry: Vec<u64> = (0..400u64)
                .flat_map(|i| {
                    let mut key = vec![0; lanes];
                    key[lanes - 1] = (u64::MAX - 200).wrapping_add(i);
                    if lanes > 1 && i > 200 {
                        key[lanes - 2] = 1;
                    }
                    layout(&key, ms_first)
                })
                .collect();
            check(&gpu, &carry, p, 4)?;
        }
    }
    Ok(())
}

#[test]
fn multiple_runs_read_as_their_merge() -> Result<()> {
    let gpu = Gpu::new()?;
    for (lanes, ms_first) in LAYOUTS {
        let inputs: Vec<Vec<u64>> = (0..3)
            .map(|i| mixed(1000 >> i, lanes, 0x1234 + i as u64))
            .collect();
        let built = inputs
            .iter()
            .enumerate()
            .map(|(i, keys)| {
                let bits = [0, 8, 12][i];
                let width = [OffsetWidth::U32, OffsetWidth::U64, OffsetWidth::U32][i];
                gpu.build_word_run(keys, params(lanes, ms_first, bits, width))
                    .map(|run| run.expect("nonempty"))
            })
            .collect::<Result<Vec<_>>>()?;
        let views: Vec<_> = built.iter().map(WordRunView::from).collect();
        let runs = WordRunsView::new(&views)?;
        // Merged oracle: older runs' rows first within a key.
        let mut merged = Oracle::new();
        let mut base = 0;
        for keys in &inputs {
            for (key, rows) in oracle(keys, lanes, ms_first) {
                merged
                    .entry(key)
                    .or_default()
                    .extend(rows.iter().map(|row| row + base));
            }
            base += (keys.len() / lanes) as u32;
        }
        let items = base as usize;
        assert_eq!(runs.len(), items);
        let probes = probes(&merged, lanes, 99);
        for probe in &probes {
            let key = layout(probe, ms_first);
            let expected = merged.get(probe);
            assert_eq!(runs.contains(&key), expected.is_some());
            assert_eq!(runs.count(&key), expected.map_or(0, Vec::len));
            let (_, items_below) = below(&merged, probe);
            let range = match expected {
                Some(rows) => (items_below, items_below + rows.len()),
                None => (items, items),
            };
            assert_eq!(runs.range(&key), range, "merged range {probe:x?}");
        }
        let per_run: Vec<Oracle> = inputs
            .iter()
            .map(|keys| oracle(keys, lanes, ms_first))
            .collect();
        let mut state = 77;
        for _ in 0..256 {
            let low = &probes[xorshift(&mut state) as usize % probes.len()];
            let high = &probes[xorshift(&mut state) as usize % probes.len()];
            let within = |map: &Oracle| -> Vec<Vec<u64>> {
                if low > high {
                    return Vec::new();
                }
                map.range::<[u64], _>((
                    Bound::Included(low.as_slice()),
                    Bound::Included(high.as_slice()),
                ))
                .map(|(key, _)| key.clone())
                .collect()
            };
            let expected = within(&merged);
            let groups: usize = per_run.iter().map(|map| within(map).len()).sum();
            let occurrences: usize = expected.iter().map(|key| merged[key].len()).sum();
            // CUDA's enumeration: runs oldest first, ascending within a run,
            // each key once, in its oldest run.
            let mut seen = std::collections::BTreeSet::new();
            let mut enumeration = Vec::new();
            for map in &per_run {
                for key in within(map) {
                    if seen.insert(key.clone()) {
                        enumeration.push(key);
                    }
                }
            }
            let (low, high) = (layout(low, ms_first), layout(high, ms_first));
            assert_eq!(runs.distinct(&low, &high), expected.len());
            assert_eq!(runs.interval_counts(&low, &high), (groups, occurrences));
            for (ordinal, key) in enumeration.iter().enumerate() {
                let (run, slot) = runs
                    .distinct_at(&low, &high, ordinal)
                    .expect("ordinal below the distinct count");
                assert_eq!(numeric(views[run].key(slot), ms_first), *key);
            }
            assert_eq!(runs.distinct_at(&low, &high, enumeration.len()), None);
        }
    }
    let empty = WordRunsView::new(&[])?;
    assert!(empty.is_empty() && !empty.contains(&[1]) && empty.range(&[1]) == (0, 0));
    Ok(())
}

#[test]
fn invalid_views_are_rejected() -> Result<()> {
    let gpu = Gpu::new()?;
    let p = params(2, false, 4, OffsetWidth::U32);
    let run = gpu
        .build_word_run(&[1, 2, 3, 4, 1, 2], p)?
        .expect("nonempty");
    let (keys, offsets, order, directory) =
        (run.keys(), run.offsets(), run.order(), run.directory());
    assert!(WordRunView::from_parts(keys, offsets, order, directory, p).is_ok());
    assert!(WordRunView::from_parts(&keys[..3], offsets, order, directory, p).is_err());
    assert!(WordRunView::from_parts(keys, offsets, &order[..2], directory, p).is_err());
    assert!(WordRunView::from_parts(keys, Offsets::U64(&[0, 2, 3]), order, directory, p).is_err());
    assert!(WordRunView::from_parts(keys, offsets, order, Offsets::U32(&[]), p).is_err());
    let other = WordRunParams { lanes: 3, ..p };
    assert!(WordRunView::from_parts(keys, offsets, order, directory, other).is_err());
    let two = WordRunView::from(&run);
    let one = gpu
        .build_word_run(&[5], params(1, false, 0, OffsetWidth::U32))?
        .expect("nonempty");
    assert!(WordRunsView::new(&[two, WordRunView::from(&one)]).is_err());
    Ok(())
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// Warm lookup plus expansion of 65,536 probes at 262,144 distinct u64 keys,
/// ~50% hits; median of 7. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn lookup_timing() -> Result<()> {
    let gpu = Gpu::new()?;
    let distinct = 262_144u64;
    let mut state = 0xfeed;
    let sparse: Vec<u64> = (0..distinct).map(|_| xorshift(&mut state) | 1).collect();
    let dense: Vec<u64> = (0..distinct).map(|i| i * 2).collect();
    for (name, keys) in [("sparse", sparse), ("dense", dense)] {
        let probes: Vec<u64> = (0..65_536)
            .map(|_| {
                let key = keys[xorshift(&mut state) as usize % keys.len()];
                if xorshift(&mut state) & 1 == 0 {
                    key
                } else {
                    key ^ 1
                }
            })
            .collect();
        for (lanes, bits) in [(1, 0), (1, 12), (2, 0), (2, 12)] {
            // Upper lanes constant: the lowest lane decides every compare.
            let widen = |key: u64| (0..lanes).map(move |lane| if lane == 0 { key } else { 7 });
            let wide: Vec<u64> = keys.iter().flat_map(|&key| widen(key)).collect();
            let wide_probes: Vec<u64> = probes.iter().flat_map(|&key| widen(key)).collect();
            let run = gpu
                .build_word_run(&wide, params(lanes, false, bits, OffsetWidth::U32))?
                .expect("nonempty");
            let view = WordRunView::from(&run);
            let mut samples = Vec::new();
            let mut hits = 0;
            for round in 0..8 {
                let start = Instant::now();
                let (mut found, mut sum) = (0usize, 0u64);
                for probe in wide_probes.chunks(lanes) {
                    let (begin, end) = view.range(probe);
                    found += (begin != end) as usize;
                    sum += view.order()[begin..end]
                        .iter()
                        .map(|&row| row as u64)
                        .sum::<u64>();
                }
                std::hint::black_box(sum);
                hits = found;
                if round > 0 {
                    samples.push(start.elapsed().as_secs_f64() * 1e3);
                }
            }
            println!(
                "{name} lanes={lanes} radix_bits={bits}: {:.3} ms median of 7, {hits} hits",
                median(samples)
            );
        }
    }
    Ok(())
}
