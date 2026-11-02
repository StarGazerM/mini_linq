//! Seeded, finite stratified programs, compiled as one differential batch.
//! The source evaluator is independent of production stratification and printing.
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub struct Program {
    pub seed: u64,
    pub edges: Vec<(i32, i32)>,
    pub rules: String,
    pub idbs: usize,
}

fn pick(state: &mut u64, n: u64) -> usize {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*state >> 32) % n) as usize
}

pub fn generate(seed: u64) -> Program {
    let mut state = seed;
    let idbs = 1 + pick(&mut state, 5);
    let nodes = 2 + pick(&mut state, 5) as i32;
    let mut edges = Vec::new();
    if seed % 8 != 0 {
        edges.push((0, 1));
        for _ in 0..(2 + pick(&mut state, 12)) {
            edges.push((
                pick(&mut state, nodes as u64) as i32,
                pick(&mut state, nodes as u64) as i32,
            ));
        }
        edges.push(edges[0]);
    }
    let mut rules = String::from("R0(*x,*y) :- Edge(x,y);\n");
    // Pick recursion topology independently of data. All computed values stay in
    // the finite node domain, and negative dependencies always point downward.
    match pick(&mut state, 3) {
        0 => rules.push_str("R0(*x,*z) :- R0(x,y), Edge(y,z);\n"),
        1 => rules.push_str("R0(*x,*z) :- R0(x,y), R0(y,z);\n"),
        _ => {
            writeln!(rules, "R0(*x,(*y + 1) % {nodes}) :- R0(x,y);").unwrap();
        }
    }
    for id in 1..idbs {
        let previous = id - 1;
        match pick(&mut state, 4) {
            0 => {
                writeln!(rules, "R{id}(*x,*y) :- Edge(x,y), !R{previous}(y,x);").unwrap();
            }
            1 => {
                writeln!(
                    rules,
                    "R{id}(*x,z) :- R{previous}(x,y), if let Some(z) = Some(*y);"
                )
                .unwrap();
            }
            2 => {
                writeln!(rules, "R{id}(*x,*y) :- R{previous}(x,y);\nR{id}(*x,*z) :- R{id}(x,y), R{previous}(y,z);").unwrap();
            }
            _ => {
                writeln!(
                    rules,
                    "R{id}(*x,*y) :- R{previous}(x,y);\nR{previous}(*x,*y) :- R{id}(x,y);"
                )
                .unwrap();
            }
        }
    }
    // Literal selection, with a random literal rather than a fixed program.
    let literal = pick(&mut state, nodes as u64);
    writeln!(rules, "R0({literal},*y) :- Edge({literal},y);").unwrap();
    Program {
        seed,
        edges,
        rules,
        idbs,
    }
}

pub const CPU_ROUTES: &[(&str, &str)] = &[
    (
        "incremental",
        "print = cq_datalog_test_passes::eval_incremental_from_round;",
    ),
    ("round", "print = cq_datalog_test_passes::eval_round;"),
    ("sequential", ""),
    (
        "rayon",
        "storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_schedule::schedule(relational), cq_datalog_parallel::partition(threshold = 0, grain = 2)]; print = cq_datalog_parallel::print_rounds;",
    ),
    (
        "sharded",
        "storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(sharded))]; round = [cq_datalog_schedule::schedule(whole), cq_datalog_parallel::partition(threshold = 0, grain = 2)]; print = cq_datalog_parallel::print_rounds;",
    ),
    (
        "runs",
        "storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(runs))]; round = [cq_datalog_schedule::schedule(whole), cq_datalog_parallel::partition(threshold = 0, grain = 2)]; print = cq_datalog_parallel::print_rounds;",
    ),
];

impl Program {
    pub fn expression(&self, pipeline: &str) -> String {
        let mut source = format!(
            "cq_datalog::datalog! {{ pipeline {{ {pipeline} }} struct Generated; relation Edge(x:i32,y:i32) = (&edges); "
        );
        for id in 0..self.idbs {
            write!(source, "relation R{id}(x:i32,y:i32); ").unwrap();
        }
        source.push_str("output ");
        source.push_str(
            &(0..self.idbs)
                .map(|id| format!("R{id}"))
                .collect::<Vec<_>>()
                .join(","),
        );
        write!(source, "; {} }}", self.rules).unwrap();
        let bindings = (0..self.idbs)
            .map(|id| format!("r{id}"))
            .collect::<Vec<_>>();
        let binding = if self.idbs == 1 {
            bindings[0].clone()
        } else {
            format!("({})", bindings.join(","))
        };
        format!(
            "{{ let {binding} = {source}; vec![{}] }}",
            bindings
                .iter()
                .map(|b| format!(
                    "{b}.into_iter().collect::<std::collections::BTreeSet<(i32,i32)>>()"
                ))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

pub fn batch_source(seed: u64, count: usize, routes: &[(&str, &str)]) -> String {
    let mut source = String::from("fn main() {\n");
    for offset in 0..count {
        let program = generate(seed.wrapping_add(offset as u64));
        writeln!(source, "if std::env::var(\"CQ_CASE\").map_or(true, |s| s == {:?}) {{ let mut edges: Vec<(i32,i32)> = vec!{:?};", program.seed.to_string(), program.edges).unwrap();
        source.push_str("if let Ok(keep) = std::env::var(\"CQ_KEEP\") { let indices: std::collections::BTreeSet<usize> = keep.split(',').filter_map(|s| s.parse().ok()).collect(); edges = edges.into_iter().enumerate().filter_map(|(i,row)| indices.contains(&i).then_some(row)).collect(); }\n");
        writeln!(
            source,
            "let expected = {};",
            program.expression("print = cq_datalog_test_passes::eval_source_from_round;")
        )
        .unwrap();
        for (route, pipeline) in routes {
            let context = format!(
                "FUZZ FAILURE seed={} route={route} stage={route} rules={}",
                program.seed, program.rules
            );
            writeln!(source, "if std::env::var(\"CQ_ROUTE\").map_or(true, |s| s == {route:?}) {{ assert_eq!({}, expected, \"{{}} data={{:?}}\", {:?}, edges); }}", program.expression(pipeline), context).unwrap();
        }
        source.push_str("}\n");
    }
    source.push_str("}\n");
    source
}

pub fn manifest(root: &Path) -> String {
    let mut manifest = String::from(
        "[package]\nname = \"datalog-generated-batch\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[profile.dev.build-override]\nopt-level = 3\n[dependencies]\nrayon = \"1\"\n",
    );
    for (name, path) in [
        ("cq-datalog", "features/datalog/facade/cq-datalog"),
        ("cq-datalog-testkit", "features/datalog/testkit"),
        ("cq-datalog-test-passes", "features/datalog/test-passes"),
        ("cq-datalog-parallel", "features/datalog-parallel"),
        (
            "cq-datalog-parallel-runtime",
            "features/datalog-parallel/runtime",
        ),
        ("cq-datalog-schedule", "features/datalog/passes/schedule"),
        ("mini-linq", "facade/mini-linq"),
        ("cq-storage", "runtime/cq-storage"),
    ] {
        writeln!(manifest, "{name} = {{ path = {:?} }}", root.join(path)).unwrap();
    }
    manifest
}

pub fn run_cpu(root: &Path, seed: u64, count: usize) {
    run_cpu_routes(root, seed, count, CPU_ROUTES);
}

pub fn run_quick(root: &Path, seed: u64) {
    run_cpu_routes(root, seed, 16, &[CPU_ROUTES[2], CPU_ROUTES[4]]);
}

fn run_cpu_routes(root: &Path, seed: u64, count: usize, routes: &[(&str, &str)]) {
    let started = std::time::Instant::now();
    let directory: PathBuf = root
        .join("target/generated-datalog")
        .join(format!("{seed}-{count}-{}", routes.len()));
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::write(directory.join("Cargo.toml"), manifest(root)).unwrap();
    std::fs::write(
        directory.join("src/main.rs"),
        batch_source(seed, count, routes),
    )
    .unwrap();
    let generation = started.elapsed();
    let build_started = std::time::Instant::now();
    let status = Command::new("nice")
        .args(["-n", "10", "cargo", "build", "--quiet", "--manifest-path"])
        .arg(directory.join("Cargo.toml"))
        .env(
            "CARGO_TARGET_DIR",
            root.join("target/generated-datalog-build"),
        )
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_BUILD_JOBS", "6")
        .env("CARGO_PROFILE_DEV_DEBUG", "line-tables-only")
        .env("CARGO_PROFILE_TEST_DEBUG", "line-tables-only")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "FUZZ FAILURE seed={seed} route=cpu stage=compile source={}",
        directory.display()
    );
    let build = build_started.elapsed();
    let run_started = std::time::Instant::now();
    let status =
        Command::new(root.join("target/generated-datalog-build/debug/datalog-generated-batch"))
            .env("RAYON_NUM_THREADS", "4")
            .status()
            .unwrap();
    eprintln!(
        "D1 seed={seed} programs={count} generation={generation:?} build={build:?} execution={:?}",
        run_started.elapsed()
    );
    if !status.success() {
        if routes.len() != CPU_ROUTES.len() {
            // Compile all recorded boundaries only after an answer mismatch.
            run_cpu(root, seed, count);
        }
        let executable = root.join("target/generated-datalog-build/debug/datalog-generated-batch");
        for offset in 0..count {
            let program = generate(seed.wrapping_add(offset as u64));
            for (route, _) in routes {
                let fails = |keep: &[usize]| {
                    Command::new(&executable)
                        .env("RAYON_NUM_THREADS", "4")
                        .env("CQ_CASE", program.seed.to_string())
                        .env("CQ_ROUTE", route)
                        .env(
                            "CQ_KEEP",
                            keep.iter()
                                .map(usize::to_string)
                                .collect::<Vec<_>>()
                                .join(","),
                        )
                        .output()
                        .map(|o| !o.status.success())
                        .unwrap()
                };
                let mut keep: Vec<_> = (0..program.edges.len()).collect();
                if !fails(&keep) {
                    continue;
                }
                let mut index = 0;
                while index < keep.len() {
                    let mut candidate = keep.clone();
                    candidate.remove(index);
                    if fails(&candidate) {
                        keep = candidate;
                    } else {
                        index += 1;
                    }
                }
                let minimized: Vec<_> = keep.iter().map(|&i| program.edges[i]).collect();
                eprintln!(
                    "FUZZ FAILURE seed={} route={route} stage={route} minimized={minimized:?} rules={}",
                    program.seed, program.rules
                );
                std::fs::write(
                    directory.join(format!("failure-{}-{route}.txt", program.seed)),
                    format!(
                        "seed={} route={route} data={minimized:?}\n{}",
                        program.seed, program.rules
                    ),
                )
                .unwrap();
            }
        }
    }
    assert!(
        status.success(),
        "FUZZ FAILURE seed={seed} route=cpu stage=execute source={}",
        directory.display()
    );
}
