mod bench;
mod explain;
#[cfg(feature = "oracle")]
mod oracle;

use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};

use cq_workloads::{GenerationConfig, QueryCase, Scale, Scenario, catalog};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".to_owned());
    let arguments = args.collect::<Vec<_>>();
    match command.as_str() {
        "list" => list(),
        "generate" => {
            let options = CommandOptions::parse(arguments, false)?;
            generate(options.selection, options.scale, options.scenarios)
        }
        "golden" => {
            #[cfg(feature = "oracle")]
            {
                let options = CommandOptions::parse(arguments, true)?;
                golden(
                    options.selection,
                    options.scale,
                    options.scenarios,
                    options.write,
                )
            }
            #[cfg(not(feature = "oracle"))]
            {
                rerun_with_oracle("golden", &arguments)
            }
        }
        "differential" => Err("differential execution is unavailable: the old whole-query Rust backend was removed; general physical selection and a host input/output contract are required before generated-query comparison can resume".into()),
        "explain" => explain::run(arguments, &repository_root()),
        "bench" => bench::run(arguments),
        "help" | "--help" | "-h" => {
            help();
            Ok(())
        }
        other => Err(format!("unknown xtask command `{other}`; run `cargo xtask help`").into()),
    }
}

fn list() -> Result<()> {
    for case in catalog::all() {
        println!(
            "{:<24} {:<7} {}",
            case.name,
            case.suite.as_str(),
            case.purpose
        );
    }
    Ok(())
}

fn generate(selection: Selection, scale: Scale, scenarios: ScenarioSelection) -> Result<()> {
    let allow_inapplicable = scenarios.allows_inapplicable();
    for case in selection.cases() {
        let module = case.module()?;
        let sql = cq_workloads::sql::translate(&module)?;
        write_text(&sql_path(case), &sql)?;
        for scenario in scenarios.values() {
            let config = generation_config(case, scenario, scale);
            if !check_or_skip_scenario(case, scenario, allow_inapplicable)? {
                let directory = data_directory(case, scenario, scale);
                if directory.exists() {
                    std::fs::remove_dir_all(&directory)?;
                }
                continue;
            }
            let dataset = cq_workloads::generate::generate(case, config)?;
            let directory = data_directory(case, scenario, scale);
            if directory.exists() {
                std::fs::remove_dir_all(&directory)?;
            }
            dataset.write_to_dir(&module, &directory)?;
            println!(
                "generated {:<24} {:<11} {}",
                case.name,
                scenario.as_str(),
                directory.display()
            );
        }
    }
    Ok(())
}

#[cfg(feature = "oracle")]
fn golden(
    selection: Selection,
    scale: Scale,
    scenarios: ScenarioSelection,
    write: bool,
) -> Result<()> {
    let allow_inapplicable = scenarios.allows_inapplicable();
    for case in selection.cases() {
        for scenario in scenarios.values() {
            let config = generation_config(case, scenario, scale);
            if !check_or_skip_scenario(case, scenario, allow_inapplicable)? {
                reject_or_remove_inapplicable_artifacts(case, scenario, scale, write)?;
                continue;
            }
            let generated = if scale == Scale::Tiny {
                Some(cq_workloads::generate::generate(case, config)?)
            } else {
                None
            };
            let (module, dataset) = read_generated(case, config, generated.as_ref())?;
            let expected = oracle::evaluate(case, &dataset)?;
            let path = golden_path(case, scenario, scale);
            if write {
                cq_workloads::dataset::write_result_rows(
                    &path,
                    &expected,
                    module.query.head.args.len(),
                )?;
                println!(
                    "wrote {:<28} {:<11} {}",
                    case.name,
                    scenario.as_str(),
                    path.display()
                );
            } else {
                let arity = module.query.head.args.len();
                let tracked = cq_workloads::dataset::read_result_rows(&path, arity)?;
                compare_rows(
                    case,
                    scenario,
                    "tracked golden",
                    &tracked,
                    "DuckDB",
                    &expected,
                )?;
                println!(
                    "golden {:<27} {:<11} {} rows",
                    case.name,
                    scenario.as_str(),
                    expected.len()
                );
            }
        }
    }
    Ok(())
}

#[cfg(not(feature = "oracle"))]
fn rerun_with_oracle(command: &str, arguments: &[String]) -> Result<()> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    eprintln!("enabling the DuckDB oracle for `cargo xtask {command}`...");
    let status = std::process::Command::new(cargo)
        .arg("run")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--locked")
        .arg("--features")
        .arg("oracle")
        .arg("--")
        .arg(command)
        .args(arguments)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("oracle-enabled `cargo xtask {command}` failed with status {status}").into())
    }
}

#[cfg(feature = "oracle")]
fn read_generated(
    case: &QueryCase,
    config: GenerationConfig,
    expected: Option<&cq_workloads::dataset::Dataset>,
) -> Result<(cq_lang_cq::Program, cq_workloads::dataset::Dataset)> {
    let directory = data_directory(case, config.scenario, config.scale);
    if !directory.join("manifest.txt").is_file() {
        return Err(format!(
            "missing generated data for `{}` scenario `{}` at {}; run `cargo xtask generate {} {} --scenario {}`",
            case.name,
            config.scenario.as_str(),
            directory.display(),
            case.suite.as_str(),
            config.scale.as_str(),
            config.scenario.as_str(),
        )
        .into());
    }
    let module = case.module()?;
    let dataset = cq_workloads::dataset::Dataset::read_from_dir(&module, &directory)?;
    dataset.validate_provenance(case.suite, case.name, config)?;

    // Tiny artifacts are checked in. Tie them byte-for-byte to the current
    // catalog, generator, and stable seed instead of accepting merely valid
    // rows with matching cardinalities.
    if expected.is_some_and(|expected| dataset != *expected) {
        return Err(format!(
            "checked-in tiny data for `{}` scenario `{}` is stale; rerun `cargo xtask generate {} tiny --scenario {}`",
            case.name,
            config.scenario.as_str(),
            case.suite.as_str(),
            config.scenario.as_str(),
        )
        .into());
    }

    let expected_sql = cq_workloads::sql::translate(&module)?;
    let path = sql_path(case);
    let actual_sql = std::fs::read_to_string(&path)?;
    if actual_sql != expected_sql {
        return Err(format!(
            "generated SQL for `{}` is stale at {}; rerun `cargo xtask generate {} {}`",
            case.name,
            path.display(),
            case.suite.as_str(),
            config.scale.as_str(),
        )
        .into());
    }
    Ok((module, dataset))
}

#[cfg(feature = "oracle")]
fn compare_rows(
    case: &QueryCase,
    scenario: Scenario,
    left_name: &str,
    left: &[Vec<i32>],
    right_name: &str,
    right: &[Vec<i32>],
) -> Result<()> {
    if left == right {
        return Ok(());
    }
    let mismatch = left
        .iter()
        .zip(right)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| left.len().min(right.len()));
    Err(format!(
        "`{}` scenario `{}` differs: {left_name} has {} rows, {right_name} has {} rows; first mismatch at {mismatch}: {left_name}={:?}, {right_name}={:?}",
        case.name,
        scenario.as_str(),
        left.len(),
        right.len(),
        left.get(mismatch),
        right.get(mismatch),
    )
    .into())
}

fn check_or_skip_scenario(
    case: &QueryCase,
    scenario: Scenario,
    allow_inapplicable: bool,
) -> Result<bool> {
    match cq_workloads::generate::check_scenario(case, scenario) {
        Ok(()) => Ok(true),
        Err(cq_workloads::generate::GenerateError::ScenarioNotApplicable {
            scenario,
            reason,
        }) if allow_inapplicable => {
            println!(
                "skip   {:<28} {:<11} {reason}",
                case.name,
                scenario.as_str()
            );
            Ok(false)
        }
        Err(cq_workloads::generate::GenerateError::ScenarioNotApplicable {
            scenario,
            reason,
        }) => Err(format!(
            "scenario `{}` is not applicable to `{}`: {reason}; use `--scenario all` to skip mathematically unavailable combinations",
            scenario.as_str(),
            case.name,
        )
        .into()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(feature = "oracle")]
fn reject_inapplicable_data(case: &QueryCase, scenario: Scenario, scale: Scale) -> Result<()> {
    let directory = data_directory(case, scenario, scale);
    if directory.exists() {
        return Err(format!(
            "stale data exists for inapplicable `{}` scenario `{}` at {}; remove it or rerun `cargo xtask generate all {} --scenario all`",
            case.name,
            scenario.as_str(),
            directory.display(),
            scale.as_str(),
        )
        .into());
    }
    Ok(())
}

#[cfg(feature = "oracle")]
fn reject_or_remove_inapplicable_artifacts(
    case: &QueryCase,
    scenario: Scenario,
    scale: Scale,
    write: bool,
) -> Result<()> {
    reject_inapplicable_data(case, scenario, scale)?;
    let path = golden_path(case, scenario, scale);
    if !path.exists() {
        return Ok(());
    }
    if write {
        std::fs::remove_file(path)?;
        return Ok(());
    }
    Err(format!(
        "stale golden exists for inapplicable `{}` scenario `{}` at {}; rerun the golden command with `--write` to remove it",
        case.name,
        scenario.as_str(),
        path.display(),
    )
    .into())
}

#[derive(Clone, Copy)]
enum Selection {
    All,
    Suite(&'static str),
    Case(&'static QueryCase),
}

impl Selection {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "all" => Ok(Self::All),
            "classic" => Ok(Self::Suite("classic")),
            "retail" => Ok(Self::Suite("retail")),
            name => cq_workloads::case(name)
                .map(Self::Case)
                .ok_or_else(|| format!("unknown suite or case `{name}`").into()),
        }
    }

    fn cases(self) -> Box<dyn Iterator<Item = &'static QueryCase>> {
        match self {
            Self::All => Box::new(catalog::all()),
            Self::Suite(name) => Box::new(catalog::named(name).into_iter().flatten()),
            Self::Case(case) => Box::new(std::iter::once(case)),
        }
    }
}

#[derive(Clone, Copy)]
enum ScenarioSelection {
    All,
    One(Scenario),
}

impl ScenarioSelection {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "all" => Ok(Self::All),
            "coverage" => Ok(Self::One(Scenario::Coverage)),
            "empty-input" => Ok(Self::One(Scenario::EmptyInput)),
            "no-match" => Ok(Self::One(Scenario::NoMatch)),
            _ => Err(format!(
                "unknown scenario `{value}`; expected coverage, empty-input, no-match, or all"
            )
            .into()),
        }
    }

    fn values(self) -> impl Iterator<Item = Scenario> {
        Scenario::ALL
            .into_iter()
            .filter(move |scenario| match self {
                Self::All => true,
                Self::One(selected) => *scenario == selected,
            })
    }

    fn allows_inapplicable(self) -> bool {
        matches!(self, Self::All)
    }
}

struct CommandOptions {
    selection: Selection,
    scale: Scale,
    scenarios: ScenarioSelection,
    #[cfg_attr(not(feature = "oracle"), allow(dead_code))]
    write: bool,
}

impl CommandOptions {
    fn parse(arguments: Vec<String>, allow_write: bool) -> Result<Self> {
        let mut positional = Vec::new();
        let mut scenarios = None;
        let mut write = false;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--scenario" => {
                    if scenarios.is_some() {
                        return Err("`--scenario` may be provided only once".into());
                    }
                    let value = arguments
                        .next()
                        .ok_or("`--scenario` requires a scenario name")?;
                    scenarios = Some(ScenarioSelection::parse(&value)?);
                }
                "--write" if allow_write => {
                    if write {
                        return Err("`--write` may be provided only once".into());
                    }
                    write = true;
                }
                option if option.starts_with('-') => {
                    return Err(format!("unknown option `{option}`").into());
                }
                value => positional.push(value.to_owned()),
            }
        }
        if positional.len() > 2 {
            return Err(format!(
                "unexpected argument `{}`; expected at most a selection and scale",
                positional[2]
            )
            .into());
        }

        Ok(Self {
            selection: Selection::parse(positional.first().map_or("all", String::as_str))?,
            scale: parse_scale(positional.get(1).map_or("tiny", String::as_str))?,
            scenarios: scenarios.unwrap_or(ScenarioSelection::One(Scenario::Coverage)),
            write,
        })
    }
}

fn parse_scale(value: &str) -> Result<Scale> {
    match value {
        "tiny" => Ok(Scale::Tiny),
        "medium" => Ok(Scale::Medium),
        "large" => Ok(Scale::Large),
        _ => Err(format!("unknown scale `{value}`; expected tiny, medium, or large").into()),
    }
}

fn generation_config(case: &QueryCase, scenario: Scenario, scale: Scale) -> GenerationConfig {
    GenerationConfig {
        scenario,
        scale,
        seed: cq_workloads::seed_for(case),
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives directly under the repository root")
        .to_path_buf()
}

fn data_directory(case: &QueryCase, scenario: Scenario, scale: Scale) -> PathBuf {
    repository_root()
        .join("workloads/data")
        .join(case.suite.as_str())
        .join(case.name)
        .join(scenario.as_str())
        .join(scale.as_str())
}

#[cfg_attr(all(not(feature = "oracle"), not(test)), allow(dead_code))]
fn golden_path(case: &QueryCase, scenario: Scenario, scale: Scale) -> PathBuf {
    repository_root()
        .join("workloads/golden")
        .join(case.suite.as_str())
        .join(case.name)
        .join(scenario.as_str())
        .join(format!("{}.csv", scale.as_str()))
}

fn sql_path(case: &QueryCase) -> PathBuf {
    repository_root()
        .join("workloads/sql")
        .join(case.suite.as_str())
        .join(format!("{}.sql", case.name))
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)?;
    Ok(())
}

fn help() {
    println!(
        "\
MiniLinq workload tasks

  cargo xtask list
  cargo xtask generate [all|classic|retail|CASE] [tiny|medium|large] [--scenario SCENARIO]
  cargo xtask golden [SELECTION] [SCALE] [--scenario SCENARIO] [--write]
  cargo xtask differential [SELECTION] [SCALE] [--scenario SCENARIO]
  cargo xtask explain CASE [--through STAGE]
  cargo xtask bench [--scale N] [--seed N]

SCENARIO is coverage, empty-input, no-match, or all. It defaults to coverage.

STAGE is cq, relational-plan, or index-requirements. It defaults to index-requirements.
Bench reports scales N, 2N and 4N (default N=128); time is never asserted.

`golden` compares DuckDB with checked-in CSV answers. `--write` is the only
mode that changes those answers. `differential` is unavailable until general
physical selection and the new host input/output contract are implemented.
`explain` records typed CQ, RA and IX payloads; it does not execute a backend."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn command_options_default_to_coverage() {
        let options = CommandOptions::parse(Vec::new(), false).unwrap();

        assert!(matches!(options.selection, Selection::All));
        assert_eq!(options.scale, Scale::Tiny);
        assert_eq!(
            options.scenarios.values().collect::<Vec<_>>(),
            vec![Scenario::Coverage]
        );
        assert!(!options.write);
    }

    #[test]
    fn scenario_all_expands_in_stable_order() {
        let options = CommandOptions::parse(
            arguments(&["classic", "medium", "--scenario", "all"]),
            false,
        )
        .unwrap();

        assert!(matches!(options.selection, Selection::Suite("classic")));
        assert_eq!(options.scale, Scale::Medium);
        assert_eq!(
            options.scenarios.values().collect::<Vec<_>>(),
            Scenario::ALL
        );
    }

    #[test]
    fn write_is_golden_only_and_surplus_arguments_are_rejected() {
        assert!(CommandOptions::parse(arguments(&["--write"]), false).is_err());
        assert!(
            CommandOptions::parse(arguments(&["--write"]), true)
                .unwrap()
                .write
        );
        assert!(CommandOptions::parse(arguments(&["all", "tiny", "extra"]), false).is_err());
    }

    #[test]
    fn artifact_paths_include_scenario_but_sql_does_not() {
        let case = catalog::all().next().unwrap();

        assert!(
            data_directory(case, Scenario::NoMatch, Scale::Large)
                .ends_with("workloads/data/classic/intersection/no-match/large")
        );
        assert!(
            golden_path(case, Scenario::EmptyInput, Scale::Tiny)
                .ends_with("workloads/golden/classic/intersection/empty-input/tiny.csv")
        );
        assert!(sql_path(case).ends_with("workloads/sql/classic/intersection.sql"));
    }

    #[test]
    fn an_explicit_inapplicable_scenario_is_an_error_but_all_may_skip_it() {
        let case = cq_workloads::case("cartesian-product").unwrap();

        assert!(check_or_skip_scenario(case, Scenario::NoMatch, false).is_err());
        assert!(!check_or_skip_scenario(case, Scenario::NoMatch, true).unwrap());
    }
}
