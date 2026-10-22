//! Core body blocks `{ B1 } | { B2 }` union answers and `{ B1 } else { B2 }`
//! select the first nonempty arm. Two nonempty branches are required; nesting
//! handles mixed operators. Common interface names escape; other binders stay
//! local. Join distributes over union, and fallback is `B1 union (B2 anti_F B1)`.
//! The complete IR records hoisting or per-row fusion, canon/witness decisions,
//! negative references and Rust source paths with inherited Before facts.
//!
mod bench;
mod cargo;
mod explain;
mod extended;
#[cfg(feature = "oracle")]
mod fuzz;
#[cfg(feature = "oracle")]
use cq_testkit::golden;
#[cfg(feature = "oracle")]
mod oracle;
#[cfg(feature = "oracle")]
mod tpch;
#[cfg(feature = "oracle")]
mod typed;

use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};

use cq_workloads::{GenerationConfig, QueryCase, Scale, Scenario, catalog};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[cfg(feature = "oracle")]
fn testkit_error(error: Box<dyn Error + Send + Sync>) -> Box<dyn Error> {
    error
}

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
        "fuzz" | "differential" => {
            #[cfg(feature = "oracle")]
            {
                fuzz::run(arguments, &repository_root())
            }
            #[cfg(not(feature = "oracle"))]
            {
                rerun_with_oracle(&command, &arguments)
            }
        }
        "explain" => explain::run(arguments, &repository_root()),
        "bench" => bench::run(arguments),
        "profile" => {
            #[cfg(feature = "oracle")]
            {
                tpch::profile::run(arguments)
            }
            #[cfg(not(feature = "oracle"))]
            {
                rerun_with_oracle("profile", &arguments)
            }
        }
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
    for case in cq_workloads::extended::CASES {
        println!("{:<24} {:<7} {}", case.name, "extended", case.family);
    }
    for case in cq_workloads::typed::CASES {
        println!("{:<24} {:<7} {}", case.name, "typed", case.purpose);
    }
    Ok(())
}

fn generate(selection: Selection, scale: Scale, scenarios: ScenarioSelection) -> Result<()> {
    if matches!(selection, Selection::Suite("typed") | Selection::Typed(_)) {
        return Err("typed CSV fixtures are authored; use `cargo xtask golden typed --write` to regenerate their Parquet copies and goldens".into());
    }
    extended::check_scale(selection, scale)?;
    let allow_inapplicable = scenarios.allows_inapplicable();
    for case in selection.cases() {
        let module = case.module()?;
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
    extended::generate(selection, scenarios)?;
    Ok(())
}

#[cfg(feature = "oracle")]
fn golden(
    selection: Selection,
    scale: Scale,
    scenarios: ScenarioSelection,
    write: bool,
) -> Result<()> {
    extended::check_scale(selection, scale)?;
    let allow_inapplicable = scenarios.allows_inapplicable();
    let mut checked = std::collections::BTreeSet::new();
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
            checked.insert(path.clone());
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
                golden::compare(&path, &expected, module.query.head.args.len())
                    .map_err(testkit_error)?;
                println!(
                    "golden {:<27} {:<11} {} rows",
                    case.name,
                    scenario.as_str(),
                    expected.len()
                );
            }
        }
    }
    checked.extend(extended::golden(selection, scenarios, write)?);
    checked.extend(typed::golden(selection, scale, scenarios, write)?);
    if matches!(selection, Selection::All)
        && scale == Scale::Tiny
        && matches!(scenarios, ScenarioSelection::All)
    {
        golden::check_inventory(
            repository_root()
                .parent()
                .and_then(Path::parent)
                .ok_or("feature has no repository parent")?,
            Path::new("features/workloads/golden"),
            &checked,
        )
        .map_err(testkit_error)?;
        println!("verified all {} golden files byte for byte", checked.len());
    }
    Ok(())
}

#[cfg(not(feature = "oracle"))]
fn rerun_with_oracle(command: &str, arguments: &[String]) -> Result<()> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    eprintln!("enabling the DuckDB oracle for `cargo xtask {command}`...");
    let status = cargo::command()
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
    let dataset = case.load(&directory)?;
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

    Ok((module, dataset))
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
    Extension(&'static cq_workloads::extended::Case),
    Typed(&'static cq_workloads::typed::Case),
}

impl Selection {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "all" => Ok(Self::All),
            "classic" => Ok(Self::Suite("classic")),
            "retail" => Ok(Self::Suite("retail")),
            "extended" => Ok(Self::Suite("extended")),
            "typed" => Ok(Self::Suite("typed")),
            name => cq_workloads::case(name)
                .map(Self::Case)
                .or_else(|| cq_workloads::extended::case(name).map(Self::Extension))
                .or_else(|| cq_workloads::typed::case(name).map(Self::Typed))
                .ok_or_else(|| format!("unknown suite or case `{name}`").into()),
        }
    }

    fn cases(self) -> Box<dyn Iterator<Item = &'static QueryCase>> {
        match self {
            Self::All => Box::new(catalog::all()),
            Self::Suite(name) => Box::new(catalog::named(name).into_iter().flatten()),
            Self::Case(case) => Box::new(std::iter::once(case)),
            Self::Extension(_) | Self::Typed(_) => Box::new(std::iter::empty()),
        }
    }

    fn extensions(self) -> Box<dyn Iterator<Item = &'static cq_workloads::extended::Case>> {
        match self {
            Self::All | Self::Suite("extended") => Box::new(cq_workloads::extended::CASES.iter()),
            Self::Extension(case) => Box::new(std::iter::once(case)),
            _ => Box::new(std::iter::empty()),
        }
    }

    #[cfg_attr(not(feature = "oracle"), allow(dead_code))]
    fn typed(self) -> Box<dyn Iterator<Item = &'static cq_workloads::typed::Case>> {
        match self {
            Self::All | Self::Suite("typed") => Box::new(cq_workloads::typed::CASES.iter()),
            Self::Typed(case) => Box::new(std::iter::once(case)),
            _ => Box::new(std::iter::empty()),
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
            scenarios: scenarios.unwrap_or(if allow_write {
                ScenarioSelection::All
            } else {
                ScenarioSelection::One(Scenario::Coverage)
            }),
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
        .expect("xtask lives inside the workload workspace")
        .to_path_buf()
}

fn data_directory(case: &QueryCase, scenario: Scenario, scale: Scale) -> PathBuf {
    repository_root()
        .join("data")
        .join(case.suite.as_str())
        .join(case.name)
        .join(scenario.as_str())
        .join(scale.as_str())
}

#[cfg_attr(all(not(feature = "oracle"), not(test)), allow(dead_code))]
fn golden_path(case: &QueryCase, scenario: Scenario, scale: Scale) -> PathBuf {
    repository_root()
        .join("golden")
        .join(case.suite.as_str())
        .join(case.name)
        .join(scenario.as_str())
        .join(format!("{}.csv", scale.as_str()))
}

fn help() {
    println!(
        "\
MiniLinq workload tasks

  cargo xtask list
  cargo xtask generate [all|classic|retail|extended|CASE] [tiny|medium|large] [--scenario SCENARIO]
  cargo xtask golden [SELECTION] [SCALE] [--scenario SCENARIO] [--write]
  cargo xtask fuzz [--cases N] [--seed S]
  cargo xtask differential [--cases N] [--seed S]
  cargo xtask explain CASE [--through STAGE]
  cargo xtask bench [--scale N] [--seed N]
  cargo xtask bench --tpch --sf 0.1|1 [--runs N] [--query NAME] [--config NAME | --reduction-comparison]
  cargo xtask profile --tpch --sf 0.1|1 [--query NAME] [--config NAME]

SCENARIO is coverage, empty-input, no-match, or all.
It defaults to all for golden, coverage for generate.

STAGE is cq, relational-plan, or index-requirements. It defaults to index-requirements.
Bench always builds and runs in the release profile; time is never asserted.
The synthetic bench reports scales N, 2N and 4N (default N=128).
The TPC-H-derived bench enables the DuckDB oracle, caches data under target/,
and reports warm medians for the selected scale factor.
The extended and typed corpora have tiny fixtures only. Typed CSV fixtures are
authored; `golden typed --write` regenerates their Parquet copies and answers.

`golden` compares DuckDB with checked-in CSV answers. `--write` is the only
mode that changes those answers. `differential` is an alias for `fuzz`, which
compiles seeded random CQs and compares their execution against DuckDB.
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
    fn golden_defaults_to_every_scenario() {
        let options = CommandOptions::parse(Vec::new(), true).unwrap();
        assert_eq!(
            options.scenarios.values().collect::<Vec<_>>(),
            Scenario::ALL
        );
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
    fn artifact_paths_include_scenario() {
        let case = catalog::all().next().unwrap();

        assert!(
            data_directory(case, Scenario::NoMatch, Scale::Large)
                .ends_with("workloads/data/classic/intersection/no-match/large")
        );
        assert!(
            golden_path(case, Scenario::EmptyInput, Scale::Tiny)
                .ends_with("workloads/golden/classic/intersection/empty-input/tiny.csv")
        );
    }

    #[test]
    fn an_explicit_inapplicable_scenario_is_an_error_but_all_may_skip_it() {
        let case = cq_workloads::case("cartesian-product").unwrap();

        assert!(check_or_skip_scenario(case, Scenario::NoMatch, false).is_err());
        assert!(!check_or_skip_scenario(case, Scenario::NoMatch, true).unwrap());
    }
}
