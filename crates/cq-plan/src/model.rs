//! Compiler-local planning options and derivation witnesses.
use super::proof::{GyoRule, JoinTree};
use std::collections::BTreeSet;

pub type Heading = BTreeSet<String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinPolicy {
    Written,
    Connected,
    GreedyWidth,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanningOptions {
    pub strategy: JoinPolicy,
    /// Bound on every named RA heading, including occurrence renames and reductions.
    /// This is column arity, not fractional hypertree width or a cardinality bound.
    pub max_arity: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct JoinPlanReport {
    pub options: PlanningOptions,
    pub analysis: Vec<GyoRule>,
    pub join_tree: Option<JoinTree>,
    pub derivation: Vec<Rule>,
    pub peak_arity: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    Introduce { occurrence: usize },
    Select { group: usize, predicate: usize },
    Begin,
    Up { child: usize },
    Down { child: usize },
    Freeze,
    Combine { left: usize, right: usize },
    Trim { group: usize },
    Output { group: usize },
    Match { clause: usize },
    Guard { clause: usize },
    Bind { clause: usize },
    Constant { clause: usize, argument: usize },
    Finish,
}
