//! Typed logical boundary checks for the paper's retained examples.
//! These functions stop at IX and do not print or execute a physical query.
//!
//! The chain is the compiler's: `cq::normal::normalize` (CQ → NF),
//! `cq_order::compile` (NF → VO, clauses as written, one subatom per atom),
//! optionally `cq_reduce::compile` (VO → VO, retaining opaque guards and
//! inserting the Yannakakis reducer when the query is acyclic),
//! `cq_elaborate::compile` (VO → RA), `cq_index_policy::compile` (RA → IX).
use cq_lang_cq as cq;
use cq_lang_ix as index_requirements;
use cq_lang_ra as ra;
use cq_lang_vo as valuation;
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
    pub normal: cq::Program,
    pub valuation: valuation::Program,
    pub relational: ra::Program,
    pub indexes: index_requirements::Program,
}

/// Plan `source` through the checked logical passes. `reduced` requests a
/// complete Yannakakis reducer when acyclic, as the external hypergraph planner
/// does. Check each pass boundary, then write and print its complete IR.
pub fn boundaries(
    directory: &Path,
    label: &str,
    source: &cq::Program,
    reduced: bool,
) -> Boundaries {
    cq::contract::check(source).unwrap();
    let normal = cq::normal::normalize(source).unwrap();
    let ordered = cq_order::compile(&normal, &cq_order::Options::default()).unwrap();
    cq_order::check(&normal, &ordered, &cq_order::Options::default()).unwrap();
    let valuation = if reduced {
        let semijoins = if cq::hypergraph::Analysis::of(&normal).acyclic {
            cq_reduce::Reductions::Tree
        } else {
            cq_reduce::Reductions::None
        };
        let options = cq_reduce::Options { semijoins };
        let reduced = cq_reduce::compile(&ordered, &options).unwrap();
        cq_reduce::check(&ordered, &reduced, &options).unwrap();
        reduced
    } else {
        ordered
    };
    valuation::contract::check(&valuation).unwrap();
    assert_eq!(valuation.cq, normal);
    let relational = cq_elaborate::compile(&valuation).unwrap();
    ra::contract::check(&relational).unwrap();
    cq_elaborate::check(&valuation, &relational).unwrap();
    let indexes = cq_index_policy::compile(&relational).unwrap();
    index_requirements::contract::check(&indexes).unwrap();
    cq_index_policy::check(&relational, &indexes).unwrap();
    write(directory, &format!("{label}-01-cq.tokens"), source);
    write(directory, &format!("{label}-02-normal.tokens"), &normal);
    write(
        directory,
        &format!("{label}-03-valuation.tokens"),
        &valuation,
    );
    write(
        directory,
        &format!("{label}-04-relational.tokens"),
        &relational,
    );
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
pub fn steps(plan: &valuation::Program) -> String {
    plan.valuation
        .steps
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
