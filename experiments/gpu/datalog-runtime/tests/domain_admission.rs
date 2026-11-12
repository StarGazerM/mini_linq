use gpu_datalog_runtime::{
    concatenate_batches, Buffer, CandidateArena, DeviceRadixRun, DomainCatalog, DomainInput,
    Pool, RadixRunParams, Word,
};
use std::sync::Arc;

type Result<T = ()> = gpu_datalog_runtime::Result<T>;
const PARAMS: RadixRunParams = RadixRunParams { ratio: 4, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 };

fn direct_admission<W: Word>() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let domains = DomainCatalog::new(pool.clone(), vec![
        DomainInput::Dictionary(vec![10, 20, 30]),
        DomainInput::Signed(Some((-2, 2))),
    ])?;
    let layout = domains.arena_layout(&[0, 1], W::BITS)?;
    let packing = layout.packing().clone();
    assert_eq!(packing.bits(), 5);
    assert_eq!(packing.word_bits(), W::BITS);
    let mut arena = CandidateArena::with_layout(pool.clone(), layout);
    let metadata = arena.output_layout()?;
    assert!(Arc::ptr_eq(&metadata, &arena.output_layout()?));
    assert_eq!(metadata.bounds(), [Some((0, 2)), Some((-2, 2))]);
    let rows = [[2, 2], [0, -2], [2, 2]];
    let words = rows.iter().map(|row| packing.pack::<W>(row)).collect::<Result<Vec<_>>>()?;
    let mut lanes = Vec::new();
    for word in words {
        for physical in 0..W::BITS / 64 {
            let lane = if W::MS_FIRST { W::BITS / 64 - 1 - physical } else { physical };
            lanes.push(word.lane(lane));
        }
    }
    let lease = arena.reserve(packing.clone(), rows.len())?;
    // SAFETY: the sole writer initializes every fixed-carrier lane in stream order.
    unsafe { lease.buffer().write_async(&lanes)?; }
    let before = gpu_datalog_runtime::readback_count();
    let batch = unsafe { lease.finish(rows.len(), vec![])? };
    assert_eq!(batch.live_bytes(), rows.len() * W::BITS as usize / 8);
    let input = concatenate_batches::<W>(pool.clone(), &packing, &[batch])?;
    let input = packing.permute_device(&input, rows.len(), &[1, 0], 256)?;
    let target = packing.permute(&[1, 0])?;
    assert_eq!(target.word_bits(), W::BITS);
    assert_eq!(gpu_datalog_runtime::readback_count(), before);
    let state = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), PARAMS, target.bits())?;
    // SAFETY: initialized measured codes have zero padding and the state's heading.
    let fresh = unsafe { state.fresh(&input, input.len())? };
    assert_eq!(gpu_datalog_runtime::readback_count() - before, 1);
    // DECODE P: diagnostic admission publishes the permuted measured fields.
    let actual = fresh.buffer().to_host()?.into_iter().map(|word| target.unpack(word)).collect::<Result<Vec<_>>>()?;
    assert_eq!(actual, [vec![-2, 0], vec![2, 2]]);
    let empty = Buffer::<W>::from_host(pool, &[])?;
    let before = gpu_datalog_runtime::readback_count();
    // SAFETY: an empty initialized prefix vacuously satisfies the state layout.
    assert!(unsafe { state.fresh(&empty, 0)? }.is_empty());
    assert_eq!(gpu_datalog_runtime::readback_count(), before);
    Ok(())
}

#[test]
fn measured_candidates_admit_directly_with_fixed_carriers() -> Result {
    direct_admission::<u64>()?;
    direct_admission::<u128>()?;
    direct_admission::<[u64; 3]>()
}

#[test]
fn catalog_layout_retains_empty_and_signed_domain_bounds() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let domains = DomainCatalog::new(pool.clone(), vec![
        DomainInput::Dictionary(vec![]),
        DomainInput::Signed(None),
        DomainInput::Dictionary(vec![u32::MAX]),
        DomainInput::Signed(Some((i32::MIN as i64, i32::MAX as i64))),
    ])?;
    let before = gpu_datalog_runtime::readback_count();
    let layout = domains.arena_layout(&[0, 1, 2, 3], 128)?;
    assert_eq!(layout.widths().collect::<Vec<_>>(), [1, 1, 1, 32]);
    assert_eq!(layout.width(3), 32);
    assert_eq!(layout.bounds(), [None, None, Some((0, 0)), Some((i32::MIN as i64, i32::MAX as i64))]);
    assert_eq!(layout.carrier_bits(), 128);
    assert!(domains.arena_layout(&[3, 3, 3], 64).is_err());
    assert!(domains.arena_layout(&[4], 64).is_err());
    assert!(domains.arena_layout(&[0], 65).is_err());
    assert_eq!(gpu_datalog_runtime::readback_count(), before);
    let mut arena = CandidateArena::with_layout(pool.clone(), layout);
    let metadata = arena.output_layout()?;
    assert!(arena.reserve(domains.packing(&[0, 1, 2, 3])?, 0).is_err());
    assert!(arena.reserve(domains.packing(&[3, 2, 1, 0])?.with_carrier_bits(128)?, 0).is_err());
    let lease = arena.reserve(metadata.packing().clone(), 4)?;
    assert_eq!(lease.allocated_bytes(), 4 * 16);
    drop(lease);
    arena.clear();
    assert!(Arc::ptr_eq(&metadata, &arena.output_layout()?));
    assert!(CandidateArena::new(pool).output_layout().is_err());
    Ok(())
}

#[test]
fn zero_field_measured_batch_needs_no_admission_barrier() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let domains = DomainCatalog::new(pool.clone(), Vec::new())?;
    let layout = domains.arena_layout(&[], 128)?;
    assert_eq!(layout.packing().bits(), 0);
    assert_eq!(layout.carrier_bits(), 128);
    let input = Buffer::from_host(pool.clone(), &[0u128, 0, 0])?;
    let state = DeviceRadixRun::<u128, u64, ()>::new_set(pool, PARAMS, 0)?;
    let before = gpu_datalog_runtime::readback_count();
    // SAFETY: there is one possible zero-field tuple, with zero carrier padding.
    assert_eq!(unsafe { state.sort_unique(&input, input.len())? }.len(), 1);
    assert_eq!(gpu_datalog_runtime::readback_count(), before);
    Ok(())
}
