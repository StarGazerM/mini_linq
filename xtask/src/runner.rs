use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cq_compiler::{
    compile_cq, compile_index_requirements, compile_iterator_pipeline, compile_pull,
    compile_relational_plan,
};
use cq_ir::{cq, iterator_pipeline, rust_access_plan};
use cq_workloads::{QueryCase, Scale};
use proc_macro2::Span;
use quote::{ToTokens, format_ident, quote};
use syn::visit_mut::{self, VisitMut};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn compile_and_run(
    case: &QueryCase,
    scale: Scale,
    data_directory: &Path,
) -> Result<Vec<Vec<i32>>> {
    let source: cq::Module = case.module()?;
    let logical = compile_cq(&source)?;
    let indexes = compile_relational_plan(&logical)?;
    let mut file = compile_index_requirements(&indexes)?;
    let mut expander = IteratorPipelineExpander::default();
    expander.visit_file_mut(&mut file);
    if let Some(error) = expander.error {
        return Err(error.into());
    }
    if expander.pull_expansions != 1 || expander.pipeline_expansions != 1 {
        return Err(format!(
            "`{}` generated {} access-plan and {} iterator-plan expressions; expected exactly one of each",
            case.name, expander.pull_expansions, expander.pipeline_expansions,
        )
        .into());
    }

    file.items.extend(support_items(&source)?);
    let build_directory = build_directory(case);
    fs::create_dir_all(&build_directory)?;
    let source_path = build_directory.join("program.rs");
    let executable_path = build_directory.join("program");
    fs::write(&source_path, file.to_token_stream().to_string())?;

    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let crate_name = format!("cq_workload_{}", case.name.replace('-', "_"));
    let compilation = Command::new(&rustc)
        .arg("--edition=2024")
        .arg("--crate-name")
        .arg(crate_name)
        .arg(&source_path)
        .arg("-o")
        .arg(&executable_path)
        .output()?;
    if !compilation.status.success() {
        return Err(format!(
            "generated Rust for `{}` failed to compile\nstdout:\n{}\nstderr:\n{}\nsource: {}",
            case.name,
            String::from_utf8_lossy(&compilation.stdout),
            String::from_utf8_lossy(&compilation.stderr),
            source_path.display(),
        )
        .into());
    }

    let stdout_path = build_directory.join("stdout.csv");
    let stderr_path = build_directory.join("stderr.txt");
    let mut child = Command::new(&executable_path)
        .arg(data_directory)
        .stdout(Stdio::from(File::create(&stdout_path)?))
        .stderr(Stdio::from(File::create(&stderr_path)?))
        .spawn()?;
    let timeout = execution_timeout(scale);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Err(format!(
                "generated Rust for `{}` exceeded its {} second {}-scale timeout",
                case.name,
                timeout.as_secs(),
                scale.as_str(),
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = fs::read(&stdout_path)?;
    let stderr = fs::read(&stderr_path)?;
    if !status.success() {
        return Err(format!(
            "generated Rust for `{}` failed at runtime\nstdout:\n{}\nstderr:\n{}",
            case.name,
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr),
        )
        .into());
    }
    parse_output(case, &source, &stdout)
}

fn execution_timeout(scale: Scale) -> Duration {
    Duration::from_secs(match scale {
        Scale::Tiny => 10,
        Scale::Medium => 60,
        Scale::Large => 300,
    })
}

fn support_items(source: &cq::Module) -> syn::Result<Vec<syn::Item>> {
    let program = &source.program.name;
    let inputs = source
        .program
        .inputs
        .iter()
        .enumerate()
        .map(|(input_index, input)| -> syn::Result<_> {
            if !input.columns.iter().all(|column| is_plain_i32(&column.ty)) {
                return Err(syn::Error::new_spanned(
                    &input.columns,
                    "the workload CSV runner currently supports only `i32` relation columns",
                ));
            }
            let file = cq_workloads::dataset::input_file_name(input_index);
            let arity = input.columns.len();
            let columns = (0..arity)
                .map(|column| {
                    format_ident!(
                        "input{input_index}_column{column}",
                        span = Span::call_site(),
                    )
                })
                .collect::<Vec<_>>();
            Ok(quote! {
                read_rows::<#arity>(&root.join(#file))
                    .into_iter()
                    .map(|[#(#columns),*]| (#(#columns,)*))
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let output_arity = source.program.query.head.args.len();
    let bindings = (0..output_arity)
        .map(|column| format_ident!("column{column}", span = Span::call_site()))
        .collect::<Vec<_>>();
    let format = std::iter::repeat_n("{}", output_arity)
        .collect::<Vec<_>>()
        .join(",");

    let file: syn::File = syn::parse2(quote! {
        fn main() {
            let root = ::std::path::PathBuf::from(
                ::std::env::args_os().nth(1).expect("missing dataset directory"),
            );
            for (#(#bindings,)*) in #program::run(#(#inputs),*) {
                ::std::println!(#format, #(#bindings),*);
            }
        }

        fn read_rows<const N: usize>(path: &::std::path::Path) -> ::std::vec::Vec<[i32; N]> {
            ::std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
                .lines()
                .enumerate()
                .filter(|(_, line)| !line.is_empty())
                .map(|(line_number, line)| {
                    line.split(',')
                        .map(|field| field.parse::<i32>().unwrap_or_else(|error| {
                            panic!("{}:{}: invalid i32 `{field}`: {error}", path.display(), line_number + 1)
                        }))
                        .collect::<::std::vec::Vec<_>>()
                        .try_into()
                        .unwrap_or_else(|row: ::std::vec::Vec<i32>| {
                            panic!("{}:{}: expected {N} columns, found {}", path.display(), line_number + 1, row.len())
                        })
                })
                .collect()
        }
    })?;
    Ok(file.items)
}

fn is_plain_i32(column_type: &syn::Type) -> bool {
    let syn::Type::Path(path) = column_type else {
        return false;
    };
    path.qself.is_none() && path.path.is_ident("i32")
}

fn parse_output(case: &QueryCase, source: &cq::Module, stdout: &[u8]) -> Result<Vec<Vec<i32>>> {
    let arity = source.program.query.head.args.len();
    let text = std::str::from_utf8(stdout)?;
    let mut rows = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        let row = line
            .split(',')
            .map(str::parse::<i32>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if row.len() != arity {
            return Err(format!(
                "`{}` output line {} has {} columns; expected {arity}",
                case.name,
                line_number + 1,
                row.len(),
            )
            .into());
        }
        rows.push(row);
    }
    Ok(rows)
}

fn build_directory(case: &QueryCase) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives under repository root")
        .join("target/cq-workloads")
        .join(format!("{}-{}", case.name, std::process::id()))
}

#[derive(Default)]
struct IteratorPipelineExpander {
    pull_expansions: usize,
    pipeline_expansions: usize,
    error: Option<syn::Error>,
}

impl VisitMut for IteratorPipelineExpander {
    fn visit_stmt_mut(&mut self, statement: &mut syn::Stmt) {
        let macro_invocation = match statement {
            syn::Stmt::Macro(statement) if statement.semi_token.is_none() => {
                Some(statement.mac.clone())
            }
            _ => None,
        };
        let Some(macro_invocation) = macro_invocation else {
            visit_mut::visit_stmt_mut(self, statement);
            return;
        };

        if is_pull(&macro_invocation.path) {
            self.pull_expansions += 1;
            let lowered =
                syn::parse2::<rust_access_plan::Plan>(macro_invocation.tokens).and_then(|plan| {
                    let target = compile_pull(&plan);
                    syn::parse2(quote! {
                        ::mini_linq::iterator_pipeline! { #target }
                    })
                });
            match lowered {
                Ok(lowered) => {
                    *statement = lowered;
                    self.visit_stmt_mut(statement);
                }
                Err(error) => self.error = Some(error),
            }
        } else if is_iterator_pipeline(&macro_invocation.path) {
            self.pipeline_expansions += 1;
            let lowered = syn::parse2::<iterator_pipeline::Pipeline>(macro_invocation.tokens)
                .map(|plan| compile_iterator_pipeline(&plan));
            match lowered {
                Ok(expression) => *statement = syn::Stmt::Expr(expression, None),
                Err(error) => self.error = Some(error),
            }
        } else {
            visit_mut::visit_stmt_mut(self, statement);
        }
    }
}

fn is_pull(path: &syn::Path) -> bool {
    path.leading_colon.is_some()
        && path.segments.len() == 2
        && path.segments[0].ident == "mini_linq"
        && path.segments[1].ident == "pull"
}

fn is_iterator_pipeline(path: &syn::Path) -> bool {
    path.leading_colon.is_some()
        && path.segments.len() == 2
        && path.segments[0].ident == "mini_linq"
        && path.segments[1].ident == "iterator_pipeline"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_runner_expands_both_typed_iterator_stages() {
        let mut file: syn::File = syn::parse2(quote! {
            fn query(left: &::std::vec::Vec<(i32,)>) {
                ::mini_linq::pull! {
                    answer(x) => ((::core::clone::Clone::clone(x),)) :-
                        Left(x) => for (x,) in (left.iter());
                }
            }
        })
        .unwrap();

        let mut expander = IteratorPipelineExpander::default();
        expander.visit_file_mut(&mut file);

        assert!(expander.error.is_none());
        assert_eq!(expander.pull_expansions, 1);
        assert_eq!(expander.pipeline_expansions, 1);
        let emitted = file.to_token_stream().to_string();
        assert!(!emitted.contains("mini_linq"));
        assert!(emitted.contains("once_with"));
        assert!(emitted.contains("Iterator :: map"));
        assert!(emitted.contains("HashSet :: new"));
    }
}
