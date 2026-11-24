//! `join-lab caps`: machine caps of this GPU (see DESIGN.md).
use cuda_core::LaunchConfig1D;
use gpu_datalog_runtime::{Buffer, Pool, Result};
use join_lab::caps::caps_kernels as kernels;
use std::sync::Arc;
use std::time::Instant;

fn grid(function: &cuda_core::CudaFunction) -> Result<u32> {
    cq_gpu_runtime::launch_grid_for_kernel(function, usize::MAX, 256, 0)
}

/// Median milliseconds of `repeat` event-timed runs of `launch` after one warmup.
fn time(pool: &Arc<Pool>, repeat: usize, mut launch: impl FnMut() -> Result<()>) -> Result<f64> {
    launch()?;
    let stream = pool.stream();
    let mut samples = Vec::with_capacity(repeat);
    for _ in 0..repeat {
        let flags = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
        let start = stream.record_event(flags)?;
        launch()?;
        let end = stream.record_event(flags)?;
        end.synchronize()?;
        samples.push(start.elapsed_ms(&end)? as f64);
    }
    samples.sort_by(f64::total_cmp);
    Ok(samples[repeat / 2])
}

fn caps(pool: &Arc<Pool>) -> Result<()> {
    let module = unsafe { kernels::load(pool.context())? };
    let stream = pool.stream();
    let out = Buffer::<u64>::zeroed(pool.clone(), 1 << 16)?;
    let config = |blocks| LaunchConfig1D::new(blocks, 256, 0);
    let resident = grid(module.prepare_stream_sum(config(1))?.function())?;
    println!("resident_blocks\t{resident}");
    let big: u64 = 1 << 28; // 2 GiB of u64
    let data = unsafe { Buffer::<u64>::uninitialized(pool.clone(), big as usize)? };
    module.iota(stream, &module.prepare_iota(config(resident))?, data.ptr(), big, 3)?;
    stream.synchronize()?;

    println!("cap\tsize_words\tms\tunit_rate");
    for words in [1u64 << 22, 1 << 28] {
        let p = module.prepare_stream_sum(config(resident))?;
        let ms = time(pool, 9, || Ok(module.stream_sum(stream, &p, data.ptr(), words, out.ptr())?))?;
        println!("stream_read\t{words}\t{ms:.4}\t{:.1} GB/s", words as f64 * 8.0 / ms / 1e6);
    }
    for words in [1u64 << 22, 1 << 28] {
        let m = 1u64 << 26;
        let p = module.prepare_gather_sum(config(resident))?;
        let ms = time(pool, 9, || Ok(module.gather_sum(stream, &p, data.ptr(), words, m, 7, out.ptr())?))?;
        println!("random_gather_8B\t{words}\t{ms:.4}\t{:.2} G loads/s", m as f64 / ms / 1e6);
    }
    for words in [1u64 << 16, 1 << 20, 1 << 22, 1 << 24, 1 << 28] {
        for sorted in [0u32, 1] {
            let m = 1u64 << 26;
            let p = module.prepare_search_sum(config(resident))?;
            let ms = time(pool, 7, || Ok(module.search_sum(stream, &p, data.ptr(), words, m, 11, sorted, out.ptr())?))?;
            let name = if sorted != 0 { "search_sorted_queries" } else { "search_random_queries" };
            println!("{name}\t{words}\t{ms:.4}\t{:.2} G searches/s", m as f64 / ms / 1e6);
        }
    }
    for slots in [1u64, 1024, 1 << 16] {
        let iters = 64u32;
        let p = module.prepare_atomic_add(config(resident))?;
        let ms = time(pool, 7, || Ok(module.atomic_add(stream, &p, out.ptr(), slots, iters)?))?;
        let ops = resident as f64 * 256.0 * iters as f64;
        println!("atomic_add_u64\tslots={slots}\t{ms:.4}\t{:.2} G atomics/s", ops / ms / 1e6);
    }
    let p = module.prepare_empty(config(1))?;
    let launches = 2000;
    let ms = time(pool, 3, || {
        for _ in 0..launches {
            module.empty(stream, &p, out.ptr())?;
        }
        Ok(())
    })?;
    println!("launch_empty\tx{launches}\t{ms:.4}\t{:.2} us/launch (stream-ordered)", ms * 1000.0 / launches as f64);
    let readbacks = 500;
    let small = Buffer::<u64>::zeroed(pool.clone(), 1)?;
    let start = Instant::now();
    for _ in 0..readbacks {
        module.empty(stream, &p, out.ptr())?;
        small.to_host()?;
    }
    let us = start.elapsed().as_secs_f64() * 1e6 / readbacks as f64;
    println!("launch_then_readback_8B\tx{readbacks}\t-\t{us:.2} us/round trip (host wall)");
    Ok(())
}

fn main() -> Result<()> {
    let pool = gpu_datalog_runtime::thread_pool()?;
    match std::env::args().nth(1).as_deref() {
        Some("caps") | None => caps(&pool),
        Some(other) => Err(format!("unknown command {other}").into()),
    }
}
