use gpu_datalog_runtime::{packed_run::PackedRun, Buffer, Packing, Pool, Result, Word};

fn download<T: cuda_core::DeviceCopy + Default + Clone>(pool: &Pool, ptr: *const T, len: usize) -> Result<Vec<T>> {
    let mut values = vec![T::default(); len];
    unsafe { cuda_core::simt::memory::memcpy_dtoh_async(values.as_mut_ptr().cast::<u8>(), ptr as u64,
        len * std::mem::size_of::<T>(), pool.stream().cu_stream())?; }
    pool.stream().synchronize()?;
    Ok(values)
}

fn check_merge<W: Word>(widths: &[u32], keys: Vec<W>, split: impl Fn(usize) -> bool) -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(widths)?;
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (i, key) in keys.into_iter().enumerate() {
        if split(i) { left.push(key); } else { right.push(key); }
    }
    left.sort_unstable(); right.sort_unstable();
    for block in [1, 17, 256] {
        let a = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &left)?, left.len(), &packing, block)? };
        let b = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &right)?, right.len(), &packing, block)? };
        let merged = unsafe { a.merge(&b)? };
        let mut expected = left.clone(); expected.extend_from_slice(&right); expected.sort_unstable();
        // DECODE P: merged packed keys are exported solely for the exact host test oracle.
        assert_eq!(merged.unpack()?.to_host()?, expected);
        let probes = Buffer::from_host(pool.clone(), &expected)?;
        assert_eq!(merged.contains(&probes, expected.len())?.to_host()?, vec![1; expected.len()]);
        let bounds = merged.ranges(&probes, &probes)?.to_host()?;
        assert_eq!(bounds, (0..expected.len() as u64).map(|i| [i, i + 1]).collect::<Vec<_>>());
    }
    Ok(())
}

#[test]
fn cross_frame_zero_and_full_width_columns() -> Result<()> {
    for width in [0, 1, 31, 32, 63, 64] {
        let mask = if width == 0 { 0 } else { u64::MAX >> (64 - width) };
        let keys: Vec<u128> = (0..1031).map(|i| {
            let code = match i % 3 { 0 => 0, 1 => mask, _ => (i as u64).wrapping_mul(0x9e3779b97f4a7c15) & mask };
            ((i as u128) << 64) | (u64::MAX - mask + code) as u128
        }).collect();
        check_merge(&[64, 64], keys, |i| i % 3 == 0)?;
    }
    Ok(())
}

#[test]
fn wide_cross_lane_frames_and_unaligned_slices() -> Result<()> {
    let keys: Vec<[u64; 4]> = (0..1553).map(|i| {
        let mut key = <[u64; 4]>::zero();
        key.insert(145, 3, (i / 512) as u64);
        key.insert(81, 64, (i as u64).wrapping_mul(0x9e3779b97f4a7c15));
        key.insert(64, 17, (i % 17) as u64);
        key.insert(0, 64, u64::MAX - i as u64);
        key
    }).collect();
    check_merge(&[3, 64, 17, 64], keys, |i| i % 7 < 3)
}

#[test]
fn output_words_cross_dense_insertions_and_leaf_tails() -> Result<()> {
    for rows in [2, 17, 63, 64, 65, 511, 513, 1025] {
        for period in [2, 3, 17] {
            check_merge(&[64], (0..rows).map(|i| 10000 + i as u64).collect(),
                |i| i % period != 0)?;
        }
    }
    Ok(())
}

#[test]
fn tile_prefix_carries_past_empty_intervals_and_balanced_splits() -> Result<()> {
    // Block size one exceeds the lane-per-owner dispatch threshold; larger
    // block sizes run the warp-per-owner schedule. One fence splits on either side.
    check_merge(&[64], (0..21_209u64).map(|i| i + 10000).collect(),
        |i| !(4097..5531).contains(&i) && i % 127 != 0)
}

#[test]
fn register_splices_cross_word_and_last_lane_boundaries() -> Result<()> {
    for n in [1, 15, 16, 31, 32, 33] {
        for first in [1, 125, 220] {
            check_merge(&[64], (0..256).map(|i| 65536u64 + i).collect(),
                |i| !(first..first + n).contains(&i))?;
        }
    }
    Ok(())
}

#[test]
fn disjoint_aligned_blocks_copy_payload_bits() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&[64])?;
    let left: Vec<u64> = (10000..10512).collect();
    let right: Vec<u64> = (20000..20512).collect();
    let a = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &left)?, left.len(), &packing, 256)? };
    let b = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &right)?, right.len(), &packing, 256)? };
    let merged = unsafe { a.merge(&b)? };
    let headers = download(&pool, merged.view().headers, merged.view().blocks as usize)?;
    assert_eq!(headers.iter().map(|h| h.rows as usize).sum::<usize>(), left.len() + right.len());
    assert!(headers.iter().all(|h| h.rows > 0 && h.rows <= 512));
    let source = download(&pool, a.view().headers, a.view().blocks as usize)?;
    let unchanged = &headers[0];
    assert_eq!((unchanged.first, unchanged.rows, unchanged.bits, unchanged.encoding),
        (source[0].first, source[0].rows, source[0].bits, source[0].encoding));
    let input_frame = download(&pool, a.view().frames, 1)?[0];
    let output_frame = download(&pool, merged.view().frames, 1)?[0];
    assert_eq!(input_frame, output_frame);
    let input_bits = download(&pool, unsafe { a.view().payload.add(source[0].offset as usize) }, source[0].words() as usize)?;
    let output_bits = download(&pool, unsafe { merged.view().payload.add(unchanged.offset as usize) }, unchanged.words() as usize)?;
    assert_eq!(input_bits, output_bits);
    let fences = download(&pool, merged.view().fences, headers.len())?;
    let all: Vec<_> = left.iter().chain(&right).copied().collect();
    assert_eq!(fences, headers.iter().map(|h| all[h.first as usize]).collect::<Vec<_>>());
    let mut expected = left; expected.extend(right);
    // DECODE P: copied-block payloads are exported solely for the exact host test oracle.
    assert_eq!(merged.unpack()?.to_host()?, expected);
    Ok(())
}

#[test]
fn metadata_exact_and_zero_plain_frames_match() -> Result<()> {
    use gpu_datalog_runtime::{packed_run::{MergeFrames, PackedSet}, SortedBatch};
    let pool = Pool::new(0, 0, 0)?;
    let mut sizes = Vec::new();
    for policy in [MergeFrames::Metadata, MergeFrames::ExactCodes, MergeFrames::ZeroPlain] {
        let mut set = PackedSet::new(pool.clone(), Packing::new(&[32, 64])?, 256, 4)?;
        set.merge_frames(policy);
        let mut expected = Vec::new();
        for parity in 0..2 {
            let keys: Vec<u128> = (0..40001).map(|i| {
                let lead = i * 2 + parity;
                ((lead as u128) << 64) | (u64::MAX - (lead % 257) as u64) as u128
            }).collect();
            let batch = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &keys)?, keys.len())? };
            unsafe { set.append_sorted(&batch)?; }
            expected.extend(keys);
        }
        expected.sort_unstable();
        // DECODE P: frame-policy variants publish complete keys solely for the exact host test oracle.
        assert_eq!(set.to_host()?, expected);
        sizes.push(set.allocated_bytes());
    }
    assert!(sizes[0] >= sizes[1]);
    assert!(sizes[2] >= sizes[0]);
    Ok(())
}

#[cuda_host::cuda_module]
mod code_probe_kernels {
    use gpu_datalog_runtime::{packed_run::{device, PackedView, read_support::checksum_row}, Word};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn probe<W: Word>(source: PackedView<W>, target: PackedView<W>, output: *mut [u64; 3]) {
        let mut i = thread::index_1d().get() as u64;
        while i < source.rows {
            unsafe { *output.add(i as usize) = [device::code_bound(target, source, i, false),
                device::code_bound(target, source, i, true), checksum_row(source, i, W::BITS)]; }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
}

#[test]
fn enumerated_codes_probe_different_frames() -> Result<()> {
    fn check<W: Word>() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&vec![64; (W::BITS / 64) as usize])?;
        let make = |i: u64| {
            let mut key = W::zero();
            for lane in 0..W::BITS / 64 {
                key.set_lane(lane, if lane == W::BITS / 64 - 1 { i } else { i.wrapping_mul(0x9e3779b97f4a7c15) });
            }
            key
        };
        let source_keys: Vec<W> = (0..1031).map(|i| make(i * 3)).collect();
        let target_keys: Vec<W> = (0..1500).map(|i| make(i * 2 + 1)).collect();
        let source = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &source_keys)?, source_keys.len(), &packing, 17)? };
        let target = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &target_keys)?, target_keys.len(), &packing, 256)? };
        let output = unsafe { Buffer::uninitialized(pool.clone(), source_keys.len())? };
        let module = unsafe { code_probe_kernels::load(pool.context())? };
        module.probe::<W>(pool.stream(), &module.prepare_probe::<W>(cuda_core::LaunchConfig1D::new(5, 256, 0))?,
            source.view(), target.view(), output.ptr())?;
        let expected: Vec<_> = source_keys.iter().map(|key| [target_keys.partition_point(|v| v < key) as u64,
            target_keys.partition_point(|v| v <= key) as u64, gpu_datalog_runtime::packed_run::read_support::checksum(*key, W::BITS)]).collect();
        assert_eq!(output.to_host()?, expected);
        Ok(())
    }
    check::<u64>()?; check::<u128>()?; check::<[u64; 3]>()?; check::<[u64; 4]>()
}

#[test]
fn many_columns_use_bounded_partition_metadata() -> Result<()> {
    let keys = (0..1031u128).map(|i| (i << 64) | i).collect();
    check_merge(&vec![1; 128], keys, |i| i % 5 < 2)
}

#[test]
fn repeated_leaf_absorption_plain_batches_and_fence_changes() -> Result<()> {
    use gpu_datalog_runtime::{packed_run::PackedSet, SortedBatch};
    let pool = Pool::new(0, 0, 0)?;
    let mut set = PackedSet::new(pool.clone(), Packing::new(&[32, 32])?, 256, 128)?;
    let mut expected = Vec::new();
    for batch in 0..12u64 {
        let count = if batch == 0 { 40000 } else { 2048 };
        let mut keys: Vec<u64> = (0..count).map(|i| {
            let leading = if batch == 0 { i * 16 + 16 } else { i * 16 + batch };
            (leading << 32) | if batch < 6 { i % 31 } else { 0xffff0000 + i % 257 }
        }).collect();
        keys.sort_unstable();
        let input = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &keys)?, keys.len())? };
        unsafe { set.append_sorted(&input)?; }
        expected.extend(keys);
        expected.sort_unstable();
        assert_eq!(set.len(), expected.len());
        let probes = Buffer::from_host(pool.clone(), &expected)?;
        assert_eq!(set.absent(&probes, expected.len())?.to_host()?, vec![0; expected.len()]);
        assert_eq!(set.runs().len(), 1);
        let run = &set.runs()[0];
        assert_eq!(run.ranges(&probes, &probes)?.to_host()?,
            (0..expected.len() as u64).map(|i| [i, i + 1]).collect::<Vec<_>>());
    }
    Ok(())
}

#[test]
fn fitting_absorption_and_widening_split_without_tightening() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&[64])?;
    let owner: Vec<u64> = (0..512).map(|i| 1000 + i * 3).collect();
    for base in [1001, 999] {
        let incoming: Vec<u64> = (0..512).map(|i| base + i * 3).collect();
        let a = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &owner)?, owner.len(), &packing, 512)? };
        let b = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &incoming)?, incoming.len(), &packing, 512)? };
        let merged = unsafe { a.merge(&b)? };
        let original = download(&pool, a.view().frames, 1)?[0];
        let frames = download(&pool, merged.view().frames, merged.view().blocks as usize)?;
        let headers = download(&pool, merged.view().headers, merged.view().blocks as usize)?;
        assert_eq!(headers.iter().map(|h| h.rows).sum::<u32>(), 1024);
        assert!(headers.iter().all(|h| h.rows <= 512));
        assert!(frames.iter().all(|f| (f.base, f.bits) == (frames[0].base, frames[0].bits)));
        if base == 1001 {
            assert_eq!((frames[0].base, frames[0].bits), (original.base, original.bits));
        } else {
            assert!(frames[0].base < original.base);
            assert!(frames[0].bits >= original.bits);
        }
        let mut expected = owner.clone();
        expected.extend(incoming);
        expected.sort_unstable();
        let probes = Buffer::from_host(pool.clone(), &expected)?;
        assert_eq!(merged.contains(&probes, expected.len())?.to_host()?, vec![1; expected.len()]);
        assert_eq!(merged.ranges(&probes, &probes)?.to_host()?,
            (0..expected.len() as u64).map(|i| [i, i + 1]).collect::<Vec<_>>());
    }
    Ok(())
}

#[test]
fn reader_ablation_keeps_geometry_or_inherited_envelopes() -> Result<()> {
    assert_eq!(std::mem::size_of::<gpu_datalog_runtime::packed_run::Header>(), 32);
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&[32, 64])?;
    let keys: Vec<u128> = (0..4099u128).map(|i| (i << 64) | (i * 17 % 513)).collect();
    let left: Vec<_> = keys.iter().enumerate().filter_map(|(i, &key)| (i % 3 != 2).then_some(key)).collect();
    let right: Vec<_> = keys.iter().enumerate().filter_map(|(i, &key)| (i % 3 == 2).then_some(key)).collect();
    let a = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &left)?, left.len(), &packing, 17)? };
    let b = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &right)?, right.len(), &packing, 17)? };
    let merged = unsafe { a.merge(&b)? };
    let input = Buffer::from_host(pool.clone(), &keys)?;
    for (fixed, inherit) in [(false, false), (true, true)] {
        let run = unsafe { merged.replay_reframe(&input, fixed, inherit)? };
        assert_eq!(run.ranges(&input, &input)?.to_host()?,
            (0..keys.len() as u64).map(|i| [i, i + 1]).collect::<Vec<_>>());
        if !fixed {
            let original = download(&pool, merged.view().headers, merged.view().blocks as usize)?;
            let exact = download(&pool, run.view().headers, run.view().blocks as usize)?;
            assert_eq!(original.iter().map(|h| (h.first, h.rows)).collect::<Vec<_>>(),
                exact.iter().map(|h| (h.first, h.rows)).collect::<Vec<_>>());
            assert!(run.payload_bytes() <= merged.payload_bytes());
        }
    }
    Ok(())
}

#[test]
fn plain_bootstrap_matches_materialized_owner_byte_for_byte() -> Result<()> {
    use gpu_datalog_runtime::{packed_run::PackedSet, SortedBatch};
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&[64, 64])?;
    let make = |n: u64, parity: u64| -> Result<_> {
        let keys: Vec<u128> = (0..n).map(|i| (((2 * i + parity) as u128) << 64)
            | i.wrapping_mul(0x9e3779b97f4a7c15) as u128).collect();
        unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &keys)?, keys.len()) }
    };
    let a = make(23003, 0)?;
    let b = make(17001, 1)?;
    let sealed = unsafe { PackedRun::seal(a.buffer(), a.len(), &packing, 256)? };
    let mut left = PackedSet::new(pool.clone(), packing.clone(), 256, 4)?;
    let mut right = PackedSet::new(pool.clone(), packing, 256, 4)?;
    unsafe { left.append_sorted(&a)?; right.append_sorted(&b)?; }
    let materialized = unsafe { sealed.merge(&right.runs()[0])? };
    let direct = unsafe { left.runs()[0].merge(&right.runs()[0])? };
    let bytes = |r: &PackedRun<u128>| -> Result<_> {
        let v = r.view();
        let blocks = v.blocks as usize;
        let headers = download(&pool, v.headers, blocks)?;
        let frames = download(&pool, v.frames, blocks * v.columns as usize)?;
        Ok((headers.iter().map(|h| (h.offset,h.first,h.rows,h.bits,h.encoding,h.reserved)).collect::<Vec<_>>(),
            frames,
            download(&pool, v.payload, r.payload_bytes() / 8)?, download(&pool, v.fences, blocks)?,
            download(&pool, v.row_directory, r.len().div_ceil(256))?))
    };
    assert_eq!(bytes(&materialized)?, bytes(&direct)?);
    Ok(())
}

#[test]
fn partial_incoming_groups_preserve_exact_and_metadata_policies() -> Result<()> {
    use gpu_datalog_runtime::packed_run::MergeFrames;
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::new(&[32, 32])?;
    let left: Vec<u64> = (0..1024).map(|i| ((2 * i) << 32) | 10).collect();
    // One incoming source block spans four owner blocks. Only its last
    // value has a nonzero suffix residual. Odd leading keys are disjoint
    // from the owner, with cuts inside the 32-row summary groups.
    let right: Vec<u64> = (0..256).map(|i| (((7 * i) | 1) << 32) | if i == 255 { 100 } else { 10 }).collect();
    let a = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &left)?, left.len(), &packing, 256)? };
    let b = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &right)?, right.len(), &packing, 256)? };
    for (policy, widths) in [(MergeFrames::Metadata, [0, 0, 0, 7]), (MergeFrames::ZeroPlain, [7; 4])] {
        let m = unsafe { a.merge_with_frames(&b, policy)? };
        let frames = download(&pool, m.view().frames, m.view().blocks as usize * 2)?;
        assert_eq!(frames.chunks_exact(2).map(|f| (f[1].base, f[1].bits)).collect::<Vec<_>>(),
            widths.map(|w| (10, w)));
    }
    Ok(())
}
