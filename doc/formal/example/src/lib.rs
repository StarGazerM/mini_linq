//! Typed logical boundary checks for the paper's retained examples.
//! These functions do not instantiate a physical provider or execute a query.
//!
//! The chain is the compiler's: `cq::normal::normalize` (CQ → NF),
//! `cq_order::compile` (NF → VO, clauses as written, one subatom per atom),
//! optionally `cq_reduce::compile` (VO → VO, single-atom guards pushed onto
//! their atoms and the Yannakakis reducer when the query is acyclic),
//! `cq_elaborate::compile` (VO → RA), `cq_index_policy::compile` (RA → IX).
use cq_ir::cq::normal::Kernel;
use cq_ir::{cq, index_requirements, relational_plan, valuation};
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

/// Every boundary of one query, each checked against its contract.
pub struct Boundaries {
    pub normal: cq::Module,
    pub valuation: valuation::Module,
    pub relational: relational_plan::Module,
    pub indexes: index_requirements::Module,
}

/// Plan `source` through the chain, `reduced` selecting the reductions the
/// `cq_to_ra_hypergraph!` entry applies; write and print every boundary.
pub fn boundaries(directory: &Path, label: &str, source: &cq::Module, reduced: bool) -> Boundaries {
    cq::contract::check(source).unwrap();
    let normal = cq::normal::normalize(source).unwrap();
    let kernel = Kernel::of(&normal);
    let ordered = cq_order::compile(&normal, &cq_order::Options::default()).unwrap();
    let valuation = if reduced {
        let semijoins = if cq::hypergraph::Analysis::of(&normal).acyclic {
            cq_reduce::Reductions::Tree
        } else {
            cq_reduce::Reductions::None
        };
        let options = cq_reduce::Options {
            guards: cq_reduce::Pushdown::Atoms,
            semijoins,
        };
        let reduced = cq_reduce::compile(&ordered, &options).unwrap();
        assert!(cq_reduce::check(&ordered, &reduced, &options));
        reduced
    } else {
        ordered
    };
    valuation::contract::check(&valuation).unwrap();
    assert_eq!(valuation::kernel(&valuation), kernel);
    let relational = cq_elaborate::compile(&valuation).unwrap();
    relational_plan::contract::check(&relational).unwrap();
    assert!(cq_elaborate::check(&valuation, &relational));
    assert_eq!(relational_plan::kernel(&relational), kernel);
    let indexes = cq_index_policy::compile(&relational).unwrap();
    index_requirements::contract::check(&indexes).unwrap();
    write(directory, &format!("{label}-01-cq.tokens"), source);
    write(directory, &format!("{label}-02-normal.tokens"), &normal);
    write(directory, &format!("{label}-03-valuation.tokens"), &valuation);
    write(directory, &format!("{label}-04-relational.tokens"), &relational);
    write(directory, &format!("{label}-05-indexes.tokens"), &indexes);
    println!("{label} VO: {}", steps(&valuation));
    println!("{label} RA: {}", relational.plan.to_token_stream());
    Boundaries {
        normal,
        valuation,
        relational,
        indexes,
    }
}

/// The plan's steps as text, one line each.
pub fn steps(plan: &valuation::Module) -> String {
    plan.steps
        .iter()
        .map(|step| step.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn write(directory: &Path, name: &str, value: &impl ToTokens) {
    std::fs::write(
        directory.join(name),
        format!("{}\n", value.to_token_stream()),
    )
    .unwrap();
}
