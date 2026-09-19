//! Typed logical boundary checks for the paper's retained examples.
//! These functions do not instantiate a physical provider or execute a query.
use cq_ir::{cq, index_requirements, relational_plan};
use cq_query_lowering::{JoinPolicy, PlanningOptions, composition};
use quote::ToTokens;
use std::path::{Path, PathBuf};

pub fn directory(name: &str) -> PathBuf {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../build")
                .join(name)
        });
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

pub fn boundaries(
    directory: &Path,
    label: &str,
    source: &cq::Module,
    planned: bool,
) -> index_requirements::Module {
    cq::contract::check(source).unwrap();
    let ra = if planned {
        let (ra, report) = composition::explain_cq_plan(
            source,
            PlanningOptions {
                strategy: JoinPolicy::Auto,
                max_arity: None,
            },
        )
        .unwrap();
        composition::check_cq_plan(source, &ra, &report).unwrap();
        std::fs::write(
            directory.join(format!("{label}-report.txt")),
            format!("{report:#?}"),
        )
        .unwrap();
        ra
    } else {
        composition::compile_cq(source).unwrap()
    };
    relational_plan::contract::check(&ra).unwrap();
    let ix = cq_index_policy::compile(&ra).unwrap();
    index_requirements::contract::check(&ix).unwrap();
    write(directory, &format!("{label}-01-cq.tokens"), source);
    write(directory, &format!("{label}-02-relational.tokens"), &ra);
    write(directory, &format!("{label}-03-indexes.tokens"), &ix);
    ix
}

pub fn write(directory: &Path, name: &str, value: &impl ToTokens) {
    std::fs::write(
        directory.join(name),
        format!("{}\n", value.to_token_stream()),
    )
    .unwrap();
}
