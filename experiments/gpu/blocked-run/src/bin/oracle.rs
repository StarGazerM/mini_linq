//! CPU-oracle checks for the GPU run builder and the device point operation.
//!
//! Run the `gpu-h05` package's `blocked-run-oracle` binary with the pinned
//! cuda-oxide toolchain. CPU sorting is used only here to construct expected
//! answers; the production builder never downloads keys or sorts on CPU.

use cuda_core::{CudaContext, CudaStream, DeviceBuffer, LaunchConfig1D};
use gpu_blocked_run::{read_stats, Buffer, MemoryResource, Run, RunStats, RankView, BLOCK_ROWS};
use rccl_memory::{ContiguousDeviceRange, CudaMemoryResource, PoolMemoryResource};
use std::error::Error;

#[cuda_device::cuda_module]
mod kernels {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    use gpu_blocked_run::{point, RankView};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn probe(
        view: RankView, queries: &[i64], begins: *mut u32, ends: *mut u32,
    ) {
        let i = thread::index_1d().get();
        if i < queries.len() {
            let (begin, end) = match unsafe { point(view, queries[i]) } {
                Some(range) => range,
                None => (u32::MAX, u32::MAX),
            };
            unsafe {
                begins.add(i).write(begin);
                ends.add(i).write(end);
            }
        }
    }
}

fn check(
    ctx: &CudaContext,
    stream: &CudaStream,
    name: &str,
    input: Vec<i64>,
    payload_columns: usize,
) -> Result<(), Box<dyn Error>> {
    let payloads: Vec<Vec<i64>> = (0..payload_columns).map(|column| {
        (0..input.len()).map(|row| {
            // Every column identifies the input row differently, so this tests
            // clustering as well as cross-column correspondence for duplicates.
            if column % 2 == 0 { row as i64 * 17 + column as i64 }
            else { -(row as i64) * 31 - column as i64 }
        }).collect()
    }).collect();
    let domain = input.iter().min().zip(input.iter().max())
        .map(|(&lo, &hi)| (lo, hi)).unwrap_or((0, -1));
    let mut padded = input.clone();
    padded.extend([i64::MIN; 7]);
    let device_keys = DeviceBuffer::from_host(stream, &padded)?;
    let device_payloads: Vec<DeviceBuffer<i64>> = payloads.iter()
        .map(|column| {
            let mut padded = column.clone(); padded.extend([i64::MAX; 7]);
            DeviceBuffer::from_host(stream, &padded)
        })
        .collect::<Result<_, _>>()?;
    let refs: Vec<&dyn ContiguousDeviceRange<i64>> = device_payloads.iter()
        .map(|p| p as &dyn ContiguousDeviceRange<i64>).collect();
    let memory = MemoryResource::new(PoolMemoryResource::new(
        CudaMemoryResource::new(stream.context().clone()), 32 << 20, 0)?);
    let count = Buffer::from_host_in(&memory, stream, &[input.len() as u32])?;
    let run = Run::build(ctx, stream, &memory, &device_keys, &refs, domain, Some(&count))?;
    let stats = read_stats(&memory, stream, &[&run])?[0];

    let mut order: Vec<usize> = (0..input.len()).collect();
    order.sort_by_key(|&row| input[row]);
    let sorted: Vec<i64> = order.iter().map(|&row| input[row]).collect();
    let actual_keys = run.keys.to_host_vec()?;
    assert_eq!(&actual_keys[..input.len()], sorted, "{name}: sorted keys");
    assert!(actual_keys[input.len()..].iter().all(|&key| key == 0), "{name}: inactive tail");
    assert_eq!(run.payloads.len(), payload_columns, "{name}: payload count");
    for (column, payload) in payloads.iter().enumerate() {
        let expected: Vec<_> = order.iter().map(|&row| payload[row]).collect();
        assert_eq!(&run.payloads[column].to_host_vec()?[..input.len()], expected,
            "{name}: payload column {column} / stable duplicate ordering");
        assert_eq!(&device_payloads[column].to_host_vec(stream)?[..input.len()], *payload,
            "{name}: input payload unchanged");
    }
    assert_eq!(device_keys.to_host_vec(stream)?, padded, "{name}: input keys unchanged");

    let mut unique = Vec::new();
    let mut offsets = Vec::new();
    for (row, &key) in sorted.iter().enumerate() {
        if row == 0 || sorted[row - 1] != key {
            unique.push(key);
            offsets.push(row as u32);
        }
    }
    offsets.push(sorted.len() as u32);
    let max_count = offsets.windows(2).map(|pair| pair[1] - pair[0]).max().unwrap_or(0);
    assert_eq!(stats, RunStats {
        rows: sorted.len() as u32, unique: unique.len() as u32, max_count, invalid: 0,
        sorted: input.windows(2).all(|pair| pair[0] <= pair[1]) as u32,
    }, "{name}: stats");
    assert_eq!(&run.offsets.to_host_vec()?[..offsets.len()], offsets, "{name}: CSR offsets and sentinel");
    assert_eq!(&run.unique_keys.to_host_vec()?[..unique.len()], unique, "{name}: unique keys");
    let fences: Vec<i64> = sorted.chunks(BLOCK_ROWS).map(|block| block[0]).collect();
    assert_eq!(&run.fences.to_host_vec()?[..fences.len()], fences, "{name}: every 2048-row fence");

    let mut queries = vec![i64::MIN, i64::MAX, -1, 0, 1];
    if let (Some(&min), Some(&max)) = (sorted.first(), sorted.last()) {
        assert_eq!((run.min_key, run.max_key), (min, max), "{name}: key extrema");
        let span = (max as i128 - min as i128 + 1) as usize;
        let mut bits = vec![0u64; span.div_ceil(64)];
        for &key in &unique {
            let position = (key as i128 - min as i128) as usize;
            bits[position / 64] |= 1u64 << (position % 64);
        }
        let mut count = 0u32;
        let prefix: Vec<_> = bits.iter().map(|word| {
            let previous = count;
            count += word.count_ones();
            previous
        }).collect();
        assert_eq!(run.bits.to_host_vec()?, bits, "{name}: bitmap");
        assert_eq!(run.prefix.to_host_vec()?, prefix, "{name}: popcount prefix");
        // Exhaust the complete dense domain, including missing positions at
        // every 64-bit word boundary and holes within the first/last word.
        queries.extend((0..span).map(|position| (min as i128 + position as i128) as i64));
        if let Some(key) = min.checked_sub(1) { queries.push(key); }
        if let Some(key) = max.checked_add(1) { queries.push(key); }
    } else {
        assert!(run.min_key > run.max_key, "{name}: empty domain");
        assert!(run.bits.to_host_vec()?.is_empty(), "{name}: empty bitmap");
        assert!(run.prefix.to_host_vec()?.is_empty(), "{name}: empty prefix");
    }
    queries.sort_unstable();
    queries.dedup();
    let device_queries = DeviceBuffer::from_host(stream, &queries)?;
    let begins = DeviceBuffer::<u32>::zeroed(stream, queries.len())?;
    let ends = DeviceBuffer::<u32>::zeroed(stream, queries.len())?;
    // SAFETY: this binary owns the probe kernel declaration and embedded ABI.
    let module = unsafe { kernels::load(stream.context())? };
    let launch = module.prepare_probe(
        LaunchConfig1D::new(queries.len().div_ceil(256) as u32, 256, 0),
    )?;
    let view: RankView = run.view();
    let copied_view = view; // Check that the exported ABI is actually Copy.
    unsafe {
        module.probe(stream, &launch, copied_view, &device_queries,
            begins.cu_deviceptr() as *mut u32, ends.cu_deviceptr() as *mut u32)?;
    }
    let begins = begins.to_host_vec(stream)?;
    let ends = ends.to_host_vec(stream)?;
    for (i, &key) in queries.iter().enumerate() {
        let begin = sorted.partition_point(|&value| value < key);
        let end = sorted.partition_point(|&value| value <= key);
        let expected = if begin == end { (u32::MAX, u32::MAX) }
            else { (begin as u32, end as u32) };
        assert_eq!((begins[i], ends[i]), expected, "{name}: point({key}) half-open range");
    }
    println!("{name}: {} rows, {} unique, {} payloads exact", input.len(), unique.len(), payload_columns);
    Ok(())
}

fn random_keys(n: usize, mut state: u64) -> Vec<i64> {
    (0..n).map(|_| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        // Reserve holes even in the interior of a densely populated domain.
        let key = ((state >> 32) % 1025) as i64 - 512;
        if key % 7 == 0 { key + 1 } else { key }
    }).collect()
}

fn main() -> Result<(), Box<dyn Error>> {
    let ctx = CudaContext::new(0)?;
    let stream = ctx.default_stream();
    let memory = MemoryResource::new(PoolMemoryResource::new(
        CudaMemoryResource::new(ctx.clone()), 32 << 20, 0)?);
    check(&ctx, &stream, "empty", Vec::new(), 0)?;
    check(&ctx, &stream, "empty-payload-columns", Vec::new(), 3)?;
    check(&ctx, &stream, "singleton-negative", vec![-17], 2)?;
    check(&ctx, &stream, "all-repeated", vec![-65; 8193], 3)?;
    check(&ctx, &stream, "word-boundary-holes",
        vec![129, 65, -130, -129, -68, -65, -63, -3, 0, 1, 2, 64, 66, 128, -130, 129], 2)?;
    check(&ctx, &stream, "near-i64-min",
        vec![i64::MIN + 130, i64::MIN, i64::MIN + 64, i64::MIN, i64::MIN + 127], 2)?;
    check(&ctx, &stream, "near-i64-max",
        vec![i64::MAX, i64::MAX - 130, i64::MAX - 64, i64::MAX, i64::MAX - 127], 2)?;
    check(&ctx, &stream, "no-payloads", random_keys(4097, 9123), 0)?;
    for n in [31, 256, 257, 2047, 2048, 2049, 32771, 65537] {
        check(&ctx, &stream, &format!("random-duplicates-{n}"), random_keys(n, n as u64 + 123), 3)?;
    }
    let descending: Vec<_> = (-4096i64..=4096).rev().collect();
    check(&ctx, &stream, "all-unique-descending", descending, 1)?;
    check(&ctx, &stream, "all-unique-ascending", (-4096i64..=4096).collect(), 3)?;
    let mut sorted_duplicates = random_keys(65537, 812);
    sorted_duplicates.sort_unstable();
    check(&ctx, &stream, "sorted-duplicates-with-holes", sorted_duplicates, 3)?;
    for inversion in [31usize, 255, 256, 2048, 65535] {
        let mut keys: Vec<_> = (0..65537i64).collect();
        keys.swap(inversion - 1, inversion);
        check(&ctx, &stream, &format!("single-inversion-{inversion}"), keys, 2)?;
    }
    // Cross bitmap words, warp groups, scan blocks and recursive word-total
    // boundaries. Sparse words test row totals separately from popcounts.
    for domain in [63i64, 64, 65, 255, 256, 257, 16383, 16384, 16385,
                   65535, 65536, 65537, 4194303, 4194304, 4194305] {
        check(&ctx, &stream, &format!("scan-domain-{domain}"),
            vec![domain - 1, 0, domain / 2, 0, domain - 1, 1], 2)?;
    }
    // A single very large bin interspersed with other keys checks that
    // provenance sorting is not limited by block size or small fanout.
    let skewed = (0..131073).map(|row| {
        if row % 4093 == 0 { (row % 129) as i64 - 64 } else { -7 }
    }).collect();
    check(&ctx, &stream, "large-interleaved-skew", skewed, 3)?;

    let keys = DeviceBuffer::from_host(&stream, &[1i64, 2])?;
    let short = DeviceBuffer::from_host(&stream, &[9i64])?;
    assert!(Run::build(&ctx, &stream, &memory, &keys, &[&short], (1, 2), None).is_err(), "reject mismatched payload rows");
    let empty = DeviceBuffer::<i64>::zeroed(&stream, 0)?;
    assert!(Run::build(&ctx, &stream, &memory, &empty, &[&short], (0, -1), None).is_err(), "validate empty payload rows");
    for input in [[i64::MIN, i64::MAX], [0, 1i64 << 32]] {
        let wide = DeviceBuffer::from_host(&stream, &input)?;
        assert!(Run::build(&ctx, &stream, &memory, &wide, &[], (input[0], input[1]), None).is_err(), "reject oversized dense domain");
    }
    let invalid_count = Buffer::from_host_in(&memory, &stream, &[3u32])?;
    let run = Run::build(&ctx, &stream, &memory, &keys, &[], (1, 2), Some(&invalid_count))?;
    assert!(read_stats(&memory, &stream, &[&run]).is_err(), "reject device count overflow");
    let run = Run::build(&ctx, &stream, &memory, &keys, &[], (1, 1), None)?;
    assert!(read_stats(&memory, &stream, &[&run]).is_err(), "reject out-of-domain device key");
    let run = Run::build(&ctx, &stream, &memory, &keys, &[], (-64, 128), None)?;
    assert_eq!(read_stats(&memory, &stream, &[&run])?[0].unique, 2);
    assert_eq!(&run.offsets.to_host_vec()?[..3], &[0, 1, 2], "sentinel with trailing domain holes");
    println!("blocked-run oracle: all cases passed");
    Ok(())
}
