use std::sync::Arc;

use cq_gpu_sequence_runtime::canon::{
    packed_columns, PackedCanon, PackedColumnsAdapter, PackedField,
};
use cq_gpu_sequence_runtime::{
    Bounds, Buffer, Codec, ColumnDescriptor, EncodedColumn, KeyDescriptor, PayloadDescriptor,
    PayloadKind, Pool, RangeBuilder, RangeDescriptor, RangeSelection, RangeWorkspace, Result,
};

fn column(pool: &Arc<Pool>, values: &[u32]) -> Result<ColumnDescriptor> {
    let encoded = Arc::new(
        EncodedColumn::from_host(pool.clone(), values, Codec::For)
            .map_err(|error| format!("source column upload: {error}"))?,
    );
    // The upload initializes exactly this immutable live prefix.
    unsafe { ColumnDescriptor::new(encoded, values.len()) }
}

#[test]
fn packed_canon_selects_lanes_from_recorded_heading() -> Result<()> {
    assert_eq!(PackedCanon::MAX_PACKED_BITS, 128);
    for bits in [0, 1, 63, 64] {
        assert_eq!(PackedCanon::key_lanes(bits)?, 1);
    }
    for bits in [65, 80, 127, 128] {
        assert_eq!(PackedCanon::key_lanes(bits)?, 2);
    }
    assert!(PackedCanon::key_lanes(129).is_err());
    let heading = [
        PackedField { shift: 0, bits: 40, base: 0 },
        PackedField { shift: 40, bits: 40, base: 0 },
    ];
    assert_eq!(PackedCanon::field_lanes(&heading)?, 2);
    Ok(())
}

#[test]
fn packed_canon_deduplicates_wide_keys_and_cross_lane_fields() -> Result<()> {
    let pool = Pool::new(0, 1 << 20, 128 << 20)?;
    let rows = [(7i64, 1i64 << 35), (7, 3), (7, 1 << 35), (9, 3)];
    let lanes = rows.iter().flat_map(|&(a, b)| {
        let key = (a as u128) | ((b as u128) << 40);
        [key as u64, (key >> 64) as u64]
    }).collect::<Vec<_>>();
    let input = Buffer::from_host(pool.clone(), &lanes)?;
    let fields = [
        PackedField { shift: 0, bits: 40, base: 0 },
        PackedField { shift: 40, bits: 40, base: 0 },
    ];
    let mut canon = PackedCanon::new(&pool, rows.len())?;
    let keys = unsafe { canon.canonicalize(&input, rows.len(), 80)? };
    assert_eq!(keys.lanes(), 2);
    assert_eq!(keys.capacity(), rows.len());
    assert_eq!(keys.logical_count()?, 3);
    let source = unsafe {
        ColumnDescriptor::from_packed(keys.keys().clone(), 3, 2, fields[1],
            Bounds::Inclusive { min: 0, max: (1i64 << 40) - 1 })?
    };
    let constant = unsafe {
        ColumnDescriptor::from_packed(keys.keys().clone(), 3, 2,
            PackedField { shift: 80, bits: 0, base: 0 }, Bounds::singleton(0))?
    };
    let gathered = RangeBuilder::new(&pool)?.build(&RangeDescriptor {
        key: vec![KeyDescriptor { column: constant, domain_size: 1 }],
        selection: None,
        payload: vec![PayloadDescriptor { column: source, kind: PayloadKind::I64 }],
    })?;
    assert_eq!(gathered.payload()[0].to_host()?, [3, 3, 1i64 << 35]);
    let dictionary = Arc::new(cq_gpu_runtime::Dictionary::default());
    let output = keys.into_columns::<(i64, i64)>(&fields, dictionary.clone(), vec![])?;
    assert_eq!(output.to_host()?, [(7, 3), (9, 3), (7, 1 << 35)]);
    let output = packed_columns::<(i64, i64)>(input, rows.len(), &fields, dictionary.clone(), vec![])?;
    assert_eq!(output.to_host()?, rows);

    // Both lanes participate in equality, including the most significant bit.
    let input = Buffer::from_host(pool.clone(), &[0u64, 0, 0, 1u64 << 63, 0, 0, u64::MAX, u64::MAX])?;
    let keys = unsafe { canon.canonicalize(&input, 4, 128)? };
    let fields = [
        PackedField { shift: 0, bits: 64, base: i64::MIN },
        PackedField { shift: 64, bits: 64, base: i64::MIN },
    ];
    let output = keys.into_columns::<(i64, i64)>(&fields, dictionary, vec![])?;
    assert_eq!(output.to_host()?, [(i64::MIN, i64::MIN), (i64::MIN, 0), (i64::MAX, i64::MAX)]);
    Ok(())
}

#[test]
fn providers_preserve_ranges_duplicate_rows_and_canonical_support() -> Result<()> {
    let pool = Pool::new(0, 1 << 20, 128 << 20)?;
    let outer = column(&pool, &[1, 0, 1, 0, 1])?;
    let inner = column(&pool, &[9, 4, 7, 4, 7])?;
    let payload = column(&pool, &[10, 11, 12, 13, 14])?;
    let selection = RangeSelection {
        column: payload.clone(),
        start: 11,
        end: 14,
    };
    let descriptor = RangeDescriptor {
        key: vec![
            KeyDescriptor {
                column: outer,
                domain_size: 3,
            },
            KeyDescriptor {
                column: inner,
                domain_size: 10,
            },
        ],
        selection: Some(selection.clone()),
        payload: vec![PayloadDescriptor {
            column: payload,
            kind: PayloadKind::U32,
        }],
    };
    let builder =
        RangeBuilder::new(&pool).map_err(|error| format!("range program load: {error}"))?;
    let selected = builder
        .build(&descriptor)
        .map_err(|error| format!("range build: {error}"))?;
    assert_eq!(selected.len(), 3);
    assert_eq!(selected.offsets().to_host()?, [0, 2, 3, 3]);
    assert_eq!(selected.payload()[0].to_host()?, [11, 13, 12]);

    let positions = cq_gpu_sequence_runtime::select(pool.clone(), &selection)?;
    assert_eq!(positions.len(), 3);

    let mut empty = descriptor.clone();
    empty.selection.as_mut().unwrap().start = 20;
    empty.selection.as_mut().unwrap().end = 30;
    let empty = builder.build(&empty)?;
    assert!(empty.is_empty());
    assert_eq!(empty.offsets().to_host()?, [0, 0, 0, 0]);
    assert!(empty.payload()[0].to_host()?.is_empty());

    let keys = Buffer::from_host(pool.clone(), &[7u64, 2, 7, 1, 2])?;
    let mut canon =
        PackedCanon::new(&pool, 5).map_err(|error| format!("canon program load: {error}"))?;
    // Keys are initialized, in the advertised three-bit range, on this stream.
    let unique = unsafe { canon.canonicalize(&keys, 5, 3)? };
    assert_eq!(unique.logical_count()?, 3);
    let fields = [PackedField {
        shift: 0,
        bits: 3,
        base: 0,
    }];
    let dictionary = Arc::new(cq_gpu_runtime::Dictionary::default());
    let output = unique.into_columns::<(u32,)>(&fields, dictionary.clone(), vec![])?;
    assert_eq!(output.to_host()?, [(1,), (2,), (7,)]);
    let output = packed_columns::<(u32,)>(keys, 5, &fields, dictionary, vec![])?;
    assert_eq!(output.to_host()?, [(7,), (2,), (7,), (1,), (2,)]);

    // Setup-time metadata can be reused without changing retained publications.
    let mut workspace = RangeWorkspace::new(pool.clone(), &descriptor)?;
    let retained = builder.build_with_workspace(&descriptor, &mut workspace)?;
    let mut changed = descriptor.clone();
    changed.selection.as_mut().unwrap().end = 12;
    let newer = builder.build_with_workspace(&changed, &mut workspace)?;
    assert_eq!(retained.payload()[0].to_host()?, [11, 13, 12]);
    assert_eq!(newer.payload()[0].to_host()?, [11]);
    assert_eq!(retained.offsets().to_host()?, [0, 2, 3, 3]);
    assert_eq!(newer.offsets().to_host()?, [0, 1, 1, 1]);
    drop((retained, newer));
    let reused = builder.build_with_workspace(&descriptor, &mut workspace)?;
    assert_eq!(reused.payload()[0].to_host()?, [11, 13, 12]);

    let mut adapter = PackedColumnsAdapter::new(pool.clone(), 1)?;
    let dictionary = Arc::new(cq_gpu_runtime::Dictionary::default());
    let first = adapter.packed_columns::<(u32,)>(
        Buffer::from_host_async(pool.clone(), &[3u64, 1])?,
        2,
        &fields,
        dictionary.clone(),
        vec![],
    )?;
    let second = adapter.packed_columns::<(u32,)>(
        Buffer::from_host_async(pool.clone(), &[4u64])?,
        1,
        &fields,
        dictionary.clone(),
        vec![],
    )?;
    assert_eq!(first.to_host()?, [(3,), (1,)]);
    assert_eq!(second.to_host()?, [(4,)]);
    drop((first, second));
    let empty = adapter.packed_columns::<(u32,)>(
        Buffer::from_host_async(pool.clone(), &[] as &[u64])?,
        0,
        &fields,
        dictionary,
        vec![],
    )?;
    assert!(empty.to_host()?.is_empty());

    let upload = Buffer::from_host_async(pool.clone(), &[1u32, 2])?;
    unsafe { upload.write_async(&[9u32])? };
    assert_eq!(upload.to_host()?, [9, 2]);
    assert!(unsafe { upload.write_async(&[1u32, 2, 3]) }.is_err());
    Ok(())
}

#[test]
fn i64_payloads_preserve_full_width_values_and_partial_vectors() -> Result<()> {
    let pool = Pool::new(0, 1 << 20, 128 << 20)?;
    let values: Vec<i64> = (0..2051)
        .map(|row| match row % 5 {
            0 => i64::MIN,
            1 => i64::MAX,
            2 => i32::MIN as i64 - row as i64,
            3 => u32::MAX as i64 + row as i64,
            _ => -1,
        })
        .collect();
    let keys: Vec<u32> = (0..values.len()).map(|row| (row % 3) as u32).collect();
    let mut expected: Vec<_> = keys.iter().copied().zip(values.iter().copied()).collect();
    expected.sort_by_key(|&(key, _)| key);
    let expected: Vec<_> = expected.into_iter().map(|(_, value)| value).collect();
    let builder = RangeBuilder::new(&pool)?;
    for codec in [Codec::Plain, Codec::For, Codec::Delta] {
        let payload = unsafe {
            ColumnDescriptor::new(
                Arc::new(EncodedColumn::from_host(pool.clone(), &values, codec)?),
                values.len(),
            )?
        };
        let mut descriptor = RangeDescriptor {
            key: vec![KeyDescriptor {
                column: column(&pool, &keys)?,
                domain_size: 4,
            }],
            selection: None,
            payload: vec![PayloadDescriptor {
                column: payload.clone(),
                kind: PayloadKind::I64,
            }],
        };
        let result = builder.build(&descriptor)?;
        assert_eq!(result.payload()[0].to_host()?, expected);
        assert_eq!(result.offsets().to_host()?, [0, 684, 1368, 2051, 2051]);
        descriptor.selection = Some(RangeSelection {
            column: payload,
            start: 1,
            end: 1,
        });
        let empty = builder.build(&descriptor)?;
        assert!(empty.payload()[0].to_host()?.is_empty());
        assert_eq!(empty.offsets().to_host()?, [0, 0, 0, 0, 0]);
    }
    Ok(())
}

#[test]
fn stored_sources_retain_strided_rows_and_packed_fields() -> Result<()> {
    let pool = Pool::new(0, 1 << 20, 128 << 20)?;
    let rows = Arc::new(Buffer::from_host(
        pool.clone(),
        &[
            1i64,
            i64::MAX,
            0,
            i64::MIN,
            1,
            -4_294_967_297,
            0,
            4_294_967_297,
        ],
    )?);
    let key = unsafe {
        ColumnDescriptor::from_rows(rows.clone(), 4, 2, 0, Bounds::Inclusive { min: 0, max: 1 })?
    };
    let payload = unsafe {
        ColumnDescriptor::from_rows(
            rows.clone(),
            4,
            2,
            1,
            Bounds::Inclusive {
                min: i64::MIN,
                max: i64::MAX,
            },
        )?
    };
    drop(rows);
    let mut descriptor = RangeDescriptor {
        key: vec![KeyDescriptor {
            column: key,
            domain_size: 3,
        }],
        selection: None,
        payload: vec![PayloadDescriptor {
            column: payload.clone(),
            kind: PayloadKind::I64,
        }],
    };
    let builder = RangeBuilder::new(&pool)?;
    let retained = builder.build(&descriptor)?;
    assert_eq!(
        retained.payload()[0].to_host()?,
        [i64::MIN, 4_294_967_297, i64::MAX, -4_294_967_297]
    );
    descriptor.selection = Some(RangeSelection {
        column: payload,
        start: i64::MIN,
        end: 0,
    });
    let negative = builder.build(&descriptor)?;
    assert_eq!(negative.offsets().to_host()?, [0, 1, 2, 2]);
    assert_eq!(negative.payload()[0].to_host()?, [i64::MIN, -4_294_967_297]);
    assert_eq!(
        cq_gpu_sequence_runtime::select(pool.clone(), descriptor.selection.as_ref().unwrap())?
            .len(),
        2
    );

    let keys = Arc::new(Buffer::from_host(
        pool.clone(),
        &[0u64, u64::MAX, 1, 1u64 << 63],
    )?);
    let key = unsafe {
        ColumnDescriptor::from_packed(
            keys.clone(),
            4,
            1,
            PackedField {
                shift: 64,
                bits: 0,
                base: 0,
            },
            Bounds::singleton(0),
        )?
    };
    let payload = unsafe {
        ColumnDescriptor::from_packed(
            keys.clone(),
            4,
            1,
            PackedField {
                shift: 0,
                bits: 64,
                base: i64::MIN,
            },
            Bounds::Inclusive {
                min: i64::MIN,
                max: i64::MAX,
            },
        )?
    };
    drop(keys);
    let packed = builder.build(&RangeDescriptor {
        key: vec![KeyDescriptor {
            column: key,
            domain_size: 2,
        }],
        selection: None,
        payload: vec![PayloadDescriptor {
            column: payload,
            kind: PayloadKind::I64,
        }],
    })?;
    assert_eq!(packed.offsets().to_host()?, [0, 4, 4]);
    assert_eq!(
        packed.payload()[0].to_host()?,
        [i64::MIN, i64::MAX, i64::MIN + 1, 0]
    );

    let keys = Arc::new(Buffer::from_host(
        pool.clone(),
        &[
            (1u64 << 42) | 3,
            (((1u64 << 40) - 1) << 2) | 3,
            (1u64 << 42) | (1u64 << 41) | 3,
            7,
        ],
    )?);
    let base = -(1i64 << 39);
    let key = unsafe {
        ColumnDescriptor::from_packed(
            keys.clone(),
            4,
            1,
            PackedField {
                shift: 42,
                bits: 1,
                base: 0,
            },
            Bounds::Inclusive { min: 0, max: 1 },
        )?
    };
    let payload = unsafe {
        ColumnDescriptor::from_packed(
            keys.clone(),
            4,
            1,
            PackedField {
                shift: 2,
                bits: 40,
                base,
            },
            Bounds::Inclusive {
                min: base,
                max: -base - 1,
            },
        )?
    };
    drop(keys);
    let shifted = builder.build(&RangeDescriptor {
        key: vec![KeyDescriptor {
            column: key,
            domain_size: 3,
        }],
        selection: None,
        payload: vec![PayloadDescriptor {
            column: payload,
            kind: PayloadKind::I64,
        }],
    })?;
    assert_eq!(shifted.offsets().to_host()?, [0, 2, 4, 4]);
    assert_eq!(
        shifted.payload()[0].to_host()?,
        [-base - 1, base + 1, base, 0]
    );
    Ok(())
}

#[test]
fn keyless_provider_preserves_one_complete_segment() -> Result<()> {
    let pool = Pool::new(0, 1 << 20, 128 << 20)?;
    let values = column(&pool, &[7, 2, 7, 4])?;
    let mut descriptor = RangeDescriptor {
        key: vec![],
        selection: None,
        payload: vec![PayloadDescriptor {
            column: values.clone(),
            kind: PayloadKind::U32,
        }],
    };
    let builder = RangeBuilder::new(&pool)?;
    let mut workspace = RangeWorkspace::new(pool.clone(), &descriptor)?;
    let complete = builder.build_with_workspace(&descriptor, &mut workspace)?;
    assert_eq!(complete.len(), 4);
    assert_eq!(complete.domain_size(), 0);
    assert!(complete.offsets().to_host()?.is_empty());
    assert_eq!(complete.payload()[0].to_host()?, [7, 2, 7, 4]);
    descriptor.selection = Some(RangeSelection { column: values, start: 4, end: 8 });
    let selected = builder.build_with_workspace(&descriptor, &mut workspace)?;
    assert_eq!(selected.payload()[0].to_host()?, [7, 7, 4]);
    descriptor.selection.as_mut().unwrap().start = 9;
    descriptor.selection.as_mut().unwrap().end = 10;
    let empty = builder.build_with_workspace(&descriptor, &mut workspace)?;
    assert!(empty.is_empty());
    assert!(empty.offsets().to_host()?.is_empty());
    assert!(empty.payload()[0].to_host()?.is_empty());
    assert_eq!(complete.payload()[0].to_host()?, [7, 2, 7, 4]);
    Ok(())
}
