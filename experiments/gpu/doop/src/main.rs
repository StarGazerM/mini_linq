use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut directory = PathBuf::from("/tmp/doop-data/cassandra");
    let mut gpu_only = false;
    for argument in std::env::args_os().skip(1) {
        if argument == "--gpu-only" {
            gpu_only = true;
        } else {
            directory = PathBuf::from(argument);
        }
    }

    // `--gpu-only` loads once and skips the CPU reference and comparison, so a
    // timeout bounds the GPU evaluation alone; set GPU_DATALOG_PHASES=1 for the
    // setup / fixpoint / output split.
    let gpu_facts = doop_shared::Facts::load(&directory)?;
    let mini_facts = if gpu_only { None } else { Some(doop_shared::Facts::load(&directory)?) };
    // Exclude CUDA context creation from evaluation time, but retain transfers,
    // layout construction and output decoding in the end-to-end measurement.
    let _pool = gpu_datalog_runtime::thread_pool()?;
    let reference = mini_facts.map(|facts| {
        let start = Instant::now();
        let expected = doop_mini::run(facts);
        (expected, start.elapsed())
    });
    let start = Instant::now();
    let mut actual = doop_gpu::run(gpu_facts);
    let gpu_elapsed = start.elapsed();
    println!("dataset\t{}", directory.display());
    let Some((mut expected, mini_elapsed)) = reference else {
        println!("comparison\tskipped (--gpu-only)");
        println!("gpu_evaluation_seconds\t{:.6}", gpu_elapsed.as_secs_f64());
        actual.print_sizes();
        return Ok(());
    };
    expected.compare(&mut actual)
        .map_err(|error| format!("GPU versus mini exact comparison failed: {error}"))?;
    println!("comparison\texact full-tuple sets, all 26 outputs");
    println!("mini_evaluation_seconds\t{:.6}", mini_elapsed.as_secs_f64());
    println!("gpu_evaluation_seconds\t{:.6}", gpu_elapsed.as_secs_f64());
    println!("timing_scope\tGPU includes upload, layout construction, execution, download; excludes fact loading and CUDA context creation; GPU_DATALOG_PHASES=1 splits setup/fixpoint/output");
    actual.print_sizes();
    Ok(())
}
