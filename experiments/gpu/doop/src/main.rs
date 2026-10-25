use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let directory = args.next().map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/doop-data/cassandra"));
    if args.next().is_some() {
        return Err("usage: doop-gpu [facts-directory]".into());
    }

    // Load twice before either timed evaluation: both engines use the same
    // process-global dictionary, including the dataset-precomputed descriptors.
    let gpu_facts = doop_shared::Facts::load(&directory)?;
    let mini_facts = doop_shared::Facts::load(&directory)?;
    // Exclude CUDA context creation from evaluation time, but retain transfers,
    // layout construction and output decoding in the end-to-end measurement.
    let _pool = gpu_datalog_runtime::thread_pool()?;
    let start = Instant::now();
    let mut expected = doop_mini::run(mini_facts);
    let mini_elapsed = start.elapsed();
    let start = Instant::now();
    let mut actual = doop_gpu::run(gpu_facts);
    let gpu_elapsed = start.elapsed();
    expected.compare(&mut actual)
        .map_err(|error| format!("GPU versus mini exact comparison failed: {error}"))?;

    println!("dataset\t{}", directory.display());
    println!("comparison\texact full-tuple sets, all 26 outputs");
    println!("mini_evaluation_seconds\t{:.6}", mini_elapsed.as_secs_f64());
    println!("gpu_evaluation_seconds\t{:.6}", gpu_elapsed.as_secs_f64());
    println!("mini_par_15_threads_reference_seconds\t1.280000");
    println!("gpu_over_mini_par_reference\t{:.6}", gpu_elapsed.as_secs_f64() / 1.28);
    println!("timing_scope\tGPU includes upload, layout construction, execution, download; excludes fact loading and CUDA context creation");
    println!("gpu_phase_split\tnot exposed by whole-program emitter");
    actual.print_sizes();
    Ok(())
}
