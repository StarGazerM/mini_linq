//! Device probe of the tile helpers: per-row ordinals, AND/OR masks, vector
//! classes and sparse-survivor payload loads. This binary defines its own
//! kernel and therefore launches no library kernels (see support/tiles.rs).
mod support;
use cq_storage::scan::*;
use cuda_core::LaunchConfig1D;
use gpu_device_columns::{scan::{DeviceScanColumn, DeviceScanView}, tile::{self, Window}, Buffer, Pool};
use std::{ops::Bound, sync::Arc};
use support::tiles::{classification, column, domain};

#[cuda_device::cuda_module]
mod probe {
    use gpu_device_columns::{scan::ScanDeviceDescriptor, tile::{self, Window, RawPayload, CountingPayload}};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn inspect(view: ScanDeviceDescriptor, a: Window, b: Window, ordinals: *mut u64, and_mask: *mut u8, or_mask: *mut u8, classes: *mut u32, loads: *mut u64) {
        let mut v = thread::blockIdx_x() as u64;
        let lane = thread::threadIdx_x();
        while v < unsafe { tile::vectors(&view) } {
            let frame = unsafe { tile::vector(&view, v) };
            let live = unsafe { tile::live(frame, lane) };
            let a_mask = unsafe { tile::test(RawPayload, frame, lane, a, live) };
            // Instrument a sparse survivor mask: b must not load the rejected rows.
            let and = unsafe { tile::test(CountingPayload { loads }, frame, lane, b, a_mask) };
            let b_mask = unsafe { tile::test(RawPayload, frame, lane, b, live) };
            if lane == 0 {
                unsafe { classes.add(v as usize).write(tile::classify(&view, v, a)); }
            }
            let mut k = 0;
            while k < tile::ROWS_PER_LANE {
                if live & (1 << k) != 0 {
                    let local = k * tile::LANES + lane;
                    let row = (v * tile::VECTOR_ROWS + local as u64) as usize;
                    unsafe {
                        ordinals.add(row).write(tile::ordinal(RawPayload, frame, local));
                        and_mask.add(row).write(((and >> k) & 1) as u8);
                        or_mask.add(row).write((((a_mask | b_mask) >> k) & 1) as u8);
                    }
                }
                k += 1;
            }
            v += thread::gridDim_x() as u64;
        }
    }
}

fn inspect<T: OrdinalDecode, F: Family>(lane: &Arc<Pool>, view: &DeviceScanView<T, F>, host: &ScanColumn<T, F>, values: &[T], a: &Predicate, b: &Predicate) {
    let live = values.len();
    let ordinals = unsafe { Buffer::<u64>::uninitialized(lane.clone(), live) }.unwrap();
    let and_mask = unsafe { Buffer::<u8>::uninitialized(lane.clone(), live) }.unwrap();
    let or_mask = unsafe { Buffer::<u8>::uninitialized(lane.clone(), live) }.unwrap();
    let classes = unsafe { Buffer::<u32>::uninitialized(lane.clone(), live.div_ceil(VECTOR_ROWS)) }.unwrap();
    let loads = Buffer::<u64>::zeroed(lane.clone(), 1).unwrap();
    if live != 0 {
        let module = unsafe { probe::load(lane.context()) }.unwrap();
        let launch = module.prepare_inspect(LaunchConfig1D::new(live.div_ceil(VECTOR_ROWS).min(568) as u32, 256, 0)).unwrap();
        unsafe {
            module.inspect(lane.stream(), &launch, view.abi(), Window::from_predicate(a).unwrap(), Window::from_predicate(b).unwrap(), ordinals.ptr(), and_mask.ptr(), or_mask.ptr(), classes.ptr(), loads.ptr()).unwrap();
        }
        // Keep the loaded module alive through readback of its queued launch.
        assert_eq!(ordinals.to_host().unwrap(), values.iter().map(|v| v.ordinal()).collect::<Vec<_>>());
    } else {
        assert!(ordinals.to_host().unwrap().is_empty());
    }
    assert_eq!(and_mask.to_host().unwrap(), values.iter().map(|v| (a.matches(v.ordinal()) && b.matches(v.ordinal())) as u8).collect::<Vec<_>>());
    assert_eq!(or_mask.to_host().unwrap(), values.iter().map(|v| (a.matches(v.ordinal()) || b.matches(v.ordinal())) as u8).collect::<Vec<_>>());
    assert_eq!(classes.to_host().unwrap(), (0..live.div_ceil(VECTOR_ROWS)).map(|v| classification(host, v, a)).collect::<Vec<_>>());
    let b = Window::from_predicate(b).unwrap();
    let expected_loads = values.iter().enumerate().filter(|(row, value)| {
        let header = &host.headers()[row / host.block_rows().get()];
        header.width != Width::W0 && a.matches(value.ordinal())
            && tile::residual_window(b, header.base, header.width as u32).is_some()
    }).count() as u64;
    assert_eq!(loads.to_host().unwrap(), [expected_loads]);
}

fn probe_all<T: OrdinalDecode, F: Family>(pool: &Arc<Pool>, values: &[T], summaries: bool, live_counts: &[usize], a: &Predicate, b: &Predicate) {
    let host = column::<T, F>(values, 2048, summaries, summaries);
    for &live in live_counts {
        let publication = DeviceScanColumn::upload(pool.clone(), host.clone(), domain::<T>(), live).unwrap();
        let lane = pool.lane().unwrap();
        let view = publication.view_on(lane.clone()).unwrap();
        drop(publication);
        inspect(&lane, &view, &host, &values[..live], a, b);
    }
}

fn narrow() -> (Predicate, Predicate) {
    (Predicate::Window { lo: Bound::Included(255), hi: Bound::Included(u64::MAX - 1) },
     Predicate::Window { lo: Bound::Excluded(256), hi: Bound::Included(65_536) })
}

#[test]
fn probe_signed_extremes_and_live_lane_tails() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let values = support::tiles::signed_values();
    let (a, b) = narrow();
    for summaries in [false, true] {
        probe_all::<i64, Plain>(&pool, &values, summaries, &support::tiles::TAILS, &a, &b);
        probe_all::<i64, Bytes>(&pool, &values, summaries, &support::tiles::TAILS, &a, &b);
        probe_all::<i64, Bytes>(&pool, &[], summaries, &[0], &a, &b);
    }
}

#[test]
fn probe_mixed_constant_and_native_width_frames() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let values = support::tiles::mixed_values();
    let (a, b) = narrow();
    for summaries in [false, true] {
        probe_all::<u64, Bytes>(&pool, &values, summaries, &[values.len(), values.len() - 1], &a, &b);
        probe_all::<u64, Plain>(&pool, &values, summaries, &[values.len()], &a, &b);
    }
    let near_max: Vec<u64> = (0..2049).map(|row| u64::MAX - (row % 2) as u64).collect();
    probe_all::<u64, Bytes>(&pool, &near_max, true, &[2049], &a, &b);
    probe_all::<u64, Bytes>(&pool, &vec![u64::MAX; 3073], true, &[3073], &a, &b);
}

#[test]
fn probe_visits_every_vector_beyond_the_grid_cap() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let (capacity, values) = support::tiles::summary_values();
    let a = Predicate::Window { lo: Bound::Included(20_100), hi: Bound::Included(40_150) };
    let b = Predicate::Window { lo: Bound::Included(30_000), hi: Bound::Included(40_200) };
    for (blocks, vectors) in [(true, true), (true, false), (false, true)] {
        let host = column::<u64, Bytes>(&values, 2048, blocks, vectors);
        for live in [capacity, capacity - 768] {
            let publication = DeviceScanColumn::upload(pool.clone(), host.clone(), domain::<u64>(), live).unwrap();
            let lane = pool.lane().unwrap();
            let view = publication.view_on(lane.clone()).unwrap();
            inspect(&lane, &view, &host, &values[..live], &a, &b);
        }
    }
}
