use std::sync::Arc;

use cq_gpu_sequence_runtime::canon::{
    packed_columns, PackedCanon, PackedColumnsAdapter, PackedField,
};
use cq_gpu_sequence_runtime::{
    Buffer, Codec, ColumnDescriptor, EncodedColumn, KeyDescriptor, PayloadDescriptor, PayloadKind,
    Pool, RangeBuilder, RangeDescriptor, RangeSelection, RangeWorkspace, Result,
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
