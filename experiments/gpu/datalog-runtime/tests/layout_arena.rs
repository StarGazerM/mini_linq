use gpu_datalog_runtime::{
    concatenate_batches, Buffer, CandidateArena, Column, DeviceRadixRun, Packing, Pool,
    RadixLayout, RadixRunParams, Result, ScalarTuple, SortedBatch, Word,
};
use std::{any::Any, sync::Arc};

const PARAMS: RadixRunParams = RadixRunParams {
    ratio: 4,
    radix_bits: 0,
    merge_tile_rows: 32,
    block_threads: 256,
};

#[test]
fn lane_workers_consume_recorded_initializer_and_release_thread_scratch() -> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static INITIALIZED: AtomicUsize = AtomicUsize::new(0);
    static RELEASED: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static SCRATCH_LIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    fn initialize(relation: usize, pool: Arc<Pool>) -> CandidateArena {
        INITIALIZED.fetch_add(1, Ordering::SeqCst);
        SCRATCH_LIVE.with(|live| {
            assert!(!live.replace(true));
        });
        let mut arena = CandidateArena::new(pool);
        arena.hole(3).record_count(100 + relation);
        arena
    }
    fn release(_: &Arc<Pool>) {
        SCRATCH_LIVE.with(|live| assert!(live.replace(false)));
        RELEASED.fetch_add(1, Ordering::SeqCst);
    }
    let pool = Pool::new(0, 0, 0)?;
    let lanes = gpu_datalog_runtime::Lanes::new(&pool, 2, initialize, release)?;
    for component in 0..2 {
        lanes.fork()?;
        for position in 0..2 {
            lanes.submit(
                1,
                position,
                Box::new(move |lane| {
                    let arena = lane.arena(9, 3);
                    assert_eq!(arena.previous_count(), 109 + position);
                    arena.record_count(110 + position);
                    let lease = arena.reserve(Packing::new(&[32]).unwrap(), 0).unwrap();
                    // SAFETY: the empty prefix is initialized vacuously.
                    unsafe { lease.finish(0, Vec::new()).unwrap() }
                }),
            )?;
        }
        let batches = lanes.join()?;
        assert_eq!(batches.len(), 2);
        drop(batches);
        lanes.release()?;
        assert_eq!(INITIALIZED.load(Ordering::SeqCst), component + 1);
        assert_eq!(RELEASED.load(Ordering::SeqCst), component + 1);
    }
    Ok(())
}

#[test]
fn lane_workers_capture_measured_layouts_without_query_readbacks() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let layout = gpu_datalog_runtime::ArenaLayout::new(
        Packing::new(&[2])?.with_carrier_bits(128)?, vec![Some((0, 2))],
    )?;
    let lanes = gpu_datalog_runtime::Lanes::new(&pool, 2, move |relation, pool| {
        assert_eq!(relation, 7);
        CandidateArena::with_layout(pool, layout.clone())
    }, |_| {})?;
    lanes.fork()?;
    lanes.submit(1, 0, Box::new(|lane| {
        let arena = lane.arena(7, 0);
        let before = gpu_datalog_runtime::readback_count();
        let layout = arena.output_layout().unwrap();
        assert_eq!(layout.width(0), 2);
        assert_eq!(layout.bounds(), [Some((0, 2))]);
        assert_eq!(layout.carrier_bits(), 128);
        let lease = arena.reserve(layout.packing().clone(), 0).unwrap();
        // SAFETY: the empty prefix is initialized without touching device rows.
        let batch = unsafe { lease.finish(0, vec![]).unwrap() };
        assert_eq!(gpu_datalog_runtime::readback_count(), before);
        batch
    }))?;
    let batches = lanes.join()?;
    assert_eq!(batches[0].1.packing().word_bits(), 128);
    lanes.release()?;
    Ok(())
}

#[test]
fn layouts_retain_runs_and_validate_native_schema() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    type Layout = RadixLayout<(i32, u32), (), u64, u64, ()>;
    let packing = Layout::derive_packing()?;
    let words = [
        packing.pack::<u64>(&[-3, 7])?,
        packing.pack::<u64>(&[9, 2])?,
    ];
    // SAFETY: uploads initialize sorted unique prefixes and no writer is kept.
    let empty = unsafe { SortedBatch::from_sorted(Buffer::<u64>::from_host_async(pool.clone(), &[])?, 0)? };
    let batch = unsafe { SortedBatch::from_sorted(Buffer::from_host_async(pool.clone(), &words)?, 2)? };
    let mut state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), PARAMS, packing.bits())?;
    unsafe {
        state.append_sorted(empty.clone())?;
        state.append_sorted(batch.clone())?;
        state.append_sorted(empty)?;
    }
    let layout = Layout::publish(&state, packing.clone())?;
    assert_eq!(layout.len(), 2);
    assert_eq!(layout.view().keys.run_count, 1);
    assert_eq!(layout.view().field_count, 2);
    assert_eq!(layout.view().bits, 64);
    assert!(Arc::ptr_eq(layout.pool(), &pool));
    layout.validate(&[Column::I32, Column::U32])?;
    assert!(layout.validate(&[Column::U32, Column::I32]).is_err());
    assert!(Layout::from_sorted(pool.clone(), Packing::new(&[32, 32])?, PARAMS, batch.clone(), 64).is_err());
    let other = Pool::new(0, 0, 0)?;
    assert!(Layout::from_sorted(other, packing, PARAMS, batch, 64).is_err());
    drop(state);
    let retained = layout.owner();
    let weak = Arc::downgrade(&retained);
    let clone = layout.clone();
    drop(layout);
    drop(retained);
    assert!(weak.upgrade().is_some());
    drop(clone);
    assert!(weak.upgrade().is_none());
    Ok(())
}

#[test]
fn republished_layouts_share_fields_and_reuse_unchanged_runs() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    type Layout = RadixLayout<(i32, u32), (), u64, u64, ()>;
    let packing = Layout::derive_packing()?;
    let words = [
        packing.pack::<u64>(&[-3, 7])?,
        packing.pack::<u64>(&[9, 2])?,
    ];
    // SAFETY: uploads initialize sorted unique prefixes and no writer is kept.
    let empty = || unsafe {
        SortedBatch::from_sorted(
            Buffer::<u64>::from_host_async(pool.clone(), &[]).unwrap(),
            0,
        )
    };
    let batch = unsafe { SortedBatch::from_sorted(Buffer::from_host_async(pool.clone(), &words)?, 2)? };
    // Ratio two retains the 2-row run when the 1-row run is published.
    let params = RadixRunParams { ratio: 2, ..PARAMS };
    let mut state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), params, packing.bits())?;
    unsafe { state.append_sorted(batch)?; }
    let layout = Layout::publish(&state, packing.clone())?;
    // The unchanged state republishes the same retained descriptors.
    let same = layout.republish(&state)?;
    assert_eq!(same.view().keys.runs, layout.view().keys.runs);
    assert_eq!(same.view().fields, layout.view().fields);
    let next_words = [packing.pack::<u64>(&[12, 8])?];
    let next_batch = unsafe {
        SortedBatch::from_sorted(Buffer::from_host_async(pool.clone(), &next_words)?, 1)?
    };
    unsafe { state.append_sorted(next_batch)?; }
    let grown = layout.republish(&state)?;
    assert_ne!(grown.view().keys.runs, layout.view().keys.runs);
    assert_eq!(grown.view().fields, layout.view().fields);
    assert_eq!((grown.len(), grown.view().keys.run_count), (3, 2));
    assert_eq!(layout.to_rows()?, vec![vec![-3, 7], vec![9, 2]]);
    // Empty publications do not create runs or invalidate the cached snapshot.
    unsafe { state.append_sorted(empty()?)?; }
    let next = grown.republish(&state)?;
    assert_eq!(next.view().keys.runs, grown.view().keys.runs);
    let empty_state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), PARAMS, packing.bits())?;
    let empty_delta = Layout::publish(&empty_state, packing.clone())?;
    let next_empty = empty_delta.republish(&empty_state)?;
    assert_eq!(next_empty.view().keys.runs, empty_delta.view().keys.runs);
    assert_eq!(next_empty.len(), 0);
    assert!(Layout::publish(&state, Packing::new(&[32, 32])?).is_err());
    let other_state = DeviceRadixRun::<u64, u64, ()>::new_set(Pool::new(0, 0, 0)?, PARAMS, packing.bits())?;
    assert!(layout.republish(&other_state).is_err());
    assert_eq!(grown.to_rows()?, vec![vec![-3, 7], vec![9, 2], vec![12, 8]]);
    Ok(())
}

#[test]
fn resident_words_are_zeroed_once_and_grow() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let mut arena = CandidateArena::new(pool);
    let first = arena.resident_words(12)?;
    assert!(first.len() >= 12);
    assert!(first.to_host()?.iter().all(|&word| word == 0));
    assert_eq!(arena.resident_words(first.len())?.ptr(), first.ptr());
    let grown = arena.resident_words(first.len() + 1)?;
    assert!(grown.len() > first.len());
    assert!(grown.to_host()?.iter().all(|&word| word == 0));
    Ok(())
}

#[test]
fn candidate_batches_retain_inputs_and_prevent_arena_reuse() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_columns(&[Column::U32, Column::U32, Column::I32, Column::U32])?;
    let mut arena = CandidateArena::new(pool);
    let lease = arena.reserve(packing.clone(), 3)?;
    let first_ptr = lease.buffer().ptr();
    let values = [
        packing.pack::<u128>(&[4, 0, -9, 0])?,
        packing.pack::<u128>(&[u32::MAX as i128, u32::MAX as i128, 7, 0])?,
    ];
    let lanes = [
        values[0] as u64,
        (values[0] >> 64) as u64,
        values[1] as u64,
        (values[1] >> 64) as u64,
    ];
    // SAFETY: the unpublished lease is the sole writer; upload and consumers
    // use its pool stream. Finish publishes exactly the two initialized words.
    unsafe {
        lease.buffer().write_async(&lanes)?;
    }
    let input: Arc<dyn Any + Send + Sync> = Arc::new(17_u32);
    let weak_input = Arc::downgrade(&input);
    let batch = unsafe { lease.finish(2, vec![input])? };
    assert_eq!(batch.len(), 2);
    assert_eq!(batch.buffer().len(), 4);
    assert_eq!(batch.packing(), &packing);
    assert_eq!(batch.buffer().to_host()?, lanes);
    let next = arena.reserve(packing.clone(), 3)?;
    assert_ne!(next.buffer().ptr(), first_ptr);
    drop(next);
    let retained = batch.owner();
    drop(batch);
    assert!(weak_input.upgrade().is_some());
    let next = arena.reserve(packing.clone(), 3)?;
    assert_ne!(next.buffer().ptr(), first_ptr);
    drop(next);
    drop(retained);
    assert!(weak_input.upgrade().is_none());
    let reused = arena.reserve(packing.clone(), 3)?;
    assert_eq!(reused.buffer().ptr(), first_ptr);
    // SAFETY: an empty prefix is initialized without any writes.
    let empty = unsafe { reused.finish(0, vec![])? };
    assert!(empty.is_empty());
    assert_eq!(empty.buffer().len(), 0);
    drop(arena);
    assert_eq!(empty.packing(), &packing);
    Ok(())
}

#[test]
fn candidate_finish_rejects_uninitialized_capacity_suffix() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let mut arena = CandidateArena::new(pool);
    let lease = arena.reserve(Packing::new(&[32])?, 0)?;
    // SAFETY: finish rejects the invalid length before touching any storage.
    assert!(unsafe { lease.finish(1, vec![]) }.is_err());
    Ok(())
}

fn concatenate_case<K: ScalarTuple, W: Word + 'static>(packing: Packing) -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let mut arena = CandidateArena::new(pool.clone());
    let high = if W::BITS >= 128 { 1u128 << 100 } else { 0 };
    let groups: &[&[u128]] = &[&[high | 1, high | 7], &[], &[high | 7, high | 19]];
    let make_word = |value| {
        let mut word = W::from_u128(value);
        if W::BITS > 128 {
            word.set_lane(W::BITS / 64 - 1, 0xa5c7_1329_ef03_6481);
        }
        word
    };
    let mut batches = Vec::new();
    for group in groups {
        let lease = arena.reserve(packing.clone(), group.len())?;
        let mut lanes = Vec::new();
        for &word in *group {
            let word = make_word(word);
            for physical in 0..W::BITS / 64 {
                let lane = if W::MS_FIRST {
                    W::BITS / 64 - 1 - physical
                } else {
                    physical
                };
                lanes.push(word.lane(lane));
            }
        }
        // SAFETY: the sole lease writer initializes all live lanes before
        // publishing exactly this prefix on the same pool stream.
        unsafe {
            lease.buffer().write_async(&lanes)?;
            batches.push(lease.finish(group.len(), vec![])?);
        }
    }
    let output = concatenate_batches::<W>(pool.clone(), &packing, &batches)?;
    assert_eq!(
        output.to_host()?,
        [high | 1, high | 7, high | 7, high | 19].map(make_word)
    );
    assert_eq!(
        concatenate_batches::<W>(pool.clone(), &packing, &[])?.len(),
        0
    );
    let other = Pool::new(0, 0, 0)?;
    assert!(concatenate_batches::<W>(other, &packing, &batches).is_err());
    let different = Packing::new(&[8])?;
    assert!(concatenate_batches::<W>(pool.clone(), &different, &batches).is_err());
    if W::BITS == 64 {
        assert!(concatenate_batches::<u128>(pool.clone(), &packing, &batches).is_err());
    } else {
        assert!(concatenate_batches::<u64>(pool.clone(), &packing, &batches).is_err());
    }
    let mut accepted = DeviceRadixRun::<W, u64, ()>::new_set(pool, PARAMS, packing.bits())?;
    let fresh = unsafe { accepted.fresh(&output, output.len()) }?;
    // SAFETY: fresh is sorted, unique and absent from the empty accepted state.
    unsafe { accepted.append_sorted(fresh)? };
    let snapshot = RadixLayout::<K, (), W, u64, ()>::publish(&accepted, packing)?;
    drop(accepted);
    let words = snapshot.to_host()?;
    assert_eq!(words, [high | 1, high | 7, high | 19].map(make_word));
    Ok(())
}

#[test]
fn concatenate_u64_candidates_and_retain_accepted_snapshot() -> Result<()> {
    concatenate_case::<(u32, u32), u64>(Packing::new(&[32, 32])?)
}

#[test]
fn concatenate_u128_candidates_and_retain_accepted_snapshot() -> Result<()> {
    concatenate_case::<(u32, u32, u32, u32), u128>(Packing::new(&[32; 4])?)
}

#[test]
fn concatenate_wide_candidates_and_retain_accepted_snapshot() -> Result<()> {
    concatenate_case::<(u32, u32, u32, u32, u32, u32), [u64; 3]>(Packing::new(&[32; 6])?)?;
    concatenate_case::<(u32, u32, u32, u32, u32, u32, u32, u32), [u64; 4]>(Packing::new(&[32; 8])?)
}

#[cuda_host::cuda_module]
mod decode_kernel {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    use gpu_datalog_runtime::{radix_run::device, RadixLayoutView, Word};

    // DECODE P: diagnostic field inversion and encoded-prefix lookup oracle.
    #[cuda_device::device]
    unsafe fn decode<W: Word>(
        view: RadixLayoutView<W, u64>,
        rows: u32,
        output: *mut i64,
        ranges: *mut u64,
    ) {
        let row = thread::index_1d().get() as u32;
        if row >= rows {
            return;
        }
        unsafe {
            let (run, position) = device::locate_key(view.keys, row as u64);
            for column in 0..view.field_count {
                let value = device::decode_key(view, run, position, column);
                *output.add((row * view.field_count + column) as usize) = value;
            }
            // DECODE I: diagnostic prefix probe decodes then re-encodes a key field; direct encoded bounds would avoid this reorganization.
            let prefix = [device::decode_key(view, run, position, 0)];
            let (low, high, valid) = device::encode_prefix(view, prefix.as_ptr(), 1);
            let (begin, end) = if valid {
                device::interval(device::run(view.keys, run), low, high)
            } else {
                (0, 0)
            };
            *ranges.add(row as usize * 2) = begin;
            *ranges.add(row as usize * 2 + 1) = end;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn decode_datalog_test_layout(
        view: RadixLayoutView<u64, u64>, rows: u32, output: *mut i64, ranges: *mut u64,
    ) {
        unsafe { decode(view, rows, output, ranges); }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn decode_datalog_test_wide_layout(
        view: RadixLayoutView<[u64; 4], u64>, rows: u32, output: *mut i64, ranges: *mut u64,
    ) {
        unsafe { decode(view, rows, output, ranges); }
    }
}

#[test]
fn runtime_widths_decode_and_encode_signed_prefixes_on_device() -> Result<()> {
    type Layout = RadixLayout<(i32, u32), (), u64, u64, ()>;
    let pool = Pool::new(0, 0, 0)?;
    let module = unsafe { decode_kernel::load(pool.context())? };
    for bits in [1, 7, 19, 31, 32] {
        let packing = Layout::derive_packing_with_fields(&[(bits, true), (bits, false)])?;
        let minimum = -(1_i128 << (bits - 1));
        let maximum = (1_i128 << (bits - 1)) - 1;
        let unsigned_maximum = (1_i128 << bits) - 1;
        let rows = [
            [minimum, 0],
            [minimum, unsigned_maximum],
            [maximum, 0],
            [maximum, unsigned_maximum],
        ];
        let words = rows.iter().map(|row| packing.pack::<u64>(row))
            .collect::<Result<Vec<_>>>()?;
        // SAFETY: signed bias makes these four initialized words sorted and unique.
        let batch = unsafe {
            SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &words)?, rows.len())?
        };
        let layout = Layout::from_sorted(pool.clone(), packing, PARAMS, batch, bits * 2)?;
        let output = Buffer::<i64>::zeroed(pool.clone(), rows.len() * 2)?;
        let ranges = Buffer::<u64>::zeroed(pool.clone(), rows.len() * 2)?;
        // DECODE P: diagnostic reconstruction and re-encoding check runtime field widths.
        module.decode_datalog_test_layout(
            pool.stream(),
            &module.prepare_decode_datalog_test_layout(cuda_core::LaunchConfig1D::new(1, 256, 0))?,
            layout.view(),
            rows.len() as u32,
            output.ptr(),
            ranges.ptr(),
        )?;
        assert_eq!(
            output.to_host()?,
            rows.into_iter().flatten().map(|value| value as i64).collect::<Vec<_>>(),
        );
        assert_eq!(ranges.to_host()?, vec![0, 2, 0, 2, 2, 4, 2, 4]);
    }
    Ok(())
}

#[test]
fn wide_layout_erased_device_decode_and_lookup() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    type Layout = RadixLayout<(u8, i32, u32, u32, u32, u32, u32), (), [u64; 4], u64, ()>;
    let packing = Layout::derive_packing()?;
    let rows = [
        [0, i32::MIN as i128, u32::MAX as i128, 0, 7, 1, 2],
        [0, -1, 0, u32::MAX as i128, 8, 3, 4],
        [255, i32::MAX as i128, 9, 0, 9, 5, 6],
    ];
    let words = rows
        .iter()
        .map(|row| packing.pack::<[u64; 4]>(row))
        .collect::<Result<Vec<_>>>()?;
    // SAFETY: these uploads are sorted and unique and no writers are retained.
    let batch = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &words)?, words.len())? };
    let layout = Layout::from_sorted(pool.clone(), packing, PARAMS, batch, 200)?;
    assert_eq!(layout.view().bits, 200);
    let output = Buffer::<i64>::zeroed(pool.clone(), rows.len() * 7)?;
    let ranges = Buffer::<u64>::zeroed(pool.clone(), rows.len() * 2)?;
    let module = unsafe { decode_kernel::load(pool.context())? };
    // DECODE P: diagnostic wide-layout launch reconstructs values for host exactness checking.
    module.decode_datalog_test_wide_layout(
        pool.stream(),
        &module.prepare_decode_datalog_test_wide_layout(cuda_core::LaunchConfig1D::new(1, 256, 0))?,
        layout.view(),
        rows.len() as u32,
        output.ptr(),
        ranges.ptr(),
    )?;
    assert_eq!(
        output.to_host()?,
        rows.into_iter()
            .flatten()
            .map(|value| value as i64)
            .collect::<Vec<_>>()
    );
    assert_eq!(ranges.to_host()?, vec![0, 2, 0, 2, 2, 3]);
    Ok(())
}

#[test]
fn permuted_provider_preserves_rows_and_searches_new_leading_key() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    type Layout = RadixLayout<(i32, u32), (), u64, u64, ()>;
    let params = RadixRunParams { ratio: 2, ..PARAMS };
    let packing = Layout::derive_packing()?;
    let rows = [[-3, 7], [9, 2], [10, 7]];
    let words = rows
        .iter()
        .map(|row| packing.pack::<u64>(row))
        .collect::<Result<Vec<_>>>()?;
    // SAFETY: each initialized batch is sorted, unique and disjoint.
    let first = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &words[..2])?, 2)? };
    let second = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &words[2..])?, 1)? };
    let empty = unsafe { SortedBatch::from_sorted(Buffer::<u64>::from_host(pool.clone(), &[])?, 0)? };
    let mut state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), params, packing.bits())?;
    unsafe {
        state.append_sorted(first)?;
        state.append_sorted(empty)?;
        state.append_sorted(second)?;
    }
    let layout = Layout::publish(&state, packing)?;
    assert_eq!(layout.view().keys.run_count, 2);
    let before = layout.to_rows()?;
    assert_eq!(before, vec![vec![-3, 7], vec![9, 2], vec![10, 7]]);
    let provider = layout.permuted_provider::<(u32, i32)>(&pool, &[1, 0], params, 256, 64)?;
    assert!(layout
        .permuted_provider::<(u32, i32)>(&pool, &[1, 0], params, 128, 64)
        .is_err());
    assert!(layout
        .permuted_provider::<(i32, u32)>(&pool, &[1, 0], params, 256, 64)
        .is_err());
    // DECODE P: diagnostic retained layout unpack exposes the unchanged post-permutation answer.
    assert_eq!(
        layout.to_rows()?,
        before,
        "permutation leaves source storage unchanged"
    );
    assert!(layout
        .permuted_provider::<(u32, i32)>(&pool, &[0, 0], params, 256, 64)
        .is_err());
    assert!(layout
        .permuted_provider::<(u32, i32)>(&Pool::new(0, 0, 0)?, &[1, 0], params, 256, 64)
        .is_err());
    assert_eq!(provider.view().keys.run_count, 2);
    let output = Buffer::<i64>::zeroed(pool.clone(), 6)?;
    let ranges = Buffer::<u64>::zeroed(pool.clone(), 6)?;
    let view = provider.view();
    let owner = provider.owner();
    drop(provider);
    drop(layout);
    drop(state);
    let module = unsafe { decode_kernel::load(pool.context())? };
    // DECODE P: diagnostic permuted-layout launch reconstructs values for host equality checking.
    module.decode_datalog_test_layout(
        pool.stream(),
        &module.prepare_decode_datalog_test_layout(cuda_core::LaunchConfig1D::new(1, 256, 0))?,
        view,
        3,
        output.ptr(),
        ranges.ptr(),
    )?;
    assert_eq!(
        output.to_host()?,
        vec![2, 9, 7, -3, 7, 10]
    );
    // Prefix intervals are local to each retained run, not one flat allocation.
    assert_eq!(ranges.to_host()?, vec![0, 1, 1, 2, 0, 1]);
    drop(owner);
    Ok(())
}

#[test]
fn typed_set_layouts_preserve_recorded_narrow_and_full_word_windows() -> Result<()> {
    type Layout = RadixLayout<(u32, u32, u32), (), [u64; 3], u64, ()>;
    let pool = Pool::new(0, 0, 0)?;
    let packing = Layout::derive_packing()?;
    assert_eq!(packing.bits(), 96);
    let rows = [[1, u32::MAX as i128, u32::MAX as i128], [2, 0, 0], [u32::MAX as i128, 0, 1]];
    let words = rows.iter().map(|row| packing.pack::<[u64; 3]>(row))
        .collect::<Result<Vec<_>>>()?;
    let batch = unsafe {
        SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &words)?, words.len())?
    };
    for active_bits in [96, 192] {
        let empty = Layout::empty(pool.clone(), packing.clone(), PARAMS, active_bits)?;
        assert_eq!(empty.snapshot().active_bits(), active_bits);
        let layout = Layout::from_sorted(pool.clone(), packing.clone(), PARAMS, batch.clone(), active_bits)?;
        assert_eq!(layout.snapshot().active_bits(), active_bits);
        let successor = empty.with_sorted(batch.clone(), PARAMS)?;
        assert_eq!(successor.snapshot().active_bits(), active_bits);
        assert_eq!(successor.to_host()?, words);
        let unchanged = successor.with_sorted(batch.clone(), PARAMS)?;
        assert_eq!(unchanged.snapshot_id(), successor.snapshot_id());
        assert_eq!(unchanged.snapshot().active_bits(), active_bits);
        gpu_datalog_runtime::reset_work_counters();
        let before = gpu_datalog_runtime::readback_count();
        let permuted = layout.permuted_provider::<(u32, u32, u32)>(&pool, &[1, 2, 0], PARAMS, 256, active_bits)?;
        assert_eq!(gpu_datalog_runtime::readback_count(), before);
        #[cfg(feature = "radix-counters")]
        {
            let work = gpu_datalog_runtime::work_counters();
            assert_eq!(work.layout.sorts, 1);
            assert_eq!(work.layout.radix_bits, active_bits as u64);
            assert_eq!(work.layout.passes, active_bits.div_ceil(8) as u64);
            assert_eq!(work.offset_materializations, 0);
        }
        assert_eq!(permuted.snapshot().active_bits(), active_bits);
        assert_eq!(permuted.to_rows()?, vec![
            vec![0, 0, 2], vec![0, 1, u32::MAX as i128], vec![u32::MAX as i128, u32::MAX as i128, 1],
        ]);
        // A materialization's recorded choice is independent of its source.
        let alternate_bits = if active_bits == 96 { 192 } else { 96 };
        let alternate = layout.permuted_provider::<(u32, u32, u32)>(&pool, &[1, 2, 0], PARAMS, 256, alternate_bits)?;
        assert_eq!(alternate.snapshot().active_bits(), alternate_bits);
        assert_eq!(alternate.to_host()?, permuted.to_host()?);
        assert_eq!(layout.to_host()?, words);
        assert_eq!(successor.to_rows()?, rows.map(Vec::from));
    }
    assert!(Layout::empty(pool.clone(), packing.clone(), PARAMS, 95).is_err());
    assert!(Layout::from_sorted(pool, packing, PARAMS, batch, 193).is_err());
    Ok(())
}
