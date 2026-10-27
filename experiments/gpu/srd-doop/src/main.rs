fn main() -> srd_doop_harness::Result<()> {
    let options = srd_doop_harness::Options::parse(false, true)?;
    // Exclude CUDA context creation, but retain transfers and result
    // materialization in the harness's evaluation measurement.
    let _pool = gpu_datalog_runtime::thread_pool()?;
    srd_doop_harness::execute(options, "gpu", 1, srd_doop_gpu::run)
}
