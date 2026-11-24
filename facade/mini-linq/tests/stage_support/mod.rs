//! Shared driver for the generated stage differential and checker mutants:
//! compile one generated case pass by pass and interpret every stage with the
//! testkit's independent per-IR evaluators. Core passes only: the optional
//! Physical rewrites are a `Rewrites` hook supplied by
//! `features/rewrites/tests/rewrite_stages.rs`, which includes this driver by
//! `#[path]`.
#![allow(dead_code)]

pub mod differential;
pub mod mutants;

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use cq_testkit::fuzz::Case;
use cq_testkit::physical_semantics::{self, TypedDatabase};
use cq_testkit::values::{Database, TypedRows, Value, typed};

/// One planner/reducer combination: the route axis R1 rotates through.
#[derive(Clone, Copy, Debug)]
pub struct Route {
    pub order: cq_order::Options,
    pub reduce: cq_reduce::Options,
}

impl Route {
    pub fn all() -> Vec<Route> {
        use cq_order::{Heuristic, Ranking, Split};
        use cq_reduce::Reductions;
        let mut routes = Vec::new();
        for heuristic in [
            Heuristic::Written,
            Heuristic::Filtered,
            Heuristic::Prefix(Ranking::Connected),
            Heuristic::Prefix(Ranking::GreedyWidth),
            Heuristic::Elimination,
        ] {
            for split in [Split::Coarse, Split::Fine] {
                for semijoins in [
                    Reductions::None,
                    Reductions::Tree,
                    Reductions::Hypergraph,
                    Reductions::Masks,
                ] {
                    routes.push(Route {
                        order: cq_order::Options { heuristic, split },
                        reduce: cq_reduce::Options { semijoins },
                    });
                }
            }
        }
        routes
    }

    pub fn name(&self) -> String {
        format!(
            "{:?}/{:?}/{:?}",
            self.order.heuristic, self.order.split, self.reduce.semijoins
        )
    }
}

/// The denotation of one stage, an execution failure, or why it was not judged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Rows(TypedRows),
    /// An interpreted execution failed; never an unsupported-fragment skip.
    Failed(physical_semantics::Error),
    /// Outside the stage evaluator's fragment; not judged.
    Unsupported(String),
}

impl From<Result<physical_semantics::Evaluation, physical_semantics::Error>> for Outcome {
    fn from(result: Result<physical_semantics::Evaluation, physical_semantics::Error>) -> Self {
        match result {
            Ok(evaluation) => Self::Rows(evaluation.output),
            Err(error @ physical_semantics::Error::Unsupported(_)) => {
                Self::Unsupported(error.to_string())
            }
            Err(error) => Self::Failed(error),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Stage {
    pub name: String,
    pub outcome: Outcome,
}

/// A pass error or checker rejection: a failure of that stage unless
/// `inapplicable` (the route refuses the query for a documented reason).
#[derive(Clone, Debug)]
pub struct PassError {
    pub stage: String,
    pub message: String,
    pub inapplicable: bool,
}

pub struct Compiled {
    pub stages: Vec<Stage>,
    /// The lowered Physical program and every rewritten Physical program.
    pub physical: Vec<(String, cq_lang_physical::Program)>,
    pub ix: cq_lang_ix::Program,
}

/// All-i32 case data as the legacy (i32) evaluator database; `None` when the
/// case has wider or string columns.
pub fn database(case: &Case) -> Option<Database> {
    let all_i32 = case
        .data
        .iter()
        .flatten()
        .flatten()
        .all(|v| matches!(v, Value::I32(_)));
    let typed_i32 = case.query.schema.iter().all(|r| {
        r.types
            .iter()
            .all(|t| *t == cq_testkit::values::ColumnType::I32)
    });
    (all_i32 && typed_i32).then(|| {
        case.query
            .schema
            .iter()
            .zip(case.i32_data())
            .map(|(relation, rows)| (table_name(&relation.name), rows.into_iter().collect()))
            .collect()
    })
}

pub fn typed_database(case: &Case) -> TypedDatabase {
    case.query
        .schema
        .iter()
        .zip(&case.data)
        .map(|(relation, rows)| (table_name(&relation.name), rows.iter().cloned().collect()))
        .collect()
}

pub fn parameters(case: &Case) -> BTreeMap<String, Value> {
    case.parameters()
}

/// Run a legacy evaluator that panics outside its fragment.
pub fn quietly<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(hook);
    result.map_err(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "panic".into())
    })
}

fn legacy(f: impl FnOnce() -> cq_testkit::values::Rows) -> Outcome {
    match quietly(f) {
        Ok(rows) => Outcome::Rows(typed(&rows)),
        Err(message) => Outcome::Unsupported(message),
    }
}

pub fn physical_outcome(
    program: &cq_lang_physical::Program,
    database: &TypedDatabase,
    parameters: &BTreeMap<String, Value>,
) -> Outcome {
    physical_semantics::evaluate(program, database, parameters).into()
}

pub fn pass<T, E: std::fmt::Display>(stage: &str, result: Result<T, E>) -> Result<T, PassError> {
    result.map_err(|e| PassError {
        stage: stage.into(),
        message: e.to_string(),
        inapplicable: false,
    })
}

/// Physical stages after lowering, each named and cumulative.
pub type Rewrites =
    fn(&cq_lang_physical::Program) -> Result<Vec<(String, cq_lang_physical::Program)>, PassError>;

/// No rewrite stage: the core pipeline ends at Physical lowering.
pub fn lowered_only(
    _: &cq_lang_physical::Program,
) -> Result<Vec<(String, cq_lang_physical::Program)>, PassError> {
    Ok(Vec::new())
}

/// Compile `case` along `route`, running each pass's checker, and interpret
/// every stage. A compile error or checker rejection names its stage.
pub fn compile(case: &Case, route: &Route, rewrites: Rewrites) -> Result<Compiled, PassError> {
    let parsed: cq_lang_cq::Program = pass("cq:parse", syn::parse_str(&case.cq()))?;
    compile_source(
        &parsed,
        database(case),
        &typed_database(case),
        &parameters(case),
        route,
        rewrites,
    )
}

/// [`compile`] for an authored CQ program; `db` is the legacy all-i32 view.
pub fn compile_source(
    parsed: &cq_lang_cq::Program,
    db: Option<Database>,
    tdb: &TypedDatabase,
    params: &BTreeMap<String, Value>,
    route: &Route,
    rewrites: Rewrites,
) -> Result<Compiled, PassError> {
    let mut stages = Vec::new();
    let normal = pass("cq:normalize", cq_lang_cq::normal::normalize(parsed))?;
    let judge = |f: &dyn Fn(&Database) -> cq_testkit::values::Rows| match &db {
        Some(db) => legacy(|| f(db)),
        None => Outcome::Unsupported("typed data".into()),
    };
    stages.push(Stage {
        name: "cq".into(),
        outcome: judge(&|db| cq_testkit::cq_semantics::evaluate_cq(&normal, db)),
    });
    let ordered = pass("vo:order", cq_order::compile(&normal, &route.order))?;
    pass("vo:order", cq_order::check(&normal, &ordered, &route.order))?;
    stages.push(Stage {
        name: "vo:order".into(),
        outcome: judge(&|db| cq_testkit::valuation_semantics::evaluate(&normal, &ordered, db)),
    });
    // Yannakakis reductions need an acyclic query: a cyclic one makes the route inapplicable.
    let reduced = cq_reduce::compile(&normal, &ordered, &route.reduce).map_err(|error| PassError {
        stage: "vo:reduce".into(),
        inapplicable: matches!(error, cq_reduce::Error::Cyclic(_)),
        message: error.to_string(),
    })?;
    pass(
        "vo:reduce",
        cq_reduce::check(&normal, &ordered, &reduced, &route.reduce),
    )?;
    stages.push(Stage {
        name: "vo:reduce".into(),
        outcome: judge(&|db| cq_testkit::valuation_semantics::evaluate(&normal, &reduced, db)),
    });
    let (ra, steps) = pass(
        "ra:relational_lowering",
        cq_relational_lowering::compile(&normal, &reduced),
    )?;
    pass(
        "ra:relational_lowering",
        cq_relational_lowering::check(&normal, &reduced, &ra, &steps),
    )?;
    stages.push(Stage {
        name: "ra:relational_lowering".into(),
        outcome: judge(&|db| cq_testkit::semantics::evaluate_ra(&ra, db).output),
    });
    let ix = pass("ix:index_policy", cq_index_policy::compile(&ra))?;
    pass("ix:index_policy", cq_index_policy::check(&ra, &ix))?;
    stages.push(Stage {
        name: "ix:index_policy".into(),
        outcome: judge(&|db| cq_testkit::semantics::evaluate_ra(&ix.ra, db).output),
    });
    let lowered = pass("physical:lower", cq_physical_lowering::compile(&ix))?;
    pass("physical:lower", cq_physical_lowering::check(&ix, &lowered))?;
    let mut physical = vec![("physical:lower".to_owned(), lowered.clone())];
    physical.extend(rewrites(&lowered)?);
    for (name, program) in &physical {
        stages.push(Stage {
            name: name.clone(),
            outcome: physical_outcome(program, tdb, params),
        });
    }
    Ok(Compiled {
        stages,
        physical,
        ix,
    })
}

/// The answer every stage must denote. Prefers the CQ-level stage.
pub fn reference(stages: &[Stage]) -> Option<(String, TypedRows)> {
    stages.iter().find_map(|stage| match &stage.outcome {
        Outcome::Rows(rows) => Some((stage.name.clone(), rows.clone())),
        Outcome::Unsupported(_) | Outcome::Failed(_) => None,
    })
}

/// The `FUZZ FAILURE` report: seed, route, stage, query and data.
pub fn failure(case: &Case, route: &str, stage: &str, detail: &str) -> String {
    format!(
        "{}\n{detail}\n",
        cq_testkit::fuzz::failure_report(case, route, stage, case)
    )
}

/// The base seed of the rotating case: `CQ_FUZZ_SEED`, else the clock.
pub fn rotating_seed() -> u64 {
    std::env::var("CQ_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        })
}

/// Case count, scaled for the slow tier by `CQ_STAGE_CASES`.
pub fn case_count(default: u64) -> u64 {
    std::env::var("CQ_STAGE_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Generate under `CQ_FUZZ_PROFILE` (`quick`, the default, or `full`).
pub fn generate(seed: u64) -> Case {
    let profile = match std::env::var("CQ_FUZZ_PROFILE").as_deref() {
        Ok("full") => cq_testkit::fuzz::Profile::full(),
        _ => cq_testkit::fuzz::Profile::quick(),
    };
    cq_testkit::fuzz::generate_with(seed, &profile)
}

/// Evaluators key tables by symbol name: `r#match` is table `match`.
fn table_name(name: &str) -> String {
    name.strip_prefix("r#").unwrap_or(name).to_owned()
}

#[test]
fn physical_execution_errors_are_not_unsupported() {
    use physical_semantics::Error;
    for error in [
        Error::Panic("overflow".into()),
        Error::Malformed("undefined read".into()),
    ] {
        assert_eq!(Outcome::from(Err(error.clone())), Outcome::Failed(error));
    }
    assert!(matches!(
        Outcome::from(Err(Error::Unsupported("expression".into()))),
        Outcome::Unsupported(_)
    ));
}
