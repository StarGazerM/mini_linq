use std::error::Error;
use std::fmt;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use cq_ir::{cq, index_requirements, relational_plan};
use cq_query_lowering::composition::compile_cq;
use cq_workloads::QueryCase;
use quote::ToTokens;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const ERROR_FILE: &str = "ERROR.md";

/// One independently inspectable compiler boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Cq,
    RelationalPlan,
    IndexRequirements,
}

impl Stage {
    const ALL: [Self; 3] = [Self::Cq, Self::RelationalPlan, Self::IndexRequirements];

    fn parse(value: &str) -> Result<Self> {
        match value {
            "cq" => Ok(Self::Cq),
            "relational-plan" => Ok(Self::RelationalPlan),
            "index-requirements" => Ok(Self::IndexRequirements),
            "physical" | "rust-access-plan" | "iterator-pipeline" | "final-rust" => Err(
                "explain stops at IX: workload-wide physical provider assignments and the new host input/output contract are not specified; the retired whole-query backend cannot be used".into()
            ),
            _ => Err(format!(
                "unknown explain stage `{value}`; expected cq, relational-plan, or index-requirements"
            )
            .into()),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Cq => "cq",
            Self::RelationalPlan => "relational-plan",
            Self::IndexRequirements => "index-requirements",
        }
    }

    const fn file_name(self) -> &'static str {
        match self {
            Self::Cq => "01-cq.tokens",
            Self::RelationalPlan => "02-relational-plan.tokens",
            Self::IndexRequirements => "03-index-requirements.tokens",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Cq => "the complete source conjunctive query",
            Self::RelationalPlan => "the complete named logical relational-algebra plan",
            Self::IndexRequirements => {
                "the logical plan plus named index demands and explicit use references"
            }
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
            through: through.unwrap_or(Stage::IndexRequirements),
        })
    }
}

pub fn run(arguments: Vec<String>, repository_root: &Path) -> Result<()> {
    let options = Options::parse(arguments)?;
    let case = cq_workloads::case(&options.case)
        .ok_or_else(|| format!("unknown catalog case `{}`", options.case))?;
    explain_case(case, options.through, repository_root)
}

fn explain_case(case: &QueryCase, through: Stage, repository_root: &Path) -> Result<()> {
    let artifacts = Artifacts::new(repository_root, case, through)?;

    let source = artifacts.attempt("catalog query -> CQ", || case.module())?;
    artifacts.write_stage(Stage::Cq, &render_cq(&source))?;
    artifacts.attempt("CQ local contract", || cq::contract::check(&source))?;
    if through == Stage::Cq {
        return artifacts.finish(through);
    }

    let logical = artifacts.attempt("CQ -> RelationalPlan", || compile_cq(&source))?;
    artifacts.attempt("RelationalPlan local contract", || {
        relational_plan::contract::check(&logical)
    })?;
    artifacts.write_stage(Stage::RelationalPlan, &render_relational_plan(&logical))?;
    if through == Stage::RelationalPlan {
        return artifacts.finish(through);
    }

    let indexes = artifacts.attempt("RelationalPlan -> IndexRequirements", || {
        cq_index_policy::compile(&logical)
    })?;
    artifacts.attempt("IndexRequirements local contract", || {
        index_requirements::contract::check(&indexes)
    })?;
    artifacts.write_stage(
        Stage::IndexRequirements,
        &render_index_requirements(&indexes),
    )?;
    artifacts.finish(through)
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
                    stage.description(),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let readme = format!(
            "# MiniLinq typed boundary report: `{}`\n\n\
             Generated from `{}` by:\n\n\
             ```text\n\
             cargo xtask explain {} --through {}\n\
             ```\n\n\
             Each `.tokens` file is a complete typed compiler payload. These are typed pass results, not captured macro expansions. \
             Open adjacent files with VS Code's **Select for Compare** and **Compare with Selected** commands. \
             Files after the requested stopping stage are deliberately absent.\n\n\
             | File | Stage | Meaning | This run |\n\
             | --- | --- | --- | --- |\n\
             {rows}\n\n\
             If a compiler edge fails, earlier files remain and `ERROR.md` names that edge.\n",
            case.name,
            case.rust_path,
            case.name,
            through.name(),
        );
        self.write_named("README.md", &readme, "trace guide")
    }

    fn write_stage(&self, stage: Stage, contents: &str) -> Result<()> {
        self.write_named(stage.file_name(), contents, stage.name())
    }

    fn write_named(&self, file_name: &str, contents: &str, label: &str) -> Result<()> {
        let path = self.directory.join(file_name);
        fs::write(&path, contents)?;
        println!("wrote {label:<20} {}", path.display());
        Ok(())
    }

    fn attempt<T, E, F>(&self, edge: &str, operation: F) -> Result<T>
    where
        E: fmt::Display,
        F: FnOnce() -> std::result::Result<T, E>,
    {
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(self.record_failure(edge, &error.to_string())?),
            Err(payload) => {
                let diagnostic = panic_diagnostic(payload.as_ref());
                Err(self.record_failure(edge, &diagnostic)?)
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
                "# MiniLinq compiler trace stopped\n\n\
                 Case: `{}`\n\n\
                 Failed edge: `{edge}`\n\n\
                 Diagnostic:\n\n{indented}\n\n\
                 Every earlier `.tokens` artifact listed in this directory was produced successfully. \
                 No later compiler pass was called.\n",
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

    fn finish(&self, through: Stage) -> Result<()> {
        println!(
            "stopped after {:<12} {}",
            through.name(),
            self.directory.display(),
        );
        Ok(())
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
            self.report.display(),
        )
    }
}

impl Error for ExplainFailure {}

fn panic_diagnostic(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        format!("compiler panicked: {message}")
    } else if let Some(message) = payload.downcast_ref::<&'static str>() {
        format!("compiler panicked: {message}")
    } else {
        "compiler panicked with a non-string payload".to_owned()
    }
}

fn render_cq(source: &cq::Module) -> String {
    format!("{}\n", format_cq_program(&source.program))
}

fn render_relational_plan(source: &relational_plan::Module) -> String {
    let body = format!(
        "{}\n{}",
        format_cq_program(&source.program),
        format_relational_block(&source.plan),
    );
    format!("{body}\n")
}

fn render_index_requirements(source: &index_requirements::Module) -> String {
    let body = format!(
        "{}\n{}\n{}",
        format_cq_program(&source.relational_plan.program),
        format_relational_block(&source.relational_plan.plan),
        format_index_block(source),
    );
    format!("{body}\n")
}

fn format_cq_program(program: &cq::Program) -> String {
    let visibility = compact_tokens(&program.visibility);
    let declaration = if visibility.is_empty() {
        format!("struct {};", program.name)
    } else {
        format!("{visibility} struct {};", program.name)
    };
    let inputs = program.declarations.iter().map(|input| {
        let mut columns = input
            .columns
            .iter()
            .map(|column| format!("{}: {}", column.name, compact_tokens(&column.ty)))
            .collect::<Vec<_>>()
            .join(", ");
        if input.columns.trailing_punct() {
            columns.push(',');
        }
        format!("relation {}({columns});", input.name)
    });
    let clauses = program
        .query
        .body
        .iter()
        .map(|item| match item {
            cq::BodyItem::Positive { atom } => format_atom(atom),
            _ => compact_tokens(item),
        })
        .collect::<Vec<_>>();
    let query = if clauses.len() == 1 {
        format!("{} :- {};", format_atom(&program.query.head), clauses[0],)
    } else {
        let mut lines = clauses
            .iter()
            .enumerate()
            .map(|(position, clause)| {
                let terminator = if position + 1 == clauses.len() {
                    ";"
                } else {
                    ","
                };
                format!("    {clause}{terminator}")
            })
            .collect::<Vec<_>>();
        lines.insert(0, format!("{} :-", format_atom(&program.query.head)));
        lines.join("\n")
    };

    std::iter::once(declaration)
        .chain(inputs)
        .chain(std::iter::once(query))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_relational_block(plan: &relational_plan::Plan) -> String {
    let definitions = plan.definitions.iter().map(|definition| {
        format!(
            "{} = {};",
            definition.result,
            format_relational_operator(&definition.operator),
        )
    });
    let output = format!(
        "output {} as {};",
        plan.output.input,
        format_atom(&plan.output.head),
    );
    let body = definitions
        .chain(std::iter::once(output))
        .collect::<Vec<_>>()
        .join("\n");
    format!("relational {{\n{}\n}}", indent_lines(&body, "    "))
}

fn format_relational_operator(operator: &relational_plan::Operator) -> String {
    match operator {
        relational_plan::Operator::Unit(_) => "unit".to_owned(),
        relational_plan::Operator::Semijoin(operator) => {
            format!("semijoin {} with {}", operator.left, operator.right)
        }
        relational_plan::Operator::Select(operator) => format!(
            "select {} if ({})",
            operator.input,
            quote::ToTokens::to_token_stream(&operator.condition)
        ),
        relational_plan::Operator::Extend(operator) => format!(
            "extend {} with {{{}}} value ({})",
            operator.input,
            quote::ToTokens::to_token_stream(&operator.columns),
            quote::ToTokens::to_token_stream(&operator.value)
        ),
        relational_plan::Operator::Rename(operator) => {
            let mapping = operator
                .mapping
                .iter()
                .map(|mapping| format!("{} -> {}", mapping.source, mapping.target))
                .collect::<Vec<_>>()
                .join(", ");
            format!("rename {} {{{mapping}}}", operator.relation)
        }
        relational_plan::Operator::NaturalJoin(operator) => {
            format!("natural_join {} with {}", operator.left, operator.right)
        }
        relational_plan::Operator::Project(operator) => format!(
            "project {} keep {{{}}}",
            operator.input,
            format_identifiers(&operator.attributes),
        ),
    }
}

fn format_identifiers<'a>(identifiers: impl IntoIterator<Item = &'a syn::Ident>) -> String {
    identifiers
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_index_block(module: &index_requirements::Module) -> String {
    let definitions = module
        .indexes
        .iter()
        .map(|index| {
            let columns = index
                .requirement
                .columns
                .iter()
                .map(|column| column.index.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{} = @{}[{columns}];",
                index.name, index.requirement.relation
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let uses = module
        .uses
        .iter()
        .map(|usage| format!("{}.right = {};", usage.result, usage.index))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "indexes {{\n{}\n}}\nuses {{\n{}\n}}",
        indent_lines(&definitions, "    "),
        indent_lines(&uses, "    ")
    )
}

fn format_atom(atom: &cq::Atom) -> String {
    let mut variables = atom
        .args
        .iter()
        .map(compact_tokens)
        .collect::<Vec<_>>()
        .join(", ");
    if atom.args.trailing_punct() {
        variables.push(',');
    }
    format!("{}({variables})", atom.relation)
}

fn compact_tokens(value: &impl ToTokens) -> String {
    // Token serialization preserves literal contents and punctuation jointness.
    // Textual whitespace replacement can change arbitrary embedded Rust.
    value.to_token_stream().to_string()
}

fn indent_lines(text: &str, indentation: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{indentation}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_stop_at_ix_and_reject_retired_backend_stages() {
        assert_eq!(
            Options::parse(vec!["triangle".into()]).unwrap().through,
            Stage::IndexRequirements
        );
        for stage in [
            "physical",
            "rust-access-plan",
            "iterator-pipeline",
            "final-rust",
        ] {
            let error = Options::parse(vec!["triangle".into(), "--through".into(), stage.into()])
                .err()
                .unwrap();
            assert!(error.to_string().contains("provider assignments"));
        }
        assert!(Options::parse(Vec::new()).is_err());
        assert!(Options::parse(vec!["triangle".into(), "extra".into()]).is_err());
    }

    #[test]
    fn every_catalog_case_reaches_ix_and_each_payload_roundtrips() {
        for case in cq_workloads::catalog::all() {
            let source = case.module().unwrap();
            assert_eq!(
                syn::parse_str::<cq::Module>(&render_cq(&source)).unwrap(),
                source
            );
            let logical = compile_cq(&source).unwrap();
            assert_eq!(
                syn::parse_str::<relational_plan::Module>(&render_relational_plan(&logical))
                    .unwrap(),
                logical
            );
            let indexes = cq_index_policy::compile(&logical).unwrap();
            assert_eq!(
                syn::parse_str::<index_requirements::Module>(&render_index_requirements(&indexes))
                    .unwrap(),
                indexes
            );
            index_requirements::contract::check(&indexes).unwrap();
        }
    }

    #[test]
    fn embedded_rust_literals_are_preserved() {
        let source: cq::Module = syn::parse_quote! {
            struct Expressions;
            relation R(value: i32);
            relation answer(value: String);
            answer(format!(" :: ( & * ! > {x}")) :-
                R(x), let label = " :: ( & * ! >".to_owned(), if *x > 0 && !label.is_empty();
        };
        assert_eq!(
            syn::parse_str::<cq::Module>(&render_cq(&source)).unwrap(),
            source
        );
    }

    #[test]
    fn shallower_runs_remove_stale_physical_artifacts() {
        let directory =
            std::env::temp_dir().join(format!("mini-linq-explain-logical-{}", std::process::id()));
        let case = cq_workloads::case("triangle").unwrap();
        let generated = directory.join("target/mini-linq-explain/triangle");
        fs::create_dir_all(&generated).unwrap();
        fs::write(generated.join("06-final-rust.rs"), "obsolete").unwrap();
        explain_case(case, Stage::Cq, &directory).unwrap();
        assert!(generated.join(Stage::Cq.file_name()).exists());
        assert!(!generated.join("06-final-rust.rs").exists());
        assert!(
            !generated
                .join(Stage::IndexRequirements.file_name())
                .exists()
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
