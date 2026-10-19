//! RCCL's own program alongside blocked-run; all storage uses the caller pool.
use cuda_core::{CudaContext, CudaStream, DeviceCopy};
use gpu_blocked_run::{Buffer, MemoryResource};
use rccl_memory::{CudaMemoryResource, PoolMemoryResource};
use std::error::Error;

struct Scratch<'a, T: DeviceCopy>(Buffer<'a, T>);
impl<'a, T: DeviceCopy> rccl_cub::WorkspaceBuffer<T> for Scratch<'a, T> {
    type Range = Buffer<'a, T>;
    fn range_mut(&mut self) -> &mut Self::Range {
        &mut self.0
    }
}
fn probe<K>(
    stream: &CudaStream,
    memory: &MemoryResource,
    program: &rccl_cub::cuda::Program,
    n: usize,
    distribution: &str,
    exponent: f64,
    begin_bit: u32,
    bits: u32,
) -> Result<(), Box<dyn Error>>
where
    K: rccl_cub::RadixKey
        + From<u32>
        + PartialEq
        + std::fmt::Debug
        + std::ops::Shl<u32, Output = K>,
{
    let domain = 1_000_000usize;
    let cdf = if exponent > 0.0 {
        let mut total = 0.0;
        let mut cdf: Vec<f64> = (1..=domain)
            .map(|k| {
                total += (k as f64).powf(-exponent);
                total
            })
            .collect();
        for p in &mut cdf {
            *p /= total;
        }
        cdf
    } else {
        Vec::new()
    };
    let mut state = 0x123456789abcdefu64;
    let input: Vec<K> = (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let key = if exponent == 0.0 {
                (state % domain as u64) as u32
            } else {
                cdf.partition_point(|&p| p < (state >> 11) as f64 / ((1u64 << 53) as f64)) as u32
            };
            // Nonzero begin bit proves selected-bit rather than full-word sorting.
            K::from(key) << begin_bit
        })
        .collect();
    let values: Vec<u32> = (0..n as u32).collect();
    let mut keys = Buffer::from_host_in(memory, stream, &input)?;
    let mut rows = Buffer::from_host_in(memory, stream, &values)?;
    let mut alternate_keys = Buffer::<K>::zeroed_in(memory, stream, n)?;
    let mut alternate_rows = Buffer::<u32>::zeroed_in(memory, stream, n)?;
    let counts = rccl_cub::__private::device_radix_sort_pairs_workspace_counts::<K, u32>(n);
    let mut workspace = rccl_cub::DeviceRadixSortBufferWorkspace::__from_parts(
        Scratch(Buffer::<u64>::zeroed_in(memory, stream, counts.bins)?),
        Scratch(Buffer::<i32>::zeroed_in(memory, stream, counts.lookback)?),
        Scratch(Buffer::<i32>::zeroed_in(memory, stream, counts.counters)?),
    );
    let launch = program.on(stream)?;
    let start = stream.record_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
    let mut buffers = rccl_cub::DoubleBuffer::paired(
        (&mut keys, &mut rows),
        (&mut alternate_keys, &mut alternate_rows),
    )
    .unwrap();
    rccl_cub::device_radix_sort_pairs_buffer(
        &launch,
        &mut buffers,
        begin_bit..bits,
        &mut workspace,
    )?;
    let end = stream.record_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
    end.synchronize()?;
    let ms = start.elapsed_ms(&end)?;
    let actual_keys = buffers.current().0.to_host_vec()?;
    let actual_rows = buffers.current().1.to_host_vec()?;
    let mut previous = 0u32;
    for i in 0..n {
        let row = actual_rows[i] as usize;
        assert!(row < n);
        assert_eq!(actual_keys[i], input[row]);
        let key = K::digit(
            K::to_ordered_ascending(actual_keys[i]),
            begin_bit,
            bits - begin_bit,
        );
        if i != 0 {
            assert!(previous <= key);
            if previous == key {
                assert!(actual_rows[i - 1] < actual_rows[i]);
            }
        }
        previous = key;
    }
    let passes = (bits - begin_bit).div_ceil(8);
    let bytes = n as f64 * (std::mem::size_of::<K>() + 4) as f64 * 2.0 * passes as f64;
    println!("{},n={},distribution={},range={}..{},ms={:.3},Gpairs/s={:.3},pass_GB/s={:.1},bound_ms={:.3}",
        std::any::type_name::<K>(), n, distribution, begin_bit, bits, ms, n as f64 / ms as f64 / 1e6,
        bytes / ms as f64 / 1e6, bytes / 960e6);
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let context = CudaContext::new(0)?;
    let stream = context.new_stream()?;
    let memory = MemoryResource::new(PoolMemoryResource::new(
        CudaMemoryResource::new(context.clone()),
        32 << 20,
        0,
    )?);
    eprintln!("loading blocked-run module");
    let empty = Buffer::<i64>::zeroed_in(&memory, &stream, 0)?;
    let run = gpu_blocked_run::Run::build(&context, &stream, &memory, &empty, &[], (0, -1), None)?;
    gpu_blocked_run::read_stats(&memory, &stream, &[&run])?;
    eprintln!("blocked-run loaded; loading RCCL own Program");
    let program = rccl_cub::cuda::Program::load(&context)?;
    eprintln!("RCCL own Program loaded without collision");
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .map(|s| s.parse())
        .collect::<Result<_, _>>()?;
    let sizes = if sizes.is_empty() {
        vec![1_000_000, 10_000_000, 100_000_000]
    } else {
        sizes
    };
    for n in sizes {
        for (name, exponent) in [("uniform", 0.0), ("zipf1", 1.0), ("zipf1.5", 1.5)] {
            probe::<u32>(&stream, &memory, &program, n, name, exponent, 3, 23)?;
            probe::<u64>(&stream, &memory, &program, n, name, exponent, 35, 55)?;
        }
    }
    Ok(())
}
