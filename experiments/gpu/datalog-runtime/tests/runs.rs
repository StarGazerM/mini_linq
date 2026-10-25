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
    // Wide keys deliberately share low halves while differing above bit 64.
    let value = if W::BITS == 128 {
        ((value as u128 >> 7) << 80) | ((value & 127) as u128)
    } else {
        value as u128
    };
    W::from_u128(value)
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
    let mut state = DeviceRuns::<W>::new(pool.clone())?;
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

#[test]
fn random_duplicate_batches_u64() -> Result {
    random_batches::<u64>()
}

#[test]
fn random_duplicate_batches_u128() -> Result {
    random_batches::<u128>()
}

fn empty_single_and_extremes<W: Word>() -> Result {
    let pool = pool()?;
    let mut state = DeviceRuns::<W>::new(pool.clone())?;
    let mut accepted = BTreeSet::new();
    assert_eq!(state.len(), 0);
    assert!(state.to_host()?.is_empty());
    assert_run_bound(&state);
    let zero = W::from_u128(0);
    let max = W::from_u128(u128::MAX >> (128 - W::BITS));
    let high = W::from_u128(1u128 << (W::BITS - 1));
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

fn million_rows<W: Word>() -> Result {
    let pool = pool()?;
    let mut state = DeviceRuns::<W>::new(pool.clone())?;
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
    let mut state = DeviceRuns::<W>::new(pool.clone())?;
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
fn append_sorted_disjoint_u64() -> Result {
    append_disjoint_runs::<u64>()
}

#[test]
fn append_sorted_disjoint_u128() -> Result {
    append_disjoint_runs::<u128>()
}

#[cuda_host::cuda_module]
mod prefix_kernel {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    use gpu_datalog_runtime::runs::{range, RunView};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn lookup(
        views: *const RunView<u128>,
        run_count: usize,
        bounds: *const u128,
        queries: usize,
        capacity: usize,
        output: *mut u128,
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

#[test]
fn permuted_layout_prefix_lookup_matches_cpu() -> Result {
    let pool = pool()?;
    let packing = Packing::new(&[40, 40, 16])?;
    let order = [2, 0, 1];
    let layout = packing.permute(&order)?;
    let mut canonical = DeviceRuns::<u128>::new(pool.clone())?;
    let mut permuted = DeviceRuns::<u128>::new(pool.clone())?;
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
            values.push(packing.pack::<u128>(&row)?);
            rows.insert(row);
        }
        if batch == 0 {
            for row in [[0, 0, 0], [(1i128 << 40) - 1, (1i128 << 40) - 1, 65_535]] {
                values.push(packing.pack::<u128>(&row)?);
                rows.insert(row);
            }
        }
        let input = Buffer::from_host(pool.clone(), &values)?;
        let fresh = canonical.admit(&input, values.len())?;
        let shuffled = packing.permute_device(fresh.buffer(), fresh.len(), &order)?;
        let source = fresh.buffer().to_host_prefix(fresh.len())?;
        let expected = source.iter().map(|&key| {
            let row = packing.unpack(key)?;
            layout.pack::<u128>(&[row[2], row[0], row[1]])
        }).collect::<Result<Vec<_>>>()?;
        assert_eq!(shuffled.to_host()?, expected);
        // Sort each disjoint permuted delta before appending to the other layout.
        let mut sorter = DeviceRuns::<u128>::new(pool.clone())?;
        let sorted = sorter.admit(&shuffled, fresh.len())?;
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
        let range = layout.prefix_range::<u128>(prefix)?;
        bounds.extend([*range.start(), *range.end()]);
    }
    let views = permuted.runs();
    let device_views = Buffer::from_host(pool.clone(), &views)?;
    let device_bounds = Buffer::from_host(pool.clone(), &bounds)?;
    let output = Buffer::<u128>::zeroed(pool.clone(), prefixes.len() * permuted.len())?;
    let counts = Buffer::<u64>::zeroed(pool.clone(), prefixes.len())?;
    let module = unsafe { prefix_kernel::load(pool.context())? };
    module.lookup(
        pool.stream(),
        &module.prepare_lookup(LaunchConfig1D::new(prefixes.len().div_ceil(256) as u32, 256, 0))?,
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
#[ignore = "throughput probe: requires GPU memory for 50M accepted rows and 10M candidates"]
fn throughput_10m_candidates_against_50m_accepted() -> Result {
    const ACCEPTED: usize = 50_000_000;
    const CANDIDATES: usize = 10_000_000;
    let pool = pool()?;
    let mut state = DeviceRuns::<u64>::new(pool.clone())?;
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
