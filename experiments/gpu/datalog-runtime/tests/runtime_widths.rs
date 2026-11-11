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

#[test]
fn resident_sources_use_runtime_widths_and_reject_overflow() -> Result {
    use gpu_device_columns::{Codec, ColumnView, SourceView};
    type Layout = RadixLayout<(u32, u32), (), u64, u64, ()>;
    let pool = Pool::new(0, 0, 0)?;
    let packing = Layout::derive_packing_with_fields(&[(19, false), (7, false)])?;
    let params = RadixRunParams { ratio: 4, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 };
    let first = Buffer::from_host(pool.clone(), &[7u64, 0, (1 << 19) - 1, 7])?;
    let second = Buffer::from_host(pool.clone(), &[127u64, 0, 127, 1])?;
    let view = |buffer: &Buffer<u64>| SourceView::from(ColumnView {
        headers: std::ptr::null(), data: buffer.ptr(), len: buffer.len() as u64,
        count: std::ptr::null(), codec: Codec::Plain as u32,
    });
    // SAFETY: both initialized columns live through this ordered build.
    let layout = unsafe { Layout::from_sources_with_packing(
        pool.clone(), params, packing.clone(), &[view(&first), view(&second)],
        &[], std::ptr::null(), 4,
    )? };
    assert_eq!(layout.snapshot().active_bits(), 26);
    // DECODE P: final integration-test publication compares all logical rows.
    assert_eq!(layout.to_rows()?, [vec![0,0], vec![7,1], vec![7,127], vec![(1 << 19)-1,127]]);
    let probes = Buffer::from_host(pool.clone(), &[
        packing.pack::<u64>(&[7,127])?, packing.pack::<u64>(&[8,127])?,
    ])?;
    let found = layout.probe_keys(&pool, &probes, 2)?.to_host()?;
    assert_eq!(found.iter().map(|probe| probe.contains).collect::<Vec<_>>(), [1,0]);
    for (a,b) in [(1u64 << 19, 0u64), (0, 128)] {
        let a = Buffer::from_host(pool.clone(), &[a])?;
        let b = Buffer::from_host(pool.clone(), &[b])?;
        // SAFETY: initialized, live sources deliberately contain a field overflow.
        let result = unsafe { Layout::from_sources_with_packing(
            pool.clone(), params, packing.clone(), &[view(&a), view(&b)],
            &[], std::ptr::null(), 1,
        ) };
        assert!(result.is_err());
    }
    Ok(())
}
