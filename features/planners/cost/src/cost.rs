//! Offline static advice from recorded candidate facts and declared safe bounds.
//! Runtime guards and compiler-default selection are deliberately outside this stage.
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use cq_evidence::Rational;
use crate::export::{ClauseKind, Version};

pub mod bounds;
pub mod enumerate;
pub mod estimate;
pub mod inferred;
pub(crate) mod rank;
pub mod variables;

/// How a target executes a partial read that introduces a variable: a
/// declared realization policy, never inferred from a target name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialReads {
    /// Every matching row is visited: a retained value repeats once per row.
    /// Sound for every realization.
    #[default]
    Rows,
    /// Each distinct retained value is enumerated once.
    Distinct,
}
/// The declared space of candidate valuations. Spaces nest in `phased`, so a
/// larger bound adds candidates and never loses one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Space {
    /// Realization of variable-introducing partial reads.
    pub partial_reads: PartialReads,
    /// The most occurrences one valuation reads in phases, a lookup or
    /// binds before the expansion. Zero is the atom orders.
    pub phased: usize,
}
impl Default for Space {
    fn default() -> Self { Self { partial_reads: PartialReads::Rows, phased: 1 } }
}
impl Space {
    fn is_default(&self) -> bool { *self == Self::default() }
}
impl std::str::FromStr for PartialReads {
    type Err = crate::export::ExportError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "rows" => Ok(Self::Rows),
            "distinct" => Ok(Self::Distinct),
            _ => Err(error("partial reads are realized as `rows` or `distinct`")),
        }
    }
}

/// A source relation read, with explicit original column correspondence.
/// `columns` are the columns this read retains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Read {
    pub source_occurrence: usize,
    pub normalized_clause: usize,
    pub logical_relation: String,
    pub input_relation: String,
    pub version: Version,
    pub columns: Vec<ReadColumn>,
    /// A sub-atom read: the occurrence has variable columns this read drops.
    /// A partial read that introduces nothing is a lookup, otherwise a bind.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadColumn {
    pub source_column: usize,
    pub input_column: String,
    /// None is a normalized closed/literal column, already fixed at entry.
    pub variable: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostStep {
    pub clause: usize,
    pub kind: ClauseKind,
    pub read: Option<Read>,
    pub required: Vec<String>,
    pub provides: Vec<String>,
    pub bound_before: Vec<String>,
    pub introduces: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexVersion { Full, Delta }
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexDemand {
    pub logical_relation: String,
    pub version: IndexVersion,
    /// Original source-column ordinals, in canonical sorted key order.
    pub key: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    /// Reorderable positive-prefix occurrences in original source IDs, in
    /// the order of their first read.
    pub order: Vec<usize>,
    /// The valuation: every step, with each read's retained columns.
    pub steps: Vec<CostStep>,
    pub demands: BTreeSet<IndexDemand>,
    pub current: bool,
    /// An explicit reason this candidate lacks a finite model, never a guessed cost.
    pub unbounded: Vec<String>,
}
impl Candidate {
    /// Whether every occurrence is read once and fully: an atom order.
    pub fn is_atom_order(&self) -> bool {
        self.steps.iter().filter_map(|step| step.read.as_ref()).all(|read| !read.partial)
    }
}

/// An affine upper certificate in this variant's current delta cardinality,
/// plus a whole-run part that bounds the sum over all rounds and is charged
/// once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaCost {
    pub coefficient: Rational,
    pub constant: Rational,
    #[serde(default = "zero", skip_serializing_if = "is_zero")]
    pub once: Rational,
}
fn zero() -> Rational { Rational { numerator: "0".into(), denominator: "1".into() } }
fn is_zero(value: &Rational) -> bool { value.numerator == "0" }
/// Whole-run upper certificate: constant plus sourced F/N variables.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearCost {
    pub constant: Rational,
    pub terms: BTreeMap<String, Rational>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepBound {
    pub clause: usize,
    pub output: Option<DeltaCost>,
    pub work: Option<DeltaCost>,
    pub inequality: String,
    pub citations: BTreeSet<String>,
    pub unknown: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedStepBound {
    pub vo_step: usize,
    pub bound: StepBound,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCost {
    pub per_round: Option<DeltaCost>,
    pub whole_run: Option<LinearCost>,
    pub steps: Vec<StepBound>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preprocessing: Vec<RecordedStepBound>,
    pub citations: BTreeSet<String>,
    pub unknown: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison { Equal, LeftDominates, RightDominates, Crossing, Unknown }

pub mod advice;

pub const REPORT_SCHEMA: &str = "mini-linq.static-advice.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision { Changed, Retained, Unknown }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionOwner { Admit, InPlan }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdmissionHint { Admit {}, At { level: usize } }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedAdmission {
    pub held_fixed: bool,
    pub owner: AdmissionOwner,
    pub hint: AdmissionHint,
    pub source_variant: usize,
    /// Other owners may be cheaper; this static-order comparison does not
    /// re-cost or select one. No numeric advantage is asserted without facts.
    pub alternative_owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateRecord {
    pub plan: Candidate,
    pub cost: CandidateCost,
    /// Estimated whole-run work in frames, which ranks valuations; the
    /// certificate in `cost` bounds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<u64>,
}

/// How a hole's space was searched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Search {
    /// Dynamic programming over every state.
    #[default]
    Exact,
    /// Past the state budget: the move of least work from each state.
    Greedy,
}
impl Search {
    fn is_exact(&self) -> bool { *self == Self::Exact }
}

/// A declared review policy, not a cardinality fact or a measured speedup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseScreen {
    pub candidate: usize,
    pub minimum_ratio: Rational,
    pub provenance: String,
}

/// What a hole's decision rests on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Certificates over the declared domain of the evidence.
    #[default]
    Certified,
    /// A read of a derived relation whose facts are estimates: the decision
    /// predicts, it does not prove.
    Predicted,
    /// PS-Decide: the degree facts of one run chose the hole's variable
    /// order. They observe that run and bound nothing.
    Measured,
    /// VP-Structural: no fact decided the hole; its clause order is
    /// SRDatalog's default.
    Structural,
}
impl Basis {
    fn is_certified(&self) -> bool { *self == Self::Certified }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoleAdvice {
    pub hole: usize,
    pub origin: crate::export::Origin,
    pub planning_rank: usize,
    /// The least estimate of the hole alone, which orders planning.
    pub standalone_estimate: Option<u64>,
    pub shared_before: BTreeSet<IndexDemand>,
    /// The incumbent and, unless it is the incumbent, the search's winner:
    /// the valuations the decision weighed.
    pub candidates: Vec<CandidateRecord>,
    pub selected: usize,
    pub decision: Decision,
    pub reason: String,
    pub citations: BTreeSet<String>,
    pub admission: Option<FixedAdmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_screen: Option<NoiseScreen>,
    #[serde(default, skip_serializing_if = "Basis::is_certified")]
    pub basis: Basis,
    #[serde(default, skip_serializing_if = "Search::is_exact")]
    pub search: Search,
}

/// The profile an analysis synthesized derived-relation facts from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileUse {
    pub hash: String,
    /// The example datasets of the profile.
    pub examples: Vec<String>,
    /// Every synthesized fact by citation id: `estimate:` facts are
    /// predictions, `structural:` facts are certified bounds.
    pub facts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdviceReport {
    pub schema: String,
    pub input_hash: String,
    pub identity: crate::export::Identity,
    pub evidence_hash: String,
    pub dataset_hash: String,
    pub target: String,
    pub target_constants: cq_evidence::TargetEvidence,
    /// The declared candidate space and partial-read realization.
    #[serde(default, skip_serializing_if = "Space::is_default")]
    pub space: Space,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileUse>,
    pub holes: Vec<HoleAdvice>,
    pub index_union: BTreeMap<usize, BTreeSet<IndexDemand>>,
    pub whole_run: Option<LinearCost>,
    /// Complete authored directive replacement, including retained old advice.
    pub advice: String,
    /// Source-level production checking is not a target execution/benchmark.
    pub validation: String,
}

fn error(message: impl Into<String>) -> crate::export::ExportError {
    crate::export::ExportError(message.into())
}
fn rational(value: &Rational) -> crate::export::Result<num_rational::BigRational> {
    let numerator = value.numerator.parse::<num_bigint::BigInt>().map_err(|_| error("invalid rational numerator"))?;
    let denominator = value.denominator.parse::<num_bigint::BigInt>().map_err(|_| error("invalid rational denominator"))?;
    if denominator <= num_bigint::BigInt::from(0) || numerator < num_bigint::BigInt::from(0) {
        return Err(error("cost rational must be nonnegative with positive denominator"));
    }
    Ok(num_rational::BigRational::new(numerator, denominator))
}
fn encoded(value: num_rational::BigRational) -> Rational {
    Rational { numerator: value.numer().to_string(), denominator: value.denom().to_string() }
}
fn stratum(hole: &crate::export::Hole) -> usize {
    match hole.origin {
        crate::export::Origin::Rule { stratum, .. } | crate::export::Origin::NegativeKey { stratum, .. } => stratum,
        crate::export::Origin::Cq {} => 0,
    }
}
/// Rank each hole's valuations by their estimated work, found by a search
/// over the declared [`Space`], and certify the ones the decision weighs.
/// The single greedy stratum order and every reused demand are recorded;
/// neither a printer nor runtime re-optimizes.
pub fn analyze(
    input: &crate::export::PlannerInput,
    evidence: &cq_evidence::DatasetEvidence,
    target: &str,
) -> crate::export::Result<AdviceReport> {
    analyze_with(input, evidence, target, Space::default())
}

/// [`analyze`] over a declared space of valuations.
pub fn analyze_with(
    input: &crate::export::PlannerInput,
    evidence: &cq_evidence::DatasetEvidence,
    target: &str,
    space: Space,
) -> crate::export::Result<AdviceReport> {
    analyze_inner(input, evidence, target, space, None)
}

/// [`analyze_with`], reading derived relations the evidence does not bind
/// through facts synthesized from a profile. Decisions that rest on an
/// estimate are marked [`Basis::Predicted`].
pub fn analyze_predicted(
    input: &crate::export::PlannerInput,
    evidence: &cq_evidence::DatasetEvidence,
    target: &str,
    space: Space,
    profile: &estimate::Profile,
) -> crate::export::Result<AdviceReport> {
    analyze_inner(input, evidence, target, space, Some(profile))
}

/// Every valuation of one hole beside its estimate, and the valuation the
/// search finds with its estimate, the hole planned alone: the listing the
/// search must agree with, and the space an oracle judges.
pub struct Listing {
    pub valuations: Vec<Valued>,
    pub searched: Option<(Candidate, f64)>,
}
/// A valuation with its whole-run estimate and the estimate of each step.
pub struct Valued {
    pub plan: Candidate,
    pub estimate: Option<f64>,
    pub steps: Vec<Option<f64>>,
}
/// The [`Listing`] of every hole, in hole order.
pub fn listed(
    input: &crate::export::PlannerInput,
    evidence: &cq_evidence::DatasetEvidence,
    target: &str,
    space: Space,
    profile: Option<&estimate::Profile>,
) -> crate::export::Result<Vec<Listing>> {
    input.check()?;
    bounds::validate_context(input, evidence, target)?;
    let synthesized = profile.map(|profile| estimate::synthesize(input, evidence, profile)).transpose()?;
    let model = match &synthesized {
        Some(extended) => bounds::BoundModel::synthesized(input, &extended.evidence, target, extended.relations.clone())?,
        None => bounds::BoundModel::new(input, evidence, target)?,
    }.with_partial_reads(space.partial_reads);
    let none = BTreeSet::new();
    input.holes.iter().map(|hole| {
        let prepared = enumerate::prepare(input, hole, space)?;
        let Some(mut estimator) = rank::Estimator::new(&model, hole, prepared.reads(), &none, &none)? else {
            return Ok(Listing { valuations: prepared.all().into_iter().map(|plan| Valued { plan, estimate: None, steps: Vec::new() }).collect(), searched: None });
        };
        let mut valuations = Vec::new();
        for plan in prepared.all() {
            let estimable = plan.unbounded.is_empty();
            let estimate = if estimable { estimator.total(&plan.steps)? } else { None };
            let steps = if estimable { estimator.steps(&plan.steps)? } else { Vec::new() };
            valuations.push(Valued { plan, estimate, steps });
        }
        let searched = prepared.best(&mut estimator, false);
        estimator.take_failure()?;
        Ok(Listing { valuations, searched: searched.map(|found| (found.candidate, found.work)) })
    }).collect()
}

fn analyze_inner(
    input: &crate::export::PlannerInput,
    evidence: &cq_evidence::DatasetEvidence,
    target: &str,
    space: Space,
    profile: Option<&estimate::Profile>,
) -> crate::export::Result<AdviceReport> {
    input.check()?;
    let synthesized = match profile {
        Some(profile) => {
            bounds::validate_context(input, evidence, target)?;
            Some(estimate::synthesize(input, evidence, profile)?)
        }
        None => None,
    };
    let model = match &synthesized {
        Some(extended) => bounds::BoundModel::synthesized(input, &extended.evidence, target, extended.relations.clone())?,
        None => bounds::BoundModel::new(input, evidence, target)?,
    }.with_partial_reads(space.partial_reads);
    // The declared relations with a predicted fact.
    let estimated: BTreeSet<&str> = synthesized.iter().flat_map(|extended| extended.facts.keys())
        .filter_map(|id| id.strip_prefix(estimate::ESTIMATE)).filter_map(|rest| rest.split(':').next()).collect();
    let prepared = input.holes.iter().map(|hole| enumerate::prepare(input, hole, space)).collect::<crate::export::Result<Vec<_>>>()?;
    // PS-Decide: the variable order VP-Index decides for every hole whose
    // degree facts the evidence holds, with the VP-Cost of its recorded order.
    let planned: Vec<Option<Measured>> = if evidence.degrees.is_empty() { vec![None; input.holes.len()] } else {
        let planner = variables::Planner::new(input, evidence)?;
        planner.decide(target)?.into_iter().enumerate().map(|(hole, decided)| Ok(match decided {
            Some(decided) => Some(Measured { recorded: planner.recorded(hole)?, decided }),
            None => None,
        })).collect::<crate::export::Result<_>>()?
    };
    let profile = match (profile, synthesized.as_ref()) {
        (Some(profile), Some(extended)) => Some(ProfileUse {
            hash: crate::export::sha256(serde_json::to_string(profile).map_err(|e| error(e.to_string()))?),
            examples: profile.examples.iter().map(|example| example.dataset_hash.clone()).collect(),
            facts: extended.facts.clone(),
        }),
        _ => None,
    };
    let evidence_hash = crate::export::sha256(evidence.canonical_json().map_err(|e| error(e.to_string()))?);
    // What production showed each hole's advice cannot be: each pass
    // compiles all advice once, and the next searches around what it found,
    // until every advised index is provisioned. That compilation also
    // validates the printed advice.
    let mut excluded = vec![Excluded::default(); input.holes.len()];
    let recorded = match &input.checked_ir {
        crate::export::CheckedIr::Datalog { .. } => Some(advice::recorded(input)?),
        _ => None,
    };
    let (mut report, compiled) = loop {
        let (holes, shared) = plan_holes(input, evidence, &model, &prepared, &planned, &estimated, &excluded)?;
        let mut report = AdviceReport {
            schema: REPORT_SCHEMA.into(), input_hash: input.input_hash.clone(), identity: input.identity.clone(),
            evidence_hash: evidence_hash.clone(), dataset_hash: evidence.dataset.hash.clone(), target: target.into(),
            target_constants: evidence.targets[target].clone(), space, profile: profile.clone(),
            whole_run: sum_selected(&holes)?, holes, index_union: shared, advice: String::new(), validation: String::new(),
        };
        advice::record_admission_constraints(input, recorded.as_ref(), &mut report)?;
        let Some(program) = &recorded else { break (report, None) };
        let changed: Vec<usize> = (0..report.holes.len()).filter(|&at| report.holes[at].decision == Decision::Changed).collect();
        let compiled = match advice::compile_report(input, program, &report) {
            Ok(Ok(compiled)) => compiled,
            outcome => {
                // Attribute the rejection to the holes production rejects alone.
                let alone = advice::each_alone(input, &changed, |program, &at|
                    advice::provisioned(program, &input.holes[at], &report.holes[at].candidates[report.holes[at].selected].plan))?;
                let mut attributed = false;
                for (&at, alone) in changed.iter().zip(alone) {
                    if let Err(reason) = alone {
                        excluded[at].rejected = Some(reason);
                        attributed = true;
                    }
                }
                if !attributed {
                    return Err(match outcome { Err(failure) => failure, Ok(Err(rejection)) => error(rejection), Ok(Ok(_)) => unreachable!() });
                }
                continue;
            }
        };
        let actual = advice::provisions(&compiled.1);
        let mut gap = false;
        for &at in &changed {
            let plan = &report.holes[at].candidates[report.holes[at].selected].plan;
            let missing: Vec<IndexDemand> = plan.demands.iter().filter(|demand| !actual.contains(*demand)).cloned().collect();
            let Some(first) = missing.first() else { continue };
            gap = true;
            // Step advice keeps the written atom permutation, from which
            // Storage derives layouts: a gap there rules out partial reads.
            if plan.is_atom_order() { excluded[at].indexes.extend(missing); } else { excluded[at].partial = Some(first.clone()); }
        }
        if !gap { break (report, Some(compiled)); }
    };
    let (text, validation) = advice::render(input, &report, compiled.as_ref())?;
    report.advice = text;
    report.validation = validation;
    Ok(report)
}

/// What production showed a hole's advice cannot be: indexes Storage does
/// not provision for an atom order; an index it does not provision under
/// step advice, which rules out valuations with partial reads; a rejection.
#[derive(Clone, Default)]
struct Excluded { indexes: BTreeSet<IndexDemand>, partial: Option<IndexDemand>, rejected: Option<String> }

/// The indexes the planned holes demand, per stratum.
type IndexUnion = BTreeMap<usize, BTreeSet<IndexDemand>>;

/// A hole's decided variable order and the VP-Cost of its recorded order.
#[derive(Clone)]
struct Measured { decided: variables::Decided, recorded: Option<f64> }

/// Plan every hole in the greedy stratum order, each with the indexes the
/// holes before it already demand; returns the advice and the index union.
fn plan_holes(
    input: &crate::export::PlannerInput,
    evidence: &cq_evidence::DatasetEvidence,
    model: &bounds::BoundModel<'_>,
    prepared: &[enumerate::Prepared],
    planned: &[Option<Measured>],
    estimated: &BTreeSet<&str>,
    excluded: &[Excluded],
) -> crate::export::Result<(Vec<HoleAdvice>, IndexUnion)> {
    // Expensive holes establish demands first. Unestimated holes do not
    // receive a guessed rank; their original hole order is retained last.
    let none = BTreeSet::new();
    let mut rank = Vec::with_capacity(input.holes.len());
    for (index, hole) in input.holes.iter().enumerate() {
        let standalone = match rank::Estimator::new(model, hole, prepared[index].reads(), &none, &excluded[index].indexes)? {
            Some(mut estimator) => {
                let found = prepared[index].best(&mut estimator, excluded[index].partial.is_some());
                estimator.take_failure()?;
                found.map(|found| found.work)
            }
            None => None,
        };
        rank.push((index, stratum(hole), standalone));
    }
    rank.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| match (a.2, b.2) {
        (Some(a), Some(b)) => b.total_cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }).then(a.0.cmp(&b.0)));
    let mut shared: BTreeMap<usize, BTreeSet<IndexDemand>> = BTreeMap::new();
    // Recorded full indexes survive component boundaries. Charge each demand
    // once for the program, while retaining per-stratum union ownership below.
    let mut known = BTreeSet::new();
    for hole in &input.holes {
        if let Some(ordering) = &hole.ordering_ir {
            let facts: cq_lang_vo::ordering::OrderingFacts = syn::parse_str(ordering)?;
            for demand in facts.shared_demand.into_iter().flatten() {
                let mut key = demand.columns;
                key.sort_unstable();
                let demand = IndexDemand {
                    logical_relation: cq_syntax::symbol_name(&demand.relation),
                    version: IndexVersion::Full, key,
                };
                known.insert(demand.clone());
                shared.entry(stratum(hole)).or_default().insert(demand);
            }
        }
    }
    let mut reports = vec![None; input.holes.len()];
    for (planning_rank, (index, component, standalone)) in rank.into_iter().enumerate() {
        let hole = &input.holes[index];
        let mut advice = plan_hole(model, hole, &prepared[index], planned[index].as_ref(), estimated, &known, &excluded[index])?;
        for fact in &evidence.relative {
            advice.citations.insert(fact.id.clone());
            advice.citations.insert(fact.factor.id.clone());
        }
        let demands = &advice.candidates[advice.selected].plan.demands;
        advice.planning_rank = planning_rank;
        advice.standalone_estimate = standalone.map(|work| work.ceil() as u64);
        advice.shared_before = known.clone();
        known.extend(demands.iter().cloned());
        shared.entry(component).or_default().extend(demands.iter().cloned());
        reports[index] = Some(advice);
    }
    let holes = reports.into_iter().map(|r| r.ok_or_else(|| error("missing planned hole"))).collect::<crate::export::Result<Vec<_>>>()?;
    Ok((holes, shared))
}

/// Decide one hole. With the hole's degree facts, its planned variable
/// order (PS-Decide); else by estimate, the incumbent against the search's
/// winner, the only two that are certified.
fn plan_hole(
    model: &bounds::BoundModel<'_>,
    hole: &crate::export::Hole,
    prepared: &enumerate::Prepared,
    planned: Option<&Measured>,
    estimated: &BTreeSet<&str>,
    known: &BTreeSet<IndexDemand>,
    excluded: &Excluded,
) -> crate::export::Result<HoleAdvice> {
    let incumbent = &prepared.incumbent;
    let predicted = incumbent.steps.iter().filter_map(|step| step.read.as_ref())
        .any(|read| estimated.contains(read.logical_relation.as_str()));
    let mut kept: Vec<(Candidate, Option<f64>)> = vec![(incumbent.clone(), None)];
    let mut selected = 0;
    let mut decision = Decision::Unknown;
    let mut noise_screen = None;
    let mut search = Search::Exact;
    let mut reason;
    if let Some(Measured { decided, recorded }) = planned {
        let order = &decided.order;
        // The estimates are recorded beside the valuations; they decide nothing.
        let mut estimator = rank::Estimator::new(model, hole, prepared.reads(), known, &excluded.indexes)?;
        if let Some(estimator) = &mut estimator { kept[0].1 = estimator.total(&incumbent.steps)?; }
        reason = format!("variable order [{}] planned from measured degree facts, VP-Cost {:.4}{}", order.order.join(", "), order.estimate,
            recorded.map_or(String::new(), |recorded| format!(" against {recorded:.4} for the recorded order")));
        if let Some(work) = decided.work {
            reason = format!("{reason}; {work:.0} bindings and {:.0} for new indexes, the least of {} orders", decided.added, decided.weighed);
        }
        if order.before > 0 {
            reason = format!("{reason}; reads the changing occurrence after {} others, once a round", order.before);
        }
        if !excluded.indexes.is_empty() || excluded.partial.is_some() {
            decision = Decision::Retained;
            reason = format!("{reason}; retain the recorded order");
        } else if enumerate::same(&order.candidate, incumbent) {
            decision = Decision::Retained;
            reason = format!("{reason}; the recorded order reads the same");
        } else {
            let work = match &mut estimator { Some(estimator) => estimator.total(&order.candidate.steps)?, None => None };
            kept.push((order.candidate.clone(), work));
            (selected, decision) = (1, Decision::Changed);
        }
        if let Some(estimator) = &mut estimator { estimator.take_failure()?; }
    } else if !incumbent.unbounded.is_empty() {
        reason = format!("the incumbent has no finite model ({}); retain recorded structural order", incumbent.unbounded.join("; "));
    } else if let Some(mut estimator) = rank::Estimator::new(model, hole, prepared.reads(), known, &excluded.indexes)? {
        kept[0].1 = estimator.total(&incumbent.steps)?;
        let winner = prepared.best(&mut estimator, excluded.partial.is_some());
        estimator.take_failure()?;
        // The winner, kept beside the incumbent unless it is the incumbent.
        let winner = winner.map(|found| {
            if !found.exact { search = Search::Greedy; }
            if enumerate::same(&found.candidate, incumbent) { return 0; }
            kept.push((found.candidate, Some(found.work)));
            kept.len() - 1
        });
        reason = match (kept[0].1, winner.and_then(|at| kept[at].1.map(|work| (at, work)))) {
            (Some(current), Some((best, least))) if best != 0 && least * 1.05 <= current => {
                (selected, decision) = (best, Decision::Changed);
                "least estimated whole-run work".into()
            }
            (Some(current), Some((best, least))) if best != 0 && least < current => {
                decision = Decision::Retained;
                noise_screen = Some(NoiseScreen {
                    candidate: best,
                    minimum_ratio: Rational { numerator: "21".into(), denominator: "20".into() },
                    provenance: "declared planner-step2 acceptance-review policy: sub-1.05 estimate ratios are not advice; not measured performance".into(),
                });
                "suppressed sub-1.05 estimate ratio under declared review policy; retain incumbent".into()
            }
            (Some(_), Some(_)) => { decision = Decision::Retained; "the incumbent has the least estimate".into() }
            _ => "no estimate for the incumbent or for every valuation; retain recorded structural order".into(),
        };
    } else {
        reason = "a read has no row fact, or the changing relation no final rows and rounds".into();
    }
    // VP-Structural: a rule hole no fact decided takes SRDatalog's default clause order.
    let mut structural = false;
    if let (Decision::Unknown, crate::export::Origin::Rule { seed, .. }) = (decision, &hole.origin) {
        if !excluded.indexes.is_empty() || excluded.partial.is_some() {
            reason = format!("{reason}; retain the recorded order");
        } else if let Some(order) = variables::structural(prepared, *seed) {
            structural = true;
            reason = format!("{reason}; the structural order: the changing occurrence, then most bound variables");
            if enumerate::same(&order, incumbent) {
                decision = Decision::Retained;
                reason = format!("{reason}; the recorded order reads the same");
            } else {
                kept.push((order, None));
                (selected, decision) = (kept.len() - 1, Decision::Changed);
            }
        }
    }
    if let (Some(rejection), Decision::Changed) = (&excluded.rejected, decision) {
        (selected, decision) = (0, Decision::Retained);
        reason = format!("{reason}; production rejects the change: {rejection}");
    }
    if let Some(gap) = &excluded.partial {
        reason = format!("{reason}; valuations with partial reads are not advised: under step advice production provisions no index on {} {:?} {:?}",
            gap.logical_relation, gap.version, gap.key);
    }
    if let Some(gap) = excluded.indexes.first() {
        reason = format!("{reason}; {} index(es) production does not provision are excluded, e.g. {} {:?} {:?}",
            excluded.indexes.len(), gap.logical_relation, gap.version, gap.key);
    }
    let candidates = kept.into_iter().map(|(plan, estimate)| Ok(CandidateRecord {
        cost: model.cost(hole, &plan, known)?, plan, estimate: estimate.map(|work| work.ceil() as u64),
    })).collect::<crate::export::Result<Vec<_>>>()?;
    Ok(HoleAdvice {
        hole: hole.id, origin: hole.origin.clone(), planning_rank: 0, standalone_estimate: None, shared_before: BTreeSet::new(),
        citations: candidates[selected].cost.citations.clone(), candidates, selected, decision, reason,
        admission: None, noise_screen, search,
        basis: if planned.is_some() { Basis::Measured } else if structural { Basis::Structural } else if predicted { Basis::Predicted } else { Basis::Certified },
    })
}

fn sum_selected(holes: &[HoleAdvice]) -> crate::export::Result<Option<LinearCost>> {
    let mut constant = num_rational::BigRational::from_integer(0.into());
    let mut terms: BTreeMap<String, num_rational::BigRational> = BTreeMap::new();
    for hole in holes {
        let Some(cost) = &hole.candidates[hole.selected].cost.whole_run else { return Ok(None) };
        constant += rational(&cost.constant)?;
        for (variable, coefficient) in &cost.terms {
            *terms.entry(variable.clone()).or_insert_with(|| num_rational::BigRational::from_integer(0.into())) += rational(coefficient)?;
        }
    }
    Ok(Some(LinearCost { constant: encoded(constant), terms: terms.into_iter().map(|(k, v)| (k, encoded(v))).collect() }))
}
