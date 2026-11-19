use gpu_datalog_runtime::{
    Buffer, DeviceRadixRun, OutputStaging, Pool, RadixLayout, RadixRunParams, Result, SortedBatch,
};

const PARAMS: RadixRunParams = RadixRunParams {
    ratio: 4,
    radix_bits: 0,
    merge_tile_rows: 32,
    block_threads: 256,
};

type Layout = RadixLayout<(i32, u32), (), u64, u64, ()>;

/// Final rows equal the key-ordered union of the retained runs: for one and
/// several runs; on one thread, on all threads (8 MiB of keys), and with the
/// longest run staged through pinned chunks (128 MiB, last chunk partial)
/// beside a pageable and a staged shorter run.
#[test]
fn published_rows_are_the_key_ordered_union_of_all_runs() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Layout::derive_packing()?;
    let mut staging = OutputStaging::new(&pool);
    for sizes in [
        vec![],
        vec![1_000],
        vec![4_000, 600, 90],
        vec![2_500_001, 500_003, 70_001],
        vec![20_000_003],
        vec![20_000_003, 4_000_001, 300],
        vec![70_000_003, 17_000_001, 1_000],
    ] {
        let mut state =
            DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), PARAMS, packing.bits())?;
        let mut layout = Layout::publish(&state, packing.clone())?;
        let mut expected = Vec::new();
        for (run, &rows) in sizes.iter().enumerate() {
            // Ascending within a run, disjoint between runs, both fields varying.
            let stride = u64::MAX / (rows as u64 + 1) / 16 * 16;
            let keys: Vec<u64> = (0..rows as u64).map(|i| i * stride + run as u64).collect();
            let batch =
                unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &keys)?, rows)? };
            unsafe { state.append_sorted(batch)? };
            layout = layout.republish(&state)?;
            expected.extend(keys);
        }
        assert_eq!(layout.run_count() > 1, sizes.len() > 1, "{sizes:?}");
        expected.sort_unstable();
        let expected: Vec<(i32, u32)> = expected
            .into_iter()
            .map(|key| (((key >> 32) as u32 ^ 1 << 31) as i32, key as u32))
            .collect();
        let actual = layout.publish_rows(&mut staging, |row| {
            (row.field(0) as i32, row.field(1) as u32)
        })?;
        assert!(actual == expected, "{sizes:?}");
    }
    Ok(())
}
