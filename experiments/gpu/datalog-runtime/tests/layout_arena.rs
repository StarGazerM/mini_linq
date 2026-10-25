use gpu_datalog_runtime::{concatenate_batches, Buffer, CandidateArena, Column, DeviceLayout, DeviceRuns, Packing, Pool, Result, Run, Word};
use std::{any::Any, sync::Arc};

#[test]
fn layouts_retain_runs_and_validate_native_schema() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    type Layout = DeviceLayout<(i32,), (u32,)>;
    let packing = Layout::derive_packing()?;
    let words = [packing.pack::<u64>(&[-3, 7])?, packing.pack::<u64>(&[9, 2])?];
    // SAFETY: uploads initialize sorted unique prefixes and no writer is kept.
    let empty = unsafe { Run::from_sorted(Buffer::<u64>::from_host_async(pool.clone(), &[])?, 0)? };
    let run = unsafe { Run::from_sorted(Buffer::from_host_async(pool.clone(), &words)?, 2)? };
    let layout = Layout::from_runs(pool.clone(), packing.clone(), vec![empty.clone(), run.clone(), empty])?;
    assert_eq!(layout.len(), 2);
    assert_eq!(layout.view().run_count, 3);
    assert_eq!(layout.view().field_count, 2);
    assert_eq!(layout.view().word_bits, 64);
    assert!(Arc::ptr_eq(layout.pool(), &pool));
    layout.validate(&[Column::I32, Column::U32])?;
    assert!(layout.validate(&[Column::U32, Column::I32]).is_err());
    assert!(Layout::from_runs(pool.clone(), Packing::new(&[32, 32])?, vec![run.clone()]).is_err());
    let other = Pool::new(0, 0, 0)?;
    assert!(Layout::from_runs(other, packing, vec![run]).is_err());
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
fn candidate_batches_retain_inputs_and_prevent_arena_reuse() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_columns(&[Column::U64, Column::I64])?;
    let mut arena = CandidateArena::new(pool);
    let lease = arena.reserve(packing.clone(), 3)?;
    let first_ptr = lease.buffer().ptr();
    let values = [packing.pack::<u128>(&[4, -9])?, packing.pack::<u128>(&[u64::MAX as i128, 7])?];
    let lanes = [values[0] as u64, (values[0] >> 64) as u64,
        values[1] as u64, (values[1] >> 64) as u64];
    // SAFETY: the unpublished lease is the sole writer; upload and consumers
    // use its pool stream. Finish publishes exactly the two initialized words.
    unsafe { lease.buffer().write_async(&lanes)?; }
    let input: Arc<dyn Any> = Arc::new(17_u32);
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

fn concatenate_case<W: Word>(packing: Packing) -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let mut arena = CandidateArena::new(pool.clone());
    let high = if W::BITS == 128 { 1u128 << 100 } else { 0 };
    let groups: &[&[u128]] = &[&[high | 1, high | 7], &[], &[high | 7, high | 19]];
    let mut batches = Vec::new();
    for group in groups {
        let lease = arena.reserve(packing.clone(), group.len())?;
        let mut lanes = Vec::new();
        for &word in *group {
            lanes.push(word as u64);
            if W::BITS == 128 { lanes.push((word >> 64) as u64); }
        }
        // SAFETY: the sole lease writer initializes all live lanes before
        // publishing exactly this prefix on the same pool stream.
        unsafe {
            lease.buffer().write_async(&lanes)?;
            batches.push(lease.finish(group.len(), vec![])?);
        }
    }
    let output = concatenate_batches::<W>(pool.clone(), &packing, &batches)?;
    assert_eq!(output.to_host()?.iter().map(|word| word.to_u128()).collect::<Vec<_>>(),
        vec![high | 1, high | 7, high | 7, high | 19]);
    assert_eq!(concatenate_batches::<W>(pool.clone(), &packing, &[])?.len(), 0);
    let other = Pool::new(0, 0, 0)?;
    assert!(concatenate_batches::<W>(other, &packing, &batches).is_err());
    let different = Packing::new(&[8])?;
    assert!(concatenate_batches::<W>(pool.clone(), &different, &batches).is_err());
    if W::BITS == 64 {
        assert!(concatenate_batches::<u128>(pool.clone(), &packing, &batches).is_err());
    } else {
        assert!(concatenate_batches::<u64>(pool.clone(), &packing, &batches).is_err());
    }
    let mut accepted = DeviceRuns::<W>::new(pool)?;
    accepted.admit(&output, output.len())?;
    let snapshot = accepted.owned_runs();
    drop(accepted);
    let words = snapshot.iter().map(|run| run.buffer().to_host())
        .collect::<Result<Vec<_>>>()?.into_iter().flatten()
        .map(|word| word.to_u128()).collect::<Vec<_>>();
    assert_eq!(words, vec![high | 1, high | 7, high | 19]);
    Ok(())
}

#[test]
fn concatenate_u64_candidates_and_retain_accepted_snapshot() -> Result<()> {
    concatenate_case::<u64>(Packing::new(&[32, 32])?)
}

#[test]
fn concatenate_u128_candidates_and_retain_accepted_snapshot() -> Result<()> {
    concatenate_case::<u128>(Packing::new(&[64, 64])?)
}
