use gpu_device_columns::{
    check_packed_layout, pack_for, BaseLayout, Bounds, Buffer, Codec, EncodedColumn, Pool,
    VectorHeader,
};
use std::sync::Arc;

fn corpus() -> Vec<Vec<i64>> {
    let mixed = |rows: usize| -> Vec<i64> {
        (0..rows as i64)
            .map(|i| match i % 7 {
                0 => i64::MIN + i,
                1 => i64::MAX - i,
                2 => -i,
                _ => i.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64) >> (i % 61),
            })
            .collect()
    };
    vec![
        vec![],
        vec![42],
        vec![-1; 1024],
        (0..1023).collect(),
        (0..1025).map(|i| 1_000_000 - 3 * i).collect(),
        mixed(1024),
        mixed(3 * 1024 + 17),
    ]
}

/// Every device reader a query can reach on a base column: download, whole-
/// column tile decode and per-row source decode (an identity gather).
fn assert_reads_back(pool: &Arc<Pool>, column: &EncodedColumn, values: &[i64]) {
    assert_eq!(column.bounds(), Bounds::from_values(values));
    assert_eq!(column.capacity(), values.len());
    assert_eq!(column.to_host().unwrap(), values);
    if values.is_empty() {
        return;
    }
    assert_eq!(
        column
            .decoded()
            .unwrap()
            .to_host_prefix(values.len())
            .unwrap(),
        values
    );
    let count = Arc::new(Buffer::from_host(pool.clone(), &[values.len() as u64]).unwrap());
    let gathered = unsafe {
        EncodedColumn::from_gather_source::<u64, i64>(
            pool.clone(),
            column.view().into(),
            std::ptr::null(),
            values.len(),
            count,
            0,
            column.bounds(),
        )
    }
    .unwrap();
    assert_eq!(gathered.to_host().unwrap(), values);
}

#[test]
fn native_columns_read_back_through_device_decoders() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    for values in corpus() {
        let native = EncodedColumn::from_host_native(pool.clone(), &values).unwrap();
        assert_eq!(native.codec(), Codec::Plain);
        assert_reads_back(&pool, &native, &values);
    }
    gpu_device_columns::release_cached_kernels();
}

/// The FastLanes DELTA layout is chosen per column (sorted keys) and must read
/// back through the same readers, including from u32 domain-ID lanes.
#[test]
fn delta_layout_selects_per_column_and_reads_back() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let sorted: Vec<i64> = (0..5000).map(|i| 1_000_000 + i * 4 + i % 3).collect();
    let column = EncodedColumn::from_host_layout(pool.clone(), &sorted, BaseLayout::Delta).unwrap();
    assert_eq!(column.codec(), Codec::Delta);
    assert_reads_back(&pool, &column, &sorted);
    let ids: Vec<u32> = (0..3000).map(|i| i * 2).collect();
    let column = EncodedColumn::from_host_layout(pool.clone(), &ids, BaseLayout::Delta).unwrap();
    assert_eq!(column.codec(), Codec::Delta);
    let widened: Vec<i64> = ids.iter().map(|&id| i64::from(id)).collect();
    assert_reads_back(&pool, &column, &widened);
    for values in corpus() {
        let column =
            EncodedColumn::from_host_layout(pool.clone(), &values, BaseLayout::Delta).unwrap();
        assert_reads_back(&pool, &column, &values);
    }
    gpu_device_columns::release_cached_kernels();
}

fn fields(header: &VectorHeader) -> (u64, i64, u32, u32, u32, u32) {
    (
        header.offset,
        header.base,
        header.width,
        header.codec,
        header.rows,
        header.padding,
    )
}

/// Host packing must reproduce the device encoder bit for bit, so a packed
/// upload is the same resident column the device FOR path would build.
#[test]
fn host_packing_matches_device_encoder_and_decodes() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let mut corpus = corpus();
    // One full vector per width 0..=64: values 0..=2^w - 1, then a partial tail.
    corpus.push(
        (0..=64u32)
            .flat_map(|width| {
                let span = if width == 64 {
                    u64::MAX
                } else {
                    (1u64 << width) - 1
                };
                (0..1024u64).map(move |i| (if i == 1023 { span } else { span / 1023 * i }) as i64)
            })
            .chain([7, -7, 0])
            .collect(),
    );
    for values in corpus {
        let (headers, payload) = pack_for(&values);
        check_packed_layout(values.len(), &headers, payload.len()).unwrap();
        let device = EncodedColumn::from_host(pool.clone(), &values, Codec::For).unwrap();
        let (device_headers, device_payload) = device.inspect().unwrap();
        assert_eq!(
            headers.iter().map(fields).collect::<Vec<_>>(),
            device_headers.iter().map(fields).collect::<Vec<_>>()
        );
        assert_eq!(payload, device_payload);
        let packed = EncodedColumn::from_host_packed(pool.clone(), &values).unwrap();
        assert_eq!(packed.codec(), Codec::For);
        assert_eq!(packed.bounds(), device.bounds());
        assert_eq!(packed.bytes(), device.bytes());
        assert_eq!(packed.to_host().unwrap(), values);
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn packed_layout_check_rejects_inconsistent_parts() {
    let values: Vec<i64> = (0..2100).map(|i| i * 3).collect();
    let (headers, payload) = pack_for(&values);
    let words = payload.len();
    assert!(check_packed_layout(values.len(), &headers, words).is_ok());
    assert!(check_packed_layout(values.len() + 1024, &headers, words).is_err());
    assert!(check_packed_layout(values.len() - 1, &headers, words).is_err());
    assert!(check_packed_layout(values.len(), &headers, words + 16).is_err());
    let mut shifted = headers.clone();
    shifted[1].offset += 16;
    assert!(check_packed_layout(values.len(), &shifted, words).is_err());
    let mut delta = headers;
    delta[2].codec = Codec::Delta as u32;
    assert!(check_packed_layout(values.len(), &delta, words).is_err());
}
