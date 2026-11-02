//! Slow-tier only: `cargo run -p cq-gpu-fuzz -- [seed] [count]` on a CUDA host.
//! All generated device execution is serialized by the shared GPU flock.
use std::{fmt::Write as _, path::Path, process::Command};
#[allow(dead_code)]
#[path = "../../../../features/datalog/oracle/src/random_programs.rs"]
mod datalog;

const GPU_DL: &str = "incremental = cq_datalog::passes::incremental(driver(delta_first);); storage_rewrites = [cq_gpu_datalog::device_storage]; cq = pipeline { physical = [cq_gpu_sequence::capture_constants]; structures = [cq_gpu_datalog::bind_inputs, cq_gpu_sequence::bind_runs]; sequence = mini_linq::passes::sequence; stage = cq_gpu_sequence::stage; decompose = [cq_gpu_sequence::decompose]; grade = cq_gpu_sequence::machine_grade; print = cq_gpu_sequence::translate_arena; }; terminals = [cq_gpu_sequence::translate_arena => batch]; round = [cq_gpu_datalog::bind_round(binding = 5)]; print = cq_gpu_datalog::print_rounds;";
const GPU_CQ: &str = "physical = [cq_gpu_sequence::capture_constants]; structures = [cq_gpu_sequence::bind_runs]; sequence = mini_linq::passes::sequence; stage = cq_gpu_sequence::stage; decompose = [cq_gpu_sequence::decompose]; grade = cq_gpu_sequence::machine_grade; print = cq_gpu_sequence::translate;";

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
        // Both routes run identical CQ-to-Physical passes, including captured
        // constants. Only Structures selection and the backend differ.
        for (route, pipeline) in [
            (
                "cpu-physical",
                "physical = [cq_gpu_sequence::capture_constants];",
            ),
            ("gpu", GPU_CQ),
        ] {
            let context = format!(
                "FUZZ FAILURE seed={} route={route} stage=execute data={:?}",
                seed + offset as u64,
                case.data
            );
            writeln!(source, "let actual: std::collections::BTreeSet<_> = mini_linq::compile! {{ pipeline {{ {pipeline} }} {} }}.into_iter().collect(); assert_eq!(actual, expected, {:?});", case.cq(), context).unwrap();
        }
        source.push_str("}\n");
    }
    source.push_str("}\n");
    source
}

fn run(root: &Path, lane: &str, seed: u64, source: String) {
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
        writeln!(manifest, "{name} = {{ git = \"https://github.com/StarGazerM/cuda-oxide.git\", rev = \"299eda41b287642ef3323144d8fef8f16fe4fbe1\" }}").unwrap();
    }
    std::fs::write(directory.join("Cargo.toml"), manifest).unwrap();
    std::fs::write(directory.join("src/main.rs"), source).unwrap();
    let status = Command::new("flock")
        .args([
            "/tmp/mini_linq-gpu.lock",
            "nice",
            "-n",
            "10",
            "cargo",
            "run",
            "--manifest-path",
        ])
        .arg(directory.join("Cargo.toml"))
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
        )
        .status()
        .unwrap();
    assert!(
        status.success(),
        "FUZZ FAILURE seed={seed} route={lane} stage=batch source={}",
        directory.display()
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seed = args.next().map(|s| s.parse().unwrap()).unwrap_or(0);
    let count = args.next().map(|s| s.parse().unwrap()).unwrap_or(16);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    run(&root, "r3", seed, cq_source(seed, count));
    run(
        &root,
        "d2",
        seed,
        datalog::batch_source(seed, count, &[("gpu", GPU_DL)]),
    );
}
