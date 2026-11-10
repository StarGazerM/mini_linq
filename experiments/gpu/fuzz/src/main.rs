//! Slow-tier only: `cargo run -p cq-gpu-fuzz -- [seed] [count]` on a CUDA host.
//! Source publication, build, and execution share one GPU flock guard, so another
//! harness cannot replace the shared binary between compilation and execution.
use std::{fmt::Write as _, path::Path, process::Command};
#[allow(dead_code)]
#[path = "../../../../features/datalog/oracle/src/random_programs.rs"]
mod datalog;

const GPU_DL: &str = "incremental = cq_datalog::passes::incremental(driver(delta_first);); storage_rewrites = [cq_gpu_datalog::device_storage]; cq = pipeline { physical = [cq_gpu_sequence::capture_constants]; structures = [cq_gpu_datalog::bind_inputs, cq_gpu_sequence::bind_runs]; sequence = mini_linq::passes::sequence; stage = cq_gpu_sequence::stage; decompose = [cq_gpu_sequence::decompose]; grade = cq_gpu_sequence::machine_grade; print = cq_gpu_sequence::translate_arena; }; terminals = [cq_gpu_sequence::translate_arena => batch]; round = [cq_gpu_datalog::bind_round(binding = 5)]; print = cq_gpu_datalog::print_rounds;";
const GPU_CQ: &str = "physical = [cq_gpu_sequence::capture_constants]; structures = [cq_gpu_sequence::bind_runs, cq_gpu_sequence::select_catalog]; sequence = mini_linq::passes::sequence; stage = cq_gpu_sequence::stage; decompose = [cq_gpu_sequence::decompose]; grade = cq_gpu_sequence::machine_grade; print = cq_gpu_sequence::translate;";

fn cq_source(seed: u64, count: usize) -> String {
    use cq_testkit::fuzz::{Profile, generate_with};
    let mut source = String::from("fn main() {\n");
    for offset in 0..count {
        let case = generate_with(seed.wrapping_add(offset as u64), &Profile::gpu());
        source.push_str("{\n");
        for (relation, rows) in case.query.schema.iter().zip(&case.data) {
            let types = relation
                .types
                .iter()
                .map(|t| format!("{},", t.rust()))
                .collect::<String>();
            let literals = rows
                .iter()
                .map(|row| {
                    format!(
                        "({},)",
                        row.iter()
                            .map(|v| v.rust_literal())
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            writeln!(
                source,
                "let {}: &[({types})] = &[{literals}];",
                relation.name
            )
            .unwrap();
        }
        for (name, value) in case.parameters() {
            writeln!(source, "let {name} = {};", value.rust_literal()).unwrap();
        }
        let outputs = case.output_types();
        let expected = case
            .reference()
            .iter()
            .map(|row| {
                let values = row
                    .iter()
                    .zip(&outputs)
                    .map(|(v, t)| {
                        if t.optional {
                            format!("Some({})", v.rust_literal())
                        } else {
                            v.rust_literal()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                if values.is_empty() {
                    "()".into()
                } else {
                    format!("({values},)")
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let types = outputs
            .iter()
            .map(|t| format!("{},", t.rust()))
            .collect::<String>();
        writeln!(source, "let expected: std::collections::BTreeSet<({types})> = [{expected}].into_iter().collect();").unwrap();
        let owned = (0..outputs.len())
            .map(|index| format!("row.{index}.to_owned(),"))
            .collect::<String>();
        let cpu_rows = format!(".into_iter().map(|row| ({owned}))");
        // Both routes run identical CQ-to-Physical passes, including captured
        // constants. Only Structures selection and the backend differ.
        for (route, pipeline, rows) in [
            (
                "cpu-physical",
                "physical = [cq_gpu_sequence::capture_constants];",
                cpu_rows.as_str(),
            ),
            ("gpu", GPU_CQ, ".to_host().unwrap().into_iter()"),
        ] {
            let context = format!(
                "FUZZ FAILURE seed={} route={route} stage=execute data={:?}",
                seed.wrapping_add(offset as u64),
                case.data
            );
            writeln!(source, "let actual: std::collections::BTreeSet<({types})> = mini_linq::compile! {{ pipeline {{ {pipeline} }} {} }}{rows}.collect(); assert_eq!(actual, expected, {:?});", case.cq(), context).unwrap();
        }
        source.push_str("}\n");
    }
    source.push_str("}\n");
    source
}

fn run(root: &Path, lane: &str, seed: u64, source: String) -> bool {
    let _lock = gpu_lock(Path::new("/tmp/mini_linq-gpu.lock"));
    let directory = root
        .join("experiments/gpu/target/fuzz")
        .join(format!("{lane}-{seed}"));
    std::fs::create_dir_all(directory.join("src")).unwrap();
    let mut manifest = datalog::manifest(root);
    for (name, path) in [
        ("cq-gpu-datalog", "datalog/facade"),
        ("cq-gpu-sequence", "sequence"),
        ("cq-gpu-sequence-runtime", "sequence-runtime"),
        ("cq-gpu-runtime", "printer-runtime"),
        ("gpu-datalog-runtime", "datalog-runtime"),
        ("gpu-compress", "compress"),
        ("gpu-compress-host", "compress-host"),
    ] {
        writeln!(
            manifest,
            "{name} = {{ path = {:?} }}",
            root.join("experiments/gpu").join(path)
        )
        .unwrap();
    }
    for name in ["cuda-device", "cuda-core", "cuda-host"] {
        writeln!(manifest, "{name} = {{ git = \"https://github.com/StarGazerM/cuda-oxide.git\", rev = \"57eacecc081fe1809a815ae4ca32643884dce15b\" }}").unwrap();
    }
    manifest.push_str("\n[profile.release.build-override]\nopt-level = 3\n");
    std::fs::write(directory.join("Cargo.toml"), manifest).unwrap();
    std::fs::write(directory.join("src/main.rs"), source).unwrap();
    let start = std::time::Instant::now();
    let build = gpu_command(root)
        .args([
            "cargo",
            "+1.99.1",
            "oxide",
            "build",
            "--arch",
            "sm_89",
            "--materialize-cubin",
            "--",
            "--release",
            "--manifest-path",
        ])
        .arg(directory.join("Cargo.toml"))
        .status()
        .unwrap();
    let success = build.success()
        && gpu_command(root)
            .arg(root.join("experiments/gpu/target/fuzz-build/release/datalog-generated-batch"))
            .status()
            .unwrap()
            .success();
    eprintln!(
        "GPU FUZZ route={lane} seed={seed} {} source={} Gate time: {:.2} s",
        if success { "PASS" } else { "FAIL" },
        directory.display(),
        start.elapsed().as_secs_f64(),
    );
    success
}

fn gpu_lock(path: &Path) -> std::fs::File {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    lock.lock().unwrap();
    lock
}

fn gpu_command(root: &Path) -> Command {
    let mut command = Command::new("nice");
    command
        .args(["-n", "10"])
        .current_dir(root.join("experiments/gpu"))
        .env(
            "CARGO_TARGET_DIR",
            root.join("experiments/gpu/target/fuzz-build"),
        )
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_BUILD_JOBS", "6")
        .env("CARGO_PROFILE_DEV_DEBUG", "line-tables-only")
        .env("CARGO_PROFILE_TEST_DEBUG", "line-tables-only")
        .env(
            "CUDA_HOME",
            "/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/cuda/13.0",
        )
        .env(
            "BINDGEN_EXTRA_CLANG_ARGS",
            "-I/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/math_libs/13.0/targets/x86_64-linux/include",
        )
        .env(
            "LIBRARY_PATH",
            "/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/math_libs/13.0/targets/x86_64-linux/lib",
        )
        .env(
            "LD_LIBRARY_PATH",
            "/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/math_libs/13.0/targets/x86_64-linux/lib",
        );
    command
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seed = args.next().map(|s| s.parse().unwrap()).unwrap_or(0);
    let count = args.next().map(|s| s.parse().unwrap()).unwrap_or(16);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let cq_passed = run(&root, "r3", seed, cq_source(seed, count));
    let datalog_passed = run(
        &root,
        "d2",
        seed,
        datalog::batch_source(seed, count, &[("gpu", GPU_DL)]),
    );
    assert!(cq_passed && datalog_passed, "GPU fuzz lane failure");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_lock_survives_child_exit_until_guard_is_dropped() {
        let path = std::env::temp_dir().join(format!("cq-gpu-fuzz-{}.lock", std::process::id()));
        let guard = gpu_lock(&path);
        let contender = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        for _ in 0..2 {
            // Model the build and execution children without Cargo or CUDA.
            assert!(Command::new("true").status().unwrap().success());
            assert!(matches!(
                contender.try_lock(),
                Err(std::fs::TryLockError::WouldBlock)
            ));
            // The parent guard must also exclude existing flock-based GPU tools.
            assert_eq!(
                Command::new("flock")
                    .arg("-n")
                    .arg(&path)
                    .arg("true")
                    .status()
                    .unwrap()
                    .code(),
                Some(1),
            );
        }
        drop(guard);
        contender.try_lock().unwrap();
        drop(contender);
        std::fs::remove_file(path).unwrap();
    }
}
