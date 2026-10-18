//! Total scalar queries use independently printed DuckDB SQL. Opaque partial
//! terms and all Rust generators use a second compiled query in written order
//! with reordering disabled, including panics during iterator advancement.
//! The denotation is `bottom + Set`: a target must return the same set without
//! panic whenever the written reference succeeds; a reference panic permits
//! either outcome. I/O, mutation and evaluation count are outside this meaning.
//! `xtask fuzz` writes a disposable Cargo consumer under target/fuzz. Its small
//! proc macro runs the selected logical passes, then calls the public facade
//! for physical lowering and realization. Each batch is compiled once; runtime
//! data travels over stdin so row minimization does not require recompilation.
#[cfg(test)]
#[path = "fuzz/compiler.rs"]
mod compiler;
use cq_testkit::fuzz as model;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use model::Case;

use crate::Result;

const DEFAULT_SEED: u64 = 0;
const BATCH: usize = 64;
type Answers = Vec<Vec<i32>>;
type ConsumerResult = std::result::Result<Answers, String>;

#[derive(Clone, Copy, Debug, Default)]
struct Config {
    reference: bool,
    planner: usize,
    merge: bool,
    canon: bool,
    smallest: bool,
    host: usize,
    ordered_distinct: bool,
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
            planner: next(seed, cq_workloads::pipelines::Planner::ALL.len() as u64),
            merge: next(seed, 2) == 1,
            canon: next(seed, 2) == 1,
            smallest: next(seed, 2) == 1,
            host: next(seed, 3),
            ordered_distinct: next(seed, 2) == 1,
            custom_index: next(seed, 2) == 1,
            lazy: next(seed, 2) == 1,
            push: next(seed, 2) == 1,
            reference: false,
        }
    }
    fn axes(self) -> [(&'static str, &'static str); 9] {
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
                "structures/ordered_dedup",
                if self.ordered_distinct {
                    "ordered-dedup"
                } else {
                    "default"
                },
            ),
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
                    "print_pull"
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
            ordered_dedup: self.ordered_distinct,
            sorted_payload: self.custom_index,
            lazy: self.lazy,
            push: self.push,
            positional: self.host == 0,
        }
    }
}

impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    let source = syn::parse_str(&case.cq())?;
    Ok(c.pipeline().rejection(&source, c.host)?.is_none())
}

pub fn run(arguments: Vec<String>, root: &Path) -> Result<()> {
    let (mut count, mut seed) = (200usize, DEFAULT_SEED);
    let mut args = arguments.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
    let started = Instant::now();
    let mut coverage = model::Coverage::default();
    let mut distribution = BTreeMap::<String, usize>::new();
    let mut rejected = 0;
    let mut cases = Vec::with_capacity(count);
    println!("fuzz: {count} cases, seed {seed}, batches of {BATCH}");
    let progress = root.join("target/fuzz/preflight-current.txt");
    std::fs::create_dir_all(progress.parent().ok_or("preflight parent")?)?;
    for index in 0..count {
        let case = model::generate(seed.wrapping_add(index as u64));
        let mut config_seed = cq_testkit::random::SplitMix64::new(case.seed ^ 0x434f_4e46_4947);
        let config = loop {
            let config = Config::sample(&mut config_seed);
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
                Ok(false) => rejected += 1,
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
        coverage.observe(&case);
        for (axis, value) in config.axes() {
            *distribution.entry(format!("{axis}/{value}")).or_default() += 1;
        }
        cases.push((case, config));
    }
    let mut runner = Runner::new(root)?;
    let mut compile_time = Duration::ZERO;
    let mut execute_time = Duration::ZERO;
    let mut oracle_time = Duration::ZERO;
    let mut nonempty = 0;
    let mut partial_cases = 0;
    let mut reference_panics = 0;
    for (batch_index, batch) in cases.chunks(BATCH).enumerate() {
        let offset = batch_index * BATCH;
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
                        if !minimal.has_partial_terms() {
                            shrink_atoms(&mut minimal, |candidate| reference(candidate).is_err());
                        }
                        if size(&minimal) == before {
                            break;
                        }
                    }
                    report(
                        root,
                        &minimal,
                        *config,
                        &format!("reference evaluation: {error}"),
                    )?;
                    return Err(format!("fuzz oracle failure at seed {}", case.seed).into());
                }
            };
            oracle_time += started.elapsed();
            nonempty += usize::from(expected.as_ref().is_ok_and(|rows| !rows.is_empty()));
            partial_cases += usize::from(case.has_partial_terms());
            reference_panics +=
                usize::from(expected.as_ref().is_err_and(|reason| reason == "PANIC"));
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
                    &format!("expected {expected:?}, actual {got:?}"),
                )?;
                return Err(
                    format!("fuzz mismatch at case {}, seed {}", offset + id, case.seed).into(),
                );
            }
        }
        println!(
            "fuzz: {}/{} passed; compilation {:.2}s, execution {:.2}s, references {:.2}s",
            offset + batch.len(),
            count,
            compile_time.as_secs_f64(),
            execute_time.as_secs_f64(),
            oracle_time.as_secs_f64()
        );
    }
    println!("query distribution: {coverage}");
    println!("configuration distribution: {distribution:?}");
    println!(
        "fuzz: passed {count}; {nonempty} nonempty reference answers; {partial_cases} partial cases, {reference_panics} written reference panics; {rejected} unavailable native-merge configurations resampled; total {:.2}s (compile {:.2}s, execute {:.2}s, references {:.2}s)",
        started.elapsed().as_secs_f64(),
        compile_time.as_secs_f64(),
        execute_time.as_secs_f64(),
        oracle_time.as_secs_f64()
    );
    Ok(())
}

fn reference(case: &Case) -> Result<Vec<Vec<i32>>> {
    if case.has_partial_terms() {
        return Err(
            "partial terms and generators require the compiled written-order reference".into(),
        );
    }
    cq_testkit::duckdb::evaluate_fuzz(case).map_err(crate::testkit_error)
}

fn refines(target: &ConsumerResult, written: &ConsumerResult) -> bool {
    match written {
        Ok(expected) => target.as_ref().ok() == Some(expected),
        Err(reason) if reason == "PANIC" => {
            target.is_ok() || target.as_ref().is_err_and(|reason| reason == "PANIC")
        }
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

fn report(root: &Path, case: &Case, config: Config, reason: &str) -> Result<()> {
    let dir = root
        .join("target/fuzz/failures")
        .join(case.seed.to_string());
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("query.cq"), case.cq())?;
    std::fs::write(dir.join("query.sql"), case.sql())?;
    std::fs::write(dir.join("data.txt"), input_line(0, case))?;
    std::fs::write(
        dir.join("configuration.txt"),
        format!("seed={}\n{config}\n{reason}\n", case.seed),
    )?;
    eprintln!(
        "FUZZ FAILURE seed={}\nconfiguration: {config}\nCQ:\n{}\nSQL:\n{}\nminimized dataset: {:?}\n{reason}\nartifacts: {}\nreproduce original: cargo xtask fuzz --cases 1 --seed {}",
        case.seed,
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
        write!(line, " {} {}", relation.arity, rows.len()).unwrap();
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
    directory: PathBuf,
    target: PathBuf,
    executable: PathBuf,
}

impl Runner {
    fn new(root: &Path) -> Result<Self> {
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
            "cq-elaborate",
            "cq-planners-plan",
        ])?);
        std::fs::write(directory.join("planner/Cargo.toml"), manifest)?;
        std::fs::copy(root.join("Cargo.lock"), directory.join("Cargo.lock"))?;
        std::fs::write(
            directory.join("planner/src/compiler.rs"),
            include_str!("fuzz/compiler.rs"),
        )?;
        std::fs::write(directory.join("planner/src/lib.rs"), PLANNER)?;
        Ok(Self {
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
        source.push_str("\nfn dispatch(id: usize, data: &Data) -> Vec<Vec<i32>> { match id {\n");
        for (id, (case, _)) in cases.iter().enumerate() {
            writeln!(source, "{id} => case_{id}::run(data),").unwrap();
            if case.has_partial_terms() {
                writeln!(source, "{} => case_{}::run(data),", id + BATCH, id + BATCH).unwrap();
            }
        }
        source.push_str("_ => panic!(\"unknown case {id}\"), } }\n");
        for (id, (case, config)) in cases.iter().enumerate() {
            source.push_str(&consumer_case(id, case, *config));
            if case.has_partial_terms() {
                source.push_str(&consumer_case(
                    id + BATCH,
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
                "2",
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
                if matches!(status, Some("PANIC" | "DUPLICATES")) {
                    return Ok(Err(status.unwrap().into()));
                }
                if status != Some("OK") {
                    return Err(format!("unknown consumer result: {line}").into());
                }
                let count: usize = words.next().ok_or("missing result row count")?.parse()?;
                let values = words
                    .map(str::parse::<i32>)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if values.len() != count * case.output_arity() {
                    return Err(format!("invalid consumer result: {line}").into());
                }
                Ok(Ok(values
                    .chunks(case.output_arity())
                    .map(<[i32]>::to_vec)
                    .collect()))
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
        if case.has_partial_terms() {
            Ok(self.execute(std::iter::once((id + BATCH, case)))?.remove(0))
        } else {
            reference(case).map(Ok)
        }
    }
}

fn consumer_case(id: usize, case: &Case, c: Config) -> String {
    let mut code = format!("\nmod case_{id} {{\n");
    code.push_str("pub(super) fn run(data: &crate::Data) -> Vec<Vec<i32>> {\n");
    for (i, relation) in case.query.schema.iter().enumerate() {
        let fields = (0..relation.arity)
            .map(|j| format!("row[{j}],"))
            .collect::<String>();
        if c.host == 0 {
            writeln!(code, "let input_{i} = data[{i}].iter().map(|row| ({fields})).collect::<Vec<_>>();\nlet {} = input_{i}.as_slice();", relation.name).unwrap();
        } else {
            let tail = (1..relation.arity)
                .map(|j| format!("row[{j}],"))
                .collect::<String>();
            writeln!(code, "let mut input_{i} = std::collections::{}::<(i32,), Vec<_>>::new();\nfor row in &data[{i}] {{ input_{i}.entry((row[0],)).or_default().push(({tail})); }}\nlet {} = &input_{i};", if c.host == 1 { "HashMap" } else { "BTreeMap" }, relation.name).unwrap();
        }
    }
    for (name, value) in case.parameters() {
        writeln!(code, "let {name}: i32 = {value};").unwrap();
    }
    let fields = (0..case.output_arity())
        .map(|i| format!("crate::integer(row.{i}),"))
        .collect::<String>();
    let invocation = if c.reference {
        format!(
            "::mini_linq::compile! {{ pipeline {{ plan = cq_fuzz_planner::written_reference; }} {} }}",
            case.written_cq()
        )
    } else {
        let source = syn::parse_str(&case.cq()).expect("preflight checked CQ");
        let configuration = c
            .pipeline()
            .configuration(&source)
            .expect("checked pipeline");
        format!("::mini_linq::compile! {{ {configuration} {} }}", case.cq())
    };
    writeln!(
        code,
        "{invocation}.map(|row| vec![{fields}]).collect()\n}}\n}}"
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
    fn refinement_keeps_abort_direction_and_set_equality() {
        let rows = Ok(vec![vec![1]]);
        let empty = Ok(Vec::new());
        let panic = Err("PANIC".into());
        let duplicates = Err("DUPLICATES".into());
        assert!(refines(&rows, &rows));
        assert!(refines(&rows, &panic));
        assert!(refines(&panic, &panic));
        assert!(!refines(&panic, &rows));
        assert!(!refines(&empty, &rows));
        assert!(!refines(&duplicates, &panic));
        assert!(!refines(&rows, &duplicates));
    }

    #[test]
    fn partial_reference_uses_written_order_and_disables_every_reordering_option() {
        let config = Config::written_reference();
        assert_eq!(config.planner, 0);
        assert!(config.reference);
        let pipeline = config.pipeline();
        assert_eq!(pipeline.plan, 0);
        assert!(!pipeline.merge && !pipeline.canon && !pipeline.smallest);
        let case = model::generate(0);
        assert!(case.has_partial_terms());
        let source = consumer_case(BATCH, &case, config);
        assert!(source.contains("cq_fuzz_planner::written_reference"));
        assert!(
            reference(&case)
                .unwrap_err()
                .to_string()
                .contains("written-order")
        );
    }

    #[test]
    fn total_generator_still_uses_compiled_reference_not_duckdb() {
        let mut case = model::generate(60);
        assert!(case.query.bindings[0].generator.is_some());
        case.query.bindings[0].expression = model::Expr::Constant(2);
        assert!(case.has_partial_terms());
        assert!(
            reference(&case)
                .unwrap_err()
                .to_string()
                .contains("generators")
        );
        let source = consumer_case(BATCH, &case, Config::written_reference());
        assert!(source.contains("cq_fuzz_planner::written_reference"));
        assert!(source.contains("for v3 in"));
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
    fn effect_reference_keeps_literal_after_same_atom_partial_expression() {
        let source = syn::parse_quote! {
            struct Fuzz;
            relation R(a: i32, b: i32, c: i32);
            relation answer(a: i32);
            answer(x) :- R(x, 10 / *x, 5);
        };
        let reference = compiler::written(&source).unwrap();
        let definitions: Vec<_> = reference.plan.definitions.iter().collect();
        let bind = definitions
            .iter()
            .position(|d| matches!(d.operator, cq_lang_ra::Operator::Bind(_)))
            .unwrap();
        let literal = definitions
            .iter()
            .position(|d| match &d.operator {
                cq_lang_ra::Operator::Select(selection) => {
                    selection.condition.constant_keys().is_some()
                }
                _ => false,
            })
            .unwrap();
        assert!(
            bind < literal,
            "a failing literal cannot suppress an earlier reference panic"
        );
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
        let mut case = model::generate(1);
        case.data[0] = vec![
            vec![3; case.query.schema[0].arity],
            vec![7; case.query.schema[0].arity],
        ];
        shrink_rows(&mut case, |candidate| {
            candidate.data[0].iter().any(|row| row[0] == 7)
        });
        assert_eq!(case.data[0].len(), 1);
        assert_eq!(case.data[0][0][0], 7);
        assert!(case.data.iter().skip(1).all(Vec::is_empty));
    }

    #[test]
    fn row_shrinking_revisits_earlier_rows_after_later_deletions() {
        let mut case = model::generate(1);
        case.data[0] = (0..3)
            .map(|value| vec![value; case.query.schema[0].arity])
            .collect();
        shrink_rows(&mut case, |candidate| {
            let contains = |value| candidate.data[0].iter().any(|row| row[0] == value);
            contains(2) && (contains(0) || !contains(1))
        });
        assert_eq!(case.data[0], vec![vec![2; case.query.schema[0].arity]]);
    }

    #[test]
    fn atom_shrinking_revisits_earlier_atoms_after_later_deletions() {
        let mut case = Case {
            seed: 0,
            query: model::Query {
                schema: (0..3)
                    .map(|i| model::Relation {
                        name: format!("r{i}"),
                        arity: 1,
                    })
                    .collect(),
                atoms: (0..3)
                    .map(|relation| model::Atom {
                        relation,
                        terms: vec![model::Term::Var(0)],
                    })
                    .collect(),
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
}
