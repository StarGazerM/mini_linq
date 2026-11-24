//! Every route compares against the independent typed model evaluator. Opaque
//! Rust and generators additionally run a compiled written-order oracle.
//! `--duckdb` adds a SQL cross-check (requires Cargo's `oracle` feature).
//! A panic in any lane fails comparison; successful runs must return equal sets.
//! `xtask fuzz-quick` uses 64 fixed-seed cases on default, Tasks, radix and
//! seeded legal advice. `xtask fuzz-full` defaults to 2,000 cases on every CPU
//! route and each native host kind; its run id is the logged seed (`--seed`
//! replays a run). Both write disposable Cargo consumers under target/fuzz.
//! Each batch is compiled once; runtime data travels over stdin so row
//! minimization does not require recompilation. Every Structures record rejects
//! zero-column built indexes, and counted closed let/guard/head probes run on
//! each route. Failures retain the seed, route, CQ, SQL and minimized input.
//! These are opt-in commands, never part of default tests or regular regression
//! selection. Unit tests below use only fixed tiny cases and never invoke Cargo.
#[cfg(test)]
#[path = "fuzz/compiler.rs"]
mod compiler;
use cq_testkit::fuzz as model;
use cq_testkit::values::Value;
#[path = "fuzz/routes.rs"]
mod routes;
#[path = "fuzz/captures.rs"]
mod captures;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use model::Case;

use crate::Result;

const DEFAULT_SEED: u64 = 0;
const BATCH: usize = 64;
const REFERENCE_OFFSET: usize = 1_000_000;
type Answers = Vec<Vec<Value>>;
type ConsumerResult = std::result::Result<Answers, String>;

#[derive(Clone, Copy, Debug, Default)]
struct Config {
    route: &'static str,
    reference: bool,
    planner: usize,
    merge: bool,
    canon: bool,
    smallest: bool,
    host: usize,
    custom_index: bool,
    lazy: bool,
    push: bool,
}

impl Config {
    fn written_reference() -> Self {
        Self {
            reference: true,
            ..Default::default()
        }
    }
    fn sample(seed: &mut cq_testkit::random::SplitMix64) -> Self {
        fn next(seed: &mut cq_testkit::random::SplitMix64, n: u64) -> usize {
            (seed.next_u64() % n) as usize
        }
        Self {
            route: "advised",
            planner: next(seed, cq_workloads::pipelines::Planner::ALL.len() as u64),
            merge: next(seed, 2) == 1,
            canon: next(seed, 2) == 1,
            smallest: next(seed, 2) == 1,
            host: next(seed, 3),
            custom_index: next(seed, 2) == 1,
            lazy: next(seed, 2) == 1,
            push: next(seed, 2) == 1,
            reference: false,
        }
    }
    fn axes(self) -> [(&'static str, &'static str); 8] {
        [
            (
                "plan",
                cq_workloads::pipelines::Planner::ALL[self.planner].name(),
            ),
            (
                "physical/merge_joins",
                if self.merge { "apply" } else { "omit" },
            ),
            (
                "physical/canon_prefixes",
                if self.canon { "apply" } else { "omit" },
            ),
            (
                "physical/smallest_intersections",
                if self.smallest { "apply" } else { "omit" },
            ),
            ("host", ["slice", "hashmap", "btreemap"][self.host]),
            (
                "structures/sorted_payload",
                if self.custom_index { "apply" } else { "omit" },
            ),
            (
                "physical/lazy_semijoins",
                if self.lazy { "apply" } else { "omit" },
            ),
            (
                "print",
                if self.push {
                    "own_push::push"
                } else {
                    "translate_pull"
                },
            ),
        ]
    }
    fn pipeline(self) -> cq_workloads::pipelines::Pipeline {
        cq_workloads::pipelines::Pipeline {
            plan: self.planner,
            merge: self.merge,
            canon: self.canon,
            smallest: self.smallest,
            sorted_payload: self.custom_index,
            lazy: self.lazy,
            push: self.push,
            positional: self.host == 0,
        }
    }
}

fn route_configuration(route: &str) -> String {
    let (plan, physical, structures, execution) = match route {
        "" | "default" => ("", "", "", ""),
        "par" => ("", "", "", "decompose = [::cq_sequence_tasks_macros::plan]; grade = ::cq_sequence_tasks_macros::tasks; print = ::cq_sequence_tasks_macros::translate_tasks;"),
        "radix" => ("", "", "::cq_rewrites::radix_runs,", ""),
        "radix_retained" => ("", "", "::cq_rewrites::radix_runs, ::cq_rewrites::retain_lookups,", ""),
        "built_hash" => ("", "::cq_rewrites::build_host_reads", "", ""),
        "built_radix" => ("", "::cq_rewrites::build_host_reads", "::cq_rewrites::radix_runs,", ""),
        "no-reduce" | "advised" => ("plan = ::mini_linq::passes::plan(no_reduce);", "", "", ""),
        "transfer" => ("plan = ::cq_planners::order(transfer);", "", "", ""),
        "le" => ("plan = ::cq_planners::order(le);", "", "", ""),
        "le_runs" => ("plan = ::cq_planners::order(le);", "", "::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups,", ""),
        "push" => ("", "", "", "print_input = structures; print = ::own_push::push;"),
        _ => panic!("unrecognized CPU route {route}"),
    };
    format!("pipeline {{ {plan} physical = [{physical}]; structures = [{structures}]; {execution} }}")
}

impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.route != "advised" {
            return write!(f, "route={}, host={}", self.route, ["slice", "hashmap", "btreemap"][self.host]);
        }
        write!(f, "route={}, ", self.route)?;
        for (i, (axis, value)) in self.axes().into_iter().enumerate() {
            if i != 0 {
                write!(f, ", ")?;
            }
            write!(f, "{axis}={value}")?;
        }
        Ok(())
    }
}

// A native host can only satisfy its declared key. Reject these *capability*
// combinations before compilation; all other compiler errors are failures.
fn eligible(case: &Case, c: Config) -> Result<bool> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| eligible_inner(case, c)))
        .unwrap_or_else(|payload| {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic payload");
            Err(format!("compiler panic: {message}").into())
        })
}

fn eligible_inner(case: &Case, c: Config) -> Result<bool> {
    // Fixed routes are checked by their actual compiled pipeline, not by an
    // unrelated sampled planner. Only legal-advice sampling needs preflight.
    if c.route != "advised" {
        return Ok(true);
    }
    let source = syn::parse_str(&case.cq())?;
    Ok(c.pipeline().rejection(&source, c.host)?.is_none())
}

pub fn run(arguments: Vec<String>, root: &Path) -> Result<()> {
    let full = arguments.iter().any(|arg| arg == "--full");
    let (mut count, mut seed) = if full {
        (2000usize, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos() as u64)
    } else {
        (64usize, DEFAULT_SEED)
    };
    let routes = if full { routes::FULL } else { routes::QUICK };
    let mut duckdb = false;
    let mut args = arguments.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--quick" | "--full" => {}
            "--duckdb" => duckdb = true,
            "--cases" => count = args.next().ok_or("--cases requires a number")?.parse()?,
            "--seed" => {
                seed = args
                    .next()
                    .ok_or("--seed requires a decimal u64")?
                    .parse()?
            }
            _ => {
                return Err(
                    format!("unknown fuzz option {arg:?}; expected --cases N --seed S").into(),
                );
            }
        }
    }
    if count == 0 {
        return Err("--cases must be positive".into());
    }
    if duckdb && !cfg!(feature = "oracle") {
        return Err("--duckdb requires building cq-xtask with --features oracle".into());
    }
    let started = Instant::now();
    let mut coverage = model::Coverage::default();
    let mut route_coverage = model::RouteCoverage::default();
    let mut distribution = BTreeMap::<String, usize>::new();
    let mut rejected = 0;
    println!("fuzz: tier={} cases={count} seed={seed} run_id={seed} routes={routes:?} batches={BATCH}", if full { "full" } else { "quick" });
    let progress = root.join("target/fuzz/preflight-current.txt");
    std::fs::create_dir_all(progress.parent().ok_or("preflight parent")?)?;
    std::fs::write(root.join(format!("target/fuzz/run-{seed}.txt")),
        format!("seed={seed}\nrun_id={seed}\ncases={count}\nroutes={routes:?}\n"))?;
    let mut runner = Runner::new(root, duckdb)?;
    let mut compile_time = Duration::ZERO;
    let mut execute_time = Duration::ZERO;
    let mut oracle_time = Duration::ZERO;
    let mut nonempty = 0;
    let mut rust_cases = 0;
    let hosts = if full { 3 } else { 1 };
    let executions = count * routes.len() * hosts;
    for first in (0..count).step_by(BATCH) {
    let mut cases = Vec::with_capacity(BATCH * routes.len() * hosts);
    let mut originals = BTreeMap::new();
    for index in first..(first + BATCH).min(count) {
        let profile = if full { model::Profile::full() } else { model::Profile::quick() };
        let case = model::generate_with(seed.wrapping_add(index as u64), &profile);
        coverage.observe(&case);
        originals.insert(case.seed, case.clone());
        for host in 0..hosts {
        let case = if host == 0 { case.clone() } else { native_case(&case) };
        for &route in routes {
        let mut config_seed = cq_testkit::random::SplitMix64::new(case.seed ^ 0x434f_4e46_4947);
        let mut attempts = 0;
        let config = loop {
            attempts += 1;
            if attempts > 256 {
                return Err(format!("no legal advice at seed {} host={host}", case.seed).into());
            }
            let config = if route == "advised" {
                Config { route, host, push: false, ..Config::sample(&mut config_seed) }
            } else {
                Config { route, host, push: route == "push", ..Default::default() }
            };
            std::fs::write(
                &progress,
                format!(
                    "case {index} seed {}\npipeline {config}\n{}\n",
                    case.seed,
                    case.cq()
                ),
            )?;
            match eligible(&case, config) {
                Ok(true) => break config,
                Ok(false) if route == "advised" => rejected += 1,
                Ok(false) => return Err(format!("route capability rejected: seed={} {config}", case.seed).into()),
                Err(error) => {
                    let fingerprint = diagnostic(&*error);
                    let mut minimal = case.clone();
                    for rows in &mut minimal.data {
                        rows.clear();
                    }
                    shrink_atoms(&mut minimal, |candidate| {
                        eligible(candidate, config)
                            .is_err_and(|error| diagnostic(&*error) == fingerprint)
                    });
                    report(
                        root,
                        &minimal,
                        config,
                        "preflight",
                        &format!("compiler preflight: {error}"),
                    )?;
                    return Err(format!(
                        "fuzz compiler failure at case {index}, seed {}",
                        case.seed
                    )
                    .into());
                }
            }
        };
        *distribution.entry(format!("route/{route}/host/{host}")).or_default() += 1;
        if route == "advised" {
            for (axis, value) in config.axes() {
                *distribution.entry(format!("advised/{axis}/{value}")).or_default() += 1;
            }
        }
        cases.push((case.clone(), config));
        }
        }
    }
    let batch_size = BATCH * routes.len();
    for (batch_index, batch) in cases.chunks(batch_size).enumerate() {
        let offset = first * routes.len() * hosts + batch_index * batch_size;
        let started = Instant::now();
        if let Err(error) = runner.compile(batch) {
            let (mut minimal, config) = match runner.isolate_compile_failure(batch) {
                Ok(case) => case,
                Err(isolation) => {
                    for (case, config) in batch {
                        report(
                            root,
                            case,
                            *config,
                            "cpu-compile",
                            &format!(
                                "batch-only compilation failure; could not minimize independently: {isolation}\n{error}"
                            ),
                        )?;
                    }
                    return Err(isolation);
                }
            };
            let fingerprint =
                diagnostic(&*runner.compile(&[(minimal.clone(), config)]).unwrap_err());
            for rows in &mut minimal.data {
                rows.clear();
            }
            shrink_atoms(&mut minimal, |candidate| {
                matches!(eligible(candidate, config), Ok(true))
                    && runner
                        .compile(&[(candidate.clone(), config)])
                        .is_err_and(|error| diagnostic(&*error) == fingerprint)
            });
            let minimized_error = runner.compile(&[(minimal.clone(), config)]).unwrap_err();
            report(
                root,
                &minimal,
                config,
                "cpu-compile",
                &format!("Rust compilation: {minimized_error}"),
            )?;
            return Err(format!("fuzz Rust compilation failed; seed {}", minimal.seed).into());
        }
        compile_time += started.elapsed();
        let started = Instant::now();
        let actual = match runner
            .execute(batch.iter().enumerate().map(|(id, (case, _))| (id, case)))
        {
            Ok(actual) => actual,
            Err(error) => {
                for (id, (case, config)) in batch.iter().enumerate() {
                    if runner.execute(std::iter::once((id, case))).is_err() {
                        let mut minimal = case.clone();
                        shrink_rows(&mut minimal, |candidate| {
                            runner.execute(std::iter::once((id, candidate))).is_err()
                        });
                        report(
                            root,
                            &minimal,
                            *config,
                            "cpu-execution",
                            &format!("consumer execution: {error}"),
                        )?;
                        return Err(format!("fuzz consumer failure at seed {}", case.seed).into());
                    }
                }
                // If only a batch reproduces it, retain all implicated queries
                // instead of pretending a single-case minimization succeeded.
                for (case, config) in batch {
                    report(
                        root,
                        case,
                        *config,
                        "cpu-execution",
                        &format!(
                            "batch-only consumer failure; could not minimize independently: {error}"
                        ),
                    )?;
                }
                return Err(error);
            }
        };
        execute_time += started.elapsed();
        for (id, (case, config)) in batch.iter().enumerate() {
            let started = Instant::now();
            let expected = match runner.reference(id, case) {
                Ok(rows) => rows,
                Err(error) => {
                    let mut minimal = case.clone();
                    loop {
                        let before = size(&minimal);
                        shrink_rows(&mut minimal, |candidate| {
                            runner.reference(id, candidate).is_err()
                        });
                        if !minimal.requires_rust_oracle() {
                            shrink_atoms(&mut minimal, |candidate| reference(candidate, runner.duckdb).is_err());
                        }
                        if size(&minimal) == before {
                            break;
                        }
                    }
                    report(
                        root,
                        &minimal,
                        *config,
                        "reference",
                        &format!("reference evaluation: {error}"),
                    )?;
                    return Err(format!("fuzz oracle failure at seed {}", case.seed).into());
                }
            };
            oracle_time += started.elapsed();
            nonempty += usize::from(expected.as_ref().is_ok_and(|rows| !rows.is_empty()));
            rust_cases += usize::from(case.requires_rust_oracle());
            let got = &actual[id];
            if !refines(got, &expected) {
                let mut minimal = case.clone();
                shrink_rows(&mut minimal, |candidate| runner.mismatch(id, candidate));
                // Atom shrinking recompiles candidates. Each accepted candidate
                // gets a fresh runtime comparison against independently printed SQL.
                loop {
                    let before = size(&minimal);
                    shrink_atoms(&mut minimal, |candidate| {
                        matches!(eligible(candidate, *config), Ok(true))
                            && runner.compile(&[(candidate.clone(), *config)]).is_ok()
                            && runner.mismatch(0, candidate)
                    });
                    runner.compile(&[(minimal.clone(), *config)])?;
                    shrink_rows(&mut minimal, |candidate| runner.mismatch(0, candidate));
                    if size(&minimal) == before {
                        break;
                    }
                }
                let expected = runner.reference(0, &minimal)?;
                let got = runner.execute(std::iter::once((0, &minimal)))?;
                report(
                    root,
                    &minimal,
                    *config,
                    "cpu-execution",
                    &format!("expected {expected:?}, actual {got:?}"),
                )?;
                return Err(
                    format!("fuzz mismatch at case {}, seed {}", offset + id, case.seed).into(),
                );
            }
            route_coverage.record(config.route, &originals[&case.seed], true);
        }
        println!(
            "fuzz: {}/{} passed; compilation {:.2}s, execution {:.2}s, references {:.2}s",
            offset + batch.len(),
            executions,
            compile_time.as_secs_f64(),
            execute_time.as_secs_f64(),
            oracle_time.as_secs_f64()
        );
    }
    }
    println!("query distribution: {coverage}");
    println!("configuration distribution: {distribution:?}");
    println!("features by route:\n{route_coverage}");
    println!(
        "fuzz: passed {count} cases / {executions} route-host executions; {nonempty} nonempty reference answers; {rust_cases} Rust oracle executions; {rejected} unavailable advice configurations resampled; total {:.2}s (compile {:.2}s, execute {:.2}s, references {:.2}s)",
        started.elapsed().as_secs_f64(),
        compile_time.as_secs_f64(),
        execute_time.as_secs_f64(),
        oracle_time.as_secs_f64()
    );
    Ok(())
}

fn native_case(source: &Case) -> Case {
    let mut case = source.clone();
    for (relation, schema) in source.query.schema.iter().enumerate() {
        if !schema.key.is_empty() {
            let mut order = schema.key.clone();
            order.extend((0..schema.arity()).filter(|column| !schema.key.contains(column)));
            case = case.reorder_columns(relation, &order);
        }
    }
    case
}

fn reference(case: &Case, duckdb: bool) -> Result<Answers> {
    let expected = case.reference().into_iter().collect::<Answers>();
    #[cfg(not(feature = "oracle"))]
    let _ = duckdb;
    #[cfg(feature = "oracle")]
    if duckdb && !case.requires_rust_oracle() {
        let sql = cq_testkit::duckdb::evaluate_fuzz(case).map_err(crate::testkit_error)?
            .into_iter().collect::<Answers>();
        if sql != expected {
            return Err(format!("reference oracles disagree: model={expected:?}, DuckDB={sql:?}").into());
        }
    }
    Ok(expected)
}

fn refines(target: &ConsumerResult, written: &ConsumerResult) -> bool {
    match written {
        Ok(expected) => target.as_ref().ok() == Some(expected),
        Err(_) => false,
    }
}

fn shrink_rows(case: &mut Case, mut fails: impl FnMut(&Case) -> bool) {
    'restart: loop {
        for relation in 0..case.data.len() {
            for row in 0..case.data[relation].len() {
                let mut candidate = case.clone();
                candidate.data[relation].remove(row);
                if fails(&candidate) {
                    *case = candidate;
                    continue 'restart;
                }
            }
        }
        break;
    }
}

fn shrink_atoms(case: &mut Case, mut fails: impl FnMut(&Case) -> bool) {
    let mut index = 0;
    while index < case.query.atoms.len() {
        if let Some(candidate) = case.without_atom(index) {
            if fails(&candidate) {
                *case = candidate;
                index = 0;
                continue;
            }
        }
        index += 1;
    }
}

fn size(case: &Case) -> (usize, usize) {
    (case.query.atoms.len(), case.data.iter().map(Vec::len).sum())
}

fn report(root: &Path, case: &Case, config: Config, stage: &str, reason: &str) -> Result<()> {
    let dir = root
        .join("target/fuzz/failures")
        .join(case.seed.to_string())
        .join(format!("{}-host{}-{stage}", config.route, config.host));
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("query.cq"), case.cq())?;
    std::fs::write(dir.join("query.sql"), case.sql())?;
    std::fs::write(dir.join("data.txt"), input_line(0, case))?;
    std::fs::write(
        dir.join("configuration.txt"),
        format!("seed={}\nstage={stage}\n{config}\n{reason}\n", case.seed),
    )?;
    eprintln!(
        "FUZZ FAILURE seed={} route={} stage={stage}\nconfiguration: {config}\nCQ:\n{}\nSQL:\n{}\nminimized dataset: {:?}\n{reason}\nartifacts: {}\nreproduce original: cargo xtask fuzz-full --cases 1 --seed {}",
        case.seed,
        config.route,
        case.cq(),
        case.sql(),
        case.data,
        dir.display(),
        case.seed
    );
    Ok(())
}

fn diagnostic(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    let line = text
        .lines()
        .find(|line| line.starts_with("error[") || line.starts_with("error:"))
        .or_else(|| text.lines().next())
        .unwrap_or("");
    // Removing an atom changes tuple widths and generated identifiers. Keep
    // the diagnostic kind and trait obligation, not those incidental details.
    line.split('`')
        .enumerate()
        .map(|(index, part)| {
            if index % 2 == 0 {
                part.to_owned()
            } else if let Some((_, obligation)) = part.rsplit_once(": ") {
                format!("`_: {obligation}`")
            } else {
                "`_`".to_owned()
            }
        })
        .collect()
}

fn input_line(id: usize, case: &Case) -> String {
    let mut line = format!("{id} {}", case.data.len());
    for (relation, rows) in case.query.schema.iter().zip(&case.data) {
        write!(line, " {} {}", relation.arity(), rows.len()).unwrap();
        for row in rows {
            for value in row {
                write!(line, " {value}").unwrap();
            }
        }
    }
    line.push('\n');
    line
}

struct Runner {
    duckdb: bool,
    directory: PathBuf,
    target: PathBuf,
    executable: PathBuf,
}

impl Runner {
    fn new(root: &Path, duckdb: bool) -> Result<Self> {
        let parent = root.join("target/fuzz");
        std::fs::create_dir_all(&parent)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let mut nonce = 0;
        let (directory, package) = loop {
            let package = format!("cq-fuzz-consumer-{stamp}-{nonce}");
            let directory = parent.join(&package);
            match std::fs::create_dir(&directory) {
                Ok(()) => break (directory, package),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => nonce += 1,
                Err(error) => return Err(error.into()),
            }
        };
        let target = root.join("target/fuzz-build");
        let executable = target.join("debug").join(&package);
        std::fs::create_dir_all(directory.join("src"))?;
        std::fs::create_dir_all(directory.join("planner/src"))?;
        let mut manifest = format!(
            "[package]\nname = {package:?}\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[profile.dev]\ndebug = 0\n[dependencies]\ncq-fuzz-planner = {{ path = \"planner\" }}\n",
        );
        manifest.push_str(&crate::cargo::dependencies(&[
            "mini-linq",
            "cq-storage",
            "cq-load",
            "cq-workloads",
            "cq-planners",
            "cq-rewrites",
            "cq-workload-passes",
            "own-push",
        ])?);
        manifest.push_str(&format!(
            "rayon = \"1\"\ncq-sequence-tasks = {{ path = {:?} }}\ncq-sequence-tasks-macros = {{ path = {:?} }}\n",
            root.join("../sequence-tasks"), root.join("../sequence-tasks/macros")
        ));
        std::fs::write(directory.join("Cargo.toml"), manifest)?;
        let mut manifest = String::from(
            "[package]\nname = \"cq-fuzz-planner\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\nproc-macro = true\n[dependencies]\nsyn = { version = \"2\", features = [\"full\"] }\nquote = \"1\"\n",
        );
        manifest.push_str(&crate::cargo::dependencies(&[
            "cq-stage",
            "cq-lang-cq",
            "cq-lang-ra",
            "cq-order",
            "cq-reduce",
            "cq-relational-lowering",
            "cq-planners-plan",
        ])?);
        std::fs::write(directory.join("planner/Cargo.toml"), manifest)?;
        std::fs::copy(crate::cargo::lockfile()?, directory.join("Cargo.lock"))?;
        std::fs::write(
            directory.join("planner/src/compiler.rs"),
            include_str!("fuzz/compiler.rs"),
        )?;
        std::fs::write(directory.join("planner/src/lib.rs"), PLANNER)?;
        Ok(Self {
            duckdb,
            directory,
            target,
            executable,
        })
    }

    fn compile(&mut self, cases: &[(Case, Config)]) -> Result<()> {
        let mut source = String::from(
            "#![recursion_limit = \"512\"]\n#![allow(unused_imports, unused_variables, dead_code, unused_macros, unused_parens)]\n",
        );
        source.push_str(include_str!("fuzz/harness.rs"));
        let mut counter_routes = std::collections::BTreeSet::new();
        for (_, config) in cases {
            if !config.reference && counter_routes.insert(config.route) {
                source.push_str(&counter_probe(config.route));
            }
        }
        source.push_str("\nfn dispatch(id: usize, data: &Data) -> Vec<Vec<String>> { match id {\n");
        let mut reference_ids = BTreeMap::new();
        for (id, (case, _)) in cases.iter().enumerate() {
            let route = cases[id].1.route.replace('-', "_");
            writeln!(source, "{id} => {{ counter_{route}(); case_{id}::run(data) }},").unwrap();
            if case.requires_rust_oracle() {
                let reference_id = *reference_ids.entry((case.seed, cases[id].1.host))
                    .or_insert(id + REFERENCE_OFFSET);
                writeln!(source, "{} => case_{reference_id}::run(data),", id + REFERENCE_OFFSET).unwrap();
            }
        }
        source.push_str("_ => panic!(\"unknown case {id}\"), } }\n");
        for (id, (case, config)) in cases.iter().enumerate() {
            source.push_str(&consumer_case(id, case, *config));
            if case.requires_rust_oracle()
                && reference_ids[&(case.seed, config.host)] == id + REFERENCE_OFFSET {
                source.push_str(&consumer_case(
                    id + REFERENCE_OFFSET,
                    case,
                    Config::written_reference(),
                ));
            }
        }
        std::fs::write(self.directory.join("src/main.rs"), source)?;
        let output = crate::cargo::command()
            .args([
                "build",
                "--offline",
                "--quiet",
                "--jobs",
                "6",
                "--manifest-path",
            ])
            .arg(self.directory.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(&self.target)
            .output()?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
        }
        Ok(())
    }

    fn isolate_compile_failure(&mut self, cases: &[(Case, Config)]) -> Result<(Case, Config)> {
        let mut candidates = cases.to_vec();
        while candidates.len() > 1 {
            let half = candidates.len() / 2;
            if self.compile(&candidates[..half]).is_err() {
                candidates.truncate(half);
            } else {
                candidates.drain(..half);
            }
        }
        if self.compile(&candidates).is_ok() {
            return Err("batch compilation failed but no individual case reproduced it".into());
        }
        Ok(candidates.remove(0))
    }

    fn execute<'a>(
        &self,
        cases: impl Iterator<Item = (usize, &'a Case)>,
    ) -> Result<Vec<ConsumerResult>> {
        let cases = cases.collect::<Vec<_>>();
        let input = cases
            .iter()
            .map(|(id, case)| input_line(*id, case))
            .collect::<String>();
        let input_path = self.directory.join("data.txt");
        std::fs::write(&input_path, input)?;
        let output = Command::new(&self.executable)
            .stdin(Stdio::from(std::fs::File::open(input_path)?))
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "consumer exited {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let text = String::from_utf8(output.stdout)?;
        if text.lines().count() != cases.len() {
            return Err(format!("consumer returned wrong number of cases: {text}").into());
        }
        text.lines()
            .zip(cases)
            .map(|(line, (id, case))| {
                let mut words = line.split_whitespace();
                if words.next() != Some(&id.to_string()) {
                    return Err(format!("unexpected consumer id: {line}").into());
                }
                let status = words.next();
                if status == Some("INVARIANT") {
                    return Ok(Err(format!("INVARIANT {}", words.next().ok_or("missing invariant rule")?)));
                }
                if matches!(status, Some("PANIC" | "DUPLICATES")) {
                    return Ok(Err(status.unwrap().into()));
                }
                if status != Some("OK") {
                    return Err(format!("unknown consumer result: {line}").into());
                }
                let count: usize = words.next().ok_or("missing result row count")?.parse()?;
                let types = case.output_types();
                let values = words.enumerate()
                    .map(|(index, text)| types[index % types.len()].column.parse(text))
                    .collect::<Vec<_>>();
                if values.len() != count * case.output_arity() {
                    return Err(format!("invalid consumer result: {line}").into());
                }
                if case.output_arity() == 0 {
                    return Ok(Ok(vec![Vec::new(); count]));
                }
                let mut rows = values.chunks(case.output_arity())
                    .map(<[Value]>::to_vec).collect::<Vec<_>>();
                rows.sort();
                Ok(Ok(rows))
            })
            .collect()
    }

    fn mismatch(&self, id: usize, case: &Case) -> bool {
        match (
            self.execute(std::iter::once((id, case))),
            self.reference(id, case),
        ) {
            (Ok(actual), Ok(expected)) => !refines(&actual[0], &expected),
            _ => false,
        }
    }

    fn reference(&self, id: usize, case: &Case) -> Result<ConsumerResult> {
        let expected = reference(case, self.duckdb)?;
        if case.requires_rust_oracle() {
            let written = self.execute(std::iter::once((id + REFERENCE_OFFSET, case)))?.remove(0);
            if written.as_ref().ok() != Some(&expected) {
                return Err(format!("reference oracles disagree: model={expected:?}, written={written:?}").into());
            }
        }
        Ok(Ok(expected))
    }
}

fn counter_probe(route: &str) -> String {
    let configuration = route_configuration(route);
    let name = route.replace('-', "_");
    format!(r#"
fn counter_{name}() {{
    static COUNTS: [std::sync::atomic::AtomicUsize; 3] = [const {{ std::sync::atomic::AtomicUsize::new(0) }}; 3];
    fn closed_value(slot: usize) -> i32 {{
        COUNTS[slot].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        7
    }}
    for count in &COUNTS {{ count.store(0, std::sync::atomic::Ordering::SeqCst); }}
    let input = (0i32..128).map(|x| (x,)).collect::<Vec<_>>();
    let R = input.as_slice();
    let rows: Vec<_> = ::mini_linq::compile! {{
        {configuration}
        struct ClosedCapture;
        relation R(x: i32);
        relation answer(x: i32, captured: i32, value: i32);
        answer(x, c, closed_value(2)) :- R(x), let c: i32 = closed_value(0),
            if closed_value(1) == 7;
    }}.map(|(x, c, value)| (crate::integer(x), crate::integer(c), crate::integer(value))).collect();
    assert_eq!(rows.into_iter().collect::<std::collections::BTreeSet<_>>(),
        (0i32..128).map(|x| (x, 7, 7)).collect(), "C2-ANSWERS route={route}");
    for count in &COUNTS {{
        crate::invariant(count.load(std::sync::atomic::Ordering::SeqCst) == 1, "C2-CLOSED-CAPTURE");
    }}
}}
"#)
}

fn consumer_case(id: usize, case: &Case, c: Config) -> String {
    let mut code = format!("\nmod case_{id} {{\n");
    let (cq, slots) = if c.reference { (case.written_cq(), 0) } else { captures::instrument(&case.cq()) };
    writeln!(code, "static CAPTURES: [std::sync::atomic::AtomicUsize; {slots}] = [const {{ std::sync::atomic::AtomicUsize::new(0) }}; {slots}];\nfn count_capture<T>(slot: usize, value: T) -> T {{ CAPTURES[slot].fetch_add(1, std::sync::atomic::Ordering::Relaxed); value }}").unwrap();
    code.push_str("pub(super) fn run(data: &crate::Data) -> Vec<Vec<String>> {\n");
    code.push_str("for count in &CAPTURES { count.store(0, std::sync::atomic::Ordering::Relaxed); }\n");
    for (i, relation) in case.query.schema.iter().enumerate() {
        let fields = relation.types.iter().enumerate()
            .map(|(j, ty)| format!("row[{j}].parse::<{}>().unwrap(),", ty.rust()))
            .collect::<String>();
        if c.host == 0 {
            writeln!(code, "let input_{i} = data[{i}].iter().map(|row| ({fields})).collect::<Vec<_>>();\nlet {} = input_{i}.as_slice();", relation.name).unwrap();
        } else {
            let keys = relation.key.len().max(1);
            let key_types = relation.types[..keys].iter().map(|ty| format!("{},", ty.rust())).collect::<String>();
            let key = relation.types[..keys].iter().enumerate().map(|(j, ty)| format!("row[{j}].parse::<{}>().unwrap(),", ty.rust())).collect::<String>();
            let tail = relation.types.iter().enumerate().skip(keys)
                .map(|(j, ty)| format!("row[{j}].parse::<{}>().unwrap(),", ty.rust()))
                .collect::<String>();
            writeln!(code, "let mut input_{i} = std::collections::{}::<({key_types}), Vec<_>>::new();\nfor row in &data[{i}] {{ input_{i}.entry(({key})).or_default().push(({tail})); }}\nlet {} = &input_{i};", if c.host == 1 { "HashMap" } else { "BTreeMap" }, relation.name).unwrap();
        }
    }
    for (name, value) in case.parameters() {
        writeln!(code, "let {name}: {} = {};", value.column_type().rust(), value.rust_literal()).unwrap();
    }
    let fields = case.output_types().iter().enumerate()
        .map(|(i, ty)| if ty.optional { format!("row.{i}.as_ref().expect(\"nonempty aggregate group\").to_string(),") } else { format!("row.{i}.to_string(),") })
        .collect::<String>();
    let invocation = if c.reference {
        format!(
            "::mini_linq::compile! {{ pipeline {{ plan = cq_fuzz_planner::written_reference; }} {} }}",
            cq
        )
    } else {
        let configuration = if c.route == "advised" {
            let source = syn::parse_str(&cq).expect("preflight checked CQ");
            c.pipeline().configuration(&source).expect("checked pipeline").to_string()
        } else {
            route_configuration(c.route)
        };
        format!("::mini_linq::compile! {{ {configuration} {cq} }}")
    };
    writeln!(
        code,
        "let answers = {invocation}.map(|row| vec![{fields}]).collect();\nfor count in &CAPTURES {{ crate::invariant(count.load(std::sync::atomic::Ordering::Relaxed) <= 1, \"C2-CLOSED-CAPTURE\"); }}\nanswers\n}}\n}}"
    )
    .unwrap();

    code
}

const PLANNER: &str = r#"
mod compiler;
#[proc_macro]
pub fn written_reference(tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_lang_cq::Program, cq_lang_ra::Program>(tokens.into(), |source, arguments| {
        if !arguments.is_empty() { return Err(syn::Error::new_spanned(arguments, "written_reference takes no arguments")); }
        compiler::written(source)
    }).into()
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refinement_requires_total_execution_and_set_equality() {
        let rows = Ok(vec![vec![Value::I32(1)]]);
        let empty = Ok(Vec::new());
        let panic = Err("PANIC".into());
        let duplicates = Err("DUPLICATES".into());
        assert!(refines(&rows, &rows));
        assert!(!refines(&rows, &panic));
        assert!(!refines(&panic, &panic));
        assert!(!refines(&panic, &rows));
        assert!(!refines(&empty, &rows));
        assert!(!refines(&duplicates, &panic));
        assert!(!refines(&rows, &duplicates));
    }

    #[test]
    fn rust_reference_uses_written_order_and_disables_every_reordering_option() {
        let config = Config::written_reference();
        assert_eq!(config.planner, 0);
        assert!(config.reference);
        let pipeline = config.pipeline();
        assert_eq!(pipeline.plan, 0);
        assert!(!pipeline.merge && !pipeline.canon && !pipeline.smallest);
        let case = model::generate_with(0, &model::Profile { max_rows: 3, ..model::Profile::quick() });
        assert!(case.requires_rust_oracle());
        assert!(reference(&case, false).is_ok());
    }

    #[test]
    fn total_generator_still_uses_compiled_reference_not_duckdb() {
        let mut case = model::generate_with(60, &model::Profile { max_rows: 3, ..model::Profile::quick() });
        assert!(case.query.bindings[0].generator.is_some());
        case.query.bindings[0].expression = model::Expr::Constant(2);
        assert!(case.requires_rust_oracle());
        assert!(reference(&case, false).is_ok());
        let query = syn::parse_str(&case.written_cq()).unwrap();
        let reference = compiler::written(&query).unwrap();
        assert!(reference.plan.definitions.iter().any(|definition| {
            matches!(&definition.operator, cq_lang_ra::Operator::Bind(binding)
                if binding.for_token.is_some())
        }));
    }

    #[test]
    fn written_reference_preserves_scalar_and_nested_generator_markers() {
        let source = syn::parse_quote! {
            struct Fuzz;
            relation R(x: i32);
            relation S(y: i32);
            relation answer(x: i32, y: i32, z: i32);
            answer(x, y, z) :- R(x), let end: i32 = x.to_owned(),
                for y in 0i32..end.to_owned(), S(y),
                for z in vec![y.to_owned(), y.to_owned()];
        };
        let reference = compiler::written(&source).unwrap();
        let markers = reference
            .plan
            .definitions
            .iter()
            .filter_map(|definition| match &definition.operator {
                cq_lang_ra::Operator::Bind(binding) => Some(binding.for_token.is_some()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(markers, [false, true, true]);
    }

    #[test]
    fn compiler_shrinking_preserves_trait_failure_when_tuple_width_changes() {
        assert_eq!(
            diagnostic("error[E0277]: the trait bound `(i32, i32): Ord` is not satisfied"),
            diagnostic("error[E0277]: the trait bound `(i32, i32, i32): Ord` is not satisfied"),
        );
        assert_ne!(
            diagnostic("error[E0277]: the trait bound `(i32, i32): Ord` is not satisfied"),
            diagnostic("error[E0277]: the trait bound `(i32, i32): Hash` is not satisfied"),
        );
    }

    #[test]
    fn row_shrinking_keeps_only_the_witness() {
        let mut case = model::generate_with(1, &model::Profile { max_rows: 3, ..model::Profile::quick() });
        case.data[0] = vec![
            vec![Value::I32(3); case.query.schema[0].arity()],
            vec![Value::I32(7); case.query.schema[0].arity()],
        ];
        shrink_rows(&mut case, |candidate| {
            candidate.data[0].iter().any(|row| row[0] == Value::I32(7))
        });
        assert_eq!(case.data[0].len(), 1);
        assert_eq!(case.data[0][0][0], Value::I32(7));
        assert!(case.data.iter().skip(1).all(Vec::is_empty));
    }

    #[test]
    fn row_shrinking_revisits_earlier_rows_after_later_deletions() {
        let mut case = model::generate_with(1, &model::Profile { max_rows: 3, ..model::Profile::quick() });
        case.data[0] = (0..3)
            .map(|value| vec![Value::I32(value); case.query.schema[0].arity()])
            .collect();
        shrink_rows(&mut case, |candidate| {
            let contains = |value| candidate.data[0].iter().any(|row| row[0] == Value::I32(value));
            contains(2) && (contains(0) || !contains(1))
        });
        assert_eq!(case.data[0], vec![vec![Value::I32(2); case.query.schema[0].arity()]]);
    }

    #[test]
    fn atom_shrinking_revisits_earlier_atoms_after_later_deletions() {
        let mut case = Case {
            seed: 0,
            query: model::Query {
                schema: (0..3)
                    .map(|i| model::Relation::i32(format!("r{i}"), 1))
                    .collect(),
                atoms: (0..3)
                    .map(|relation| model::Atom {
                        relation,
                        terms: vec![model::Term::Var(0)],
                    })
                    .collect(),
                negations: Vec::new(),
                folds: Vec::new(),
                raw_names: false,
                bindings: Vec::new(),
                guards: Vec::new(),
                constraints: Vec::new(),
                blocks: Vec::new(),
                head: vec![model::Expr::Var(0)],
                order: Vec::new(),
            },
            data: vec![Vec::new(); 3],
        };
        shrink_atoms(&mut case, |candidate| {
            let contains = |relation| {
                candidate
                    .query
                    .atoms
                    .iter()
                    .any(|atom| atom.relation == relation)
            };
            contains(2) && (contains(0) || !contains(1))
        });
        assert_eq!(case.query.atoms.len(), 1);
        assert_eq!(case.query.atoms[0].relation, 2);
    }

    #[test]
    fn native_unsorted_key_permutation_preserves_the_answer() {
        use cq_testkit::values::ColumnType;
        let case = Case {
            seed: 0,
            query: model::Query {
                schema: vec![model::Relation {
                    name: "r".into(),
                    types: vec![ColumnType::I64, ColumnType::I32, ColumnType::I64],
                    key: vec![2, 0],
                }],
                atoms: vec![model::Atom { relation: 0, terms: (0..3).map(model::Term::Var).collect() }],
                negations: vec![], bindings: vec![], guards: vec![],
                constraints: vec![], blocks: vec![], folds: vec![],
                head: (0..3).map(model::Expr::Var).collect(),
                order: vec![], raw_names: false,
            },
            data: vec![vec![vec![Value::I64(1 << 32), Value::I32(7), Value::I64(-3)]]],
        };
        let native = native_case(&case);
        assert_eq!(native.query.schema[0].key, [0, 1]);
        assert_eq!(native.data[0][0], [Value::I64(-3), Value::I64(1 << 32), Value::I32(7)]);
        assert_eq!(native.reference(), case.reference());
    }
}
