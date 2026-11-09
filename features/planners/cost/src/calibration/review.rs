//! Observation-quality diagnostics, not additional cardinality certificates.
use super::*;
use crate::cost::{Decision, bounds::BoundModel};
use num_rational::BigRational as Q;
use num_traits::Zero;
#[cfg(test)]
use num_traits::One;
use super::report_bounds::wire;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Looseness {
    pub observed_rows: ev::Rational,
    pub bound_sum: ev::Rational,
    /// None for zero observed rows: 0/0 and positive/0 are not finite ratios.
    pub ratio: Option<ev::Rational>,
    pub packets: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedStepReview {
    pub clause: usize,
    pub logical_relation: Option<String>,
    pub version: Option<crate::export::Version>,
    pub key_columns: Vec<usize>,
    pub same_executed_prefix: bool,
    pub comparison: Option<Looseness>,
    pub scope: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundReview {
    pub hole: usize,
    pub proposed_order: Vec<usize>,
    pub claimed_bound_ratio: Option<ev::Rational>,
    pub classification: String,
    pub incumbent: Option<Looseness>,
    pub proposed_steps: Vec<ProposedStepReview>,
    pub calibrated_fanout_recost: Option<ev::Rational>,
    pub explanation: String,
}
fn looseness<'a>(observations: impl Iterator<Item = &'a PhysicalObservation>) -> Result<Option<Looseness>> {
    let mut observed = Q::zero(); let mut bound = Q::zero(); let mut packets = 0;
    for observation in observations {
        let measured = &observation.measured;
        let Some(comparison) = &measured.comparison else { continue; };
        let rows = measured.rows.value.ok_or_else(|| invalid("observation lacks exact rows"))?;
        observed += Q::from_integer(rows.into());
        bound += comparison.computed.value.as_ref().ok_or_else(|| invalid("comparison lacks bound"))?.exact().map_err(evidence_error)?;
        packets += 1;
    }
    Ok((packets != 0).then(|| Looseness {
        ratio: (!observed.is_zero()).then(|| wire(&bound / &observed)),
        observed_rows: wire(observed), bound_sum: wire(bound), packets,
    }))
}
fn classify(ratio: Option<&Q>) -> &'static str {
    match ratio {
        Some(ratio) if ratio < &Q::new(21.into(), 20.into()) => "C: suppressed sub-1.05 bound ratio",
        Some(ratio) if ratio >= &Q::from_integer(2.into()) => "B: large bound ratio; winning-key evidence unvalidated",
        Some(_) => "D: intermediate ratio [1.05,2)",
        None => "Unknown: no finite exact claimed ratio",
    }
}

/// Compare recorded bounds to observations without transferring counts across
/// changed prefixes, keys, versions, rounds, or fused multi-VO definitions.
/// This does not manufacture per-key maxima from aggregate row counters.
pub fn assess_bounds(report: &CalibrationReport, advice: &AdviceReport, plans: &PlannerInput,
    evidence: &ev::DatasetEvidence) -> Result<Vec<BoundReview>> {
    require(report.measured_input_hash == plans.input_hash && advice.input_hash == plans.input_hash
        && report.measured_dataset_hash == evidence.dataset.hash && advice.dataset_hash == evidence.dataset.hash,
        "bound review records name different inputs or datasets")?;
    let model = BoundModel::new(plans, evidence, &advice.target)?;
    let mut by_hole: BTreeMap<String, Vec<&PhysicalObservation>> = BTreeMap::new();
    for observation in &report.physical_steps {
        by_hole.entry(observation.measured.hole.clone()).or_default().push(observation);
    }
    let mut result = Vec::new();
    for hole in &advice.holes {
        let proposed_index = match &hole.noise_screen {
            Some(screen) => screen.candidate,
            None if hole.decision == Decision::Changed => hole.selected,
            _ => continue,
        };
        let proposed = hole.candidates.get(proposed_index).ok_or_else(|| invalid("review candidate is absent"))?;
        let current = hole.candidates.iter().find(|candidate| candidate.plan.current).ok_or_else(|| invalid("review incumbent is absent"))?;
        let values = |cost: &Option<crate::cost::LinearCost>| -> Result<Option<Q>> {
            cost.as_ref().map(|cost| model.exact_value(cost)).transpose()?.flatten()
                .map(|value| value.exact().map_err(evidence_error)).transpose()
        };
        let ratio = match (values(&current.cost.whole_run)?, values(&proposed.cost.whole_run)?) {
            (Some(old), Some(new)) if !new.is_zero() => Some(old / new),
            _ => None,
        };
        let recorded = plans.holes.iter().find(|record| record.id == hole.hole).ok_or_else(|| invalid("review hole is absent"))?;
        let normal = syn::parse_str(&recorded.normalized_cq)?;
        let valuation = syn::parse::Parser::parse_str(
            |input: syn::parse::ParseStream| cq_lang_vo::Program::parse(input, &normal), &recorded.checked_vo,
        )?;
        let clauses = crate::cost::enumerate::recorded_clause_indices(recorded, &valuation)?;
        let observations = by_hole.get(&format!("hole:{}", hole.hole)).map(Vec::as_slice).unwrap_or(&[]);
        let first_read = current.plan.steps.iter().position(|step| step.read.is_some());
        let preprocessed = valuation.valuation.steps.iter().any(|step|
            matches!(step, cq_lang_vo::Step::Restrict(..) | cq_lang_vo::Step::Reduce(..)));
        let mut steps = Vec::new();
        for (index, step) in proposed.plan.steps.iter().enumerate() {
            let same = current.plan.steps.get(..=index) == proposed.plan.steps.get(..=index);
            let unrestricted_root = first_read == Some(index) && !preprocessed
                && step.read.as_ref().is_some_and(|read| !read.columns.is_empty()
                    && read.columns.iter().all(|column| column.variable.as_ref()
                        .is_some_and(|variable| !step.bound_before.contains(variable))));
            let compared = if same {
                looseness(observations.iter().copied().filter(|observation| {
                    let measured = &observation.measured;
                    if measured.vo_steps.len() != 1 { return false; }
                    let Ok(vo) = measured.vo_steps[0].parse::<usize>() else { return false; };
                    if clauses.get(vo).copied().flatten() != Some(step.clause) { return false; }
                    // A raw RHS setup is not this VO join's output. A genuine
                    // root read is the one case where those scopes coincide.
                    observation.bound_role == "VO stream/preprocessing output"
                        || (unrestricted_root && observation.bound_role == "input/setup caps plus remaining covered VO caps")
                }))?
            } else { None };
            steps.push(ProposedStepReview {
                clause: step.clause,
                logical_relation: step.read.as_ref().map(|read| read.logical_relation.clone()),
                version: step.read.as_ref().map(|read| read.version),
                key_columns: step.read.iter().flat_map(|read| read.columns.iter().filter(|column|
                    column.variable.as_ref().is_none_or(|variable| step.bound_before.contains(variable)))
                    .map(|column| column.source_column)).collect(),
                same_executed_prefix: same,
                scope: if compared.is_some() {
                    "same hole and exact executed VO prefix/key/version; singleton VO-output observations only; proposed physical order was not executed"
                } else { "unobserved proposed prefix/key/version, fused scope, or no exact output observation; no count transfer" }.into(),
                comparison: compared,
            });
        }
        result.push(BoundReview {
            hole: hole.hole, proposed_order: proposed.plan.order.clone(),
            classification: classify(ratio.as_ref()).into(), claimed_bound_ratio: ratio.map(wire),
            incumbent: looseness(observations.iter().copied())?, proposed_steps: steps,
            calibrated_fanout_recost: None,
            explanation: "No A certification: aggregate counters contain neither per-key maxima nor a complete winning-key domain. B records only the numeric >=2 limb, not a claim that every saving is caused solely by loose chain terms. D is necessary because the requested A/B/C thresholds leave [1.05,2) uncovered. Calibrated fan-out re-cost is Unknown, not an average substituted for a maximum.".into(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ratio_classes_keep_exact_boundaries_and_the_unassigned_middle() {
        assert!(classify(Some(&Q::one())).starts_with("C:"));
        assert!(classify(Some(&Q::new(21.into(),20.into()))).starts_with("D:"));
        assert!(classify(Some(&Q::new(199999.into(),100000.into()))).starts_with("D:"));
        assert!(classify(Some(&Q::from_integer(2.into()))).starts_with("B:"));
        assert!(classify(None).starts_with("Unknown:"));
    }
}
