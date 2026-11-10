//! Deterministic certificates, not cardinality estimates. Every selected affine
//! expression is valid on its entire domain; selection never takes an affine
//! coefficient-wise minimum of crossing certificates.
use std::collections::{BTreeMap, BTreeSet};

use cq_evidence::{self as ev, Claim, CountFact, DatasetEvidence, Fact,
    Premise, Rational, RelationEvidence, RelationScope, Source};
use num_bigint::BigInt;
use num_rational::BigRational as Q;
use num_traits::{One, Zero};

use super::{Candidate, CandidateCost, Comparison, DeltaCost, IndexDemand,
    IndexVersion, LinearCost, Read, StepBound};
use crate::export::{self, ClauseKind, DatasetIdentity, EvidenceBinding,
    EvidenceView, ExportError, Hole, Origin, PlannerInput, Target, Version};

type Result<T, E = ExportError> = export::Result<T, E>;
fn error(message: impl Into<String>) -> ExportError { ExportError(message.into()) }
fn q(n: u64) -> Q { Q::from_integer(BigInt::from(n)) }
fn rational(value: &Q) -> Rational {
    Rational { numerator: value.numer().to_string(), denominator: value.denom().to_string() }
}
fn exact(value: &Rational) -> Result<Q> { value.exact().map_err(|e| error(e.to_string())) }
fn safe_upper<T>(fact: &Fact<T>) -> Option<&T> {
    if fact.source == Source::Hll { return None; }
    match fact.claim { Claim::Equal | Claim::Upper => fact.value.as_ref(),
        Claim::Interval => fact.upper.as_ref(), _ => None }
}
fn safe_lower(fact: &CountFact) -> Q {
    if fact.source == Source::Hll { return Q::zero(); }
    match fact.claim { Claim::Equal | Claim::Lower => fact.value,
        Claim::Interval => fact.lower, _ => None }.map(q).unwrap_or_else(Q::zero)
}

/// Check the declared target and run identities without recompiling checked IR.
/// The caller owns `PlannerInput::check`; missing optional statistics stay unknown.
pub fn validate_context(input: &PlannerInput, evidence: &DatasetEvidence, target: &str) -> Result<()> {
    evidence.check().map_err(|e| error(e.to_string()))?;
    let expected = match input.configuration.target { Target::Cpu => "cpu", Target::CpuPar => "cpu-par", Target::Gpu => "gpu" };
    if target != expected { return Err(error("requested target differs from checked compiler target")); }
    let profile = evidence.targets.get(target).ok_or_else(|| error(format!("missing required target h/m constants: {target}")))?;
    if profile.configuration_hash != input.identity.configuration_hash {
        return Err(error("target configuration differs from checked compiler configuration"));
    }
    if let DatasetIdentity::Bound { dataset_hash, schema_hash, evidence_hash } = &input.identity.dataset {
        if dataset_hash != &evidence.dataset.hash || schema_hash != &evidence.dataset.schema_hash {
            return Err(error("evidence dataset/schema differs from checked compiler input"));
        }
        let canonical = evidence.canonical_json().map_err(|e| error(e.to_string()))?;
        if evidence_hash != &export::sha256(canonical) {
            let attached = if let Some(record) = input.holes.iter().find_map(|hole| hole.evidence.as_ref()) {
                ev::parse_json(&record.canonical_json).map_err(|e| error(e.to_string()))?
            } else {
                let source: cq_lang_datalog_round::incremental::source::Program = syn::parse_str(&input.source)?;
                source.declared_evidence.ok_or_else(|| error("bound input has no evidence attachment"))?.document.as_ref().clone()
            };
            if attached.planning_facts() != evidence.planning_facts() {
                return Err(error("cost planning facts differ from checked compiler attachment"));
            }
        }
    }
    let run = |run: &ev::RunIdentity| -> Result<()> {
        if run.program_hash != input.identity.program_hash || run.configuration_hash != input.identity.configuration_hash {
            return Err(error("evidence run program/configuration differs from checked compiler input"));
        }
        Ok(())
    };
    // Only typed provenance is inspected; no relation/name inference or JSON walk.
    let provenance = |p: &ev::Provenance| -> Result<()> { if let Some(r) = &p.run { run(r)?; } Ok(()) };
    for relation in evidence.relations.values() {
        if let Some(f) = &relation.rows { provenance(&f.provenance)?; }
        for column in relation.columns.values() {
            for f in [&column.distinct, &column.max_degree].into_iter().flatten() { provenance(&f.provenance)?; }
            for f in [&column.min, &column.max].into_iter().flatten() { provenance(&f.provenance)?; }
        }
        for key in &relation.key_sets {
            for f in [&key.distinct, &key.max_degree].into_iter().flatten() { provenance(&f.provenance)?; }
        }
        for key in &relation.keys { provenance(&key.provenance)?; }
        if let Some(r) = &relation.recursive {
            run(&r.run)?;
            for f in [Some(&r.final_rows), Some(&r.rounds), r.seed_rows.as_ref(), r.unconsumed_final_delta_rows.as_ref()].into_iter().flatten() { provenance(&f.provenance)?; }
        }
    }
    for premise in &evidence.premises { match premise {
        Premise::DeltaSubset { provenance: p, .. } | Premise::DeltaPartition { provenance: p, .. } => provenance(p)?,
    } }
    for relative in &evidence.relative { provenance(&relative.provenance)?; provenance(&relative.factor.provenance)?; }
    for f in [&profile.build_per_row_h, &profile.merge_per_row_m] { provenance(&f.provenance)?; }
    Ok(())
}

#[derive(Clone)]
struct Bound { a: Q, b: Q, citations: BTreeSet<String>, proof: String }
impl Bound {
    fn constant(n: Q, proof: impl Into<String>) -> Self {
        Self { a: Q::zero(), b: n, citations: BTreeSet::new(), proof: proof.into() }
    }
    fn fact(fact: &CountFact) -> Option<Self> {
        safe_upper(fact).map(|&n| { let mut b = Self::constant(q(n), format!("upper({})", fact.id)); b.citations.insert(fact.id.clone()); b })
    }
    fn delta() -> Self { Self { a: Q::one(), b: Q::zero(), citations: BTreeSet::new(), proof: "current changing occurrence rows = delta".into() } }
    fn wire(&self) -> DeltaCost { DeltaCost { coefficient: rational(&self.a), constant: rational(&self.b) } }
    fn add(&self, other: &Self) -> Self {
        Self { a: &self.a + &other.a, b: &self.b + &other.b,
            citations: self.citations.union(&other.citations).cloned().collect(),
            proof: "sum of selected work certificates".into() }
    }
    fn multiply(&self, other: &Self, inequality: &str) -> Option<Self> {
        if !self.a.is_zero() && !other.a.is_zero() { return None; }
        Some(Self { a: &self.a * &other.b + &self.b * &other.a, b: &self.b * &other.b,
            citations: self.citations.union(&other.citations).cloned().collect(),
            proof: format!("{inequality}; premises: ({}), ({})", self.proof, other.proof) })
    }
    fn annotate(mut self, proof: &str) -> Self { self.proof = format!("{proof}; {}", self.proof); self }
}
fn pick(bounds: impl IntoIterator<Item = Bound>, delta_anchor: Option<&Q>) -> Option<Bound> {
    // Comparing values merely chooses ONE globally valid witness. It does not
    // assert that this witness is pointwise best throughout the interval. The
    // fallback one is a deterministic witness tie policy, not a cardinality fact.
    let one = Q::one();
    let x = delta_anchor.unwrap_or(&one);
    bounds.into_iter().min_by(|l, r| {
        (&l.a * x + &l.b).cmp(&(&r.a * x + &r.b))
            .then_with(|| l.a.cmp(&r.a)).then_with(|| l.b.cmp(&r.b))
            .then_with(|| l.proof.cmp(&r.proof))
    })
}
fn product(left: Option<&Bound>, right: Option<&Bound>, proof: &str) -> Option<Bound> {
    // Zero annihilates even an unknown finite relation bound.
    if let Some(zero) = left.into_iter().chain(right).find(|b| b.a.is_zero() && b.b.is_zero()) {
        return Some(zero.clone().annotate(proof));
    }
    left?.multiply(right?, proof)
}

fn binding<'a>(hole: &'a Hole, read: &Read) -> Option<&'a EvidenceBinding> {
    hole.evidence.as_ref()?.bindings.iter().find(|b| b.input_relation == read.input_relation)
}
fn subset_premises(evidence: &DatasetEvidence, relation: &str) -> BTreeSet<String> {
    evidence.premises.iter().filter_map(|p| match p {
        Premise::DeltaSubset { id, relations, .. } if relations.iter().any(|r| r == relation) => Some(id.clone()),
        Premise::DeltaPartition { id, relation: r, .. } if r == relation => Some(id.clone()),
        _ => None,
    }).collect()
}
struct Relation<'a> { name: &'a str, facts: &'a RelationEvidence, columns: BTreeMap<usize, &'a str>, premises: BTreeSet<String> }
fn relation<'a>(hole: &'a Hole, read: &Read, evidence: &'a DatasetEvidence) -> Result<Option<Relation<'a>>> {
    let Some(binding) = binding(hole, read) else { return Ok(None); };
    let facts = evidence.relations.get(&binding.dataset_relation).ok_or_else(|| error("explicit evidence binding names an absent dataset relation"))?;
    if facts.scope == RelationScope::RecursiveFinal
        && !facts.recursive.as_ref().is_some_and(|r| r.run.admission == ev::Admission::InsertOnlySet) {
        // Final values cannot bound intermediate replacement/deletion/bag states.
        return Ok(None);
    }
    let mut columns = BTreeMap::new();
    for column in &read.columns {
        let mapped = binding.columns.iter().find(|c| c.input_column == column.input_column)
            .ok_or_else(|| error("explicit evidence binding omits a read column"))?;
        if !facts.columns.contains_key(&mapped.dataset_column) { return Err(error("explicit evidence binding names an absent dataset column")); }
        columns.insert(column.source_column, mapped.dataset_column.as_str());
    }
    let premises = if binding.view != EvidenceView::Full || read.version == Version::Delta {
        let p = subset_premises(evidence, &binding.dataset_relation);
        if p.is_empty() { return Ok(None); }
        p
    } else { BTreeSet::new() };
    Ok(Some(Relation { name: &binding.dataset_relation, facts, columns, premises }))
}
impl Relation<'_> {
    fn certify(&self, mut bound: Bound) -> Bound {
        bound.citations.extend(self.premises.iter().cloned());
        if !self.premises.is_empty() { bound.proof.push_str("; subset inherits upper bounds only"); }
        bound
    }
    fn rows(&self) -> Option<Bound> {
        let fact = if self.facts.scope == RelationScope::RecursiveFinal { self.facts.recursive.as_ref().map(|r| &r.final_rows) } else { self.facts.rows.as_ref() };
        fact.and_then(Bound::fact).map(|b| self.certify(b))
    }
    fn degree(&self, key: &BTreeSet<usize>) -> Option<Bound> {
        let names: BTreeSet<_> = key.iter().filter_map(|c| self.columns.get(c).copied()).collect();
        let mut bounds = Vec::new();
        if let Some(rows) = self.rows() { bounds.push(rows.annotate("M_R(K) <= U_R (rows-as-degree-bound)")); }
        if !key.is_empty() {
            for name in &names {
                if let Some(bound) = self.facts.columns[*name].max_degree.as_ref().and_then(Bound::fact) {
                    bounds.push(bound.annotate("M_R(K) <= M_R({c}) for c in K"));
                }
            }
            for set in &self.facts.key_sets {
                if set.columns.iter().all(|c| names.contains(c.as_str())) {
                    if let Some(bound) = set.max_degree.as_ref().and_then(Bound::fact) { bounds.push(bound.annotate("M_R(K) <= M_R(subkey)")); }
                }
            }
            for declared in &self.facts.keys {
                if declared.columns.iter().all(|c| names.contains(c.as_str())) {
                    // Key declarations have no fact ID in evidence.v1. Cite the
                    // exact checked declaration path, not an invented fact ID.
                    bounds.push(Bound::constant(Q::one(), format!("declared key {}[{}] from {}: M_R(K) <= 1", self.name, declared.columns.join(","), declared.provenance.locator)));
                }
            }
            if self.facts.semantics == ev::RelationSemantics::Set
                && self.facts.columns.keys().all(|c| names.contains(c.as_str())) {
                bounds.push(Bound::constant(Q::one(), "set full-tuple key: M_R(K) <= 1"));
            }
        }
        pick(bounds, None).map(|b| self.certify(b))
    }
    fn distinct(&self, key: &BTreeSet<usize>) -> Option<Bound> {
        let names: BTreeSet<_> = key.iter().filter_map(|c| self.columns.get(c).copied()).collect();
        let mut bounds: Vec<_> = self.rows().into_iter().map(|b| b.annotate("D_R(K) <= U_R")).collect();
        if names.is_empty() { bounds.push(Bound::constant(Q::one(), "empty support has at most one value")); }
        if names.len() == 1 {
            if let Some(bound) = self.facts.columns[*names.first().unwrap()].distinct.as_ref().and_then(Bound::fact) { bounds.push(bound); }
        }
        for set in &self.facts.key_sets {
            // Projection cannot increase support size. Unlike degree, a
            // superset's distinct count bounds a subset's distinct count.
            if names.iter().all(|name| set.columns.iter().any(|c| c.as_str() == *name)) {
                if let Some(bound) = set.distinct.as_ref().and_then(Bound::fact) { bounds.push(bound.annotate("projection support <= superset support")); }
            }
        }
        pick(bounds, None).map(|b| self.certify(b))
    }
    fn known_keys(&self) -> Vec<BTreeSet<usize>> {
        let mut sets: BTreeSet<BTreeSet<usize>> = self.columns.keys().map(|&c| BTreeSet::from([c])).collect();
        for names in self.facts.key_sets.iter().map(|k| &k.columns).chain(self.facts.keys.iter().map(|k| &k.columns)) {
            let key: BTreeSet<_> = self.columns.iter().filter_map(|(&column, &name)| {
                names.iter().any(|c| c.as_str() == name).then_some(column)
            }).collect();
            if key.len() == names.len() { sets.insert(key); }
        }
        sets.into_iter().collect()
    }
}

#[derive(Default)]
struct Prefix<'a> {
    rows: Option<Bound>,
    degrees: Vec<(BTreeSet<String>, Bound)>,
    supports: Vec<(BTreeSet<String>, Bound)>,
    ranges: Vec<(String, &'a ev::ColumnEvidence, BTreeSet<String>)>,
}
impl Prefix<'_> {
    fn degree(&self, key: &BTreeSet<String>, delta_upper: Option<&Q>) -> Option<Bound> {
        pick(self.rows.iter().cloned().map(|b| b.annotate("M_P(K) <= U_P")).chain(
            self.degrees.iter().filter(|(k, _)| k.is_subset(key)).map(|(_, b)| b.clone())), delta_upper)
    }
    fn distinct(&self, key: &BTreeSet<String>, delta_upper: Option<&Q>) -> Option<Bound> {
        pick(self.rows.iter().cloned().chain(self.supports.iter().filter(|(k, _)| key.is_subset(k)).map(|(_, b)| b.clone())), delta_upper)
    }
    fn disjoint(&self, read: &Read, relation: &Relation<'_>) -> Option<Bound> {
        for column in &read.columns {
            let Some(variable) = &column.variable else { continue; };
            let current = &relation.facts.columns[relation.columns[&column.source_column]];
            for (_, previous, premises) in self.ranges.iter().filter(|(v, _, _)| v == variable) {
                if current.domain != previous.domain { continue; }
                if matches!(current.domain, ev::ScalarDomain::Unordered { .. } | ev::ScalarDomain::Symbol { ordered: false, .. }) { continue; }
                for (max, min) in [(previous.max.as_ref(), current.min.as_ref()), (current.max.as_ref(), previous.min.as_ref())] {
                    let (Some(max), Some(min)) = (max, min) else { continue; };
                    let Some(upper) = safe_upper(max) else { continue; };
                    let lower = if min.source == Source::Hll { None } else { match min.claim {
                        Claim::Equal | Claim::Lower => min.value.as_ref(), Claim::Interval => min.lower.as_ref(), _ => None,
                    } };
                    let Some(lower) = lower else { continue; };
                    if scalar_cmp(upper, lower) == Some(std::cmp::Ordering::Less) {
                        let mut zero = Bound::constant(Q::zero(), format!("disjoint certified extrema on {variable}: max < min implies |P join R| = 0"));
                        zero.citations.extend([max.id.clone(), min.id.clone()]);
                        zero.citations.extend(premises.iter().cloned());
                        zero.citations.extend(relation.premises.iter().cloned());
                        return Some(zero);
                    }
                }
            }
        }
        None
    }
}
fn scalar_cmp(left: &ev::Scalar, right: &ev::Scalar) -> Option<std::cmp::Ordering> {
    use ev::Scalar;
    match (left, right) {
        (Scalar::SignedInteger { value: l }, Scalar::SignedInteger { value: r }) => Some(l.cmp(r)),
        (Scalar::Date { value: l, encoding: le }, Scalar::Date { value: r, encoding: re }) if le == re => Some(l.cmp(r)),
        (Scalar::Decimal { unscaled: l, scale: ls }, Scalar::Decimal { unscaled: r, scale: rs }) if ls == rs =>
            Some(BigInt::parse_bytes(l.as_bytes(), 10)?.cmp(&BigInt::parse_bytes(r.as_bytes(), 10)?)),
        (Scalar::Utf8 { value: l }, Scalar::Utf8 { value: r }) => Some(l.cmp(r)),
        (Scalar::Symbol { code: l, dictionary_hash: lh }, Scalar::Symbol { code: r, dictionary_hash: rh }) if lh == rh => Some(l.cmp(r)),
        _ => None,
    }
}
fn variables(read: &Read, key: &BTreeSet<usize>) -> BTreeSet<String> {
    read.columns.iter().filter(|c| key.contains(&c.source_column)).filter_map(|c| c.variable.clone()).collect()
}

/// A validated catalog shared by all candidates/holes of one planner input.
/// Construction checks facts and identities once; costing never recompiles IR
/// or reparses/re-hashes evidence.
pub struct BoundModel<'a> {
    input: &'a PlannerInput,
    evidence: &'a DatasetEvidence,
    target: &'a str,
    h: Q,
    m: Q,
}
impl<'a> BoundModel<'a> {
    pub fn new(input: &'a PlannerInput, evidence: &'a DatasetEvidence, target: &'a str) -> Result<Self> {
        validate_context(input, evidence, target)?;
        let profile = &evidence.targets[target];
        let h = exact(profile.build_per_row_h.value.as_ref().ok_or_else(|| error("missing h"))?)?;
        let m = exact(profile.merge_per_row_m.value.as_ref().ok_or_else(|| error("missing m"))?)?;
        Ok(Self { input, evidence, target, h, m })
    }
    pub fn cost(&self, hole: &Hole, candidate: &Candidate, shared: &BTreeSet<IndexDemand>) -> Result<CandidateCost> {
        cost_checked(self.input, hole, candidate, self.evidence, self.target, shared, &self.h, &self.m)
    }
    pub fn upper(&self, cost: &LinearCost) -> Result<Option<Rational>> { upper_checked(cost, self.evidence) }
    /// Evaluate only a singleton declared domain, never divide unrelated upper bounds.
    pub fn exact_value(&self, cost: &LinearCost) -> Result<Option<Rational>> {
        let terms = cost.terms.iter().map(|(k, v)| Ok((k.clone(), exact(v)?))).collect::<Result<_>>()?;
        let (lower, upper) = range(exact(&cost.constant)?, terms, self.evidence)?;
        Ok(match (lower, upper) {
            (Some(lower), Some(upper)) if lower == upper => Some(rational(&upper)),
            _ => None,
        })
    }
    pub fn compare(&self, left: &LinearCost, right: &LinearCost) -> Result<Comparison> {
        compare_checked(left, right, self.evidence)
    }
}

/// Cost one complete candidate, preserving unknown work rather than favorable
/// partial sums. Shared maintenance is marginal and charged by the caller once.
pub fn cost_candidate(input: &PlannerInput, hole: &Hole, candidate: &Candidate,
    evidence: &DatasetEvidence, target: &str, shared: &BTreeSet<IndexDemand>) -> Result<CandidateCost> {
    BoundModel::new(input, evidence, target)?.cost(hole, candidate, shared)
}

/// Raw input/setup rows are distinct from a joined prefix's output rows.
/// Calibration uses this certificate for actual source-materialization nodes;
/// it does not add a second maintenance charge to the logical cost objective.
#[cfg(feature = "planner-calibration")]
pub(crate) fn input_rows(hole: &Hole, read: &Read, evidence: &DatasetEvidence) -> Result<StepBound> {
    let bound = if read.version == Version::Delta && matches!(hole.origin, Origin::Rule { seed: false, .. }) {
        Some(Bound::delta())
    } else {
        relation(hole, read, evidence)?.as_ref().and_then(Relation::rows)
    };
    Ok(StepBound {
        clause: read.normalized_clause,
        output: bound.as_ref().map(Bound::wire),
        work: bound.as_ref().map(Bound::wire),
        inequality: "raw input/setup cardinality; before joining to the valuation prefix".into(),
        citations: bound.as_ref().map(|b| b.citations.clone()).unwrap_or_default(),
        unknown: if bound.is_some() { Vec::new() } else { vec![format!("raw input bound for {}", read.input_relation)] },
    })
}
fn cost_checked(input: &PlannerInput, hole: &Hole, candidate: &Candidate,
    evidence: &DatasetEvidence, target: &str, shared: &BTreeSet<IndexDemand>, h: &Q, m: &Q) -> Result<CandidateCost> {
    let profile = &evidence.targets[target];
    let mut citations = BTreeSet::from([profile.build_per_row_h.id.clone(), profile.merge_per_row_m.id.clone()]);
    let mut unknown = candidate.unbounded.clone();
    let preprocessing = preprocessing(hole, candidate, evidence)?;
    unknown.extend(preprocessing.unknown.iter().cloned());
    for step in &preprocessing.steps { citations.extend(step.bound.citations.iter().cloned()); }
    let steady = matches!(hole.origin, Origin::Rule { seed: false, .. });
    let delta_read = candidate.steps.iter().filter_map(|s| s.read.as_ref()).find(|r| r.version == Version::Delta);
    if candidate.steps.iter().filter_map(|s| s.read.as_ref()).filter(|r| r.version == Version::Delta).count() > 1 {
        return Err(error("an affine candidate may have only one changing occurrence"));
    }
    let delta_relation = delta_read.map(|r| relation(hole, r, evidence)).transpose()?.flatten();
    // With declared endpoints choose each affine witness by a*upper(F)+
    // b*upper(N), not by per-round work at delta=F. The chosen expression
    // remains valid for every delta; this never treats a pointwise min as affine.
    let delta_anchor = delta_relation.as_ref().and_then(|r| {
        let rows = r.rows()?.b;
        let rounds = safe_upper(&r.facts.recursive.as_ref()?.rounds).copied()?;
        (rounds != 0).then(|| rows / q(rounds))
    });
    let mut prefix = Prefix { rows: Some(Bound::constant(Q::one(), "unit initial prefix")), ..Prefix::default() };
    let mut total = preprocessing.work;
    let mut steps = Vec::new();
    let mut first_atom = true;
    for step in &candidate.steps {
        let mut missing = Vec::new();
        let work;
        let proof;
        if preprocessing.absorbed.contains(&step.clause) {
            work = Some(Bound::constant(Q::zero(), "predicate discharged by recorded occurrence restrictions"));
            proof = "recorded Restrict discharges this source predicate; no additional stream predicate is costed".into();
        } else if matches!(step.kind, ClauseKind::OpaqueFor | ClauseKind::Block) {
            prefix = Prefix::default();
            work = None;
            proof = "arbitrary For/block has no recorded finite work/fanout bound".into();
            missing.push(format!("clause {} finite opaque work/fanout", step.clause));
        } else if matches!(step.kind, ClauseKind::OpaqueLet | ClauseKind::Equality) {
            // Scalar lets/equalities preserve rows. Equalities that act as
            // filters also cost at most one operation per input row.
            work = prefix.rows.clone();
            proof = "scalar let/equality: output <= input; scalar work <= input; no row multiplication".into();
        } else if step.read.is_none() || !step.introduces {
            work = prefix.rows.clone();
            proof = "filter/membership/negation: output <= input; probe work <= input".into();
        } else {
            let read = step.read.as_ref().unwrap();
            let rel = relation(hole, read, evidence)?;
            let changing = steady && read.version == Version::Delta;
            let rows = if changing { Some(Bound::delta()) } else { rel.as_ref().and_then(Relation::rows) };
            let key: BTreeSet<_> = read.columns.iter().filter(|c| c.variable.as_ref().is_none_or(|v| step.bound_before.contains(v)))
                .map(|c| c.source_column).collect();
            let vars = variables(read, &key);
            let mr = if key.is_empty() { rows.clone() } else { rel.as_ref().and_then(|r| r.degree(&key)) };
            let mp = prefix.degree(&vars, delta_anchor.as_ref());
            let dr = rel.as_ref().and_then(|r| r.distinct(&key));
            let dp = prefix.distinct(&vars, delta_anchor.as_ref());
            let mut alternatives = Vec::new();
            alternatives.extend(product(prefix.rows.as_ref(), mr.as_ref(), "|P join_K R| <= U_P M_R(K)"));
            alternatives.extend(product(rows.as_ref(), mp.as_ref(), "|P join_K R| <= U_R M_P(K)"));
            alternatives.extend(product(prefix.rows.as_ref(), rows.as_ref(), "|P join_K R| <= U_P U_R"));
            let degrees = product(mp.as_ref(), mr.as_ref(), "M_P(K) M_R(K)");
            for support in [dp.as_ref(), dr.as_ref()] {
                alternatives.extend(product(support, degrees.as_ref(), "|P join_K R| <= D(K) M_P(K) M_R(K) (support-distinct cap)"));
            }
            if let Some(rel) = &rel { alternatives.extend(prefix.disjoint(read, rel)); }
            let output = pick(alternatives, delta_anchor.as_ref());
            proof = output.as_ref().map(|b| b.proof.clone()).unwrap_or_else(|| "join lacks all valid upper certificates".into());
            if output.is_none() { missing.push(format!("clause {}: rows/degree/support for explicit binding {} and prefix", step.clause, read.input_relation)); }
            // Previous keys may have been duplicated: propagate their degree
            // through the actual maximum number of matches, never keep degree 1.
            prefix.degrees = prefix.degrees.into_iter().filter_map(|(k, d)| {
                product(Some(&d), mr.as_ref(), "M_new(K_old) <= M_old(K_old) M_R(join_key)").map(|d| (k, d))
            }).collect();
            if let Some(rel) = &rel {
                for column in &read.columns {
                    if let Some(variable) = &column.variable {
                        prefix.ranges.push((variable.clone(), &rel.facts.columns[rel.columns[&column.source_column]], rel.premises.clone()));
                    }
                }
                for key in rel.known_keys() {
                    let vars = variables(read, &key);
                    let degree = rel.degree(&key);
                    if let Some(d) = product(degree.as_ref(), mp.as_ref(), "M_new(K_R) <= M_R(K_R) M_P(join_key)") {
                        prefix.degrees.push((vars.clone(), d));
                    }
                    if let Some(d) = rel.distinct(&key) { prefix.supports.push((vars, d.annotate("retained base-domain support bounds prefix support"))); }
                }
            }
            prefix.rows = output.clone();
            work = if first_atom && matches!(hole.origin, Origin::Cq {}) {
                Some(Bound::constant(Q::zero(), "CQ leaf cardinality is not charged as a join result"))
            } else { output };
            first_atom = false;
        }
        if work.is_none() && missing.is_empty() { missing.push(format!("clause {} input work bound", step.clause)); }
        if let Some(b) = &work { citations.extend(b.citations.iter().cloned()); }
        if let Some(b) = &prefix.rows { citations.extend(b.citations.iter().cloned()); }
        total = match (total, &work) { (Some(sum), Some(work)) => Some(sum.add(work)), _ => None };
        let step_citations = work.iter().chain(prefix.rows.iter()).flat_map(|b| b.citations.iter().cloned()).collect();
        unknown.extend(missing.iter().cloned());
        steps.push(StepBound { clause: step.clause, output: prefix.rows.as_ref().map(Bound::wire), work: work.as_ref().map(Bound::wire), inequality: proof, citations: step_citations, unknown: missing });
    }
    if matches!(hole.origin, Origin::NegativeKey { .. }) {
        // The checked Round key set is built once from this single-read
        // comprehension's answers, before the component's seed/steady work.
        // Charge insertion attempts using the output upper bound, not an
        // independently estimated distinct-key count.
        let build = prefix.rows.as_ref().and_then(|rows|
            rows.multiply(&Bound::constant(h.clone(), "declared key-set build cost h"), "key construction <= h * answer rows"));
        if let Some(build) = &build { citations.extend(build.citations.iter().cloned()); }
        total = match (total, build) { (Some(work), Some(build)) => Some(work.add(&build)), _ => None };
        citations.insert("compiler:DL-Round:key-build-once".into());
        if total.is_none() { unknown.push("component key construction lacks an answer-row bound".into()); }
    }
    if !unknown.is_empty() { total = None; }
    let mut per_round = total.as_ref().map(Bound::wire);
    let mut whole = total.as_ref().map(|b| LinearCost { constant: rational(&b.b), terms: BTreeMap::new() });
    if steady {
        whole = None;
        if let (Some(total), Some(rel)) = (&total, &delta_relation) {
            if let Some(recursive) = &rel.facts.recursive {
                let partition = evidence.premises.iter().find_map(|p| match p {
                    Premise::DeltaPartition { id, relation, includes_seed_delta: true, insert_only_set: true, complete_consumption: true, .. } if relation == rel.name => Some(id), _ => None,
                });
                // A subset assertion alone does not establish disjointness.
                // The separately recorded completed insert-only-set run admits
                // the conservative consumed <= F bound (including last-delta gaps).
                if partition.is_some() || recursive.run.admission == ev::Admission::InsertOnlySet {
                    if let Some(id) = partition { citations.insert(id.clone()); }
                    citations.insert(recursive.final_rows.id.clone());
                    citations.insert(recursive.rounds.id.clone());
                    let mut cost = LinearCost { constant: rational(&Q::zero()), terms: BTreeMap::new() };
                    add_term(&mut cost, format!("final_rows:{}", rel.name), &total.a)?;
                    add_term(&mut cost, format!("rounds:{}", recursive.stratum), &total.b)?;
                    whole = Some(cost);
                    let premise = if partition.is_some() { "sum consumed delta = F under cited complete delta_partition" } else { "sum consumed delta <= F under recorded completed insert-only-set admission; delta_subset is not a partition" };
                    for step in &mut steps { step.inequality.push_str(&format!("; whole-run: {premise}; constants charged once per executed steady round, excluding seed/terminal-empty")); }
                } else { unknown.push(format!("{} consumed delta sum/insert-only-set admission", rel.name)); }
            } else { unknown.push(format!("{} recursive final rows and rounds/run scope", rel.name)); }
        } else { unknown.push("steady variant changing-occurrence explicit binding and work certificate".into()); }
    } else if matches!(hole.origin, Origin::NegativeKey { .. }) {
        // Checked Round admits only key_scope=component, and its printer
        // constructs key sets once before the component's seeds/steady loop.
        citations.insert("compiler:DL-Round:key_scope=component".into());
        for step in &mut steps { step.inequality.push_str("; recorded component key construction executes once"); }
    } else {
        for step in &mut steps { step.inequality.push_str("; seed/CQ executes once (not multiplied by steady rounds)"); }
    }
    for demand in candidate.demands.difference(shared) {
        if steady && demand.version == IndexVersion::Delta {
            if recorded_policy(input, demand) == Some("replace_each_run") {
                if let Some(per_round) = &mut per_round {
                    per_round.coefficient = rational(&(exact(&per_round.coefficient)? + h));
                }
                if let Some(step) = steps.last_mut() {
                    step.inequality.push_str("; recorded replace_each_run delta index contributes h*delta per round, independently of missing F/N");
                }
            } else {
                per_round = None;
                whole = None;
                unknown.push(format!("delta index {} recorded replace_each_run policy", demand.logical_relation));
            }
        }
        if let Some(cost) = &mut whole {
            if !maintenance(input, hole, candidate, demand, evidence, h, m, cost, &mut citations)? {
                whole = None;
                unknown.push(format!("index maintenance policy/bound: {} {:?} {:?}", demand.logical_relation, demand.version, demand.key));
            } else {
                if let Some(step) = steps.last_mut() {
                    step.inequality.push_str(&format!("; marginal demand {} {:?} {:?}: recorded exact-key or same-relation uniform maintenance; admitted_rows full=m*F once, replace_each_run delta=h*F (h*delta per round), frozen/input=h*rows once; full replace_each_run<=N*h*upper(F)", demand.logical_relation, demand.version, demand.key));
                }
            }
        }
    }
    unknown.sort(); unknown.dedup();
    Ok(CandidateCost { per_round, whole_run: whole, steps, preprocessing: preprocessing.steps, citations, unknown })
}
fn add_term(cost: &mut LinearCost, name: String, value: &Q) -> Result<()> {
    if value.is_zero() { return Ok(()); }
    let old = cost.terms.get(&name).map(exact).transpose()?.unwrap_or_else(Q::zero);
    cost.terms.insert(name, rational(&(old + value)));
    Ok(())
}
fn recorded_policy<'a>(input: &'a PlannerInput, demand: &IndexDemand) -> Option<&'a str> {
    let version = match demand.version { IndexVersion::Full => Version::Full, IndexVersion::Delta => Version::Delta };
    let layouts: Vec<_> = input.layouts.iter().filter(|l| l.logical_relation == demand.logical_relation && l.version == version).collect();
    let exact_layout = layouts.iter().find(|l| {
        let key: BTreeSet<_> = l.columns.iter().take(l.key).copied().collect();
        key == demand.key.iter().copied().collect()
    });
    // New keys require a uniform recorded lifetime for this relation/version;
    // the parent separately checks the provider of each compiled selected demand.
    exact_layout.map(|l| l.maintenance.as_str()).or_else(|| {
        let first = layouts.first()?;
        layouts.iter().all(|l| l.maintenance == first.maintenance).then_some(first.maintenance.as_str())
    })
}
fn maintenance(input: &PlannerInput, hole: &Hole, candidate: &Candidate, demand: &IndexDemand,
    evidence: &DatasetEvidence, h: &Q, m: &Q, cost: &mut LinearCost, citations: &mut BTreeSet<String>) -> Result<bool> {
    let version = match demand.version { IndexVersion::Full => Version::Full, IndexVersion::Delta => Version::Delta };
    let Some(read) = candidate.steps.iter().filter_map(|s| s.read.as_ref()).find(|r| r.logical_relation == demand.logical_relation && r.version == version) else { return Ok(false); };
    let Some(rel) = relation(hole, read, evidence)? else { return Ok(false); };
    citations.extend(rel.premises.iter().cloned());
    if matches!(hole.origin, Origin::Cq {}) {
        let Some(rows) = rel.rows() else { return Ok(false); };
        citations.extend(rows.citations);
        cost.constant = rational(&(exact(&cost.constant)? + h * rows.b));
        return Ok(true);
    }
    let policy = recorded_policy(input, demand);
    if !matches!(policy, Some("admitted_rows" | "replace_each_run")) { return Ok(false); }
    if rel.facts.scope != RelationScope::RecursiveFinal {
        let Some(rows) = rel.rows() else { return Ok(false); };
        citations.extend(rows.citations);
        if policy == Some("replace_each_run") && matches!(hole.origin, Origin::Rule { seed: false, .. }) {
            let Some(delta) = candidate.steps.iter().filter_map(|s| s.read.as_ref()).find(|r| r.version == Version::Delta) else { return Ok(false); };
            let Some(delta) = relation(hole, delta, evidence)? else { return Ok(false); };
            let Some(recursive) = &delta.facts.recursive else { return Ok(false); };
            citations.insert(recursive.rounds.id.clone());
            add_term(cost, format!("rounds:{}", recursive.stratum), &(h * rows.b))?;
        } else {
            cost.constant = rational(&(exact(&cost.constant)? + h * rows.b));
        }
        return Ok(true);
    }
    let Some(recursive) = &rel.facts.recursive else { return Ok(false); };
    citations.insert(recursive.final_rows.id.clone());
    match (version, policy) {
        (Version::Full, Some("admitted_rows")) if recursive.run.admission == ev::Admission::InsertOnlySet => {
            add_term(cost, format!("final_rows:{}", rel.name), m)?;
        }
        (Version::Delta, Some("replace_each_run")) if recursive.run.admission == ev::Admission::InsertOnlySet => {
            add_term(cost, format!("final_rows:{}", rel.name), h)?;
        }
        (Version::Full, Some("replace_each_run")) => {
            // N*h*F is not linear in both variables. Freeze a certified final
            // upper bound and retain N, a valid affine whole-run certificate.
            let Some(rows) = rel.rows() else { return Ok(false); };
            citations.extend(rows.citations);
            citations.insert(recursive.rounds.id.clone());
            add_term(cost, format!("rounds:{}", recursive.stratum), &(h * rows.b))?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn subject(subject: &ev::FactSubject) -> String {
    format!("{}:{}", match subject.measure { ev::RowMeasure::Rows => "rows", ev::RowMeasure::FinalRows => "final_rows" }, subject.relation)
}
// Sparse exact Fourier-Motzkin elimination. Constraints are sum(a_i*x_i)<=b.
// It handles relative evidence over its actual feasible polyhedron, not an
// independently tightened rectangle. No floating solver tolerance or overflow.
#[derive(Clone)]
struct Constraint { terms: BTreeMap<String, Q>, bound: Q }
fn normalize(constraints: Vec<Constraint>) -> Result<Vec<Constraint>> {
    let mut unique: BTreeMap<Vec<(String, Q)>, Q> = BTreeMap::new();
    for mut c in constraints {
        c.terms.retain(|_, v| !v.is_zero());
        let Some(first) = c.terms.values().next() else {
            if c.bound < Q::zero() { return Err(error("empty feasible cost evidence domain")); }
            continue;
        };
        let scale = if first < &Q::zero() { -first } else { first.clone() };
        let key: Vec<_> = c.terms.into_iter().map(|(k, v)| (k, v / &scale)).collect();
        let bound = c.bound / scale;
        unique.entry(key).and_modify(|old| { if bound < *old { *old = bound.clone(); } }).or_insert(bound);
    }
    Ok(unique.into_iter().map(|(terms, bound)| Constraint { terms: terms.into_iter().collect(), bound }).collect())
}
fn eliminate(constraints: Vec<Constraint>, variable: &str) -> Result<Vec<Constraint>> {
    let (mut positive, mut negative, mut output) = (Vec::new(), Vec::new(), Vec::new());
    for mut c in constraints {
        match c.terms.remove(variable) {
            Some(a) if a > Q::zero() => positive.push((a, c)),
            Some(a) if a < Q::zero() => negative.push((-a, c)),
            _ => output.push(c),
        }
    }
    for (pa, p) in &positive { for (na, n) in &negative {
        let mut terms: BTreeMap<_, _> = p.terms.iter().map(|(k, v)| (k.clone(), v / pa)).collect();
        for (k, v) in &n.terms { *terms.entry(k.clone()).or_insert_with(Q::zero) += v / na; }
        output.push(Constraint { terms, bound: &p.bound / pa + &n.bound / na });
    } }
    normalize(output)
}
fn fact_constraints(name: &str, fact: Option<&CountFact>, constraints: &mut Vec<Constraint>) {
    let lower = fact.map(safe_lower).unwrap_or_else(Q::zero);
    constraints.push(Constraint { terms: BTreeMap::from([(name.into(), -Q::one())]), bound: -lower });
    if let Some(upper) = fact.and_then(safe_upper) {
        constraints.push(Constraint { terms: BTreeMap::from([(name.into(), Q::one())]), bound: q(*upper) });
    }
}
fn range(constant: Q, terms: BTreeMap<String, Q>, evidence: &DatasetEvidence) -> Result<(Option<Q>, Option<Q>)> {
    let mut constraints = Vec::new();
    let mut names = BTreeSet::new();
    for (name, relation) in &evidence.relations {
        let rows = format!("rows:{name}");
        fact_constraints(&rows, relation.rows.as_ref(), &mut constraints); names.insert(rows);
        if let Some(recursive) = &relation.recursive {
            let final_rows = format!("final_rows:{name}");
            fact_constraints(&final_rows, Some(&recursive.final_rows), &mut constraints); names.insert(final_rows);
            let rounds = format!("rounds:{}", recursive.stratum);
            fact_constraints(&rounds, Some(&recursive.rounds), &mut constraints); names.insert(rounds);
        }
    }
    for relative in &evidence.relative {
        let Some(factor) = safe_upper(&relative.factor) else { continue; };
        let mut row = BTreeMap::new();
        *row.entry(subject(&relative.left)).or_insert_with(Q::zero) += Q::one();
        *row.entry(subject(&relative.right)).or_insert_with(Q::zero) -= exact(factor)?;
        constraints.push(Constraint { terms: row, bound: Q::zero() });
    }
    // A sourced variable absent from the catalog has the honest nonnegative,
    // unbounded domain. It cannot accidentally become a zero-valued symbol.
    for name in terms.keys() {
        if names.insert(name.clone()) { fact_constraints(name, None, &mut constraints); }
    }
    const OBJECTIVE: &str = "\0cost-objective";
    if names.contains(OBJECTIVE) { return Err(error("reserved cost variable")); }
    let mut equation: BTreeMap<_, _> = terms.into_iter().map(|(k, v)| (k, -v)).collect();
    equation.insert(OBJECTIVE.into(), Q::one());
    constraints.push(Constraint { terms: equation.clone(), bound: constant.clone() });
    constraints.push(Constraint { terms: equation.into_iter().map(|(k, v)| (k, -v)).collect(), bound: -constant });
    constraints = normalize(constraints)?;
    // Eliminate cheap dimensions first. This is exact and has no cutoff that
    // could silently turn a partial search into a favorable answer.
    while !names.is_empty() {
        let name = names.iter().min_by_key(|name| {
            let p = constraints.iter().filter(|c| c.terms.get(*name).is_some_and(|v| v > &Q::zero())).count();
            let n = constraints.iter().filter(|c| c.terms.get(*name).is_some_and(|v| v < &Q::zero())).count();
            p.saturating_mul(n)
        }).unwrap().clone();
        names.remove(&name);
        constraints = eliminate(constraints, &name)?;
    }
    let (mut lower, mut upper): (Option<Q>, Option<Q>) = (None, None);
    for c in constraints {
        if let Some(a) = c.terms.get(OBJECTIVE) {
            let endpoint = c.bound / a;
            if a > &Q::zero() { upper = Some(upper.map_or(endpoint.clone(), |old| old.min(endpoint))); }
            else { lower = Some(lower.map_or(endpoint.clone(), |old| old.max(endpoint))); }
        }
    }
    if matches!((&lower, &upper), (Some(l), Some(u)) if l > u) { return Err(error("empty feasible cost evidence domain")); }
    Ok((lower, upper))
}

/// Exact upper value over declared feasible intervals and relative constraints.
/// `None` means unbounded/unknown, never zero.
pub fn upper(cost: &LinearCost, evidence: &DatasetEvidence) -> Result<Option<Rational>> {
    evidence.check().map_err(|e| error(e.to_string()))?;
    upper_checked(cost, evidence)
}
fn upper_checked(cost: &LinearCost, evidence: &DatasetEvidence) -> Result<Option<Rational>> {
    let terms = cost.terms.iter().map(|(k, v)| Ok((k.clone(), exact(v)?))).collect::<Result<_>>()?;
    Ok(range(exact(&cost.constant)?, terms, evidence)?.1.as_ref().map(rational))
}

/// Compare certificates on the same complete feasible evidence domain.
/// A crossing or unknown comparison must retain the parent's structural order.
pub fn compare(left: &LinearCost, right: &LinearCost, evidence: &DatasetEvidence) -> Result<Comparison> {
    evidence.check().map_err(|e| error(e.to_string()))?;
    compare_checked(left, right, evidence)
}
fn compare_checked(left: &LinearCost, right: &LinearCost, evidence: &DatasetEvidence) -> Result<Comparison> {
    let mut terms = left.terms.iter().map(|(k, v)| Ok((k.clone(), exact(v)?))).collect::<Result<BTreeMap<_, _>>>()?;
    for (k, v) in &right.terms { *terms.entry(k.clone()).or_insert_with(Q::zero) -= exact(v)?; }
    terms.retain(|_, v| !v.is_zero());
    let (lower, upper) = range(exact(&left.constant)? - exact(&right.constant)?, terms, evidence)?;
    Ok(if lower.as_ref().is_some_and(Zero::is_zero) && upper.as_ref().is_some_and(Zero::is_zero) { Comparison::Equal }
        else if upper.as_ref().is_some_and(|v| v <= &Q::zero()) { Comparison::LeftDominates }
        else if lower.as_ref().is_some_and(|v| v >= &Q::zero()) { Comparison::RightDominates }
        else if lower.as_ref().is_some_and(|v| v < &Q::zero()) && upper.as_ref().is_some_and(|v| v > &Q::zero()) { Comparison::Crossing }
        else { Comparison::Unknown })
}

/// A checked Round negative/key input is a full relation or a filtered/projected
/// set view. Its setup cardinality is at most the source rows. This is distinct
/// from the negative probe's prefix-output bound and does not infer a delta
/// subset premise from an alias name.
#[cfg(feature = "planner-calibration")]
pub(crate) fn negative_input_rows(hole: &Hole, clause: usize, alias: &str, evidence: &DatasetEvidence) -> Result<StepBound> {
    let binding = hole.evidence.as_ref().and_then(|e| e.bindings.iter().find(|b| b.input_relation == alias));
    let bound = binding.and_then(|binding| {
        if binding.view == EvidenceView::Delta { return None; }
        let facts = evidence.relations.get(&binding.dataset_relation)?;
        let fact = if let Some(recursive) = &facts.recursive {
            if recursive.run.admission != ev::Admission::InsertOnlySet { return None; }
            &recursive.final_rows
        } else { facts.rows.as_ref()? };
        let mut bound = Bound::fact(fact)?;
        if binding.view == EvidenceView::Subset {
            bound.citations.insert(format!("compiler:Round-filtered-key-projection:{alias}"));
        }
        Some(bound)
    });
    Ok(StepBound {
        clause, output: bound.as_ref().map(Bound::wire), work: bound.as_ref().map(Bound::wire),
        inequality: "negative input setup: projected/filtered rows <= checked full source rows".into(),
        citations: bound.as_ref().map(|b| b.citations.clone()).unwrap_or_default(),
        unknown: if bound.is_some() { Vec::new() } else { vec![format!("negative setup input {alias} lacks a scoped source row cap")] },
    })
}

struct Preprocessing {
    work: Option<Bound>,
    absorbed: BTreeSet<usize>,
    steps: Vec<super::RecordedStepBound>,
    unknown: Vec<String>,
}

fn preprocessing(hole: &Hole, candidate: &Candidate, evidence: &DatasetEvidence) -> Result<Preprocessing> {
    let mut result = Preprocessing {
        work: Some(Bound::constant(Q::zero(), "no preprocessing work")),
        absorbed: BTreeSet::new(), steps: Vec::new(), unknown: Vec::new(),
    };
    if candidate.unbounded.iter().any(|reason| reason == super::RESTRICTED_VIEW_GAP) {
        result.work = None;
        result.unknown.push(super::RESTRICTED_VIEW_GAP.into());
        return Ok(result);
    }
    if !hole.candidate.steps.iter().any(|step| step.preprocessing) { return Ok(result); }
    let actual = hole.candidate.steps.iter().map(|step|
        syn::parse_str::<cq_lang_vo::Step>(&step.ir)).collect::<syn::Result<Vec<_>>>()?;
    let restrictions = actual.iter().enumerate().filter_map(|(index, step)| match step {
        cq_lang_vo::Step::Restrict(restriction, _) => Some((index, restriction)),
        _ => None,
    }).collect::<Vec<_>>();
    if restrictions.is_empty() { return Ok(result); }
    if hole.candidate.positive_clauses.len() > 1 {
        result.work = None;
        result.unknown.push(super::RESTRICTED_VIEW_GAP.into());
        return Ok(result);
    }
    for step in &candidate.steps {
        if step.introduces || !matches!(step.kind, ClauseKind::Constraint | ClauseKind::Equality) { continue; }
        let reference: cq_lang_vo::Step = syn::parse_str(&hole.candidate_reference.steps[step.clause].ir)?;
        if restrictions.iter().any(|(_, restriction)| {
            cq_lang_vo::restrict::step_states(&reference, &restriction.predicate)
                && !actual.iter().any(|actual| cq_lang_vo::restrict::step_states(actual, &restriction.predicate))
        }) {
            result.absorbed.insert(step.clause);
        }
    }
    for (index, restriction) in restrictions {
        let read = candidate.steps.iter().filter_map(|step| step.read.as_ref())
            .find(|read| read.normalized_clause == restriction.clause());
        let bound = match read {
            Some(read) if read.version == Version::Delta && matches!(hole.origin, Origin::Rule { seed: false, .. }) => Some(Bound::delta()),
            Some(read) => relation(hole, read, evidence)?.as_ref().and_then(Relation::rows),
            None => None,
        };
        let unknown = if bound.is_none() { vec![format!("Restrict step {index} lacks its source row cap")] } else { Vec::new() };
        result.unknown.extend(unknown.iter().cloned());
        result.work = match (result.work, &bound) { (Some(sum), Some(bound)) => Some(sum.add(bound)), _ => None };
        result.steps.push(super::RecordedStepBound {
            vo_step: index,
            bound: StepBound {
                clause: restriction.clause(), output: bound.as_ref().map(Bound::wire), work: bound.as_ref().map(Bound::wire),
                inequality: "single-read occurrence restriction: filtered output <= source rows; predicate work <= source rows".into(),
                citations: bound.as_ref().map(|bound| bound.citations.clone()).unwrap_or_default(), unknown,
            },
        });
    }
    Ok(result)
}
