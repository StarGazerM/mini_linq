use gpu_datalog_runtime::{Buffer, Column, Pool, RadixLayout, RadixRunParams};
use gpu_datalog_runtime::SortedBatch;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn nonbyte_layout_validates_probes_and_publishes() -> Result {
    type Layout = RadixLayout<(u32, u32), (), u64, u64, ()>;
    let pool = Pool::new(0, 0, 0)?;
    let packing = Layout::derive_packing_with_fields(&[(19, false), (7, false)])?;
    let rows = [[0, 0], [7, 1], [7, 127], [(1 << 19) - 1, 127]];
    let words = rows.iter().map(|row| packing.pack::<u64>(row)).collect::<Result<Vec<_>>>()?;
    // SAFETY: packing preserves tuple order; the entire uploaded prefix is unique.
    let batch = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &words)?, words.len())? };
    let params = RadixRunParams { ratio: 4, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 };
    let layout = Layout::from_sorted(pool.clone(), packing.clone(), params, batch, packing.bits())?;
    layout.validate(&[Column::U32, Column::U32])?;
    assert!(layout.validate(&[Column::U16, Column::U32]).is_err());
    assert!(layout.validate(&[Column::I32, Column::U32]).is_err());
    assert!(layout.validate(&[Column::U32]).is_err());
    let probes = Buffer::from_host(pool.clone(), &[words[1], packing.pack::<u64>(&[7, 2])?])?;
    let found = layout.probe_keys(&pool, &probes, 2)?.to_host()?;
    assert_eq!(found.iter().map(|probe| probe.contains).collect::<Vec<_>>(), [1, 0]);
    let interval = layout.prefix_interval(&[7])?;
    let low = Buffer::from_host(pool.clone(), &[*interval.start()])?;
    let high = Buffer::from_host(pool.clone(), &[*interval.end()])?;
    assert_eq!(layout.probe_intervals(&pool, &low, &high, 1)?.to_host()?[0].groups, 2);
    // DECODE P: final integration-test publication compares all logical rows.
    assert_eq!(layout.to_rows()?, rows.map(Vec::from));
    Ok(())
}
