//! Library tile kernels (count) over uploaded S1 scan columns.
mod support;
use cq_storage::scan::*;
use gpu_device_columns::{scan::DeviceScanColumn, tile::{self, Window}, Buffer, Pool};
use std::{ops::Bound, sync::Arc};
use support::tiles::{classification, column, domain, predicates};

fn exercise<T: OrdinalDecode, F: Family>(pool: &Arc<Pool>, values: &[T], rows: u32, summaries: bool, live_counts: &[usize]) {
    let host = column::<T, F>(values, rows, summaries, summaries);
    for &live in live_counts {
        let publication = DeviceScanColumn::upload(pool.clone(), host.clone(), domain::<T>(), live).unwrap();
        let lane = pool.lane().unwrap();
        let view = publication.view_on(lane.clone()).unwrap();
        drop(publication);
        for predicate in predicates() {
            let expected = values[..live].iter().filter(|v| predicate.matches(v.ordinal())).count() as u64;
            let window = Window::from_predicate(&predicate).unwrap();
            let raw = view.count_matches(window, false, None).unwrap();
            let loads = Buffer::<u64>::zeroed(lane.clone(), 1).unwrap();
            let counted = view.count_matches(window, summaries, Some(&loads)).unwrap();
            assert_eq!(raw.to_host().unwrap(), [expected]);
            assert_eq!(counted.to_host().unwrap(), [expected]);
        }
        assert_eq!(view.count_matches(Window::empty(), false, None).unwrap().to_host().unwrap(), [0]);
        assert_eq!(view.count_matches(Window::all(), false, None).unwrap().to_host().unwrap(), [live as u64]);
    }
}

#[test]
fn windows_signed_extremes_and_live_lane_tails() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let values = support::tiles::signed_values();
    for summaries in [false, true] {
        exercise::<i64, Plain>(&pool, &values, 2048, summaries, &support::tiles::TAILS);
        exercise::<i64, Bytes>(&pool, &values, 2048, summaries, &support::tiles::TAILS);
        exercise::<i64, Bytes>(&pool, &[], 2048, summaries, &[0]);
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn mixed_constant_and_native_width_frames() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let values = support::tiles::mixed_values();
    // Bytes narrows to constant/u8/u16 frames; wider spans keep the native carrier.
    let host = column::<u64, Bytes>(&values, 2048, true, true);
    assert_eq!(host.headers().iter().map(|h| h.width).collect::<Vec<_>>(), [Width::W0, Width::W8, Width::W16, Width::W64, Width::W64]);
    for summaries in [false, true] {
        exercise::<u64, Bytes>(&pool, &values, 2048, summaries, &[values.len(), values.len() - 1]);
        exercise::<u64, Plain>(&pool, &values, 2048, summaries, &[values.len()]);
    }
    // A u8 frame whose representable carrier extends past u64::MAX must clip,
    // not wrap, before its bounds narrow to u8.
    let near_max: Vec<u64> = (0..2049).map(|row| u64::MAX - (row % 2) as u64).collect();
    exercise::<u64, Bytes>(&pool, &near_max, 2048, false, &[2049]);
    exercise::<u64, Bytes>(&pool, &near_max, 2048, true, &[2049]);
    let constant = vec![u64::MAX; 3073];
    exercise::<u64, Bytes>(&pool, &constant, 2048, true, &[3073]);
    let publication = DeviceScanColumn::upload(pool.clone(), column::<u64, Bytes>(&constant, 2048, false, false), domain::<u64>(), constant.len()).unwrap();
    let lane = pool.lane().unwrap();
    let view = publication.view_on(lane.clone()).unwrap();
    let loads = Buffer::zeroed(lane, 1).unwrap();
    assert_eq!(view.count_matches(Window::all(), false, Some(&loads)).unwrap().to_host().unwrap(), [constant.len() as u64]);
    assert_eq!(loads.to_host().unwrap(), [0]);
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn summaries_count_exactly_ambivalent_vector_payload() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let (capacity, values) = support::tiles::summary_values();
    for (blocks, vectors) in [(true, true), (true, false), (false, true)] {
        let host = column::<u64, Bytes>(&values, 2048, blocks, vectors);
        for live in [capacity, capacity - 768] {
            let publication = DeviceScanColumn::upload(pool.clone(), host.clone(), domain::<u64>(), live).unwrap();
            let lane = pool.lane().unwrap();
            let view = publication.view_on(lane.clone()).unwrap();
            let last = (live / VECTOR_ROWS * 10_000) as u64;
            let cases = [
                Predicate::Point(u64::MAX),
                Predicate::Window { lo: Bound::Unbounded, hi: Bound::Unbounded },
                Predicate::Window { lo: Bound::Included(20_100), hi: Bound::Included(40_150) },
                Predicate::Point(568 * 10_000 + 100),
                Predicate::Window { lo: Bound::Included(last + 100), hi: Bound::Included(last + 200) },
                Predicate::Window { lo: Bound::Included(0), hi: Bound::Included(9999) },
                Predicate::Window { lo: Bound::Excluded(u64::MAX), hi: Bound::Unbounded },
            ];
            for predicate in cases {
                let loads = Buffer::<u64>::zeroed(lane.clone(), 1).unwrap();
                let output = view.count_matches(Window::from_predicate(&predicate).unwrap(), true, Some(&loads)).unwrap();
                let expected = values[..live].iter().filter(|&&v| predicate.matches(v)).count() as u64;
                let expected_loads: u64 = (0..live.div_ceil(VECTOR_ROWS))
                    .filter(|&v| classification(&host, v, &predicate) == tile::AMBIVALENT)
                    .map(|v| (live - v * VECTOR_ROWS).min(VECTOR_ROWS) as u64).sum();
                assert_eq!(output.to_host().unwrap(), [expected]);
                assert_eq!(loads.to_host().unwrap(), [expected_loads]);
            }
        }
    }
    gpu_device_columns::release_cached_kernels();
}
#[test]
fn count_rejects_missing_summaries_and_unordered_counter_lanes() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let host = column::<u64, Bytes>(&[1, 2, 3], 2048, false, false);
    let publication = DeviceScanColumn::upload(pool.clone(), host, domain::<u64>(), 3).unwrap();
    let lane = pool.lane().unwrap();
    let view = publication.view_on(lane.clone()).unwrap();
    assert!(view.count_matches(Window::all(), true, None).is_err());
    let empty = Buffer::zeroed(lane.clone(), 0).unwrap();
    assert!(view.count_matches(Window::all(), false, Some(&empty)).is_err());
    let other_lane = Buffer::zeroed(pool, 1).unwrap();
    assert!(view.count_matches(Window::all(), false, Some(&other_lane)).is_err());
    let loads = Buffer::zeroed(lane, 1).unwrap();
    // Multiple queued calls accumulate in the caller-owned counter without resetting it.
    let first = view.count_matches(Window::all(), false, Some(&loads)).unwrap();
    let second = view.count_matches(Window::all(), false, Some(&loads)).unwrap();
    drop(view);
    drop(publication);
    assert_eq!(first.to_host().unwrap(), [3]);
    assert_eq!(second.to_host().unwrap(), [3]);
    assert_eq!(loads.to_host().unwrap(), [6]);
    gpu_device_columns::release_cached_kernels();
}
