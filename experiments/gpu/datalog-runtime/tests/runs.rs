use std::{collections::BTreeSet, sync::Arc};

use cuda_core::LaunchConfig1D;
use gpu_datalog_runtime::{
    pack::{Packing, Word},
    DeviceRadixRun, SortedBatch, RadixRunParams,
};
use gpu_device_columns::{Buffer, Pool};

type Result<T = (), E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

const PARAMS: RadixRunParams = RadixRunParams {
    ratio: 4,
    radix_bits: 0,
    merge_tile_rows: 32,
    block_threads: 256,
};

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

fn word<W: Word>(value: u64) -> W {
    // Populate every lane while preserving value order in the leading lane.
    // Low lanes repeat, exposing implementations that ignore high lanes.
    let mut word = W::zero();
    for lane in 0..W::BITS / 64 {
        word.set_lane(
            lane,
            if lane == W::BITS / 64 - 1 {
                value
            } else {
                value.rotate_left(lane * 11) & 127
            },
        );
    }
    word
}

fn host_keys<W: Word>(state: &DeviceRadixRun<W, u64, ()>) -> Result<Vec<W>> {
    let mut keys = state.to_host()?.into_iter().flat_map(|run| run.keys).collect::<Vec<_>>();
    keys.sort_unstable();
    Ok(keys)
}

fn assert_run_bound<W: Word>(state: &DeviceRadixRun<W, u64, ()>) {
    let views = state.run_lengths();
    assert_eq!(
        views.iter().map(|view| view.0).sum::<usize>(),
        state.len()
    );
    assert!(views.iter().all(|view| view.0 > 0));
    if state.len() == 0 {
        assert!(views.is_empty());
        return;
    }
    // floor(log_4(n)) + 2, avoiding rounding at exact powers of four.
    let mut remaining = state.len();
    let mut bound = 2;
    while remaining >= 4 {
        remaining /= 4;
        bound += 1;
    }
    assert!(
        views.len() <= bound,
        "{} runs for {} rows; bound {bound}",
        views.len(),
        state.len()
    );
}

/// Ascent-shaped admission: fresh against accepted runs, then publication.
fn admit<W: Word>(state: &mut DeviceRadixRun<W, u64, ()>, input: &Buffer<W>, rows: usize) -> Result<SortedBatch<W>> {
    let fresh = unsafe { state.fresh(input, rows) }?;
    // SAFETY: fresh is sorted, unique and absent from every accepted run.
    unsafe { state.append_sorted(fresh.clone())? };
    Ok(fresh)
}

fn admit_against_cpu<W: Word>(
    pool: &Arc<Pool>,
    state: &mut DeviceRadixRun<W, u64, ()>,
    accepted: &mut BTreeSet<W>,
    values: &[W],
) -> Result<SortedBatch<W>> {
    let input = Buffer::from_host(pool.clone(), values)?;
    let batch: BTreeSet<_> = values.iter().copied().collect();
    let expected_delta: Vec<_> = batch.difference(accepted).copied().collect();
    let before = state.len();
    assert_eq!(
        unsafe { state.fresh(&input, values.len()) }?.len(),
        expected_delta.len()
    );
    assert_eq!(state.len(), before, "fresh must not publish rows");
    let fresh = admit(state, &input, values.len())?;
    assert_eq!(fresh.len(), expected_delta.len());
    assert_eq!(fresh.buffer().to_host_prefix(fresh.len())?, expected_delta);
    assert_eq!(fresh.len(), fresh.len());
    assert_eq!(input.to_host()?, values, "admission mutated its input");
    accepted.extend(batch);
    assert_eq!(state.len(), accepted.len());
    assert_eq!(
        host_keys(&state)?,
        accepted.iter().copied().collect::<Vec<_>>()
    );
    assert_run_bound(state);
    Ok(fresh)
}

fn random_batches<W: Word>() -> Result {
    let pool = pool()?;
    let mut state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, <W as Word>::BITS)?;
    let mut accepted = BTreeSet::new();
    let mut random = Random(0xa523_4b19_c018_762d);
    let mut retained = Vec::new();
    for batch in 0..192 {
        let size = match batch % 12 {
            0 => 0,
            1 => 1,
            2 => 17,
            3 => 257,
            4 => 4096,
            _ => (random.next() % 1024) as usize,
        };
        let mut values = Vec::with_capacity(size);
        for row in 0..size {
            // Both within-batch duplicates and overlap with previous batches.
            let value = if row > 0 && row % 3 == 0 {
                values[row / 3]
            } else {
                word::<W>(random.next() % 65_537)
            };
            values.push(value);
        }
        let fresh = admit_against_cpu(&pool, &mut state, &mut accepted, &values)?;
        if batch % 19 == 0 {
            let expected = fresh.buffer().to_host_prefix(fresh.len())?;
            retained.push((fresh.clone(), expected));
        }
    }
    // Compaction must not invalidate previously returned immutable deltas.
    drop(state);
    for (run, expected) in retained {
        assert_eq!(run.buffer().to_host_prefix(run.len())?, expected);
    }
    Ok(())
}

fn known_unique_sort<W: Word>() -> Result {
    let pool = pool()?;
    let state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, <W as Word>::BITS)?;
    for rows in [0, 1, 17, 257, 4097] {
        let mut values: Vec<_> = (0..rows).rev().map(|row| word::<W>(row as u64)).collect();
        if rows > 1 {
            let mut maximum = W::zero();
            for lane in 0..W::BITS / 64 {
                maximum.set_lane(lane, u64::MAX);
            }
            values[0] = maximum;
        }
        // The repeated suffix must not be included in the live prefix.
        let mut upload = values.clone();
        upload.extend([W::zero(), W::zero()]);
        let input = Buffer::from_host(pool.clone(), &upload)?;
        let before = gpu_datalog_runtime::readback_count();
        // SAFETY: distinct leading lanes make every prefix word unique.
        let sorted = unsafe { state.sort_known_unique(&input, rows)? };
        assert_eq!(gpu_datalog_runtime::readback_count(), before);
        assert_eq!(sorted.len(), rows);
        values.sort_unstable();
        assert_eq!(sorted.buffer().to_host_prefix(rows)?, values);
        assert_eq!(input.to_host()?, upload);
        assert!(state.is_empty());
    }
    Ok(())
}

#[test]
fn known_unique_sort_preserves_rows_without_readback() -> Result {
    known_unique_sort::<u64>()?;
    known_unique_sort::<u128>()?;
    known_unique_sort::<[u64; 3]>()?;
    known_unique_sort::<[u64; 4]>()?;
    known_unique_sort::<[u64; 5]>()
}

#[test]
fn random_duplicate_batches_u64() -> Result {
    random_batches::<u64>()
}

#[test]
fn random_duplicate_batches_u128() -> Result {
    random_batches::<u128>()
}

#[test]
fn random_duplicate_batches_three_lanes() -> Result {
    random_batches::<[u64; 3]>()
}

#[test]
fn random_duplicate_batches_four_lanes() -> Result {
    random_batches::<[u64; 4]>()
}

fn empty_single_and_extremes<W: Word>() -> Result {
    let pool = pool()?;
    let mut state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, <W as Word>::BITS)?;
    let mut accepted = BTreeSet::new();
    assert_eq!(state.len(), 0);
    assert!(host_keys(&state)?.is_empty());
    assert_run_bound(&state);
    let zero = W::zero();
    let mut max = W::zero();
    for lane in 0..W::BITS / 64 {
        max.set_lane(lane, u64::MAX);
    }
    let mut high = W::zero();
    high.set_lane(W::BITS / 64 - 1, 1_u64 << 63);
    for values in [
        vec![],
        vec![zero],
        vec![zero],
        vec![],
        vec![max],
        vec![high, max, high, zero],
        vec![],
    ] {
        admit_against_cpu(&pool, &mut state, &mut accepted, &values)?;
    }
    // Only the declared live prefix participates, not spare initialized capacity.
    let input = Buffer::from_host(pool.clone(), &[zero, W::from_u128(7)])?;
    assert_eq!(admit(&mut state, &input, 1)?.len(), 0);
    assert_eq!(admit(&mut state, &input, 0)?.len(), 0);
    assert_eq!(
        host_keys(&state)?,
        accepted.iter().copied().collect::<Vec<_>>()
    );
    assert_run_bound(&state);
    Ok(())
}

#[test]
fn empty_single_and_extremes_u64() -> Result {
    empty_single_and_extremes::<u64>()
}

#[test]
fn empty_single_and_extremes_u128() -> Result {
    empty_single_and_extremes::<u128>()
}

#[test]
fn empty_single_and_extremes_wide_lanes() -> Result {
    empty_single_and_extremes::<[u64; 3]>()?;
    empty_single_and_extremes::<[u64; 4]>()
}

fn million_rows<W: Word>() -> Result {
    let pool = pool()?;
    let mut state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, <W as Word>::BITS)?;
    let mut random = Random(0x6b73_d147_590a_821f);
    let values: Vec<_> = (0..1_000_000)
        .map(|_| word::<W>(random.next() % 600_001))
        .collect();
    let mut expected = values.clone();
    expected.sort_unstable();
    expected.dedup();
    let input = Buffer::from_host(pool.clone(), &values)?;
    let fresh = admit(&mut state, &input, values.len())?;
    assert_eq!(fresh.buffer().to_host_prefix(fresh.len())?, expected);
    assert_eq!(state.len(), expected.len());
    assert_eq!(host_keys(&state)?, expected);
    assert_run_bound(&state);
    let duplicate = admit(&mut state, &input, values.len())?;
    assert_eq!(duplicate.len(), 0);
    assert_eq!(state.len(), expected.len());
    assert_eq!(host_keys(&state)?, expected);
    assert_run_bound(&state);
    Ok(())
}

#[test]
fn million_rows_u64() -> Result {
    million_rows::<u64>()
}

#[test]
fn million_rows_u128() -> Result {
    million_rows::<u128>()
}

fn append_disjoint_runs<W: Word>() -> Result {
    let pool = pool()?;
    let mut state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, <W as Word>::BITS)?;
    let mut accepted = BTreeSet::new();
    for batch in 0..128 {
        // Runs interleave numerically; they are not adjacent sorted chunks.
        let values: Vec<_> = (0..33).map(|row| word::<W>(row * 128 + batch)).collect();
        let buffer = Buffer::from_host(pool.clone(), &values)?;
        // Values are strictly increasing and each batch has a distinct residue.
        let run = unsafe { SortedBatch::from_sorted(buffer, values.len())? };
        // SAFETY: different batch residues make these sorted runs disjoint.
        unsafe { state.append_sorted(run.clone())? };
        accepted.extend(values.iter().copied());
        assert_eq!(run.buffer().to_host_prefix(run.len())?, values);
        assert_run_bound(&state);
    }
    let empty = Buffer::<W>::from_host(pool, &[])?;
    // SAFETY: an empty initialized run is sorted, unique, and disjoint.
    unsafe { state.append_sorted(SortedBatch::from_sorted(empty, 0)?)? };
    assert_eq!(host_keys(&state)?, accepted.into_iter().collect::<Vec<_>>());
    assert_run_bound(&state);
    Ok(())
}

#[test]
fn recorded_ratio_changes_compaction_without_changing_rows() -> Result {
    let pool = pool()?;
    assert!(DeviceRadixRun::<u64, u64, ()>::new(pool.clone(), RadixRunParams { ratio: 1, ..PARAMS }).is_err());
    for bad in [
        RadixRunParams {
            merge_tile_rows: 0,
            ..PARAMS
        },
        RadixRunParams {
            block_threads: 0,
            ..PARAMS
        },
        RadixRunParams {
            block_threads: 128,
            ..PARAMS
        },
        RadixRunParams {
            block_threads: 512,
            ..PARAMS
        },
    ] {
        assert!(DeviceRadixRun::<u64, u64, ()>::new(pool.clone(), bad).is_err());
    }
    for (ratio, expected_runs) in [(2, 2), (4, 2), (8, 1)] {
        let mut state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), RadixRunParams { ratio, ..PARAMS }, <u64 as Word>::BITS)?;
        let first = Buffer::from_host(pool.clone(), &[0u64, 1, 2, 3, 4, 5])?;
        let second = Buffer::from_host(pool.clone(), &[6u64])?;
        admit(&mut state, &first, 6)?;
        admit(&mut state, &second, 1)?;
        assert_eq!(state.run_count(), expected_runs);
        assert_eq!(host_keys(&state)?, (0u64..7).collect::<Vec<_>>());
        let duplicate = admit(&mut state, &second, 1)?;
        assert!(duplicate.is_empty());
        assert_eq!(host_keys(&state)?, (0u64..7).collect::<Vec<_>>());
    }
    Ok(())
}

#[test]
fn recorded_merge_tile_preserves_wide_union() -> Result {
    let pool = pool()?;
    for merge_tile_rows in [1, 7, 32, 129] {
        let mut state = DeviceRadixRun::<[u64; 3], u64, ()>::new_set(pool.clone(), RadixRunParams {
            merge_tile_rows,
            ..PARAMS
        }, <[u64; 3] as Word>::BITS)?;
        for parity in [0, 1] {
            let rows = (0..137)
                .map(|i| word::<[u64; 3]>(i * 2 + parity))
                .collect::<Vec<_>>();
            let input = Buffer::from_host(pool.clone(), &rows)?;
            admit(&mut state, &input, rows.len())?;
        }
        assert_eq!(state.run_count(), 1);
        assert_eq!(
            host_keys(&state)?,
            (0..274).map(word::<[u64; 3]>).collect::<Vec<_>>()
        );
    }
    Ok(())
}

#[test]
fn append_sorted_disjoint_u64() -> Result {
    append_disjoint_runs::<u64>()
}

#[test]
fn append_sorted_disjoint_u128() -> Result {
    append_disjoint_runs::<u128>()
}

#[test]
fn append_sorted_disjoint_wide_lanes() -> Result {
    append_disjoint_runs::<[u64; 3]>()?;
    append_disjoint_runs::<[u64; 4]>()
}

fn merge_wide_tile_boundaries<W: Word>() -> Result {
    let pool = pool()?;
    for lane in 0..W::BITS / 64 {
        for (first_len, second_len) in [
            (1, 1),
            (31, 33),
            (32, 32),
            (33, 31),
            (257, 65),
            (1, 8193),
            (8193, 2049),
            (8191, 8193),
        ] {
            for pattern in 0..3 {
                let make_word = |value| {
                    // Hold every other lane fixed, forcing comparisons through
                    // each possible lane, including the least-significant one.
                    let mut word = W::zero();
                    for fixed in 0..W::BITS / 64 {
                        word.set_lane(fixed, u64::MAX);
                    }
                    word.set_lane(lane, value);
                    word
                };
                let first: Vec<_> = (0..first_len)
                    .map(|row| {
                        make_word(match pattern {
                            0 => row * 2,
                            1 => row,
                            _ => second_len + row,
                        } as u64)
                    })
                    .collect();
                let second: Vec<_> = (0..second_len)
                    .map(|row| {
                        make_word(match pattern {
                            0 => row * 2 + 1,
                            1 => first_len + row,
                            _ => row,
                        } as u64)
                    })
                    .collect();
                let mut expected = first.clone();
                expected.extend_from_slice(&second);
                expected.sort_unstable();
                let mut state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, <W as Word>::BITS)?;
                let first_buffer = Buffer::from_host(pool.clone(), &first)?;
                let second_buffer = Buffer::from_host(pool.clone(), &second)?;
                // SAFETY: both sequences increase in the selected lane; their
                // ranks are disjoint in every pattern.
                let snapshot = unsafe { SortedBatch::from_sorted(first_buffer, first_len)? };
                unsafe {
                    state.append_sorted(snapshot.clone())?;
                    state.append_sorted(SortedBatch::from_sorted(second_buffer, second_len)?)?;
                }
                // All selected length ratios must exercise the actual merge,
                // not merely the diagnostic host merge of separate runs.
                assert_eq!(state.run_count(), 1);
                let merged = &state.to_host()?[0];
                assert_eq!(
                    merged.keys,
                    expected,
                    "lane {lane}, lengths {first_len}/{second_len}, pattern {pattern}"
                );
                assert_eq!(snapshot.buffer().to_host_prefix(snapshot.len())?, first);
            }
        }
    }
    Ok(())
}

#[test]
fn append_sorted_wide_merge_tile_boundaries() -> Result {
    merge_wide_tile_boundaries::<[u64; 3]>()?;
    merge_wide_tile_boundaries::<[u64; 4]>()
}

#[cuda_host::cuda_module]
mod prefix_kernel {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    use gpu_datalog_runtime::radix_run::{device, RadixKeysView};
    use gpu_datalog_runtime::Word;

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn lookup<W: Word>(
        views: RadixKeysView<W, u64>,
        bounds: *const W,
        queries: usize,
        capacity: usize,
        output: *mut W,
        counts: *mut u64,
    ) {
        let query = thread::index_1d().get();
        if query >= queries {
            return;
        }
        unsafe {
            let lo = *bounds.add(query * 2);
            let hi = *bounds.add(query * 2 + 1);
            let mut count = 0;
            for run in 0..views.run_count {
                let view = device::run(views, run);
                let (start, end) = device::interval(view, lo, hi);
                for row in start..end {
                    *output.add(query * capacity + count) = *view.keys.add(row as usize);
                    count += 1;
                }
            }
            *counts.add(query) = count as u64;
        }
    }
}

fn permutation_against_cpu<W: Word>(packing: Packing) -> Result {
    let pool = pool()?;
    let fields = packing.packed_fields().collect::<Vec<_>>();
    let mut random = Random(0x8b37_6ce4_a905_d12f);
    let mut tuples = Vec::new();
    // Extremes, each individual packed bit, and random full-width values make
    // every lane observable independently of sorting/admission correctness.
    for row in 0..packing.bits() as usize + 259 {
        let values = fields
            .iter()
            .map(|field| {
                let mask = u64::MAX >> (64 - field.bits);
                let encoded = if row == 0 {
                    0
                } else if row == 1 {
                    mask
                } else if row < packing.bits() as usize + 2 {
                    let bit = (row - 2) as u32;
                    if bit >= field.shift && bit - field.shift < field.bits {
                        1_u64 << (bit - field.shift)
                    } else {
                        0
                    }
                } else {
                    random.next() & mask
                };
                encoded as i128
                    - if field.signed != 0 {
                        1_i128 << (field.bits - 1)
                    } else {
                        0
                    }
            })
            .collect::<Vec<_>>();
        tuples.push(values);
    }
    let source = tuples
        .iter()
        .map(|row| packing.pack::<W>(row))
        .collect::<Result<Vec<_>>>()?;
    let input = Buffer::from_host(pool.clone(), &source)?;
    let identity = (0..packing.len()).collect::<Vec<_>>();
    let reverse = identity.iter().rev().copied().collect::<Vec<_>>();
    let mut rotated = identity.clone();
    if !rotated.is_empty() {
        rotated.rotate_left(1);
    }
    for order in [identity, reverse, rotated] {
        let layout = packing.permute(&order)?;
        let mut inverse = vec![0; order.len()];
        for (destination, &original) in order.iter().enumerate() {
            inverse[original] = destination;
        }
        let expected = tuples
            .iter()
            .map(|row| {
                layout.pack::<W>(&order.iter().map(|&column| row[column]).collect::<Vec<_>>())
            })
            .collect::<Result<Vec<_>>>()?;
        // Include a short initialized prefix, so capacity is not mistaken for
        // the row count, plus an empty launch and a partial final thread block.
        for rows in [0, 1, source.len() - 1, source.len()] {
            let shuffled = packing.permute_device(&input, rows, &order, 256)?;
            assert_eq!(shuffled.len(), rows);
            assert_eq!(
                shuffled.to_host()?,
                expected[..rows],
                "{}-bit permutation {order:?}, {rows} rows",
                W::BITS
            );
            let restored = layout.permute_device(&shuffled, rows, &inverse, 256)?;
            assert_eq!(restored.to_host()?, source[..rows]);
        }
    }
    assert_eq!(input.to_host()?, source, "permutation mutated its input");
    Ok(())
}

#[test]
fn native_permutation_retains_every_lane() -> Result {
    use gpu_datalog_runtime::pack::Column;
    permutation_against_cpu::<u64>(Packing::new(&[1, 31, 32])?)?;
    permutation_against_cpu::<u64>(Packing::new(&[64])?)?;
    permutation_against_cpu::<u64>(Packing::new(&[])?)?;
    for widths in [&[40, 40, 16][..], &[63, 1, 64], &[64, 64], &[1; 128], &[]] {
        permutation_against_cpu::<u128>(Packing::new(widths)?)?;
    }
    permutation_against_cpu::<u128>(Packing::with_columns(&[
        Column::I8,
        Column::U64,
        Column::I16,
        Column::U32,
    ])?)
}

#[test]
fn wide_permutation_retains_lanes_and_u32_metadata() -> Result {
    use gpu_datalog_runtime::pack::Column;
    permutation_against_cpu::<[u64; 2]>(Packing::new(&[40, 40, 16])?)?;
    permutation_against_cpu::<[u64; 3]>(Packing::new(&[63, 64, 64])?)?;
    permutation_against_cpu::<[u64; 5]>(Packing::new(&[7, 64, 64, 64, 57])?)?;
    // Exercise shifts and column counts beyond u8, as well as all five lanes.
    permutation_against_cpu::<[u64; 5]>(Packing::new(&[64; 5])?)?;
    permutation_against_cpu::<[u64; 5]>(Packing::new(&[1; 257])?)?;
    permutation_against_cpu::<[u64; 5]>(Packing::new(&[])?)?;
    permutation_against_cpu::<[u64; 5]>(Packing::with_columns(&[
        Column::I64,
        Column::U8,
        Column::I64,
        Column::U64,
        Column::I16,
    ])?)
}

fn permuted_layout_prefix_lookup<W: Word>(widths: &[u32]) -> Result {
    let pool = pool()?;
    let packing = Packing::new(widths)?;
    let order = [2, 0, 1];
    let layout = packing.permute(&order)?;
    let mut canonical = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, packing.bits())?;
    let mut permuted = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, layout.bits())?;
    let mut rows = BTreeSet::new();
    let mut random = Random(0xd9e2_051b_c76a_438f);
    for batch in 0..12 {
        let mut values = Vec::new();
        for _ in 0..513 {
            let row = [
                (1i128 << 35) + (random.next() % 17) as i128,
                (1i128 << 36) + (random.next() % 13) as i128,
                (random.next() % 7) as i128,
            ];
            values.push(packing.pack::<W>(&row)?);
            rows.insert(row);
        }
        if batch == 0 {
            for row in [[0, 0, 0], [(1i128 << 40) - 1, (1i128 << 40) - 1, 65_535]] {
                values.push(packing.pack::<W>(&row)?);
                rows.insert(row);
            }
        }
        let input = Buffer::from_host(pool.clone(), &values)?;
        let fresh = admit(&mut canonical, &input, values.len())?;
        let shuffled = packing.permute_device(fresh.buffer(), fresh.len(), &order, 256)?;
        let source = fresh.buffer().to_host_prefix(fresh.len())?;
        let expected = source
            .iter()
            .map(|&key| {
                let row = packing.unpack(key)?;
                layout.pack::<W>(&[row[2], row[0], row[1]])
            })
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(shuffled.to_host()?, expected);
        // Sort each disjoint permuted delta before appending to the other layout.
        let before = gpu_datalog_runtime::readback_count();
        // SAFETY: the permutation is bijective and fresh is already unique.
        let sorted = unsafe { permuted.sort_known_unique(&shuffled, fresh.len())? };
        assert_eq!(gpu_datalog_runtime::readback_count(), before);
        // SAFETY: admission removed old canonical keys; the permutation is
        // bijective, so its sorted delta is disjoint from this layout's state.
        unsafe { permuted.append_sorted(sorted)? };
        assert_run_bound(&canonical);
        assert_run_bound(&permuted);
    }
    let expected_rows: BTreeSet<_> = rows.iter().map(|row| [row[2], row[0], row[1]]).collect();
    let actual_rows = host_keys(&permuted)?
        .into_iter()
        .map(|key| layout.unpack(key).map(|row| [row[0], row[1], row[2]]))
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(
        actual_rows,
        expected_rows.iter().copied().collect::<Vec<_>>()
    );

    let mut prefixes = vec![
        vec![],
        vec![0, 0, 0],
        vec![65_535],
        vec![65_535, (1i128 << 40) - 1, (1i128 << 40) - 1],
    ];
    for first in 0..9 {
        prefixes.push(vec![first]);
        for second in [
            1i128 << 35,
            (1i128 << 35) + 8,
            (1i128 << 35) + 16,
            (1i128 << 35) + 17,
        ] {
            prefixes.push(vec![first, second]);
            prefixes.push(vec![first, second, (1i128 << 36) + 6]);
        }
    }
    let mut bounds = Vec::with_capacity(prefixes.len() * 2);
    for prefix in &prefixes {
        let range = layout.prefix_range::<W>(prefix)?;
        bounds.extend([*range.start(), *range.end()]);
    }
    let snapshot = permuted.snapshot()?;
    let device_bounds = Buffer::from_host(pool.clone(), &bounds)?;
    let output = Buffer::<W>::zeroed(pool.clone(), prefixes.len() * permuted.len())?;
    let counts = Buffer::<u64>::zeroed(pool.clone(), prefixes.len())?;
    let module = unsafe { prefix_kernel::load(pool.context())? };
    module.lookup::<W>(
        pool.stream(),
        &module.prepare_lookup::<W>(LaunchConfig1D::new(
            prefixes.len().div_ceil(256) as u32,
            256,
            0,
        ))?,
        snapshot.keys_view(),
        device_bounds.ptr(),
        prefixes.len(),
        permuted.len(),
        output.ptr(),
        counts.ptr(),
    )?;
    let counts = counts.to_host()?;
    let output = output.to_host()?;
    for (query, prefix) in prefixes.iter().enumerate() {
        let expected: Vec<_> = expected_rows
            .iter()
            .filter(|row| row.starts_with(prefix))
            .copied()
            .collect();
        let count = counts[query] as usize;
        assert!(count <= permuted.len());
        let mut actual = output[query * permuted.len()..query * permuted.len() + count]
            .iter()
            .map(|&key| layout.unpack(key).map(|row| [row[0], row[1], row[2]]))
            .collect::<Result<Vec<_>>>()?;
        actual.sort_unstable();
        assert_eq!(actual, expected, "prefix {prefix:?}");
    }
    Ok(())
}

#[test]
fn permuted_layout_prefix_lookup_matches_cpu() -> Result {
    permuted_layout_prefix_lookup::<u128>(&[40, 40, 16])
}

#[test]
fn permuted_wide_layout_prefix_lookup_matches_cpu() -> Result {
    permuted_layout_prefix_lookup::<[u64; 3]>(&[64, 64, 16])?;
    permuted_layout_prefix_lookup::<[u64; 4]>(&[64, 64, 64])
}

#[test]
#[ignore = "throughput probe: requires GPU memory for 50M accepted rows and 10M candidates"]
fn throughput_10m_candidates_against_50m_accepted() -> Result {
    const ACCEPTED: usize = 50_000_000;
    const CANDIDATES: usize = 10_000_000;
    let pool = pool()?;
    let mut state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), PARAMS, <u64 as Word>::BITS)?;
    let seed: Vec<_> = (0..ACCEPTED as u64).map(|row| row * 2).collect();
    let buffer = Buffer::from_host(pool.clone(), &seed)?;
    drop(seed);
    // SAFETY: the upload initialized the sorted unique seed; state is empty.
    unsafe { state.append_sorted(SortedBatch::from_sorted(buffer, ACCEPTED)?)? };
    let mut random = Random(0x49ac_651f_0382_bd77);
    let mut candidates: Vec<_> = (0..CANDIDATES)
        .map(|_| random.next() % (2 * ACCEPTED as u64))
        .collect();
    let input = Buffer::from_host(pool, &candidates)?;
    candidates.retain(|value| value % 2 == 1);
    candidates.sort_unstable();
    candidates.dedup();
    let fresh = admit(&mut state, &input, CANDIDATES)?;
    assert_eq!(fresh.buffer().to_host_prefix(fresh.len())?, candidates);
    assert_eq!(state.len(), ACCEPTED + candidates.len());
    assert_run_bound(&state);
    Ok(())
}

#[test]
fn full_key_admission_is_read_only_across_runs_and_adopts_shared_batches() -> Result {
    use gpu_datalog_runtime::RadixLayout;
    for radix_bits in [0, 4] {
        let pool = pool()?;
        let params = RadixRunParams { radix_bits, ..PARAMS };
        let packing = Packing::with_columns(&[gpu_datalog_runtime::Column::U64])?;
        let mut state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), params, <u64 as Word>::BITS)?;
        for values in [(0..64).collect::<Vec<_>>(), vec![100, 101, 102, 103]] {
            let input = Buffer::from_host(pool.clone(), &values)?;
            let batch = unsafe { state.fresh(&input, values.len()) }?;
            let before = gpu_datalog_runtime::readback_count();
            unsafe { state.append_sorted(batch)?; }
            assert_eq!(gpu_datalog_runtime::readback_count(), before);
        }
        assert_eq!(state.run_count(), 2);
        let old = RadixLayout::<(u64,), (), u64, u64, ()>::publish(&state, packing.clone())?;
        let original = old.to_host()?;
        let snapshot = state.snapshot()?;
        let input = Buffer::from_host(pool.clone(), &[103, 7, 200, 200, 101, 201, 7])?;
        let batch = unsafe { state.fresh(&input, 7) }?;
        assert_eq!(batch.buffer().to_host()?, vec![200, 201]);
        assert!(Arc::ptr_eq(&snapshot, &state.snapshot()?));
        assert_eq!(host_keys(&state)?, original);
        assert_eq!(input.to_host()?, vec![103, 7, 200, 200, 101, 201, 7]);
        let retained_bytes = batch.allocated_bytes();
        let before = gpu_datalog_runtime::readback_count();
        let delta = RadixLayout::<(u64,), (), u64, u64, ()>::from_sorted(
            pool.clone(), packing.clone(), params, batch.clone(),
        )?;
        assert_eq!(gpu_datalog_runtime::readback_count(), before);
        let directory_bytes = if radix_bits == 0 { 0 } else { ((1usize << radix_bits) + 1) * 8 };
        assert_eq!(delta.allocated_bytes(), retained_bytes + directory_bytes
            + std::mem::size_of::<gpu_datalog_runtime::radix_run::RadixRunView<u64, u64>>()
            + std::mem::size_of::<gpu_datalog_runtime::PackedField>() + 8);
        unsafe { state.append_sorted(batch)?; }
        assert_eq!(old.to_host()?, original);
        let output = host_keys(&state)?;
        assert_eq!(output.len(), output.iter().collect::<BTreeSet<_>>().len());
        assert_eq!(output.len(), 70);
        for run in state.to_host()? {
            assert_eq!(run.offsets, (0..=run.keys.len() as u64).collect::<Vec<_>>());
        }
        let duplicate = unsafe { state.fresh(&input, 7) }?;
        assert!(duplicate.is_empty());
        state.replace_sorted(duplicate)?;
        assert!(state.is_empty());
        assert_eq!(delta.to_host()?, vec![200, 201]);
        assert_eq!(old.to_host()?, original);
    }
    Ok(())
}

#[test]
fn empty_heading_and_projection_have_their_own_unique_admission() -> Result {
    let pool = pool()?;
    let mut state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), PARAMS, 0)?;
    let candidates = Buffer::from_host(pool.clone(), &[0, 0, 0])?;
    let batch = unsafe { state.fresh(&candidates, 3) }?;
    assert_eq!(batch.len(), 1);
    unsafe { state.append_sorted(batch)?; }
    assert!(unsafe { state.fresh(&candidates, 3) }?.is_empty());
    // Projecting distinct complete rows can collapse them to one negative key.
    let projected = unsafe { state.sort_unique(&candidates, 3) }?;
    assert_eq!(projected.buffer().to_host()?, vec![0]);
    let empty = unsafe { state.sort_unique(&candidates, 0) }?;
    state.replace_sorted(empty)?;
    assert_eq!(state.len(), 0);
    assert_eq!(state.live_bytes(), 0);
    Ok(())
}

#[test]
fn full_key_readers_reject_occurrence_publications_and_delta_reuses_metadata() -> Result {
    use gpu_datalog_runtime::{Column, RadixLayout};
    type Layout = RadixLayout<(u64,), (), u64, u64, ()>;
    let pool = pool()?;
    let packing = Layout::derive_packing()?;
    let input = Buffer::from_host(pool.clone(), &[7u64, 7, 9])?;
    let mut state = DeviceRadixRun::<u64, u64, ()>::build(
        pool.clone(), PARAMS, &input, &(), 3,
    )?;
    let occurrences = Layout::publish(&state, packing.clone())?;
    assert!(occurrences.validate(&[Column::U64]).is_err());
    assert!(occurrences.permuted_provider::<(u64,)>(&pool, &[0], PARAMS, 256).is_err());
    let batch = unsafe { state.sort_unique(&input, 3) }?;
    assert!(unsafe { state.append_sorted(batch.clone()) }.is_err());
    assert!(state.replace_sorted(batch.clone()).is_err());
    let mut state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), PARAMS, packing.bits())?;
    state.replace_sorted(batch.clone())?;
    let set = Layout::publish(&state, packing)?;
    set.validate(&[Column::U64])?;
    assert!(occurrences.validate(&[Column::U64]).is_err());
    let before = gpu_datalog_runtime::readback_count();
    let same = set.with_sorted(batch.clone(), PARAMS)?;
    assert_eq!(same.snapshot_id(), set.snapshot_id());
    assert_eq!(same.fields_ptr(), set.fields_ptr());
    let replacement = occurrences.with_sorted(batch.clone(), PARAMS)?;
    assert_ne!(replacement.snapshot_id(), occurrences.snapshot_id());
    assert_eq!(replacement.fields_ptr(), occurrences.fields_ptr());
    replacement.validate(&[Column::U64])?;
    let empty = unsafe { state.sort_unique(&input, 0) }?;
    let cleared = replacement.with_sorted(empty.clone(), PARAMS)?;
    assert!(cleared.is_empty());
    assert_eq!(cleared.fields_ptr(), replacement.fields_ptr());
    assert_eq!(cleared.with_sorted(empty, PARAMS)?.snapshot_id(), cleared.snapshot_id());
    assert_eq!(gpu_datalog_runtime::readback_count(), before);
    assert_eq!(set.to_host()?, vec![7, 9]);
    assert_eq!(replacement.to_host()?, vec![7, 9]);
    assert_eq!(batch.buffer().to_host()?, vec![7, 9]);
    Ok(())
}

fn active_window<W: Word>(bits: u32) -> Result {
    use gpu_datalog_runtime::{
        readback_count, reset_work_counters, set_work_counters_enabled, work_counters,
    };
    let pool = pool()?;
    let state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, bits)?;
    let baseline = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, W::BITS)?;
    assert_eq!(state.active_bits(), bits);
    assert_eq!(state.snapshot()?.active_bits(), bits);
    for rows in [0, 1, 17, 4097] {
        let values = (0..rows).map(|row| {
            let mut word = W::zero();
            for lane in 0..bits.div_ceil(64) {
                let lane_bits = (bits - lane * 64).min(64);
                let value = ((row % 257) as u64)
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    .rotate_left(lane * 11);
                word.set_lane(lane, value & (u64::MAX >> (64 - lane_bits)));
            }
            word
        }).collect::<Vec<_>>();
        let expected = values.iter().copied().collect::<BTreeSet<_>>()
            .into_iter().collect::<Vec<_>>();
        let input = Buffer::from_host(pool.clone(), &values)?;
        set_work_counters_enabled(true);
        reset_work_counters();
        let before = readback_count();
        // SAFETY: every generated word was masked to the declared window.
        let batch = unsafe { state.sort_unique(&input, rows)? };
        let narrow = work_counters();
        if bits == 0 || rows == 0 {
            assert_eq!(readback_count(), before);
        }
        assert_eq!(batch.buffer().to_host()?, expected);
        assert_eq!(narrow.canonical.sorts, u64::from(rows != 0));
        assert_eq!(narrow.canonical.passes, if rows == 0 { 0 } else { bits.div_ceil(8) as u64 });
        assert_eq!(narrow.canonical.bytes,
            rows as u64 * narrow.canonical.passes * 2 * if W::BITS == 64 { 8 } else { 12 });
        reset_work_counters();
        let full = unsafe { baseline.sort_unique(&input, rows)? };
        let conservative = work_counters();
        assert_eq!(full.buffer().to_host()?, expected);
        assert!(narrow.canonical.passes <= conservative.canonical.passes);
        assert!(narrow.canonical.bytes <= conservative.canonical.bytes);
        if bits < W::BITS && rows != 0 {
            assert!(narrow.canonical.passes < conservative.canonical.passes);
        }
        let unique = expected.iter().rev().copied().collect::<Vec<_>>();
        let input = Buffer::from_host(pool.clone(), &unique)?;
        reset_work_counters();
        let before = readback_count();
        // SAFETY: BTreeSet established uniqueness without changing packing.
        let sorted = unsafe { state.sort_known_unique(&input, unique.len())? };
        assert_eq!(readback_count(), before);
        assert_eq!(sorted.buffer().to_host()?, expected);
        let known = work_counters();
        assert_eq!(known.canonical.sorts, 0);
        assert_eq!(known.layout.sorts, u64::from(!unique.is_empty()));
        assert_eq!(known.layout.passes, narrow.canonical.passes);
        set_work_counters_enabled(false);
    }
    Ok(())
}

#[test]
fn declared_active_windows_preserve_scalar_and_physical_wide_words() -> Result {
    for bits in [0, 1, 32, 64] {
        active_window::<u64>(bits)?;
        active_window::<[u64; 1]>(bits)?;
    }
    for bits in [0, 1, 32, 65, 96, 128] {
        active_window::<u128>(bits)?;
    }
    for bits in [0, 1, 32, 65, 96, 129, 160, 192] {
        active_window::<[u64; 3]>(bits)?;
        active_window::<[u64; 5]>(bits)?;
    }
    active_window::<[u64; 4]>(256)
}

fn distinct_lanes<W: Word>(bits: u32) -> Result {
    let pool = pool()?;
    let state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, bits)?;
    let mut values = vec![W::zero()];
    for bit in [0, 31, 63, 64, 95, 127, 128, 159, 191, 192, 255, 256, 319] {
        if bit < bits {
            let mut word = W::zero();
            word.set_lane(bit / 64, 1 << (bit % 64));
            values.push(word);
        }
    }
    let expected = values.clone();
    values.reverse();
    values.extend(expected.iter().copied());
    let input = Buffer::from_host(pool, &values)?;
    let sorted = unsafe { state.sort_unique(&input, values.len())? };
    assert_eq!(sorted.buffer().to_host()?, expected);
    Ok(())
}

#[test]
fn declared_windows_do_not_drop_distinct_high_or_middle_lanes() -> Result {
    distinct_lanes::<u128>(96)?;
    distinct_lanes::<u128>(128)?;
    distinct_lanes::<[u64; 3]>(65)?;
    distinct_lanes::<[u64; 3]>(96)?;
    distinct_lanes::<[u64; 3]>(192)?;
    distinct_lanes::<[u64; 5]>(129)?;
    distinct_lanes::<[u64; 5]>(320)
}

#[test]
fn invalid_declared_windows_are_rejected_before_sorting() -> Result {
    let pool = pool()?;
    for bits in [65, 96, u32::MAX] {
        assert!(DeviceRadixRun::<u64, u32, ()>::new_set(pool.clone(), PARAMS, bits).is_err());
    }
    assert!(DeviceRadixRun::<u128, u32, ()>::new_set(pool.clone(), PARAMS, 129).is_err());
    assert!(DeviceRadixRun::<[u64; 3], u32, ()>::new_set(pool, PARAMS, 193).is_err());
    Ok(())
}
