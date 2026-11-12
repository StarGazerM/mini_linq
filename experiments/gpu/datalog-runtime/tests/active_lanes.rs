use gpu_datalog_runtime::{Buffer, DeviceRadixRun, Packing, Pool, RadixLayout, RadixRunParams, Word};
use std::collections::BTreeSet;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PARAMS: RadixRunParams = RadixRunParams {
    ratio: 2, radix_bits: 0, merge_tile_rows: 32, block_threads: 256,
};

fn narrow_set<W: Word + 'static>() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_fields(&[(17, false)])?;
    let mut state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, 17)?;
    let mut expected = BTreeSet::new();
    gpu_datalog_runtime::reset_work_counters();
    let before = gpu_datalog_runtime::readback_count();
    for values in [vec![7u32, 1, 7, 100], vec![3, 7, 131071], vec![2, 100, 1024], vec![0, 3, 131070]] {
        let words = values.iter().map(|&v| W::from_u128(v as u128)).collect::<Vec<_>>();
        let input = Buffer::from_host(pool.clone(), &words)?;
        let old = expected.len();
        expected.extend(values);
        // SAFETY: every initialized candidate has zero padding above bit 17.
        let fresh = unsafe { state.fresh(&input, words.len())? };
        assert_eq!(fresh.len(), expected.len() - old);
        // SAFETY: fresh establishes sortedness, uniqueness and disjointness.
        unsafe { state.append_sorted(fresh)?; }
    }
    assert_eq!(gpu_datalog_runtime::readback_count() - before, 7);
    if cfg!(feature = "radix-counters") {
        let counters = gpu_datalog_runtime::work_counters();
        assert_eq!(counters.canonical.sorts, 4);
        assert_eq!(counters.canonical.passes, 12);
        assert_eq!(counters.canonical.radix_bits, 68);
        assert!(counters.merges > 0);
    }
    let layout = RadixLayout::<(u32,), (), W, u64, ()>::publish(&state, packing)?;
    let mut probes = [0u128, 1, 4, 131071, 1 << 17, 1 << 64, 1 << 100]
        .map(W::from_u128).to_vec();
    if W::BITS > 128 {
        let mut outside = W::zero();
        outside.set_lane(2, 1);
        probes.push(outside);
    }
    let keys = Buffer::from_host(pool.clone(), &probes)?;
    let found = layout.probe_keys(&pool, &keys, probes.len())?.to_host()?;
    let mut expected_found = vec![1, 1, 0, 1, 0, 0, 0];
    if W::BITS > 128 { expected_found.push(0); }
    assert_eq!(found.iter().map(|p| p.contains).collect::<Vec<_>>(), expected_found);
    // Both lower and upper probes may exceed the recorded width. They must not
    // become low-lane matches after high padding is skipped by the inner search.
    let lows = [0u128, 1 << 64, 3, 1 << 17].map(W::from_u128);
    let highs = [1u128 << 100, 1 << 100, 100, (1 << 17) + 1].map(W::from_u128);
    let intervals = layout.probe_intervals(&pool,
        &Buffer::from_host(pool.clone(), &lows)?,
        &Buffer::from_host(pool.clone(), &highs)?, lows.len())?.to_host()?;
    assert_eq!(intervals.iter().map(|p| p.groups).collect::<Vec<_>>(),
        [expected.len() as u64, 0, 3, 0]);
    // DECODE P: active-lane regression publication restores final numeric keys.
    assert_eq!(layout.to_rows()?, expected.into_iter().map(|v| vec![v as i128]).collect::<Vec<_>>());
    Ok(())
}

#[test]
fn narrow_u128_probes_dedup_subtract_and_merge() -> Result { narrow_set::<u128>() }

#[test]
fn narrow_array_probes_dedup_subtract_and_wide_merge() -> Result { narrow_set::<[u64; 3]>() }

#[test]
fn zero_width_probes_keep_full_carrier_guards() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let mut state = DeviceRadixRun::<u128, u64, ()>::new_set(pool.clone(), PARAMS, 0)?;
    let input = Buffer::from_host(pool.clone(), &[0u128, 0])?;
    let fresh = unsafe { state.fresh(&input, 2)? };
    unsafe { state.append_sorted(fresh)?; }
    let layout = RadixLayout::<(), (), u128, u64, ()>::publish(&state, Packing::with_fields(&[])?)?;
    let keys = Buffer::from_host(pool.clone(), &[0u128, 1, 1 << 100])?;
    let found = layout.probe_keys(&pool, &keys, 3)?.to_host()?;
    assert_eq!(found.iter().map(|p| p.contains).collect::<Vec<_>>(), [1, 0, 0]);
    Ok(())
}

#[test]
fn recorded_lane_count_controls_lexicographic_order() {
    use std::cmp::Ordering;
    for bits in [0u32, 1, 17, 50, 64, 65, 96, 128] {
        let lanes = bits.div_ceil(64);
        let mask = if bits == 128 { u128::MAX } else { (1u128 << bits) - 1 };
        for (a,b) in [(0u128, 0u128), (1, 2), (u64::MAX as u128, 1u128 << 64), (mask, 0)] {
            let (a,b) = (a & mask, b & mask);
            assert_eq!(a.compare_lanes(&b, lanes), a.cmp(&b));
        }
    }
    assert_eq!((1u128 << 100).compare(&0), Ordering::Greater);
    assert_eq!((1u128 << 100).compare_lanes(&0, 1), Ordering::Equal);
}
