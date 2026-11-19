use gpu_datalog_runtime::{packed_run::PackedRun, Buffer, Packing, Pool, Result, Word};

fn codec<W: Word>() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
    for block in [1, 17, 1024] {
        for n in [0, 1, block - 1, block, block + 1, block * 3 + 7] {
            for full in [false, true] {
                let mut values = Vec::new();
                for i in 0..n {
                    let mut word = W::zero();
                    for lane in 0..W::BITS / 64 {
                        word.set_lane(lane, if full { (i as u64).wrapping_mul(0x9e3779b97f4a7c15).rotate_left(lane * 7) } else if lane == 0 { i as u64 } else { 42 });
                    }
                    values.push(word);
                }
                values.sort_unstable(); values.dedup();
                let input = Buffer::from_host(pool.clone(), &values)?;
                let run = unsafe { PackedRun::seal(&input, values.len(), &packing, block as u32)? };
                assert_eq!(run.unpack()?.to_host()?, values, "block={block} n={n} full={full}");
                assert_eq!(run.allocated_bytes(), run.payload_bytes() + run.header_bytes() + run.fence_bytes());
            }
        }
    }
    Ok(())
}
#[test] fn codec_u64() -> Result<()> { codec::<u64>() }
#[test] fn codec_u128() -> Result<()> { codec::<u128>() }
#[test] fn codec_three_lanes() -> Result<()> { codec::<[u64; 3]>() }
#[test] fn codec_four_lanes() -> Result<()> { codec::<[u64; 4]>() }

#[test]
fn every_residual_width_and_geometry_roundtrips() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&[64, 64])?;
    for block in [256u32, 1024] {
        for width in 0..=64 {
            let mask = if width == 0 { 0 } else { u64::MAX >> (64 - width) };
            let base = u64::MAX - mask;
            let values: Vec<u128> = (0..block as usize * 2 + 33).map(|i| {
                let residual = match i % block as usize {
                    0 => 0, 1 => mask, _ => (i as u64).wrapping_mul(0x9e3779b97f4a7c15) & mask,
                };
                ((i as u128) << 64) | (base + residual) as u128
            }).collect();
            let input = Buffer::from_host(pool.clone(), &values)?;
            let run = unsafe { PackedRun::seal(&input, values.len(), &packing, block)? };
            assert_eq!(run.unpack()?.to_host()?, values, "width={width} block={block}");
            let probes: Vec<_> = values.iter().step_by(7).copied().collect();
            let device = Buffer::from_host(pool.clone(), &probes)?;
            assert_eq!(run.contains(&device, probes.len())?.to_host()?, vec![1; probes.len()]);
            let actual = run.ranges(&device, &device)?.to_host()?;
            let expected: Vec<_> = probes.iter().map(|key| {
                let position = values.binary_search(key).unwrap() as u64;
                [position, position + 1]
            }).collect();
            assert_eq!(actual, expected);
        }
    }
    Ok(())
}
#[test] fn crossed_fields_and_empty_schema() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&[3, 64, 17])?;
    let values: Vec<u128> = (0..2050).map(|i| (5u128 << 81) | ((i as u128) << 17) | 99).collect();
    let input = Buffer::from_host(pool.clone(), &values)?;
    let run = unsafe { PackedRun::seal(&input, values.len(), &packing, 1024)? };
    assert_eq!(run.unpack()?.to_host()?, values);
    assert!(run.allocated_bytes() < values.len() * 16);
    let input = Buffer::from_host(pool, &[0u64])?;
    let run = unsafe { PackedRun::seal(&input, 1, &Packing::new(&[])?, 1024)? };
    assert_eq!(run.unpack()?.to_host()?, vec![0]);
    Ok(())
}

fn reads<W: Word>() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&[16, 16])?;
    let values: Vec<W> = (1..19u64).flat_map(|lead| (0..37u64).map(move |tail| W::from_u128(((lead << 16) | (tail * 2)) as u128))).collect();
    let input = Buffer::from_host(pool.clone(), &values)?;
    for block in [1, 17, 1024] {
        let run = unsafe { PackedRun::seal(&input, values.len(), &packing, block)? };
        let low: Vec<W> = (0..21u64).map(|lead| W::from_u128((lead << 16) as u128)).collect();
        let high: Vec<W> = (0..21u64).map(|lead| W::from_u128(((lead << 16) | 65535) as u128)).collect();
        let ranges = run.ranges(&Buffer::from_host(pool.clone(), &low)?, &Buffer::from_host(pool.clone(), &high)?)?.to_host()?;
        for (i, range) in ranges.iter().enumerate() {
            assert_eq!(*range, [values.partition_point(|v| *v < low[i]) as u64, values.partition_point(|v| *v <= high[i]) as u64]);
        }
        let queries: Vec<W> = (0..21u64).flat_map(|lead| (0..80u64).map(move |tail| W::from_u128(((lead << 16) | tail) as u128))).collect();
        let candidates = Buffer::from_host(pool.clone(), &queries)?;
        let expected: Vec<u32> = queries.iter().map(|key| values.binary_search(key).is_ok() as u32).collect();
        assert_eq!(run.contains(&candidates, queries.len())?.to_host()?, expected);
        assert_eq!(run.absent(&candidates, queries.len())?.to_host()?, expected.iter().map(|v| 1 - v).collect::<Vec<_>>());
        let empty = unsafe { PackedRun::seal(&input, 0, &packing, block)? };
        assert_eq!(gpu_datalog_runtime::packed_run::membership(&[&empty, &run], &candidates, queries.len(), false)?.to_host()?, expected);
        assert_eq!(empty.contains(&candidates, queries.len())?.to_host()?, vec![0; queries.len()]);
    }
    Ok(())
}
#[test] fn reads_u64() -> Result<()> { reads::<u64>() }
#[test] fn reads_u128() -> Result<()> { reads::<u128>() }
#[test] fn reads_three_lanes() -> Result<()> { reads::<[u64; 3]>() }
#[test] fn reads_four_lanes() -> Result<()> { reads::<[u64; 4]>() }

fn merges<W: Word>() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
    for block in [1, 17, 1024] {
        for pattern in 0..5 {
            for size in [0, 1, 31, 1024, 2081] {
                let mut left = Vec::new();
                let mut right = Vec::new();
                for i in 0..size * 2 {
                    let mut word = W::zero();
                    for lane in 0..W::BITS / 64 {
                        word.set_lane(lane, if lane == W::BITS / 64 - 1 { i as u64 } else { (i as u64).wrapping_mul(0x9e3779b97f4a7c15).rotate_left(lane * 7) });
                    }
                    let first = match pattern {
                        0 => i < size,
                        1 => i >= size,
                        2 => i % 2 == 0,
                        3 => i >= size / 2 && i < size + size / 2,
                        _ => ((i as u64).wrapping_mul(0x9e3779b97f4a7c15) >> 61) < 3,
                    };
                    if first { left.push(word); } else { right.push(word); }
                }
                left.sort_unstable(); right.sort_unstable();
                let a = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &left)?, left.len(), &packing, block)? };
                let b = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &right)?, right.len(), &packing, block)? };
                let result = unsafe { a.merge(&b)? };
                left.extend(right); left.sort_unstable();
                assert_eq!(result.unpack()?.to_host()?, left, "block={block} pattern={pattern} size={size}");
            }
        }
    }
    Ok(())
}
#[test] fn merges_u64() -> Result<()> { merges::<u64>() }
#[test] fn merges_u128() -> Result<()> { merges::<u128>() }
#[test] fn merges_three_lanes() -> Result<()> { merges::<[u64; 3]>() }
#[test] fn merges_four_lanes() -> Result<()> { merges::<[u64; 4]>() }

fn admissions<W: Word>() -> Result<()> {
    use std::collections::BTreeSet;
    use gpu_datalog_runtime::packed_run::PackedSet;
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
    let mut state = PackedSet::new(pool.clone(), packing, 1024, 4)?;
    state.profile(true);
    let mut oracle = BTreeSet::new();
    let mut random = 0x1234567812345678u64;
    for admission in 0..48 {
        let n = [0, 1, 31, 257, 1024, 4097][admission % 6];
        let mut batch = BTreeSet::new();
        for _ in 0..n {
            random ^= random << 13; random ^= random >> 7; random ^= random << 17;
            let value = random % 16384;
            let mut word = W::zero();
            for lane in 0..W::BITS / 64 { word.set_lane(lane, value.rotate_left(lane * 7)); }
            batch.insert(word);
        }
        let candidates: Vec<_> = batch.iter().copied().collect();
        let expected: Vec<_> = batch.difference(&oracle).copied().collect();
        let fresh = unsafe { state.admit(gpu_datalog_runtime::SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &candidates)?, candidates.len())?)? };
        assert_eq!(fresh.buffer().to_host_prefix(fresh.len())?, expected);
        oracle.extend(batch);
        assert_eq!(state.to_host()?, oracle.iter().copied().collect::<Vec<_>>());
        assert!(state.runs().windows(2).all(|pair| pair[0].len() / 4 >= pair[1].len()));
    }
    Ok(())
}
#[test] fn admissions_u64() -> Result<()> { admissions::<u64>() }
#[test] fn admissions_u128() -> Result<()> { admissions::<u128>() }
#[test] fn admissions_three_lanes() -> Result<()> { admissions::<[u64; 3]>() }
#[test] fn admissions_four_lanes() -> Result<()> { admissions::<[u64; 4]>() }

#[test]
fn small_run_allocation_sweep() -> Result<()> {
    fn sweep<W: Word>() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
        for n in [1, 8, 32, 64, 128, 256, 1024] {
            let values: Vec<W> = (0..n).map(|i| W::from_u128(i as u128)).collect();
            let input = Buffer::from_host(pool.clone(), &values)?;
            let run = unsafe { PackedRun::seal(&input, n, &packing, 1024)? };
            println!("small-run bits={} rows={} plain={} packed={}", W::BITS, n, input.allocated_bytes(), run.allocated_bytes());
        }
        Ok(())
    }
    sweep::<u64>()?; sweep::<u128>()?; sweep::<[u64; 3]>()?; sweep::<[u64; 4]>()
}

#[test]
fn residuals_preserve_numeric_key_order() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let input = Buffer::from_host(pool.clone(), &[0x1fu64, 0x20, 0x21])?;
    let run = unsafe { PackedRun::seal(&input, 3, &Packing::new(&[4, 4])?, 256)? };
    let mut payload = 0u64;
    let mut header = gpu_datalog_runtime::packed_run::Header::default();
    unsafe {
        cuda_core::simt::memory::memcpy_dtoh_async(&mut payload as *mut u64, run.view().payload as u64, 8, pool.stream().cu_stream())?;
        cuda_core::simt::memory::memcpy_dtoh_async(&mut header as *mut _ as *mut u8, run.view().headers as u64, std::mem::size_of_val(&header), pool.stream().cu_stream())?;
    }
    pool.stream().synchronize()?;
    if header.encoding == 1 {
        assert_eq!([payload & 31, (payload >> 5) & 31, (payload >> 10) & 31], [15, 16, 17]);
    } else if header.encoding == 0 {
        // Lane-tail padding can make this three-row block prefer plain storage.
        assert_eq!(payload, 0x1f);
    }
    assert_eq!(run.unpack()?.to_host()?, [0x1f, 0x20, 0x21]);
    assert_eq!(run.ranges(&input, &input)?.to_host()?, [[0, 1], [1, 2], [2, 3]]);
    Ok(())
}

#[test]
fn tiny_plain_cutover() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let mut state = gpu_datalog_runtime::packed_run::PackedSet::new(pool.clone(), Packing::new(&[32])?, 256, 4)?;
        let cutoff = gpu_datalog_runtime::packed_run::SMALL_RUN_ROWS;
        let mut previous = 0;
        for end in (1..40).chain([cutoff - 1, cutoff, cutoff + 1, cutoff + 2]) {
            let values: Vec<W> = (0..end).map(|i| W::from_u128(i as u128)).collect();
            let fresh = unsafe { state.admit(gpu_datalog_runtime::SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &values)?, values.len())?)? };
            assert_eq!(fresh.len(), end - previous);
            previous = end;
            assert_eq!(state.to_host()?, values);
            for run in state.runs() {
                if run.len() <= gpu_datalog_runtime::packed_run::SMALL_RUN_ROWS {
                    assert_eq!(run.allocated_bytes(), run.len() * std::mem::size_of::<W>());
                }
            }
        }
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn unsealed_batch_merges_directly() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        for block in [17, 256, 1024] {
            let mut state = gpu_datalog_runtime::packed_run::PackedSet::new(pool.clone(), Packing::new(&[32])?, block, 4)?;
            let count = gpu_datalog_runtime::packed_run::SMALL_RUN_ROWS + 7;
            for parity in 0..2 {
                let values: Vec<W> = (0..count).map(|i| W::from_u128((i * 2 + parity) as u128)).collect();
                let mut backing = values.clone();
                if parity != 0 { backing.resize(count * 2, W::zero()); }
                let batch = unsafe { gpu_datalog_runtime::SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &backing)?, values.len())? };
                unsafe { state.append_sorted(&batch)?; }
                assert_eq!(batch.buffer().to_host_prefix(batch.len())?, values);
            }
            assert_eq!(state.runs().len(), 1);
            assert!(state.runs()[0].view().plain.is_null());
            assert_eq!(state.to_host()?, (0..count * 2).map(|i| W::from_u128(i as u128)).collect::<Vec<_>>());
        }
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn residual_bounds_translate_outside_frames() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        for (widths, values) in [
            (vec![4, 4], (3..7).flat_map(|x| (5..9).map(move |y| W::from_u128((x << 4) | y))).collect::<Vec<_>>()),
            (vec![4, 4], vec![W::from_u128(0x35)]),
            (vec![], vec![W::zero()]),
            (vec![64], vec![W::zero(), W::from_u128(1u128 << 63), W::from_u128(u64::MAX as u128)]),
        ] {
            let input = Buffer::from_host(pool.clone(), &values)?;
            for block in [1, 7, 256] {
                let run = unsafe { PackedRun::seal(&input, values.len(), &Packing::new(&widths)?, block)? };
                let mut probes: Vec<W> = (0..512).map(W::from_u128).collect();
                probes.extend([W::from_u128((1u128 << 63) - 1), W::from_u128(1u128 << 63), W::from_u128(u64::MAX as u128)]);
                let queries = Buffer::from_host(pool.clone(), &probes)?;
                let ranges = run.ranges(&queries, &queries)?.to_host()?;
                for (key, range) in probes.iter().zip(ranges) {
                    assert_eq!(range, [values.partition_point(|v| v < key) as u64, values.partition_point(|v| v <= key) as u64]);
                }
            }
        }
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn fused_small_unsorted_duplicates_and_control() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        use gpu_datalog_runtime::{packed_run::PackedSet, SortedBatch};
        use std::collections::BTreeSet;
        let pool = Pool::new(0, 0, 0)?;
        for plain in [false, true] {
            let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
            let mut set = if plain { PackedSet::new_plain(pool.clone(), packing, 256, 4)? }
                else { PackedSet::new(pool.clone(), packing, 256, 4)? };
            let mut oracle = BTreeSet::new();
            for rows in [0, 1, 2, 7, 32, 256, 1024] {
                let mut input: Vec<W> = (0..rows).map(|i| W::from_u128(((i * 17) % 313) as u128)).collect();
                input.reverse();
                if rows > 1 { input[0] = W::from_u128(u64::MAX as u128); }
                let expected: Vec<_> = input.iter().copied().collect::<BTreeSet<_>>().difference(&oracle).copied().collect();
                let buffer = Buffer::from_host(pool.clone(), &input)?;
                let fresh = unsafe { set.fresh_small(&buffer, rows)? };
                assert_eq!(fresh.buffer().to_host_prefix(fresh.len())?, expected);
                unsafe { set.append_sorted(&fresh)?; }
                oracle.extend(input);
                assert_eq!(set.to_host()?, oracle.iter().copied().collect::<Vec<_>>());
            }
            let initial: Vec<W> = (1000..41000).map(|v| W::from_u128(v)).collect();
            unsafe { set.append_sorted(&SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &initial)?, initial.len())?)?; }
            oracle.extend(initial);
            let input: Vec<W> = (0..1024).rev().map(|i| W::from_u128(40000 + i)).collect();
            let expected: Vec<_> = input.iter().copied().collect::<BTreeSet<_>>().difference(&oracle).copied().collect();
            let fresh = unsafe { set.fresh_small(&Buffer::from_host(pool.clone(), &input)?, input.len())? };
            assert_eq!(fresh.buffer().to_host_prefix(fresh.len())?, expected);
            if plain { assert!(set.runs().iter().all(|r| !r.view().plain.is_null())); }
        }
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn varying_digit_sort_preserves_constant_bits() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let set = gpu_datalog_runtime::packed_run::PackedSet::new(pool.clone(), Packing::new(&vec![64; (W::BITS / 64) as usize])?, 256, 4)?;
        for pattern in 0..5 {
            for rows in [0, 1, 97, 4097, 111950] {
                let values: Vec<W> = (0..rows).rev().map(|i| {
                    let mut key = W::zero();
                    for lane in 0..W::BITS / 64 {
                        let v = match pattern {
                            0 => 0xa000000000000005 | (((i * 17 % 997) as u64) << 8),
                            1 if lane == W::BITS / 64 - 1 => ((i * 19 % 997) as u64) << 32,
                            1 => 123,
                            2 => 42,
                            4 if lane == 0 => 0x8000000000000000 | (((i * 19 % 31) as u64) << 32) | (i * 17 % 997) as u64,
                            4 => (i * 23 % 165127) as u64,
                            _ => (i as u64).wrapping_mul(0x9e3779b97f4a7c15).rotate_left(lane * 7),
                        };
                        key.set_lane(lane, v);
                    }
                    key
                }).collect();
                let mut expected = values.clone(); expected.sort_unstable(); expected.dedup();
                let (actual, digits) = unsafe { set.sort_varying(&Buffer::from_host(pool.clone(), &values)?, values.len())? };
                assert_eq!(actual.buffer().to_host_prefix(actual.len())?, expected);
                if pattern == 2 || rows <= 1 { assert_eq!(digits, 0); }
            }
        }
        gpu_datalog_runtime::packed_run::release_sort_scratch();
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn payload_offsets_cross_scan_tiles() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
        for rows in [256 * 17 - 1, 256 * 17, 256 * 17 + 1, 513 * 17] {
            let values: Vec<W> = (0..rows).map(|i| W::from_u128(i as u128 * 123456789)).collect();
            let run = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &values)?, rows, &packing, 17)? };
            assert_eq!(run.unpack()?.to_host()?, values);
        }
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn rule_scans_and_membership_match_oracle() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        use gpu_datalog_runtime::{packed_run::{PackedSet, read_support::{packed_read, plain_read, checksum}}, DeviceRadixRun, RadixRunParams, SortedBatch};
        let pool = Pool::new(0, 0, 0)?;
        for plain in [false, true] {
            let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
            let mut engine = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), RadixRunParams { ratio: 4, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 }, packing.bits())?;
            let mut set = if plain { PackedSet::new_plain(pool.clone(), packing, 256, 4)? }
                else { PackedSet::new(pool.clone(), packing, 256, 4)? };
            let mut values = Vec::new();
            for rows in [0, 40001, 17, 20000] {
                let base = values.len();
                let batch: Vec<W> = (base..base + rows).map(|i| W::from_u128(i as u128 * 3 + 3)).collect();
                unsafe {
                    let fresh = SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &batch)?, rows)?;
                    set.append_sorted(&fresh)?;
                    engine.append_sorted(fresh)?;
                }
                values.extend(batch);
                let lows: Vec<W> = [1000, 0, 900, 120003, 999999, 3, 2000].into_iter().map(W::from_u128).collect();
                let highs: Vec<W> = [2000, 2, 900, 120100, 999999, 120003, 1000].into_iter().map(W::from_u128).collect();
                let low = Buffer::from_host(pool.clone(), &lows)?;
                let high = Buffer::from_host(pool.clone(), &highs)?;
                let expected: Vec<_> = lows.iter().zip(&highs).map(|(&lo, &hi)| {
                    let a = values.partition_point(|v| *v < lo); let b = values.partition_point(|v| *v <= hi);
                    [a as u64, b as u64, if a < b { values[a..b].iter().fold(0u64, |sum, &v| sum.wrapping_add(checksum(v, W::BITS))) } else { 0 }]
                }).collect();
                assert_eq!(packed_read(&set, &low, &high, W::BITS, false)?.to_host()?, expected);
                // The engine interval primitive clamps reversed intervals to
                // [lower(low), lower(low)); packed bounds remain independent.
                let engine_expected: Vec<_> = expected.iter().map(|r| [r[0], r[1].max(r[0]), r[2]]).collect();
                assert_eq!(plain_read(&engine, &low, &high, W::BITS, false)?.to_host()?, engine_expected);
                let expected: Vec<_> = lows.iter().map(|v| [values.binary_search(v).is_ok() as u64, 0, 0]).collect();
                assert_eq!(packed_read(&set, &low, &low, W::BITS, true)?.to_host()?, expected);
            }
        }
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn scans_cross_fields_blocks_and_suffix_masks() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        use gpu_datalog_runtime::{packed_run::{PackedSet, read_support::{packed_read, checksum}}, SortedBatch};
        let pool = Pool::new(0, 0, 0)?;
        let mut widths = if W::BITS == 64 { vec![3, 61] } else { vec![3, 64, 61] };
        while widths.iter().sum::<u32>() < W::BITS { widths.push(64); }
        for full in [false, true] {
            let mut values: Vec<W> = (0..40001).map(|i| {
                let mut word = W::zero();
                for lane in 0..W::BITS / 64 {
                    word.set_lane(lane, if full { (i as u64).wrapping_mul(0x9e3779b97f4a7c15).rotate_left(lane * 7) } else if lane == 0 { i as u64 } else { 42 });
                }
                word
            }).collect();
            values.sort_unstable(); values.dedup();
            let mut set = PackedSet::new(pool.clone(), Packing::new(&widths)?, 256, 4)?;
            unsafe { set.append_sorted(&SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &values)?, values.len())?)?; }
            for probes in [7, 20001] {
                let low: Vec<_> = (0..probes).map(|i| values[(i * 257) % 39000]).collect();
                let high: Vec<_> = (0..probes).map(|i| values[(i * 257) % 39000 + if probes == 7 { 999 } else { 2 }]).collect();
                let lows = Buffer::from_host(pool.clone(), &low)?;
                let highs = Buffer::from_host(pool.clone(), &high)?;
                for suffix in [0, 1, 17, 63, 64, W::BITS - 1, W::BITS] {
                    let expected: Vec<_> = low.iter().zip(&high).map(|(&lo, &hi)| {
                        let a = values.partition_point(|v| *v < lo); let b = values.partition_point(|v| *v <= hi);
                        [a as u64, b as u64, values[a..b].iter().fold(0u64, |sum, &v| sum.wrapping_add(checksum(v, suffix)))]
                    }).collect();
                    assert_eq!(packed_read(&set, &lows, &highs, suffix, false)?.to_host()?, expected);
                }
            }
        }
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn wide_residual_bounds_match_full_keys() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
        let mut values: Vec<W> = (0..2049).map(|i| {
            let mut word = W::zero();
            for lane in 0..W::BITS / 64 { word.set_lane(lane, ((i * (lane as u64 + 3) + 17) % 4093) << 40); }
            word
        }).collect();
        values.sort_unstable(); values.dedup();
        let mut probes = vec![W::zero()];
        for &key in &values {
            probes.push(key);
            let mut below = key; below.set_lane(0, key.lane(0).saturating_sub(1)); probes.push(below);
            let mut above = key; above.set_lane(0, key.lane(0) + 1); probes.push(above);
        }
        let input = Buffer::from_host(pool.clone(), &values)?;
        let queries = Buffer::from_host(pool.clone(), &probes)?;
        for block in [17, 256] {
            let run = unsafe { PackedRun::seal(&input, values.len(), &packing, block)? };
            assert!(run.payload_bytes() < values.len() * std::mem::size_of::<W>());
            let output = run.ranges(&queries, &queries)?.to_host()?;
            for (&key, range) in probes.iter().zip(output) {
                assert_eq!(range, [values.partition_point(|v| *v < key) as u64, values.partition_point(|v| *v <= key) as u64]);
            }
        }
        Ok(())
    }
    check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn specialized_readers_follow_storage_transitions() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        use gpu_datalog_runtime::{packed_run::{PackedSet, replay_support::packed_ranges, read_support::{packed_read, checksum}}, SortedBatch};
        let pool = Pool::new(0, 0, 0)?;
        let mut set = PackedSet::new(pool.clone(), Packing::new(&vec![64; (W::BITS / 64) as usize])?, 256, 4)?;
        let mut values = Vec::new();
        for count in [17, 40001, 3] {
            let first = values.len();
            let batch: Vec<W> = (first..first + count).map(|i| W::from_u128(i as u128 * 3 + 3)).collect();
            unsafe { set.append_sorted(&SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &batch)?, count)?)?; }
            values.extend(batch);
            // More probes than rows forces the serial specialization; few probes
            // exercise cooperative scans before and after the plain-to-packed cutover.
            for probes in [7, values.len() + 1] {
                let keys: Vec<W> = (0..probes).map(|i| W::from_u128((i % (values.len() * 3 + 5)) as u128)).collect();
                let input = Buffer::from_host(pool.clone(), &keys)?;
                let ranges: Vec<_> = keys.iter().map(|key| [values.partition_point(|v| v < key) as u64, values.partition_point(|v| v <= key) as u64]).collect();
                assert_eq!(packed_ranges(&set, &input, &input)?.to_host()?, ranges);
                let membership: Vec<_> = ranges.iter().map(|r| [u64::from(r[0] != r[1]), 0, 0]).collect();
                assert_eq!(packed_read(&set, &input, &input, W::BITS, true)?.to_host()?, membership);
                let scans: Vec<_> = ranges.iter().zip(&keys).map(|(r, &key)| [r[0], r[1], if r[0] != r[1] { checksum(key, W::BITS) } else { 0 }]).collect();
                assert_eq!(packed_read(&set, &input, &input, W::BITS, false)?.to_host()?, scans);
            }
        }
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn balanced_scans_cross_task_and_prefix_tiles() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        use gpu_datalog_runtime::{packed_run::{PackedSet, read_support::{packed_read, checksum}}, SortedBatch};
        let pool = Pool::new(0, 0, 0)?;
        let values: Vec<W> = (0..600001).map(|i| W::from_u128(i * 3 + 3)).collect();
        let mut sums = vec![0u64];
        for &key in &values { sums.push(sums.last().unwrap().wrapping_add(checksum(key, W::BITS))); }
        for (plain, merged) in [(true, false), (false, false), (false, true)] {
            let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
            let mut set = if plain { PackedSet::new_plain(pool.clone(), packing, 256, 4)? }
                else { PackedSet::new(pool.clone(), packing, 256, 4)? };
            if merged {
                // A matched interleaved merge produces non-warp-aligned leaves.
                // Exercise partial fragments on both sides of their boundaries.
                for side in [false, true] {
                    let part: Vec<_> = values.iter().enumerate()
                        .filter_map(|(i, &v)| ((i % 3 == 2) == side).then_some(v)).collect();
                    unsafe { set.append_sorted(&SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &part)?, part.len())?)?; }
                }
                assert_eq!(set.runs().len(), 1);
                assert_eq!(set.runs()[0].view().block_rows, 0);
            } else {
                unsafe { set.append_sorted(&SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &values)?, values.len())?)?; }
            }
            for probes in [4097, 65537] {
                let mut lows = Vec::new();
                let mut highs = Vec::new();
                let mut expected = Vec::new();
                for i in 0..probes {
                    let a = if i == 0 { 0 } else { i * 257 % (values.len() - 2048) };
                    let b = if i == 0 { values.len() } else { a + [0, 3, 31, 32, 33, 1025][i % 6] };
                    let lo = W::from_u128(a as u128 * 3 + 3);
                    let hi = W::from_u128(b as u128 * 3 + 2);
                    lows.push(lo); highs.push(hi);
                    expected.push([a as u64, b as u64, sums[b].wrapping_sub(sums[a])]);
                }
                let low = Buffer::from_host(pool.clone(), &lows)?;
                let high = Buffer::from_host(pool.clone(), &highs)?;
                assert_eq!(packed_read(&set, &low, &high, W::BITS, false)?.to_host()?, expected);
            }
        }
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}
