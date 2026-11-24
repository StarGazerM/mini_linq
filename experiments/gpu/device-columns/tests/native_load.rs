use gpu_device_columns::{
    detect_integer_codec, native::NativeView, BaseLayout, Bounds, Buffer, Codec, EncodedColumn,
    Lane, Pool, VectorHeader,
};
use std::sync::Arc;

const LAYOUTS: [BaseLayout; 4] = [
    BaseLayout::For,
    BaseLayout::Plain64,
    BaseLayout::Delta,
    BaseLayout::Native,
];

fn fields(header: &VectorHeader) -> (u64, i64, u32, u32, u32, u32) {
    (header.offset, header.base, header.width, header.codec, header.rows, header.padding)
}

fn check<L: Lane>(pool: &Arc<Pool>, values: &[L], positions: Option<&[u32]>, layout: BaseLayout) -> Codec {
    let input = Buffer::from_host(pool.clone(), values).unwrap();
    let order = positions.map(|positions| Buffer::from_host(pool.clone(), positions).unwrap());
    let expected: Vec<i64> = match positions {
        Some(positions) => positions.iter().map(|&row| values[row as usize].widen()).collect(),
        None => values.iter().map(|value| value.widen()).collect(),
    };
    let column = unsafe {
        EncodedColumn::from_native(
            pool.clone(),
            NativeView { data: input.ptr().cast(), element: L::ELEMENT },
            values.len(),
            order.as_ref(),
            layout,
        )
    }.unwrap();
    assert_eq!(column.capacity(), expected.len());
    assert_eq!(column.bounds(), Bounds::from_values(&expected));
    assert_eq!(column.to_host().unwrap(), expected);
    let reference = EncodedColumn::from_host_layout(pool.clone(), &expected, layout).unwrap();
    assert_eq!(column.codec(), reference.codec());
    assert_eq!(column.bytes(), reference.bytes());
    let (headers, payload) = column.inspect().unwrap();
    let (reference_headers, reference_payload) = reference.inspect().unwrap();
    assert_eq!(headers.iter().map(fields).collect::<Vec<_>>(), reference_headers.iter().map(fields).collect::<Vec<_>>());
    assert_eq!(payload, reference_payload);
    if layout == BaseLayout::Native {
        assert!(headers.is_empty());
    }
    column.codec()
}

#[test]
fn native_widths_and_selected_rows_match_all_resident_layouts() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let signed: Vec<i32> = (0..3101).map(|i| match i % 5 {
        0 => i32::MIN,
        1 => i32::MAX,
        _ => i * 7 - 12000,
    }).collect();
    let unsigned: Vec<u32> = (0..3101).map(|i| match i % 5 {
        0 => 0,
        1 => u32::MAX,
        _ => (1u32 << 31) + i * 13,
    }).collect();
    let wide: Vec<i64> = (0..3101).map(|i| match i % 5 {
        0 => i64::MIN,
        1 => i64::MAX,
        _ => i * 10000000003 - 9000000000000,
    }).collect();
    // Reordered and repeated selections cross both lane and vector boundaries.
    let selected: Vec<u32> = (0..2307).map(|i| (i * 103 + 3) % 3101).collect();
    // Exclude every source extremum: bounds must describe only selected rows.
    let interior: Vec<u32> = (0..3101).filter(|i| i % 5 >= 2).collect();
    for layout in LAYOUTS {
        for order in [None, Some(selected.as_slice()), Some(interior.as_slice()), Some(&[][..])] {
            check(&pool, &signed, order, layout);
            check(&pool, &unsigned, order, layout);
            check(&pool, &wide, order, layout);
        }
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn native_extrema_empty_and_partial_vectors() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    for rows in [0, 1, 2, 63, 64, 65, 1023, 1024, 1025] {
        let values: Vec<i64> = (0..rows).map(|i| if i % 2 == 0 { i64::MIN } else { i64::MAX }).collect();
        for layout in LAYOUTS {
            check(&pool, &values, None, layout);
        }
    }
    for value in [i64::MIN, i64::MAX, 0, -1] {
        for layout in LAYOUTS {
            check(&pool, &vec![value; 1025], None, layout);
        }
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn native_delta_detection_preserves_exact_host_policy() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let sorted: Vec<i64> = (0..5000).map(|i| 1000000 + i * 4 + i % 3).collect();
    assert_eq!(check(&pool, &sorted, None, BaseLayout::Delta), Codec::Delta);
    let reversed: Vec<u32> = (0..5000).rev().collect();
    assert_eq!(check(&pool, &sorted, Some(&reversed), BaseLayout::Delta), Codec::For);
    let ascending: Vec<u32> = (0..5000).step_by(2).collect();
    assert_eq!(check(&pool, &sorted, Some(&ascending), BaseLayout::Delta), Codec::Delta);
    let ids: Vec<u32> = (0..3000).map(|i| (1u32 << 31) + i * 2).collect();
    assert_eq!(check(&pool, &ids, None, BaseLayout::Delta), Codec::Delta);
    let signed: Vec<i32> = (0..3000).map(|i| i32::MIN + i * 2).collect();
    assert_eq!(check(&pool, &signed, None, BaseLayout::Delta), Codec::Delta);
    // The difference MIN -> MAX wraps to -1. The logical bounds must still
    // span all signed values, not the tiny residual domain used by DELTA.
    let mut extrema = vec![i64::MAX; 1025];
    extrema[0] = i64::MIN;
    assert_eq!(check(&pool, &extrema, None, BaseLayout::Delta), Codec::Delta);
    for swaps in [0, 64, 65, 100] {
        let mut values: Vec<i64> = (0..1024).map(|i| i * 16).collect();
        for pair in 0..swaps {
            values.swap(pair * 2, pair * 2 + 1);
        }
        let expected = detect_integer_codec(&values);
        assert_eq!(check(&pool, &values, None, BaseLayout::Delta), expected);
    }
    // Descents at vector boundaries do not count in the existing heuristic.
    let reset: Vec<i64> = (0..3073).map(|i| (i % 1024) * 4).collect();
    assert_eq!(check(&pool, &reset, None, BaseLayout::Delta), detect_integer_codec(&reset));
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn native_compact_offsets_span_multiple_scan_blocks() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let values: Vec<i64> = (0..(257 * 1024 + 17)).map(|i| {
        let width = (i / 1024 % 63) as u32;
        ((i % 1024) as i64).wrapping_mul(1i64 << width)
    }).collect();
    let positions: Vec<u32> = (0..values.len() as u32).rev().collect();
    for layout in [BaseLayout::For, BaseLayout::Plain64, BaseLayout::Delta] {
        check(&pool, &values, Some(&positions), layout);
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn native_host_packing_and_unknown_elements_are_rejected() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let source = NativeView { data: std::ptr::null(), element: <i64 as Lane>::ELEMENT };
    let error = unsafe { EncodedColumn::from_native(pool.clone(), source, 0, None, BaseLayout::HostFor) }
        .err().unwrap().to_string();
    assert!(error.contains("HostFor"));
    let source = NativeView { data: std::ptr::null(), element: u32::MAX };
    let error = unsafe { EncodedColumn::from_native(pool, source, 0, None, BaseLayout::For) }
        .err().unwrap().to_string();
    assert!(error.contains("element"));
}

const SELECTED_LAYOUTS: [BaseLayout; 5] = [
    BaseLayout::For,
    BaseLayout::Plain64,
    BaseLayout::Delta,
    BaseLayout::Native,
    BaseLayout::HostFor,
];

fn check_selected(
    pool: &Arc<Pool>,
    source: &EncodedColumn,
    values: &[i64],
    positions: &[u32],
    layout: BaseLayout,
) -> Codec {
    let order = Buffer::from_host(pool.clone(), positions).unwrap();
    let expected: Vec<i64> = positions.iter().map(|&row| values[row as usize]).collect();
    let column = EncodedColumn::from_selected(pool.clone(), source, &order, layout).unwrap();
    drop(order);
    assert_eq!(column.capacity(), expected.len());
    assert_eq!(column.bounds(), Bounds::from_values(&expected));
    // DECODE P: selected-column diagnostic download publishes the exact gathered answer for assertions.
    assert_eq!(column.to_host().unwrap(), expected);
    let reference = EncodedColumn::from_host_layout(pool.clone(), &expected, layout).unwrap();
    assert_eq!(column.codec(), reference.codec());
    assert_eq!(column.bytes(), reference.bytes());
    let (headers, payload) = column.inspect().unwrap();
    let (reference_headers, reference_payload) = reference.inspect().unwrap();
    assert_eq!(
        headers.iter().map(fields).collect::<Vec<_>>(),
        reference_headers.iter().map(fields).collect::<Vec<_>>(),
    );
    assert_eq!(payload, reference_payload);
    column.codec()
}

#[test]
fn selected_encoded_layout_matrix_preserves_reordering_and_exact_bounds() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let values: Vec<i64> = (0..3101).map(|i| match i % 5 {
        0 => i64::MIN,
        1 => i64::MAX,
        _ => i * 10000000003 - 9000000000000,
    }).collect();
    // Each adjacent pair repeats a row; the stride reorders across vectors.
    let repeated: Vec<u32> = (0..2307).map(|i| ((i / 2) * 103 + 3) % 3101).collect();
    let interior: Vec<u32> = (0..3101).filter(|i| i % 5 >= 2).collect();
    for source_layout in SELECTED_LAYOUTS {
        let source = EncodedColumn::from_host_layout(pool.clone(), &values, source_layout).unwrap();
        for layout in SELECTED_LAYOUTS {
            for positions in [&repeated[..], &interior[..], &[][..]] {
                check_selected(&pool, &source, &values, positions, layout);
            }
        }
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn selected_encoded_empty_partial_and_extreme_vectors() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    for rows in [0, 1, 2, 63, 64, 65, 1023, 1024, 1025] {
        let values: Vec<i64> = (0..rows).map(|i| if i % 2 == 0 { i64::MIN } else { i64::MAX }).collect();
        let positions: Vec<u32> = (0..rows as u32).rev().collect();
        for source_layout in SELECTED_LAYOUTS {
            let source = EncodedColumn::from_host_layout(pool.clone(), &values, source_layout).unwrap();
            for layout in SELECTED_LAYOUTS {
                check_selected(&pool, &source, &values, &positions, layout);
            }
        }
    }
    for value in [i64::MIN, i64::MAX, 0, -1] {
        let values = vec![value; 1025];
        let positions = vec![1024; 1025];
        for source_layout in SELECTED_LAYOUTS {
            let source = EncodedColumn::from_host_layout(pool.clone(), &values, source_layout).unwrap();
            for layout in SELECTED_LAYOUTS {
                check_selected(&pool, &source, &values, &positions, layout);
            }
        }
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn selected_delta_sources_and_destination_detection_match_host_policy() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let sorted: Vec<i64> = (0..5000).map(|i| 1000000 + i * 4 + i % 3).collect();
    let mut extrema = vec![i64::MAX; 5000];
    extrema[0] = i64::MIN;
    let ascending: Vec<u32> = (0..5000).step_by(2).collect();
    let descending: Vec<u32> = (0..5000).rev().collect();
    for values in [&sorted, &extrema] {
        let source = EncodedColumn::from_host_layout(pool.clone(), values, BaseLayout::Delta).unwrap();
        assert_eq!(source.codec(), Codec::Delta);
        for layout in SELECTED_LAYOUTS {
            for positions in [&ascending, &descending] {
                check_selected(&pool, &source, values, positions, layout);
            }
        }
    }
    for source_layout in SELECTED_LAYOUTS {
        let source = EncodedColumn::from_host_layout(pool.clone(), &sorted, source_layout).unwrap();
        assert_eq!(check_selected(&pool, &source, &sorted, &ascending, BaseLayout::Delta), Codec::Delta);
        assert_eq!(check_selected(&pool, &source, &sorted, &descending, BaseLayout::Delta), Codec::For);
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn selected_sources_and_positions_can_drop_after_submission() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let values: Vec<i64> = (0..3101).map(|i| i * 4 - 9000).collect();
    let positions: Vec<u32> = (0..2307).map(|i| (i * 103 + 3) % 3101).collect();
    let expected: Vec<i64> = positions.iter().map(|&i| values[i as usize]).collect();
    for source_layout in SELECTED_LAYOUTS {
        for layout in SELECTED_LAYOUTS {
            let source = EncodedColumn::from_host_layout(pool.clone(), &values, source_layout).unwrap();
            let order = Buffer::from_host(pool.clone(), &positions).unwrap();
            let selected = EncodedColumn::from_selected(pool.clone(), &source, &order, layout).unwrap();
            drop(source);
            drop(order);
            // Force allocation reuse after the deferred input frees.
            let replacement = Buffer::from_host(pool.clone(), &vec![i64::MIN; values.len()]).unwrap();
            // DECODE P: post-drop diagnostic publication checks that selected values outlive their source allocations.
            assert_eq!(selected.to_host().unwrap(), expected);
            drop(replacement);
        }
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn selected_rejects_foreign_pools_and_out_of_range_positions() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let other = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let source = EncodedColumn::from_host_layout(pool.clone(), &[1i64, 2], BaseLayout::For).unwrap();
    let local = Buffer::from_host(pool.clone(), &[0u32]).unwrap();
    let foreign = Buffer::from_host(other.clone(), &[0u32]).unwrap();
    assert!(EncodedColumn::from_selected(other, &source, &foreign, BaseLayout::For)
        .err().unwrap().to_string().contains("source"));
    assert!(EncodedColumn::from_selected(pool.clone(), &source, &foreign, BaseLayout::For)
        .err().unwrap().to_string().contains("positions"));
    for layout in SELECTED_LAYOUTS {
        for positions in [&[0u32, 2][..], &[u32::MAX][..]] {
            let order = Buffer::from_host(pool.clone(), positions).unwrap();
            assert!(EncodedColumn::from_selected(pool.clone(), &source, &order, layout)
                .err().unwrap().to_string().contains("logical rows"));
        }
        let empty = EncodedColumn::from_host_layout(pool.clone(), &[] as &[i64], layout).unwrap();
        assert!(EncodedColumn::from_selected(pool.clone(), &empty, &local, layout)
            .err().unwrap().to_string().contains("logical rows"));
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn selected_respects_device_logical_count_not_capacity() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let values = [7i64, -9, 12, i64::MAX];
    for codec in [Codec::Plain, Codec::For, Codec::Delta] {
        let input = Buffer::from_host(pool.clone(), &values).unwrap();
        let count = Arc::new(Buffer::from_host(pool.clone(), &[3u64]).unwrap());
        let source = unsafe {
            EncodedColumn::from_device(
                pool.clone(), input.ptr(), values.len(), count, 0, codec,
                Bounds::from_values(&values),
            )
        }.unwrap();
        for layout in SELECTED_LAYOUTS {
            check_selected(&pool, &source, &values, &[2, 0, 1, 2], layout);
            let order = Buffer::from_host(pool.clone(), &[3u32]).unwrap();
            assert!(EncodedColumn::from_selected(pool.clone(), &source, &order, layout)
                .err().unwrap().to_string().contains("logical rows"));
        }
    }
    let input = Buffer::from_host(pool.clone(), &values).unwrap();
    let count = Arc::new(Buffer::from_host(pool.clone(), &[3u64]).unwrap());
    let source = EncodedColumn::from_plain_buffer(input, count, 0, Bounds::from_values(&values)).unwrap();
    for layout in SELECTED_LAYOUTS {
        check_selected(&pool, &source, &values, &[2, 0, 1, 2], layout);
        let order = Buffer::from_host(pool.clone(), &[3u32]).unwrap();
        assert!(EncodedColumn::from_selected(pool.clone(), &source, &order, layout)
            .err().unwrap().to_string().contains("logical rows"));
    }
    gpu_device_columns::release_cached_kernels();
}
