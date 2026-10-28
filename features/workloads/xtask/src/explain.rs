use std::error::Error;
use std::fmt;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use cq_workloads::QueryCase;
use quote::ToTokens;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

const ERROR_FILE: &str = "ERROR.md";

/// One independently inspectable output of the facade's resolved queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Cq,
    Vo,
    RelationalPlan,
    IndexRequirements,
    Physical,
    Structures,
    Repr,
    Stage,
    Sequence,
    Decompose,
    Grade,
    Rust,
}

impl Stage {
    const ALL: [Self; 12] = [
        Self::Cq,
        Self::Vo,
        Self::RelationalPlan,
        Self::IndexRequirements,
        Self::Physical,
        Self::Structures,
        Self::Repr,
        Self::Stage,
        Self::Sequence,
        Self::Decompose,
        Self::Grade,
        Self::Rust,
    ];

    fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|stage| stage.name() == value)
            .ok_or_else(|| {
                format!(
                    "unknown explain stage `{value}`; expected {}",
                    Self::ALL.map(Self::name).join(", "),
                )
                .into()
            })
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Cq => "cq",
            Self::Vo => "vo",
            Self::RelationalPlan => "relational-plan",
            Self::IndexRequirements => "index-requirements",
            Self::Physical => "physical",
            Self::Structures => "structures",
            Self::Repr => "represent",
            Self::Stage => "stage",
            Self::Sequence => "sequence",
            Self::Decompose => "decompose",
            Self::Grade => "grade",
            Self::Rust => "final-rust",
        }
    }

    fn file_name(self) -> String {
        format!("{:02}-{}.tokens", self as usize + 1, self.name())
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Cq => "the checked source conjunctive query",
            Self::Vo => "the shared default planner's checked valuation order and reductions",
            Self::RelationalPlan => "the complete named logical relational-algebra plan",
            Self::IndexRequirements => "the logical plan plus named index demands",
            Self::Physical => "recorded execution decisions",
            Self::Structures => "recorded runtime representations and boundaries",
            Self::Repr => "recorded result carriers",
            Self::Stage => "recorded total batch boundaries",
            Self::Sequence => "the complete lowered Sequence program",
            Self::Decompose => "Sequence after the configured decomposition passes",
            Self::Grade => "Sequence after target grading",
            Self::Rust => "the terminal translator's Rust expression",
        }
    }
}

struct Options {
    case: String,
    through: Stage,
}

impl Options {
    fn parse(arguments: Vec<String>) -> Result<Self> {
        let mut case = None;
        let mut through = None;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--through" => {
                    if through.is_some() {
                        return Err("`--through` may be provided only once".into());
                    }
                    let value = arguments
                        .next()
                        .ok_or("`--through` requires a stage name")?;
                    through = Some(Stage::parse(&value)?);
                }
                option if option.starts_with('-') => {
                    return Err(format!("unknown explain option `{option}`").into());
                }
                value if case.is_none() => case = Some(value.to_owned()),
                value => {
                    return Err(format!(
                        "unexpected explain argument `{value}`; expected exactly one case name"
                    )
                    .into());
                }
            }
        }
        Ok(Self {
            case: case.ok_or("`cargo xtask explain` requires a catalog case name")?,
            through: through.unwrap_or(Stage::Structures),
        })
    }
}

pub fn run(arguments: Vec<String>, repository_root: &Path) -> Result<()> {
    let options = Options::parse(arguments)?;
    let case = cq_workloads::case(&options.case)
        .ok_or_else(|| format!("unknown catalog case `{}`", options.case))?;
    explain_case(case, options.through, repository_root)
}

/// Formal rule DR-Explain: Each artifact observes a prefix of the ordinary facade queue, never a driver-owned compiler composition.
fn explain_case(case: &QueryCase, through: Stage, repository_root: &Path) -> Result<()> {
    let artifacts = Artifacts::new(repository_root, case, through)?;
    let source = artifacts.attempt("catalog query -> CQ", || case.module())?;
    let observer = artifacts.attempt("prepare facade observer", || {
        Observer::new(repository_root, &artifacts.directory)
    })?;
    let source = source.to_token_stream().to_string();
    for stage in Stage::ALL.into_iter().take_while(|stage| *stage <= through) {
        let payload = artifacts.attempt(stage.name(), || observer.capture(stage, &source))?;
        artifacts.write_named(&stage.file_name(), &payload, stage.name())?;
    }
    println!(
        "stopped after {:<12} {}",
        through.name(),
        artifacts.directory.display()
    );
    Ok(())
}

/// A generated consumer expands the public facade, then prints only its captured
/// tokens. The query's generated Rust is never type-checked or executed.
struct Observer {
    directory: PathBuf,
    target: PathBuf,
}

impl Observer {
    fn new(root: &Path, artifacts: &Path) -> Result<Self> {
        let directory = artifacts.join("observer");
        fs::create_dir_all(directory.join("src"))?;
        let dependencies = crate::cargo::dependencies(&["mini-linq"])?;
        fs::write(
            directory.join("Cargo.toml"),
            format!(
                "[package]\nname = \"mini-linq-explain-observer\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\n{dependencies}"
            ),
        )?;
        fs::copy(root.join("Cargo.lock"), directory.join("Cargo.lock"))?;
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(|path| PathBuf::from(path).join("explain-observers"))
            .unwrap_or_else(|| root.join("target/explain-observers"));
        Ok(Self { directory, target })
    }

    fn capture(&self, stage: Stage, source: &str) -> Result<String> {
        fs::write(
            self.directory.join("src/main.rs"),
            observer_source(stage, source),
        )?;
        let output = crate::cargo::command()
            .current_dir(&self.directory)
            .args(["run", "--quiet", "--offline"])
            .env("CARGO_TARGET_DIR", &self.target)
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "facade observation failed: {}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(String::from_utf8(output.stdout)?)
    }
}

fn observer_source(stage: Stage, source: &str) -> String {
    let boundary = stage.name().replace('-', "_");
    format!(
        "#![recursion_limit = \"512\"]\nfn main() {{\n    let payload: &str = ::mini_linq::compile! {{\n        observe {{ boundary = {boundary}; stage = ::core::stringify; }}\n        pipeline {{}}\n        {source}\n    }};\n    println!(\"{{payload}}\");\n}}\n"
    )
}

struct Artifacts {
    case_name: &'static str,
    directory: PathBuf,
}

impl Artifacts {
    fn new(repository_root: &Path, case: &QueryCase, through: Stage) -> Result<Self> {
        let directory = repository_root
            .join("target/mini-linq-explain")
            .join(case.name);
        if directory.exists() {
            fs::remove_dir_all(&directory)?;
        }
        fs::create_dir_all(&directory)?;
        let artifacts = Self {
            case_name: case.name,
            directory: directory.canonicalize()?,
        };
        artifacts.write_readme(case, through)?;
        Ok(artifacts)
    }

    fn write_readme(&self, case: &QueryCase, through: Stage) -> Result<()> {
        let rows = Stage::ALL
            .into_iter()
            .map(|stage| {
                let status = if stage <= through {
                    "generated on success"
                } else {
                    "not requested"
                };
                format!(
                    "| `{}` | `{}` | {} | {status} |",
                    stage.file_name(),
                    stage.name(),
                    stage.description()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.write_named("README.md", &format!(
            "# MiniLinq typed boundary report: `{}`\n\n\
             Generated from `{}` by:\n\n\
             ```text\n\
             cargo xtask explain {} --through {}\n\
             ```\n\n\
             Each `.tokens` file captures a checked payload with `mini_linq::compile!` \
             and `stage = ::core::stringify`. The facade resolves and executes the \
             same default queue used for compilation, stopping at the selected boundary. \
             VO exposes the shared default planner's valuation prefix. Final Rust is \
             captured after translation; the query is not executed.\n\n\
             Open adjacent files with VS Code's **Select for Compare** and **Compare with Selected** commands. \
             Files after the requested stopping stage are deliberately absent.\n\n\
             | File | Stage | Meaning | This run |\n\
             | --- | --- | --- | --- |\n\
             {rows}\n\n\
             If observation fails, earlier files remain and `ERROR.md` names the failed boundary. \
             The `observer` directory contains the exact final attempted facade invocation.\n",
            case.name, case.rust_path, case.name, through.name(),
        ), "trace guide")
    }

    fn write_named(&self, file_name: &str, contents: &str, label: &str) -> Result<()> {
        let path = self.directory.join(file_name);
        fs::write(&path, contents)?;
        println!("wrote {label:<20} {}", path.display());
        Ok(())
    }

    fn attempt<T, E: fmt::Display>(
        &self,
        edge: &str,
        operation: impl FnOnce() -> std::result::Result<T, E>,
    ) -> Result<T> {
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(self.record_failure(edge, &error.to_string())?),
            Err(payload) => {
                let diagnostic = if let Some(message) = payload.downcast_ref::<String>() {
                    message.as_str()
                } else if let Some(message) = payload.downcast_ref::<&str>() {
                    message
                } else {
                    "compiler panicked with a non-string payload"
                };
                Err(self.record_failure(edge, diagnostic)?)
            }
        }
    }

    fn record_failure(&self, edge: &str, diagnostic: &str) -> Result<Box<dyn Error>> {
        let path = self.directory.join(ERROR_FILE);
        let indented = diagnostic
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            &path,
            format!(
                "# MiniLinq compiler trace stopped\n\nCase: `{}`\n\nFailed boundary: `{edge}`\n\n\
             Diagnostic:\n\n{indented}\n\n\
             Every earlier `.tokens` artifact was produced successfully. \
             No later boundary was requested.\n",
                self.case_name,
            ),
        )?;
        eprintln!("wrote error report         {}", path.display());
        Ok(Box::new(ExplainFailure {
            case_name: self.case_name,
            edge: edge.to_owned(),
            diagnostic: diagnostic.to_owned(),
            report: path,
        }))
    }
}

#[derive(Debug)]
struct ExplainFailure {
    case_name: &'static str,
    edge: String,
    diagnostic: String,
    report: PathBuf,
}

impl fmt::Display for ExplainFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "explaining `{}` failed at `{}`: {}; see {}",
            self.case_name,
            self.edge,
            self.diagnostic,
            self.report.display()
        )
    }
}
impl Error for ExplainFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stage_uses_the_same_facade_observation_protocol() {
        for stage in Stage::ALL {
            assert_eq!(Stage::parse(stage.name()).unwrap(), stage);
            let source = observer_source(stage, "struct Q; relation R(x:i32); answer(x) :- R(x);");
            syn::parse_file(&source).unwrap();
            assert!(source.contains("stage = ::core::stringify"));
            assert!(source.contains("pipeline {}"));
            assert!(source.contains(&format!("boundary = {};", stage.name().replace('-', "_"))));
        }
    }

    #[test]
    fn invalid_and_duplicate_stage_options_are_rejected() {
        assert!(Stage::parse("missing").is_err());
        assert!(Options::parse(vec!["triangle".into(), "--through".into()]).is_err());
        assert!(
            Options::parse(vec![
                "triangle".into(),
                "--through".into(),
                "vo".into(),
                "--through".into(),
                "grade".into()
            ])
            .is_err()
        );
    }

    #[test]
    fn shallower_runs_remove_stale_artifacts() {
        let directory =
            std::env::temp_dir().join(format!("mini-linq-explain-logical-{}", std::process::id()));
        let case = cq_workloads::case("triangle").unwrap();
        let generated = directory.join("target/mini-linq-explain/triangle");
        fs::create_dir_all(&generated).unwrap();
        fs::write(generated.join(Stage::Rust.file_name()), "obsolete").unwrap();
        let artifacts = Artifacts::new(&directory, case, Stage::Cq).unwrap();
        artifacts
            .write_named(&Stage::Cq.file_name(), "source", "cq")
            .unwrap();
        assert!(generated.join(Stage::Cq.file_name()).exists());
        assert!(!generated.join(Stage::Rust.file_name()).exists());
        assert!(
            !generated
                .join(Stage::IndexRequirements.file_name())
                .exists()
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
