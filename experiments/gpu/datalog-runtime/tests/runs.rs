use std::{collections::BTreeSet, sync::Arc};

use cuda_core::LaunchConfig1D;
use gpu_datalog_runtime::{
    pack::{Packing, Word},
    runs::{DeviceRuns, Run},
};
use gpu_device_columns::{Buffer, Pool};

type Result<T = (), E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

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
        word.set_lane(lane, if lane == W::BITS / 64 - 1 { value } else {
            value.rotate_left(lane * 11) & 127
        });
    }
    word
}

fn assert_run_bound<W: Word>(state: &DeviceRuns<W>) {
    let views = state.runs();
    assert_eq!(views.iter().map(|view| view.len).sum::<usize>(), state.len());
    assert!(views.iter().all(|view| view.len > 0));
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
    assert!(views.len() <= bound, "{} runs for {} rows; bound {bound}", views.len(), state.len());
}

fn admit_against_cpu<W: Word>(
    pool: &Arc<Pool>,
    state: &mut DeviceRuns<W>,
    accepted: &mut BTreeSet<W>,
    values: &[W],
) -> Result<Run<W>> {
    let input = Buffer::from_host(pool.clone(), values)?;
    let batch: BTreeSet<_> = values.iter().copied().collect();
    let expected_delta: Vec<_> = batch.difference(accepted).copied().collect();
    let before = state.len();
    assert_eq!(state.count_fresh(&input, values.len())?, expected_delta.len());
    assert_eq!(state.len(), before, "counter observation must not admit rows");
    let fresh = state.admit(&input, values.len())?;
    assert_eq!(fresh.len(), expected_delta.len());
    assert_eq!(fresh.buffer().to_host_prefix(fresh.len())?, expected_delta);
    assert_eq!(fresh.view().len, fresh.len());
    assert_eq!(input.to_host()?, values, "admission mutated its input");
    accepted.extend(batch);
    assert_eq!(state.len(), accepted.len());
    assert_eq!(state.to_host()?, accepted.iter().copied().collect::<Vec<_>>());
    assert_run_bound(state);
    Ok(fresh)
}

fn random_batches<W: Word>() -> Result {
    let pool = pool()?;
    let mut state = DeviceRuns::<W>::new(pool.clone(), 4)?;
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
    let state = DeviceRuns::<W>::new(pool.clone(), 4)?;
    for rows in [0, 1, 17, 257, 4097] {
        let mut values: Vec<_> = (0..rows).rev().map(|row| word::<W>(row as u64)).collect();
        if rows > 1 {
            let mut maximum = W::zero();
            for lane in 0..W::BITS / 64 { maximum.set_lane(lane, u64::MAX); }
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
    let mut state = DeviceRuns::<W>::new(pool.clone(), 4)?;
    let mut accepted = BTreeSet::new();
    assert_eq!(state.len(), 0);
    assert!(state.to_host()?.is_empty());
    assert_run_bound(&state);
    let zero = W::zero();
    let mut max = W::zero();
    for lane in 0..W::BITS / 64 { max.set_lane(lane, u64::MAX); }
    let mut high = W::zero();
    high.set_lane(W::BITS / 64 - 1, 1_u64 << 63);
    for values in [vec![], vec![zero], vec![zero], vec![], vec![max], vec![high, max, high, zero], vec![]] {
        admit_against_cpu(&pool, &mut state, &mut accepted, &values)?;
    }
    // Only the declared live prefix participates, not spare initialized capacity.
    let input = Buffer::from_host(pool.clone(), &[zero, W::from_u128(7)])?;
    assert_eq!(state.admit(&input, 1)?.len(), 0);
    assert_eq!(state.admit(&input, 0)?.len(), 0);
    assert_eq!(state.to_host()?, accepted.iter().copied().collect::<Vec<_>>());
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
    let mut state = DeviceRuns::<W>::new(pool.clone(), 4)?;
    let mut random = Random(0x6b73_d147_590a_821f);
    let values: Vec<_> = (0..1_000_000).map(|_| word::<W>(random.next() % 600_001)).collect();
    let mut expected = values.clone();
    expected.sort_unstable();
    expected.dedup();
    let input = Buffer::from_host(pool.clone(), &values)?;
    let fresh = state.admit(&input, values.len())?;
    assert_eq!(fresh.buffer().to_host_prefix(fresh.len())?, expected);
    assert_eq!(state.len(), expected.len());
    assert_eq!(state.to_host()?, expected);
    assert_run_bound(&state);
    let duplicate = state.admit(&input, values.len())?;
    assert_eq!(duplicate.len(), 0);
    assert_eq!(state.len(), expected.len());
    assert_eq!(state.to_host()?, expected);
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
    let mut state = DeviceRuns::<W>::new(pool.clone(), 4)?;
    let mut accepted = BTreeSet::new();
    for batch in 0..128 {
        // Runs interleave numerically; they are not adjacent sorted chunks.
        let values: Vec<_> = (0..33).map(|row| word::<W>(row * 128 + batch)).collect();
        let buffer = Buffer::from_host(pool.clone(), &values)?;
        // Values are strictly increasing and each batch has a distinct residue.
        let run = unsafe { Run::from_sorted(buffer, values.len())? };
        // SAFETY: different batch residues make these sorted runs disjoint.
        unsafe { state.append_sorted(run.clone())? };
        accepted.extend(values.iter().copied());
        assert_eq!(run.buffer().to_host_prefix(run.len())?, values);
        assert_run_bound(&state);
    }
    let empty = Buffer::<W>::from_host(pool, &[])?;
    // SAFETY: an empty initialized run is sorted, unique, and disjoint.
    unsafe { state.append_sorted(Run::from_sorted(empty, 0)?)? };
    assert_eq!(state.to_host()?, accepted.into_iter().collect::<Vec<_>>());
    assert_run_bound(&state);
    Ok(())
}

#[test]
fn recorded_ratio_changes_compaction_without_changing_rows() -> Result {
    let pool = pool()?;
    assert!(DeviceRuns::<u64>::new(pool.clone(), 1).is_err());
    for (ratio, expected_runs) in [(2, 2), (4, 2), (8, 1)] {
        let mut state = DeviceRuns::<u64>::new(pool.clone(), ratio)?;
        let first = Buffer::from_host(pool.clone(), &[0u64, 1, 2, 3, 4, 5])?;
        let second = Buffer::from_host(pool.clone(), &[6u64])?;
        state.admit(&first, 6)?;
        state.admit(&second, 1)?;
        assert_eq!(state.retained_buffers(), expected_runs);
        assert_eq!(state.to_host()?, (0u64..7).collect::<Vec<_>>());
        let duplicate = state.admit(&second, 1)?;
        assert!(duplicate.is_empty());
        assert_eq!(state.to_host()?, (0u64..7).collect::<Vec<_>>());
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
            (1, 1), (31, 33), (32, 32), (33, 31),
            (257, 65), (1, 8193), (8193, 2049), (8191, 8193),
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
                let first: Vec<_> = (0..first_len).map(|row| make_word(match pattern {
                    0 => row * 2,
                    1 => row,
                    _ => second_len + row,
                } as u64)).collect();
                let second: Vec<_> = (0..second_len).map(|row| make_word(match pattern {
                    0 => row * 2 + 1,
                    1 => first_len + row,
                    _ => row,
                } as u64)).collect();
                let mut expected = first.clone();
                expected.extend_from_slice(&second);
                expected.sort_unstable();
                let mut state = DeviceRuns::<W>::new(pool.clone(), 4)?;
                let first_buffer = Buffer::from_host(pool.clone(), &first)?;
                let second_buffer = Buffer::from_host(pool.clone(), &second)?;
                // SAFETY: both sequences increase in the selected lane; their
                // ranks are disjoint in every pattern.
                let snapshot = unsafe { Run::from_sorted(first_buffer, first_len)? };
                unsafe {
                    state.append_sorted(snapshot.clone())?;
                    state.append_sorted(Run::from_sorted(second_buffer, second_len)?)?;
                }
                // All selected length ratios must exercise the actual merge,
                // not merely the diagnostic host merge of separate runs.
                assert_eq!(state.retained_buffers(), 1);
                let merged = &state.owned_runs()[0];
                assert_eq!(merged.buffer().to_host_prefix(merged.len())?, expected,
                    "lane {lane}, lengths {first_len}/{second_len}, pattern {pattern}");
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
    use gpu_datalog_runtime::runs::{range, RunView};
    use gpu_datalog_runtime::Word;

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn lookup<W: Word>(
        views: *const RunView<W>,
        run_count: usize,
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
            for run in 0..run_count {
                let view = *views.add(run);
                let (start, end) = range(view, lo, hi);
                for row in start..end {
                    *output.add(query * capacity + count) = *view.ptr.add(row);
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
        let values = fields.iter().map(|field| {
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
            encoded as i128 - if field.signed != 0 { 1_i128 << (field.bits - 1) } else { 0 }
        }).collect::<Vec<_>>();
        tuples.push(values);
    }
    let source = tuples.iter().map(|row| packing.pack::<W>(row)).collect::<Result<Vec<_>>>()?;
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
        let expected = tuples.iter().map(|row| {
            layout.pack::<W>(&order.iter().map(|&column| row[column]).collect::<Vec<_>>())
        }).collect::<Result<Vec<_>>>()?;
        // Include a short initialized prefix, so capacity is not mistaken for
        // the row count, plus an empty launch and a partial final thread block.
        for rows in [0, 1, source.len() - 1, source.len()] {
            let shuffled = packing.permute_device(&input, rows, &order)?;
            assert_eq!(shuffled.len(), rows);
            assert_eq!(shuffled.to_host()?, expected[..rows],
                "{}-bit permutation {order:?}, {rows} rows", W::BITS);
            let restored = layout.permute_device(&shuffled, rows, &inverse)?;
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
    permutation_against_cpu::<u128>(Packing::with_columns(
        &[Column::I8, Column::U64, Column::I16, Column::U32],
    )?)
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
    permutation_against_cpu::<[u64; 5]>(Packing::with_columns(
        &[Column::I64, Column::U8, Column::I64, Column::U64, Column::I16],
    )?)
}

fn permuted_layout_prefix_lookup<W: Word>(widths: &[u32]) -> Result {
    let pool = pool()?;
    let packing = Packing::new(widths)?;
    let order = [2, 0, 1];
    let layout = packing.permute(&order)?;
    let mut canonical = DeviceRuns::<W>::new(pool.clone(), 4)?;
    let mut permuted = DeviceRuns::<W>::new(pool.clone(), 4)?;
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
        let fresh = canonical.admit(&input, values.len())?;
        let shuffled = packing.permute_device(fresh.buffer(), fresh.len(), &order)?;
        let source = fresh.buffer().to_host_prefix(fresh.len())?;
        let expected = source.iter().map(|&key| {
            let row = packing.unpack(key)?;
            layout.pack::<W>(&[row[2], row[0], row[1]])
        }).collect::<Result<Vec<_>>>()?;
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
    let actual_rows = permuted.to_host()?.into_iter().map(|key| {
        layout.unpack(key).map(|row| [row[0], row[1], row[2]])
    }).collect::<Result<Vec<_>>>()?;
    assert_eq!(actual_rows, expected_rows.iter().copied().collect::<Vec<_>>());

    let mut prefixes = vec![vec![], vec![0, 0, 0], vec![65_535], vec![65_535, (1i128 << 40) - 1, (1i128 << 40) - 1]];
    for first in 0..9 {
        prefixes.push(vec![first]);
        for second in [1i128 << 35, (1i128 << 35) + 8, (1i128 << 35) + 16, (1i128 << 35) + 17] {
            prefixes.push(vec![first, second]);
            prefixes.push(vec![first, second, (1i128 << 36) + 6]);
        }
    }
    let mut bounds = Vec::with_capacity(prefixes.len() * 2);
    for prefix in &prefixes {
        let range = layout.prefix_range::<W>(prefix)?;
        bounds.extend([*range.start(), *range.end()]);
    }
    let views = permuted.runs();
    let device_views = Buffer::from_host(pool.clone(), &views)?;
    let device_bounds = Buffer::from_host(pool.clone(), &bounds)?;
    let output = Buffer::<W>::zeroed(pool.clone(), prefixes.len() * permuted.len())?;
    let counts = Buffer::<u64>::zeroed(pool.clone(), prefixes.len())?;
    let module = unsafe { prefix_kernel::load(pool.context())? };
    module.lookup::<W>(
        pool.stream(),
        &module.prepare_lookup::<W>(LaunchConfig1D::new(prefixes.len().div_ceil(256) as u32, 256, 0))?,
        device_views.ptr(), views.len(), device_bounds.ptr(), prefixes.len(), permuted.len(),
        output.ptr(), counts.ptr(),
    )?;
    let counts = counts.to_host()?;
    let output = output.to_host()?;
    for (query, prefix) in prefixes.iter().enumerate() {
        let expected: Vec<_> = expected_rows.iter().filter(|row| row.starts_with(prefix)).copied().collect();
        let count = counts[query] as usize;
        assert!(count <= permuted.len());
        let mut actual = output[query * permuted.len()..query * permuted.len() + count].iter().map(|&key| {
            layout.unpack(key).map(|row| [row[0], row[1], row[2]])
        }).collect::<Result<Vec<_>>>()?;
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
    let mut state = DeviceRuns::<u64>::new(pool.clone(), 4)?;
    let seed: Vec<_> = (0..ACCEPTED as u64).map(|row| row * 2).collect();
    let buffer = Buffer::from_host(pool.clone(), &seed)?;
    drop(seed);
    // SAFETY: the upload initialized the sorted unique seed; state is empty.
    unsafe { state.append_sorted(Run::from_sorted(buffer, ACCEPTED)?)? };
    let mut random = Random(0x49ac_651f_0382_bd77);
    let mut candidates: Vec<_> = (0..CANDIDATES).map(|_| random.next() % (2 * ACCEPTED as u64)).collect();
    let input = Buffer::from_host(pool, &candidates)?;
    candidates.retain(|value| value % 2 == 1);
    candidates.sort_unstable();
    candidates.dedup();
    state.enable_timing(true);
    let fresh = state.admit(&input, CANDIDATES)?;
    let timings = state.last_timings()?;
    println!(
        "DeviceRuns<u64>: accepted_before={ACCEPTED} candidates={CANDIDATES} fresh={} accepted_after={} sort_unique_ms={:.3} anti_join_ms={:.3} merge_ms={:.3}",
        fresh.len(), state.len(), timings.sort_unique_ms, timings.anti_join_ms, timings.merge_ms,
    );
    assert_eq!(fresh.buffer().to_host_prefix(fresh.len())?, candidates);
    assert_eq!(state.len(), ACCEPTED + candidates.len());
    assert_run_bound(&state);
    Ok(())
}
