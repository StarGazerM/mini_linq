use gpu_datalog_runtime::{Buffer, DeviceRadixRun, DomainCatalog, DomainInput, Packing, Pool, RadixRunParams};

type Result<T = ()> = gpu_datalog_runtime::Result<T>;
const PARAMS: RadixRunParams = RadixRunParams { ratio: 4, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 };

#[test]
fn domain_validation_shares_admission_count_and_survives_permutation() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let domains = DomainCatalog::new(pool.clone(), vec![
        DomainInput::Dictionary(vec![10, 20, 30]),
        DomainInput::Signed(Some((1, 2))),
    ])?;
    let source = Packing::with_fields(&[(32, false), (32, true)])?;
    let target = domains.packing(&[1, 0])?;
    let state = DeviceRadixRun::<u64, u64, ()>::new_set(pool.clone(), PARAMS, target.bits())?;
    let words = [[2, 2], [0, 1], [2, 2]].iter().map(|row| source.pack::<u64>(row)).collect::<Result<Vec<_>>>()?;
    let input = Buffer::from_host(pool.clone(), &words)?;
    let before = gpu_datalog_runtime::readback_count();
    let pending = domains.repack_codes(&input, &source, &[0, 1])?.permuted(&[1, 0], 256)?;
    assert_eq!(gpu_datalog_runtime::readback_count(), before);
    // SAFETY: the pending batch is ordered on this pool and has the state's heading.
    let fresh = unsafe { state.fresh_codes(pending)? };
    assert_eq!(gpu_datalog_runtime::readback_count() - before, 1);
    // DECODE P: diagnostic domain admission publishes its permuted field codes.
    let actual = fresh.buffer().to_host()?.into_iter().map(|word| target.unpack(word)).collect::<Result<Vec<_>>>()?;
    assert_eq!(actual, [vec![1, 0], vec![2, 2]]);

    for row in [[3, 2], [0, 3]] {
        let input = Buffer::from_host(pool.clone(), &[source.pack::<u64>(&row)?])?;
        let before = gpu_datalog_runtime::readback_count();
        let pending = domains.repack_codes(&input, &source, &[0, 1])?.permuted(&[1, 0], 256)?;
        assert_eq!(gpu_datalog_runtime::readback_count(), before);
        // SAFETY: range errors are carried by the batch, not silently discarded.
        let error = match unsafe { state.fresh_codes(pending) } {
            Ok(_) => panic!("invalid candidate escaped validation"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("outside its recorded equality component"));
        assert_eq!(gpu_datalog_runtime::readback_count() - before, 1);
        assert!(state.is_empty());
    }
    let input = Buffer::<u64>::from_host(pool, &[])?;
    let before = gpu_datalog_runtime::readback_count();
    let pending = domains.repack_codes(&input, &source, &[0, 1])?;
    // SAFETY: the empty batch retains the same packing and stream.
    assert!(unsafe { state.fresh_codes(pending)? }.is_empty());
    assert_eq!(gpu_datalog_runtime::readback_count(), before);
    Ok(())
}

#[test]
fn zero_field_domain_batch_needs_no_validation_barrier() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let domains = DomainCatalog::new(pool.clone(), Vec::new())?;
    let packing = Packing::new(&[])?;
    let input = Buffer::from_host(pool.clone(), &[0u64, 0, 0])?;
    let state = DeviceRadixRun::<u64, u64, ()>::new_set(pool, PARAMS, 0)?;
    let before = gpu_datalog_runtime::readback_count();
    let pending = domains.repack_codes(&input, &packing, &[])?;
    // SAFETY: there is one possible zero-field tuple, with zero padding.
    assert_eq!(unsafe { state.sort_codes(pending)? }.len(), 1);
    assert_eq!(gpu_datalog_runtime::readback_count(), before);
    Ok(())
}
