//! Offline static advice from recorded candidate facts and declared safe bounds.
//! Runtime guards and compiler-default selection are deliberately outside this stage.
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use cq_evidence::Rational;
use crate::export::{ClauseKind, Version};

pub mod bounds;
pub mod enumerate;

pub(crate) const RESTRICTED_VIEW_GAP: &str = "restricted-view-lifetime: joined filtered views may require per-invocation materialization; candidate-specific lifetime/build facts are not recorded";

/// A source relation read, with explicit original column correspondence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Read {
    pub source_occurrence: usize,
    pub normalized_clause: usize,
    pub logical_relation: String,
    pub input_relation: String,
    pub version: Version,
    pub columns: Vec<ReadColumn>,
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
    /// Complete reorderable positive-prefix occurrence order in original source IDs.
    pub order: Vec<usize>,
    pub steps: Vec<CostStep>,
    pub demands: BTreeSet<IndexDemand>,
    pub current: bool,
    /// An explicit reason this candidate lacks a finite model, never a guessed cost.
    pub unbounded: Vec<String>,
}

/// An affine upper certificate in this variant's current delta cardinality.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaCost {
    pub coefficient: Rational,
    pub constant: Rational,
}
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
pub enum Decision { Changed, Retained, Unknown, Crossing }

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
}

/// A declared review policy, not a cardinality fact or a measured speedup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseScreen {
    pub candidate: usize,
    pub minimum_ratio: Rational,
    pub provenance: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoleAdvice {
    pub hole: usize,
    pub origin: crate::export::Origin,
    pub planning_rank: usize,
    pub standalone_upper: Option<Rational>,
    pub shared_before: BTreeSet<IndexDemand>,
    pub candidates: Vec<CandidateRecord>,
    pub selected: usize,
    pub decision: Decision,
    pub reason: String,
    pub citations: BTreeSet<String>,
    pub admission: Option<FixedAdmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_screen: Option<NoiseScreen>,
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
fn incumbent(candidates: &[CandidateRecord]) -> crate::export::Result<usize> {
    let current = candidates.iter().enumerate().filter(|(_, c)| c.plan.current).map(|(i, _)| i).collect::<Vec<_>>();
    if current.len() != 1 { return Err(error("candidate records must identify exactly one incumbent")); }
    Ok(current[0])
}
fn structural_rank(candidate: &Candidate, hole: &crate::export::Hole) -> Vec<usize> {
    if let crate::export::Origin::Rule { permutation, .. } = &hole.origin {
        candidate.order.iter().map(|position| permutation.iter().position(|p| p == position).unwrap_or(*position)).collect()
    } else {
        candidate.order.clone()
    }
}
fn choose(
    model: &bounds::BoundModel<'_>,
    hole: &crate::export::Hole,
    candidates: &[CandidateRecord],
) -> crate::export::Result<(usize, Decision, String)> {
    let current = incumbent(candidates)?;
    if candidates.iter().any(|c| c.cost.whole_run.is_none() || !c.plan.unbounded.is_empty()) {
        return Ok((current, Decision::Unknown, "incomplete bound comparison; retain recorded structural order".into()));
    }
    // A universal minimum, if present, survives this dominance tournament.
    // One final linear pass certifies it; avoid quadratic comparisons over
    // the complete eight-atom permutation space.
    let mut selected = current;
    for (index, candidate) in candidates.iter().enumerate() {
        if index == selected { continue; }
        match model.compare(candidates[selected].cost.whole_run.as_ref().unwrap(), candidate.cost.whole_run.as_ref().unwrap())? {
            Comparison::RightDominates => selected = index,
            Comparison::Equal if index == current || (selected != current
                && structural_rank(&candidate.plan, hole) < structural_rank(&candidates[selected].plan, hole)) =>
            { selected = index; }
            _ => {}
        }
    }
    let mut crossing = false;
    let mut unresolved = false;
    let mut dominates = true;
    for (index, candidate) in candidates.iter().enumerate() {
        if index == selected { continue; }
        match model.compare(candidates[selected].cost.whole_run.as_ref().unwrap(), candidate.cost.whole_run.as_ref().unwrap())? {
            Comparison::Equal | Comparison::LeftDominates => {}
            Comparison::RightDominates => dominates = false,
            Comparison::Crossing => { dominates = false; crossing = true; }
            Comparison::Unknown => { dominates = false; unresolved = true; }
        }
    }
    if dominates {
        return Ok((selected, if selected == current { Decision::Retained } else { Decision::Changed },
            if selected == current { "incumbent is minimum throughout the declared domain; structural ties retained" }
            else { "lower whole-run bound throughout the declared domain" }.into()));
    }
    if crossing && !unresolved {
        Ok((current, Decision::Crossing, "cost bounds cross; guarded advice is design-only, so retain structural order".into()))
    } else {
        Ok((current, Decision::Unknown, "incomparable declared domain; retain recorded structural order".into()))
    }
}

fn review_margin(
    model: &bounds::BoundModel<'_>,
    current: &LinearCost,
    proposed: &LinearCost,
) -> crate::export::Result<bool> {
    // Reject only a proven sub-5% bound difference throughout the declared
    // domain. Comparing rounded point estimates would not establish this.
    let factor = num_rational::BigRational::new(21.into(), 20.into());
    let scaled = LinearCost {
        constant: encoded(rational(&proposed.constant)? * &factor),
        terms: proposed.terms.iter().map(|(name, coefficient)|
            Ok((name.clone(), encoded(rational(coefficient)? * &factor))))
            .collect::<crate::export::Result<_>>()?,
    };
    Ok(model.compare(current, &scaled)? == Comparison::LeftDominates)
}

/// Compare only complete legal candidates. The single greedy stratum order and
/// every reused demand are recorded; neither a printer nor runtime re-optimizes.
pub fn analyze(
    input: &crate::export::PlannerInput,
    evidence: &cq_evidence::DatasetEvidence,
    target: &str,
) -> crate::export::Result<AdviceReport> {
    input.check()?;
    let model = bounds::BoundModel::new(input, evidence, target)?;
    let mut prepared = Vec::with_capacity(input.holes.len());
    let mut rank = Vec::with_capacity(input.holes.len());
    for (index, hole) in input.holes.iter().enumerate() {
        let candidates = enumerate::enumerate_checked(input, hole)?.into_iter().map(|plan| {
            let cost = model.cost(hole, &plan, &BTreeSet::new())?;
            Ok(CandidateRecord { plan, cost })
        }).collect::<crate::export::Result<Vec<_>>>()?;
        let (selected, _, _) = choose(&model, hole, &candidates)?;
        let upper = candidates[selected].cost.whole_run.as_ref().map(|cost| model.upper(cost)).transpose()?.flatten();
        let numeric = upper.as_ref().map(rational).transpose()?;
        rank.push((index, stratum(hole), numeric));
        prepared.push((candidates, upper));
    }
    // Expensive independent bounds establish demands first. Unknown bounds do
    // not receive a guessed rank; their original hole order is retained last.
    rank.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| match (&a.2, &b.2) {
        (Some(a), Some(b)) => b.cmp(a),
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
    for (planning_rank, (index, component, _)) in rank.into_iter().enumerate() {
        let hole = &input.holes[index];
        let (candidates, standalone_upper) = &mut prepared[index];
        for candidate in candidates.iter_mut() {
            candidate.cost = model.cost(hole, &candidate.plan, &known)?;
        }
        let (mut selected, mut decision, mut reason) = choose(&model, hole, candidates)?;
        let mut noise_screen = None;
        let current = incumbent(candidates)?;
        if decision == Decision::Changed && review_margin(&model,
            candidates[current].cost.whole_run.as_ref().unwrap(),
            candidates[selected].cost.whole_run.as_ref().unwrap())?
        {
            noise_screen = Some(NoiseScreen {
                candidate: selected,
                minimum_ratio: Rational { numerator: "21".into(), denominator: "20".into() },
                provenance: "declared planner-step2 acceptance-review policy: sub-1.05 bound ratios are not advice; not measured performance".into(),
            });
            selected = current;
            decision = Decision::Retained;
            reason = "suppressed sub-1.05 whole-run bound ratio under declared review policy; retain incumbent".into();
        }
        let mut citations = candidates[selected].cost.citations.clone();
        for fact in &evidence.relative {
            citations.insert(fact.id.clone());
            citations.insert(fact.factor.id.clone());
        }
        let shared_before = known.clone();
        known.extend(candidates[selected].plan.demands.iter().cloned());
        shared.entry(component).or_default().extend(candidates[selected].plan.demands.iter().cloned());
        reports[index] = Some(HoleAdvice {
            hole: hole.id, origin: hole.origin.clone(), planning_rank,
            standalone_upper: standalone_upper.take(), shared_before,
            candidates: std::mem::take(candidates), selected, decision, reason, citations,
            admission: None, noise_screen,
        });
    }
    let holes = reports.into_iter().map(|r| r.ok_or_else(|| error("missing planned hole"))).collect::<crate::export::Result<Vec<_>>>()?;
    let whole_run = sum_selected(&holes)?;
    let mut report = AdviceReport {
        schema: REPORT_SCHEMA.into(), input_hash: input.input_hash.clone(), identity: input.identity.clone(),
        evidence_hash: crate::export::sha256(evidence.canonical_json().map_err(|e| error(e.to_string()))?),
        dataset_hash: evidence.dataset.hash.clone(), target: target.into(),
        target_constants: evidence.targets[target].clone(), holes, index_union: shared,
        whole_run, advice: String::new(), validation: String::new(),
    };
    advice::record_admission_constraints(input, &mut report)?;
    let (text, validation) = advice::validate_and_render(input, &report)?;
    report.advice = text;
    report.validation = validation;
    Ok(report)
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
