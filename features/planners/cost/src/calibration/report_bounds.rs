use super::*;
use crate::cost::{CandidateRecord, DeltaCost, StepBound};
use num_rational::BigRational as Q;
use num_traits::{One, Zero};

#[derive(Clone)]
pub(super) struct Bound {
    pub value: Option<DeltaCost>,
    pub citations: BTreeSet<String>,
    pub unknown: Vec<String>,
}
impl Bound {
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self { value: None, citations: BTreeSet::new(), unknown: vec![reason.into()] }
    }
    pub fn unit() -> Self {
        Self { value: Some(DeltaCost { coefficient: wire(Q::zero()), constant: wire(Q::one()) }),
            citations: BTreeSet::from(["compiler:unit-or-empty-key-group-at-most-one-row".into()]), unknown: Vec::new() }
    }
    pub fn from_step(step: &StepBound) -> Self {
        Self { value: step.output.clone(), citations: step.citations.clone(), unknown: step.unknown.clone() }
    }
    pub fn sum(bounds: impl IntoIterator<Item = Self>) -> Result<Self> {
        let mut a = Q::zero(); let mut b = Q::zero();
        let mut citations = BTreeSet::new(); let mut unknown = BTreeSet::new();
        let mut complete = true;
        for bound in bounds {
            citations.extend(bound.citations);
            unknown.extend(bound.unknown);
            if let Some(value) = bound.value {
                a += value.coefficient.exact().map_err(evidence_error)?;
                b += value.constant.exact().map_err(evidence_error)?;
            } else { complete = false; }
        }
        if !complete && unknown.is_empty() { unknown.insert("one covered VO site lacks a finite row bound".into()); }
        Ok(Self { value: complete.then(|| DeltaCost { coefficient: wire(a), constant: wire(b) }), citations, unknown: unknown.into_iter().collect() })
    }
    pub fn at(&self, delta: u64) -> Result<Option<ev::Rational>> {
        self.value.as_ref().map(|value| Ok(wire(value.coefficient.exact().map_err(evidence_error)? * Q::from_integer(delta.into())
            + value.constant.exact().map_err(evidence_error)?))).transpose()
    }
}
pub(super) fn wire(value: Q) -> ev::Rational { ev::Rational { numerator: value.numer().to_string(), denominator: value.denom().to_string() } }

pub(super) struct Catalog {
    streams: BTreeMap<String, Bound>,
    pub inputs: BTreeMap<usize, Bound>,
    clauses: BTreeMap<String, usize>,
}
impl Catalog {
    pub fn new(hole: &crate::export::Hole, current: &CandidateRecord, evidence: &ev::DatasetEvidence) -> Result<Self> {
        let normal = syn::parse_str(&hole.normalized_cq)?;
        let valuation = syn::parse::Parser::parse_str(
            |input: syn::parse::ParseStream| cq_lang_vo::Program::parse(input, &normal), &hole.checked_vo,
        )?;
        let clauses = crate::cost::enumerate::recorded_clause_indices(hole, &valuation)?;
        let mut inputs = BTreeMap::new();
        for step in &current.plan.steps {
            if let Some(read) = &step.read {
                inputs.insert(step.clause, Bound::from_step(&crate::cost::bounds::input_rows(hole, read, evidence)?));
            }
        }
        for (clause, item) in normal.query.body.iter().enumerate() {
            if let cq_lang_cq::BodyItem::Negation(negative) = item {
                let cap = crate::cost::bounds::negative_input_rows(hole, clause,
                    &cq_syntax::symbol_name(&negative.atom.relation), evidence)?;
                inputs.insert(clause, Bound::from_step(&cap));
            }
        }
        let mut by_clause: BTreeMap<usize, std::collections::VecDeque<_>> = BTreeMap::new();
        for step in &current.cost.steps { by_clause.entry(step.clause).or_default().push_back(step); }
        let mut streams = BTreeMap::new();
        let mut mapped = BTreeMap::new();
        let mut prefix = Bound::unit();
        for (index, (step, clause)) in valuation.valuation.steps.iter().zip(clauses).enumerate() {
            let value = match step {
                cq_lang_vo::Step::Restrict(restrict, _) => inputs.get(&restrict.clause()).cloned()
                    .unwrap_or_else(|| Bound::unknown("restriction input lacks a row cap")),
                cq_lang_vo::Step::Reduce(reduce, _) => {
                    let cq_lang_vo::Reduction::With(_, support) = &reduce.reduction;
                    Bound::sum([
                        inputs.get(&reduce.clause()).cloned().unwrap_or_else(|| Bound::unknown("reduction input lacks a row cap")),
                        inputs.get(&support.clause()).cloned().unwrap_or_else(|| Bound::unknown("reduction support lacks a row cap")),
                        prefix.clone(),
                    ])?
                }
                _ => {
                    let clause = clause.ok_or_else(|| invalid("stream VO site has no source obligation"))?;
                    mapped.insert(index.to_string(), clause);
                    let bound = by_clause.get_mut(&clause).and_then(|queue| queue.pop_front())
                        .map(Bound::from_step).unwrap_or_else(|| Bound::unknown("recorded stream site lacks its candidate bound"));
                    prefix = bound.clone();
                    bound
                }
            };
            streams.insert(index.to_string(), value);
        }
        streams.insert(cq_stage::calibration::OUTPUT.into(), prefix);
        Ok(Self { streams, inputs, clauses: mapped })
    }
    /// Setup scans are bounded by their input, not by a potentially empty join
    /// output. Other covered sites retain their own stream/preprocessing cap.
    pub fn combined(&self, origins: &[String], raw_inputs: &BTreeSet<usize>, unit: bool) -> Result<Bound> {
        Bound::sum(origins.iter().map(|origin| {
            if let Some(clause) = self.clauses.get(origin).filter(|clause| raw_inputs.contains(clause)) {
                return self.inputs.get(clause).cloned().unwrap_or_else(|| Bound::unknown("raw setup input is unbounded"));
            }
            if unit { return Bound::unit(); }
            self.streams.get(origin).cloned().unwrap_or_else(|| Bound::unknown(format!("nested VO site {origin} has no finite scope-work certificate")))
        }))
    }
}
