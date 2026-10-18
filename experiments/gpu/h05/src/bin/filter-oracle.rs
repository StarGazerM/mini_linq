#[path = "../kernels.rs"]
mod kernels;
use cuda_core::{CudaContext, DeviceBuffer, LaunchConfig1D};
use gpu_blocked_run::{Buffer, MemoryResource};
use rccl_memory::{CudaMemoryResource, PoolMemoryResource, StatisticsMemoryResource};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = CudaContext::new(0)?;
    let stream = ctx.default_stream();
    let module = unsafe { kernels::kernels::load(&ctx)? };
    let memory: MemoryResource = StatisticsMemoryResource::new(PoolMemoryResource::new(CudaMemoryResource::new(ctx.clone()), 64 << 20, 64 << 20)?);
    for n in [0usize, 1, 255, 256, 257, 65537, 131075] {
        for mask in 0..4 {
            let keys = (0..n as i64).collect::<Vec<_>>();
            let customers = keys.iter().map(|k| k * 7 + 3).collect::<Vec<_>>();
            let dates = keys.iter().map(|&k| match mask {0 => 0, 1 => 1, 2 => k % 3, _ => (k * 1664525 + 1013904223) % 11}).collect::<Vec<_>>();
            let dk = DeviceBuffer::from_host(&stream, &keys)?;
            let dc = DeviceBuffer::from_host(&stream, &customers)?;
            let dd = DeviceBuffer::from_host(&stream, &dates)?;
            let outk = Buffer::<i64>::zeroed_in(&memory, &stream, n.max(1))?;
            let outc = Buffer::<i64>::zeroed_in(&memory, &stream, n.max(1))?;
            let blocks = Buffer::<u32>::zeroed_in(&memory, &stream, n.div_ceil(256).max(1))?;
            let launch = module.prepare_filter_orders(LaunchConfig1D::new(blocks.len() as u32, 256, 64))?;
            module.filter_orders(&stream, &launch, &dk, &dc, &dd, 1, 2, outk.cu_deviceptr() as *mut i64, outc.cu_deviceptr() as *mut i64, blocks.cu_deviceptr() as *mut u32, false)?;
            let levels = gpu_blocked_run::exclusive_scan(&stream, &memory, &blocks)?;
            module.filter_orders(&stream, &launch, &dk, &dc, &dd, 1, 2, outk.cu_deviceptr() as *mut i64, outc.cu_deviceptr() as *mut i64, blocks.cu_deviceptr() as *mut u32, true)?;
            let count = levels.last().unwrap().to_host_vec()?[0] as usize;
            let expected = keys.iter().zip(&customers).zip(&dates).filter_map(|((&k, &c), &d)| (d == 1).then_some((k,c))).collect::<Vec<_>>();
            let actual = outk.to_host_vec()?.into_iter().zip(outc.to_host_vec()?).take(count).collect::<Vec<_>>();
            assert_eq!(actual, expected, "n={n} mask={mask}");
        }
    }
    println!("stable filter oracle: 28 cases exact (empty/full/sparse/partial/recursive scan)");
    Ok(())
}
