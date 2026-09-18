use std::error::Error;
use std::fmt;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use cq_access_lowering::to_pipeline::compile as compile_pull;
use cq_compose::{compile_cq, select_indexes};
use cq_ir::{cq, index_requirements, iterator_pipeline, relational_plan, rust_access_plan};
use cq_pipeline_lowering::distinct_iter::compile as compile_iterator_pipeline;
use cq_workloads::QueryCase;
use quote::{ToTokens, quote};
use syn::visit_mut::{self, VisitMut};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const ERROR_FILE: &str = "ERROR.md";

/// One independently inspectable compiler boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Cq,
    RelationalPlan,
    IndexRequirements,
    RustAccessPlan,
    IteratorPipeline,
    FinalRust,
}

impl Stage {
    const ALL: [Self; 6] = [
        Self::Cq,
        Self::RelationalPlan,
        Self::IndexRequirements,
        Self::RustAccessPlan,
        Self::IteratorPipeline,
        Self::FinalRust,
    ];

    fn parse(value: &str) -> Result<Self> {
        match value {
            "cq" => Ok(Self::Cq),
            "relational-plan" => Ok(Self::RelationalPlan),
            "index-requirements" => Ok(Self::IndexRequirements),
            "rust-access-plan" => Ok(Self::RustAccessPlan),
            "iterator-pipeline" => Ok(Self::IteratorPipeline),
            "final-rust" => Ok(Self::FinalRust),
            _ => Err(format!(
                "unknown explain stage `{value}`; expected cq, relational-plan, index-requirements, rust-access-plan, iterator-pipeline, or final-rust"
            )
            .into()),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Cq => "cq",
            Self::RelationalPlan => "relational-plan",
            Self::IndexRequirements => "index-requirements",
            Self::RustAccessPlan => "rust-access-plan",
            Self::IteratorPipeline => "iterator-pipeline",
            Self::FinalRust => "final-rust",
        }
    }

    const fn file_name(self) -> &'static str {
        match self {
            Self::Cq => "01-cq.rs",
            Self::RelationalPlan => "02-relational-plan.rs",
            Self::IndexRequirements => "03-index-requirements.rs",
            Self::RustAccessPlan => "04-rust-access-plan.rs",
            Self::IteratorPipeline => "05-iterator-pipeline.rs",
            Self::FinalRust => "06-final-rust.rs",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Cq => "the complete source conjunctive query",
            Self::RelationalPlan => "the complete named logical relational-algebra plan",
            Self::IndexRequirements => {
                "the logical plan plus its exact equality-index requirements"
            }
            Self::RustAccessPlan => {
                "the complete staged Rust file containing one RustAccessPlan in `access_to_distinct_iter!`"
            }
            Self::IteratorPipeline => {
                "the same staged Rust file after `access_to_distinct_iter!` exposes named lazy operators"
            }
            Self::FinalRust => "the complete ordinary Rust file after iterator lowering",
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
            through: through.unwrap_or(Stage::FinalRust),
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
        select_indexes(&logical)
    })?;
    artifacts.attempt("IndexRequirements local contract", || {
        index_requirements::contract::check(&indexes)
    })?;
    artifacts.write_stage(
        Stage::IndexRequirements,
        &render_index_requirements(&indexes),
    )?;
    if through == Stage::IndexRequirements {
        return artifacts.finish(through);
    }

    let file = artifacts.attempt("IndexRequirements -> RustAccessPlan", || {
        if cq_relation_lowering::base_hash::check_domain(&indexes).is_ok() {
            cq_relation_lowering::base_hash::compile(
                &indexes,
                &syn::parse_quote!(::mini_linq::access_to_distinct_iter),
            )
        } else {
            cq_relation_lowering::prefix_hash::compile(
                &indexes,
                &syn::parse_quote!(::mini_linq::access_to_distinct_iter),
            )
        }
    })?;
    artifacts.attempt("RustAccessPlan local contract", || {
        check_rust_access_plan_once(&file)
    })?;
    artifacts.write_stage(Stage::RustAccessPlan, &render_rust_access_file(&file))?;
    if through == Stage::RustAccessPlan {
        return artifacts.finish(through);
    }

    let mut pipeline_file = file;
    artifacts.attempt("RustAccessPlan -> IteratorPipeline", || {
        lower_pull_once(&mut pipeline_file)
    })?;
    artifacts.write_stage(
        Stage::IteratorPipeline,
        &render_iterator_pipeline_file(&pipeline_file),
    )?;
    if through == Stage::IteratorPipeline {
        return artifacts.finish(through);
    }

    let mut final_file = pipeline_file;
    artifacts.attempt("IteratorPipeline -> final Rust", || {
        lower_iterator_pipeline_once(&mut final_file)
    })?;
    artifacts.write_stage(Stage::FinalRust, &render_rust_file(&final_file))?;
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
            "# MiniLinq compiler trace: `{}`\n\n\
             Generated from `{}` by:\n\n\
             ```text\n\
             cargo xtask explain {} --through {}\n\
             ```\n\n\
             Each `.rs` file is the complete program at one compiler boundary, not a fragment. \
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
                 Every earlier `.rs` artifact listed in this directory was produced successfully. \
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
    render_ir_macro("mini_linq", &format_cq_program(&source.program))
}

fn render_relational_plan(source: &relational_plan::Module) -> String {
    let body = format!(
        "{}\n{}",
        format_cq_program(&source.program),
        format_relational_block(&source.plan),
    );
    render_ir_macro("relational_plan", &body)
}

fn render_index_requirements(source: &index_requirements::Module) -> String {
    let body = format!(
        "{}\n{}\n{}",
        format_cq_program(&source.relational_plan.program),
        format_relational_block(&source.relational_plan.plan),
        format_index_block(source),
    );
    render_ir_macro("index_requirements", &body)
}

fn render_rust_file(file: &syn::File) -> String {
    prettyplease::unparse(file)
}

fn render_rust_access_file(file: &syn::File) -> String {
    render_file_with_readable_macro(file, NestedMacro::Pull)
        .unwrap_or_else(|| render_rust_file(file))
}

fn render_iterator_pipeline_file(file: &syn::File) -> String {
    render_file_with_readable_macro(file, NestedMacro::IteratorPipeline)
        .unwrap_or_else(|| render_rust_file(file))
}

fn render_ir_macro(name: &str, body: &str) -> String {
    format!(
        "::mini_linq::{name}! {{\n{}\n}}\n",
        indent_lines(body, "    "),
    )
}

fn format_cq_program(program: &cq::Program) -> String {
    let visibility = compact_tokens(&program.visibility);
    let declaration = if visibility.is_empty() {
        format!("struct {};", program.name)
    } else {
        format!("{visibility} struct {};", program.name)
    };
    let inputs = program.declarations.iter().map(|input| {
        let columns = input
            .columns
            .iter()
            .map(|column| format!("{}: {}", column.name, compact_tokens(&column.ty)))
            .collect::<Vec<_>>()
            .join(", ");
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
    let body = module
        .indexes
        .iter()
        .map(|index| {
            let columns = index
                .key_columns
                .iter()
                .map(|column| column.index.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let namespace = if index.result_token.is_some() {
                "@"
            } else {
                ""
            };
            format!("{namespace}{}[{columns}];", index.relation)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("indexes {{\n{}\n}}", indent_lines(&body, "    "))
}

fn format_atom(atom: &cq::Atom) -> String {
    let variables = atom
        .args
        .iter()
        .map(compact_tokens)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}({variables})", atom.relation)
}

fn compact_tokens(value: &impl ToTokens) -> String {
    // Token serialization preserves literal contents and punctuation jointness.
    // Textual whitespace replacement can change arbitrary embedded Rust.
    value.to_token_stream().to_string()
}

fn format_rust_expression(expression: &syn::Expr) -> String {
    let file: syn::File = syn::parse2(quote! {
        fn __mini_linq_format_expression() {
            #expression
        }
    })
    .expect("an existing Rust expression must remain valid in a function body");
    let rendered = prettyplease::unparse(&file);
    let body_start = rendered
        .find("{\n")
        .expect("prettyplease function output has an opening brace")
        + 2;
    let body_end = rendered
        .rfind("\n}")
        .expect("prettyplease function output has a closing brace");
    rendered[body_start..body_end]
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
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

#[derive(Clone, Copy)]
enum NestedMacro {
    Pull,
    IteratorPipeline,
}

impl NestedMacro {
    const fn name(self) -> &'static str {
        match self {
            Self::Pull => "access_to_distinct_iter",
            Self::IteratorPipeline => "pipeline_to_distinct_iter",
        }
    }

    fn matches(self, path: &syn::Path) -> bool {
        path.leading_colon.is_some()
            && path.segments.len() == 2
            && path.segments[0].ident == "mini_linq"
            && path.segments[1].ident == self.name()
    }
}

const MACRO_BODY_SENTINEL: &str = "__mini_linq_explain_macro_body__";

fn render_file_with_readable_macro(file: &syn::File, target: NestedMacro) -> Option<String> {
    let mut masked = file.clone();
    let mut masker = MacroBodyMasker {
        target,
        count: 0,
        body: None,
    };
    masker.visit_file_mut(&mut masked);
    if masker.count != 1 {
        return None;
    }
    let tokens = masker.body?;
    let body = match target {
        NestedMacro::Pull => {
            let plan = syn::parse2::<rust_access_plan::Plan>(tokens).ok()?;
            format_rust_access_plan(&plan)
        }
        NestedMacro::IteratorPipeline => {
            let plan = syn::parse2::<iterator_pipeline::Pipeline>(tokens).ok()?;
            format_iterator_pipeline(&plan)
        }
    };

    let rendered = prettyplease::unparse(&masked);
    let sentinel = rendered.find(MACRO_BODY_SENTINEL)?;
    let line_start = rendered[..sentinel]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let indentation = &rendered[line_start..sentinel];
    if !indentation.chars().all(char::is_whitespace) {
        return None;
    }
    let body = indent_lines(&body, indentation);
    Some(rendered.replacen(MACRO_BODY_SENTINEL, body.trim_start(), 1))
}

struct MacroBodyMasker {
    target: NestedMacro,
    count: usize,
    body: Option<proc_macro2::TokenStream>,
}

impl VisitMut for MacroBodyMasker {
    fn visit_macro_mut(&mut self, invocation: &mut syn::Macro) {
        if self.target.matches(&invocation.path) {
            self.count += 1;
            if self.count == 1 {
                self.body = Some(invocation.tokens.clone());
            }
            invocation.tokens = quote!(__mini_linq_explain_macro_body__);
        } else {
            visit_mut::visit_macro_mut(self, invocation);
        }
    }
}

fn format_rust_access_plan(plan: &rust_access_plan::Plan) -> String {
    let mut lines = format_wrapped_expression(
        format!("{} => (", format_atom(&plan.head.atom)),
        format_rust_expression(&plan.head.output),
        ") :-".to_owned(),
    );
    for (position, clause) in plan.body.iter().enumerate() {
        let terminator = if position + 1 == plan.body.len() {
            ";"
        } else {
            ","
        };
        let item = match &clause.item {
            cq::BodyItem::Positive { atom } => format_atom(atom),
            item => compact_tokens(item),
        };
        match &clause.access {
            rust_access_plan::RustAccess::For {
                pattern, source, ..
            } => {
                let prefix = format!("    {item} => for {} in (", compact_tokens(pattern),);
                lines.extend(format_wrapped_expression(
                    prefix,
                    format_rust_expression(source),
                    format!("){terminator}"),
                ));
            }
            rust_access_plan::RustAccess::If { condition, .. } => {
                lines.extend(format_wrapped_expression(
                    format!("    {item} => if ("),
                    format_rust_expression(condition),
                    format!("){terminator}"),
                ));
            }
        }
    }
    lines.join("\n")
}

fn format_iterator_pipeline(plan: &iterator_pipeline::Pipeline) -> String {
    let mut lines = Vec::new();
    for definition in &plan.definitions {
        let stream = &definition.stream;
        match &definition.operator {
            iterator_pipeline::Operator::Unit(operator) => lines.push(format!(
                "{stream} = unit yield {};",
                compact_tokens(&operator.binding),
            )),
            iterator_pipeline::Operator::Scan(operator) => {
                lines.extend(format_iterator_source(
                    format!(
                        "{stream} = scan {} in (",
                        compact_tokens(&operator.item_pattern),
                    ),
                    format_rust_expression(&operator.source),
                    format!(") yield {};", compact_tokens(&operator.binding)),
                ));
            }
            iterator_pipeline::Operator::Join(operator) => {
                lines.extend(format_iterator_source(
                    format!(
                        "{stream} = join {} as {} with {} in (",
                        operator.input_stream,
                        compact_tokens(&operator.input_pattern),
                        compact_tokens(&operator.item_pattern),
                    ),
                    format_rust_expression(&operator.source),
                    format!(") yield {};", compact_tokens(&operator.binding)),
                ));
            }
            iterator_pipeline::Operator::Filter(operator) => {
                lines.extend(format_wrapped_expression(
                    format!(
                        "{stream} = filter {} as {} if (",
                        operator.input_stream,
                        compact_tokens(&operator.input_pattern),
                    ),
                    format_rust_expression(&operator.condition),
                    ");".to_owned(),
                ));
            }
            iterator_pipeline::Operator::Project(operator) => {
                lines.extend(format_wrapped_expression(
                    format!(
                        "{stream} = project {} as {} yield (",
                        operator.input_stream,
                        compact_tokens(&operator.input_pattern),
                    ),
                    format_rust_expression(&operator.output),
                    ");".to_owned(),
                ));
            }
            iterator_pipeline::Operator::Distinct(operator) => {
                lines.push(format!("{stream} = distinct {};", operator.input_stream,))
            }
        }
    }
    lines.push(format!("return {};", plan.return_stream.stream));
    lines.join("\n")
}

fn format_iterator_source(prefix: String, source: String, suffix: String) -> Vec<String> {
    format_wrapped_expression(prefix, source, suffix)
}

fn format_wrapped_expression(prefix: String, expression: String, suffix: String) -> Vec<String> {
    if !expression.contains('\n') && prefix.len() + expression.len() + suffix.len() <= 92 {
        vec![format!("{prefix}{expression}{suffix}")]
    } else {
        let mut lines = vec![prefix];
        lines.extend(expression.lines().map(|line| format!("    {line}")));
        lines.push(suffix);
        lines
    }
}

fn lower_pull_once(file: &mut syn::File) -> syn::Result<()> {
    require_exactly_one_macro(file, NestedMacro::Pull)?;
    let mut lowerer = PullLowerer::default();
    lowerer.visit_file_mut(file);
    lowerer.finish()
}

fn check_rust_access_plan_once(file: &syn::File) -> syn::Result<()> {
    require_exactly_one_macro(file, NestedMacro::Pull)?;
    let mut copy = file.clone();
    let mut collector = MacroBodyMasker {
        target: NestedMacro::Pull,
        count: 0,
        body: None,
    };
    collector.visit_file_mut(&mut copy);
    let tokens = collector
        .body
        .expect("exactly one access_to_distinct_iter macro was counted");
    let source: rust_access_plan::Plan = syn::parse2(tokens)?;
    rust_access_plan::contract::check(&source)
}

fn lower_iterator_pipeline_once(file: &mut syn::File) -> syn::Result<()> {
    require_exactly_one_macro(file, NestedMacro::IteratorPipeline)?;
    let mut lowerer = IteratorPipelineLowerer::default();
    lowerer.visit_file_mut(file);
    lowerer.finish()
}

fn require_exactly_one_macro(file: &syn::File, target: NestedMacro) -> syn::Result<()> {
    let mut copy = file.clone();
    let mut counter = MacroCounter { target, count: 0 };
    counter.visit_file_mut(&mut copy);
    if counter.count == 1 {
        Ok(())
    } else {
        Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "staged Rust contains {} `::mini_linq::{}!` invocations; expected exactly one",
                counter.count,
                target.name(),
            ),
        ))
    }
}

struct MacroCounter {
    target: NestedMacro,
    count: usize,
}

impl VisitMut for MacroCounter {
    fn visit_macro_mut(&mut self, invocation: &mut syn::Macro) {
        if self.target.matches(&invocation.path) {
            self.count += 1;
        }
        visit_mut::visit_macro_mut(self, invocation);
    }
}

#[derive(Default)]
struct PullLowerer {
    replacements: usize,
    error: Option<syn::Error>,
}

impl PullLowerer {
    fn lower(&mut self, invocation: &syn::Macro) -> syn::Result<iterator_pipeline::Pipeline> {
        let source: rust_access_plan::Plan = syn::parse2(invocation.tokens.clone())?;
        let target = compile_pull(&source)?;
        iterator_pipeline::contract::check(&target)?;
        Ok(target)
    }

    fn finish(self) -> syn::Result<()> {
        if let Some(error) = self.error {
            Err(error)
        } else if self.replacements == 1 {
            Ok(())
        } else {
            Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "the one `::mini_linq::access_to_distinct_iter!` invocation is not in expression position",
            ))
        }
    }
}

impl VisitMut for PullLowerer {
    fn visit_stmt_mut(&mut self, statement: &mut syn::Stmt) {
        let invocation = match statement {
            syn::Stmt::Macro(statement)
                if statement.semi_token.is_none()
                    && NestedMacro::Pull.matches(&statement.mac.path) =>
            {
                Some(statement.mac.clone())
            }
            _ => None,
        };
        let Some(invocation) = invocation else {
            visit_mut::visit_stmt_mut(self, statement);
            return;
        };

        match self.lower(&invocation).and_then(|target| {
            syn::parse2(quote! {
                ::mini_linq::pipeline_to_distinct_iter! { #target }
            })
        }) {
            Ok(lowered) => {
                *statement = lowered;
                self.replacements += 1;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
        let invocation = match expression {
            syn::Expr::Macro(expression) if NestedMacro::Pull.matches(&expression.mac.path) => {
                Some(expression.mac.clone())
            }
            _ => None,
        };
        let Some(invocation) = invocation else {
            visit_mut::visit_expr_mut(self, expression);
            return;
        };

        match self.lower(&invocation).and_then(|target| {
            syn::parse2(quote! {
                ::mini_linq::pipeline_to_distinct_iter! { #target }
            })
        }) {
            Ok(lowered) => {
                *expression = lowered;
                self.replacements += 1;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

#[derive(Default)]
struct IteratorPipelineLowerer {
    replacements: usize,
    error: Option<syn::Error>,
}

impl IteratorPipelineLowerer {
    fn lower(&mut self, invocation: &syn::Macro) -> syn::Result<syn::Expr> {
        let source: iterator_pipeline::Pipeline = syn::parse2(invocation.tokens.clone())?;
        compile_iterator_pipeline(&source)
    }

    fn finish(self) -> syn::Result<()> {
        if let Some(error) = self.error {
            Err(error)
        } else if self.replacements == 1 {
            Ok(())
        } else {
            Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "the one `::mini_linq::pipeline_to_distinct_iter!` invocation is not in expression position",
            ))
        }
    }
}

impl VisitMut for IteratorPipelineLowerer {
    fn visit_stmt_mut(&mut self, statement: &mut syn::Stmt) {
        let invocation = match statement {
            syn::Stmt::Macro(statement)
                if statement.semi_token.is_none()
                    && NestedMacro::IteratorPipeline.matches(&statement.mac.path) =>
            {
                Some(statement.mac.clone())
            }
            _ => None,
        };
        let Some(invocation) = invocation else {
            visit_mut::visit_stmt_mut(self, statement);
            return;
        };

        match self.lower(&invocation) {
            Ok(lowered) => {
                *statement = syn::Stmt::Expr(lowered, None);
                self.replacements += 1;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
        let invocation = match expression {
            syn::Expr::Macro(expression)
                if NestedMacro::IteratorPipeline.matches(&expression.mac.path) =>
            {
                Some(expression.mac.clone())
            }
            _ => None,
        };
        let Some(invocation) = invocation else {
            visit_mut::visit_expr_mut(self, expression);
            return;
        };

        match self.lower(&invocation) {
            Ok(lowered) => {
                *expression = lowered;
                self.replacements += 1;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestRoot {
        path: PathBuf,
    }

    impl TestRoot {
        fn new() -> Self {
            loop {
                let nonce = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "mini-linq-explain-test-{}-{nonce}",
                    std::process::id(),
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!(
                        "failed to create explain test directory {}: {error}",
                        path.display()
                    ),
                }
            }
        }

        fn case_directory(&self, case: &QueryCase) -> PathBuf {
            self.path.join("target/mini-linq-explain").join(case.name)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap_or_else(|error| {
                panic!(
                    "failed to remove explain test directory {}: {error}",
                    self.path.display()
                )
            });
        }
    }

    fn triangle() -> &'static QueryCase {
        cq_workloads::case("triangle").unwrap()
    }

    #[test]
    fn expression_artifacts_preserve_literals_arguments_and_result_schema() {
        let source: cq::Module = syn::parse_quote! {
            struct Expressions;
            relation R(value: i32);
            relation answer(value: String);
            answer(format!(" :: ( & * ! > {x}")) :-
                R(x), let label = " :: ( & * ! >".to_owned(),
                if *x > 0 && !label.is_empty();
        };
        let rendered = format_cq_program(&source.program);
        let restored: cq::Module = syn::parse_str(&rendered).unwrap();
        assert_eq!(restored, source);
        let extended: relational_plan::Operator = syn::parse_quote! {
            extend r0 with {label: String} value ((" :: ( & * ! >".to_owned(),))
        };
        let restored: relational_plan::Operator =
            syn::parse_str(&format_relational_operator(&extended)).unwrap();
        assert_eq!(restored, extended);
    }

    #[test]
    fn formatted_query_and_access_bodies_roundtrip_with_semicolons() {
        for body in [
            "R(x)",
            "R(x), R(x)",
            r#"R("a , b :: c ( d )"), if accept(x, " ; ")"#,
            "R(x), let y = (|a, b| a + b)(x, 1), !R(f(y))",
            "R(x), agg y = (factory(config))(value) in R(f(x, value))",
        ] {
            let program: cq::Program = syn::parse_str(&format!(
                "struct P; relation R(a:i32); answer(x) :- {body};"
            ))
            .unwrap();
            assert_eq!(
                syn::parse_str::<cq::Program>(&format_cq_program(&program)).unwrap(),
                program
            );
        }
        let plan: rust_access_plan::Plan = syn::parse_str(
            "answer(x) => ((x,)) :- R(x) => for (x,) in (rows.iter()), R(x) => if (x.is_positive());",
        )
        .unwrap();
        assert_eq!(
            syn::parse_str::<rust_access_plan::Plan>(&format_rust_access_plan(&plan)).unwrap(),
            plan
        );
    }

    #[test]
    fn options_require_one_known_shape_and_default_to_final_rust() {
        let options = Options::parse(vec!["triangle".to_owned()]).unwrap();
        assert_eq!(options.case, "triangle");
        assert_eq!(options.through, Stage::FinalRust);

        assert!(Options::parse(Vec::new()).is_err());
        assert!(Options::parse(vec!["triangle".into(), "extra".into()]).is_err());
        assert!(Options::parse(vec!["triangle".into(), "--bad".into()]).is_err());
        assert!(Options::parse(vec!["triangle".into(), "--through".into()]).is_err());
        assert!(
            Options::parse(vec![
                "triangle".into(),
                "--through".into(),
                "not-a-stage".into(),
            ])
            .is_err()
        );
    }

    #[test]
    fn unknown_catalog_case_is_rejected_before_creating_artifacts() {
        let root = TestRoot::new();
        let result = run(vec!["not-a-case".to_owned()], &root.path);

        assert!(result.is_err());
        assert!(!root.path.join("target/mini-linq-explain").exists());
    }

    #[test]
    fn cq_stage_stops_without_calling_or_writing_later_stages() {
        let root = TestRoot::new();
        explain_case(triangle(), Stage::Cq, &root.path).unwrap();
        let directory = root.case_directory(triangle());

        assert!(directory.join("README.md").is_file());
        assert!(directory.join(Stage::Cq.file_name()).is_file());
        for stage in [
            Stage::RelationalPlan,
            Stage::IndexRequirements,
            Stage::RustAccessPlan,
            Stage::IteratorPipeline,
            Stage::FinalRust,
        ] {
            assert!(!directory.join(stage.file_name()).exists());
        }
    }

    #[test]
    fn a_shallower_run_removes_stale_later_artifacts() {
        let root = TestRoot::new();
        let stale = Artifacts::new(&root.path, triangle(), Stage::FinalRust).unwrap();
        for stage in Stage::ALL {
            stale.write_stage(stage, "stale\n").unwrap();
        }
        explain_case(triangle(), Stage::Cq, &root.path).unwrap();
        let directory = root.case_directory(triangle());

        assert!(directory.join(Stage::Cq.file_name()).is_file());
        for stage in Stage::ALL.into_iter().skip(1) {
            assert!(!directory.join(stage.file_name()).exists());
        }
    }

    // These end-to-end checks become runnable after all three basic passes.
    #[test]
    fn every_catalog_case_reaches_complete_final_rust() {
        let root = TestRoot::new();

        for case in cq_workloads::catalog::all() {
            explain_case(case, Stage::FinalRust, &root.path).unwrap_or_else(|error| {
                panic!("`{}` did not reach final Rust: {error}", case.name)
            });
            let final_source =
                fs::read_to_string(root.case_directory(case).join(Stage::FinalRust.file_name()))
                    .unwrap();
            syn::parse_file(&final_source).unwrap_or_else(|error| {
                panic!("`{}` final artifact is not Rust syntax: {error}", case.name)
            });
        }
    }

    #[test]
    fn triangle_final_artifact_compiles_as_standalone_rust() {
        let root = TestRoot::new();
        explain_case(triangle(), Stage::FinalRust, &root.path).unwrap();
        let source = root
            .case_directory(triangle())
            .join(Stage::FinalRust.file_name());
        for stage in Stage::ALL {
            let path = root.case_directory(triangle()).join(stage.file_name());
            let text = fs::read_to_string(&path).unwrap();
            syn::parse_file(&text).unwrap_or_else(|error| {
                panic!(
                    "{} is not a complete Rust macro/file artifact: {error}",
                    path.display()
                )
            });
        }
        let library = root.path.join("triangle.rlib");
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
        let output = Command::new(rustc)
            .arg("--edition=2024")
            .arg("--crate-name")
            .arg("mini_linq_explained_triangle")
            .arg("--crate-type=lib")
            .arg(&source)
            .arg("-o")
            .arg(&library)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "final artifact did not compile\nstdout:\n{}\nstderr:\n{}\nsource: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            source.display(),
        );
    }

    #[test]
    fn custom_ir_and_nested_plan_bodies_are_readable_multiline_code() {
        let root = TestRoot::new();
        explain_case(triangle(), Stage::IteratorPipeline, &root.path).unwrap();
        let directory = root.case_directory(triangle());

        let cq = fs::read_to_string(directory.join(Stage::Cq.file_name())).unwrap();
        assert!(cq.contains("mini_linq! {\n    pub struct"));
        assert!(cq.contains("\n    relation R(c0: i32, c1: i32);"));
        assert!(cq.contains("\n        R(x, y),"));

        let relational =
            fs::read_to_string(directory.join(Stage::RelationalPlan.file_name())).unwrap();
        assert!(relational.contains("\n    relational {\n        r0"));
        assert!(relational.contains("\n        output r5 as triangle(x, y, z);"));
        assert!(relational.contains("natural_join r0 with r1"));
        assert!(relational.contains("natural_join r2 with r3"));

        let access = fs::read_to_string(directory.join(Stage::RustAccessPlan.file_name())).unwrap();
        assert!(access.contains("::mini_linq::access_to_distinct_iter! {\n"));
        assert!(access.contains("\n                R(x, y) => for"));
        assert!(access.contains("\n                T(z, x) => if"));

        let pipeline =
            fs::read_to_string(directory.join(Stage::IteratorPipeline.file_name())).unwrap();
        assert!(pipeline.contains("::mini_linq::pipeline_to_distinct_iter! {\n"));
        assert!(pipeline.contains("\n            iter0 = scan"));
        assert!(pipeline.contains("if (\n"));
        assert!(pipeline.contains("yield (\n"));
        assert!(pipeline.contains("\n            return iter"));
        assert!(pipeline.lines().all(|line| line.len() <= 100));
    }

    #[test]
    fn structural_lowerings_replace_only_their_one_typed_macro() {
        let mut file: syn::File = syn::parse2(quote! {
            fn query() {
                unrelated!();
                ::mini_linq::access_to_distinct_iter! {
                    answer(x) => (x) :- R(x) => for (x,) in ([1].iter());
                }
            }
        })
        .unwrap();
        let item_count = file.items.len();

        lower_pull_once(&mut file).unwrap();
        assert_eq!(file.items.len(), item_count);
        assert_eq!(count_macros(&file, NestedMacro::Pull), 0);
        assert_eq!(count_macros(&file, NestedMacro::IteratorPipeline), 1);
        assert!(file.to_token_stream().to_string().contains("unrelated !"));

        lower_iterator_pipeline_once(&mut file).unwrap();
        assert_eq!(file.items.len(), item_count);
        assert_eq!(count_macros(&file, NestedMacro::IteratorPipeline), 0);
        assert!(file.to_token_stream().to_string().contains("unrelated !"));
        assert!(file.to_token_stream().to_string().contains("once_with"));
    }

    #[test]
    fn duplicate_target_macros_are_rejected_before_rewriting() {
        let mut file: syn::File = syn::parse2(quote! {
            fn query() {
                ::mini_linq::access_to_distinct_iter! {
                    answer(x) => (x) :- R(x) => for x in ([1].iter());
                };
                ::mini_linq::access_to_distinct_iter! {
                    answer(x) => (x) :- R(x) => for x in ([1].iter());
                }
            }
        })
        .unwrap();
        let before = file.clone();

        assert!(lower_pull_once(&mut file).is_err());
        assert_eq!(file, before);
    }

    #[test]
    fn error_report_preserves_earlier_artifacts_and_names_the_edge() {
        let root = TestRoot::new();
        let artifacts = Artifacts::new(&root.path, triangle(), Stage::FinalRust).unwrap();
        artifacts.write_stage(Stage::Cq, "cq\n").unwrap();
        artifacts
            .write_stage(Stage::RelationalPlan, "relational\n")
            .unwrap();

        let result: Result<()> = artifacts.attempt("RelationalPlan -> IndexRequirements", || {
            Err::<(), _>("deliberate test failure")
        });
        assert!(result.is_err());

        let directory = root.case_directory(triangle());
        assert_eq!(
            fs::read_to_string(directory.join(Stage::Cq.file_name())).unwrap(),
            "cq\n"
        );
        assert_eq!(
            fs::read_to_string(directory.join(Stage::RelationalPlan.file_name())).unwrap(),
            "relational\n"
        );
        let report = fs::read_to_string(directory.join(ERROR_FILE)).unwrap();
        assert!(report.contains("RelationalPlan -> IndexRequirements"));
        assert!(report.contains("deliberate test failure"));
        assert!(
            !directory
                .join(Stage::IndexRequirements.file_name())
                .exists()
        );
    }

    #[test]
    fn panic_report_preserves_earlier_artifact_and_names_the_edge() {
        let root = TestRoot::new();
        let artifacts = Artifacts::new(&root.path, triangle(), Stage::FinalRust).unwrap();
        artifacts.write_stage(Stage::Cq, "cq\n").unwrap();

        let result: Result<()> = artifacts.attempt(
            "CQ -> RelationalPlan",
            || -> std::result::Result<(), &'static str> {
                panic!("student pass is not implemented")
            },
        );
        assert!(result.is_err());

        let directory = root.case_directory(triangle());
        assert!(directory.join(Stage::Cq.file_name()).is_file());
        let report = fs::read_to_string(directory.join(ERROR_FILE)).unwrap();
        assert!(report.contains("CQ -> RelationalPlan"));
        assert!(report.contains("compiler panicked: student pass is not implemented"));
        assert!(!directory.join(Stage::RelationalPlan.file_name()).exists());
    }

    fn count_macros(file: &syn::File, target: NestedMacro) -> usize {
        let mut copy = file.clone();
        let mut counter = MacroCounter { target, count: 0 };
        counter.visit_file_mut(&mut copy);
        counter.count
    }
}
