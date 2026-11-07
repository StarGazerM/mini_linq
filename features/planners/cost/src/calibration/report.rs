use super::*;
use cq_stage::calibration::Trace;
use super::lineage::{checked, nodes};
use cq_storage::counters::OperatorRecord;
use crate::cost::{CandidateRecord, Decision};
use crate::export::Origin;
use super::report_bounds::{Bound, Catalog};
use num_rational::BigRational;

pub const REPORT_SCHEMA: &str = "mini-linq.calibration-report.v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalObservation {
    pub measured: ev::PhysicalStepObservation,
    pub bound_role: String,
    pub planned_comparison: Option<ev::PhysicalBoundComparison>,
    pub planned_unknown: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDifference {
    pub hole: usize,
    pub executed_order: Vec<usize>,
    pub prior_advised_order: Vec<usize>,
    pub measured_advised_order: Vec<usize>,
    pub prior_decision: Decision,
    pub measured_decision: Decision,
    pub admission_held_fixed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceProvenance {
    pub hole: usize,
    pub semantic_vo_hash: String,
    pub host_aliases: BTreeMap<String, String>,
    pub descriptor: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnobservedPhysicalStep {
    pub hole: usize,
    pub program: usize,
    pub physical_step: String,
    pub trace_hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationReport {
    pub schema: String,
    pub run: ev::RunIdentity,
    pub original_input_hash: String,
    pub original_evidence_hash: String,
    pub measured_input_hash: String,
    pub measured_evidence_hash: String,
    pub original_dataset_hash: String,
    pub measured_dataset_hash: String,
    pub traces: BTreeMap<String, TraceProvenance>,
    pub physical_steps: Vec<PhysicalObservation>,
    pub unexecuted_holes: Vec<usize>,
    pub unobserved_physical_steps: Vec<UnobservedPhysicalStep>,
    pub excluded_auxiliary_counters: Vec<String>,
    pub dropped_relative_facts: Vec<String>,
    pub plan_differences: Vec<PlanDifference>,
    #[serde(default)]
    pub bound_reviews: Vec<BoundReview>,
    pub runtime_adoption: String,
}
impl CalibrationReport {
    pub(super) fn finish(&mut self, before: &AdviceReport, after: &AdviceReport, plans: &PlannerInput, evidence: &ev::DatasetEvidence) -> Result<()> {
        self.measured_input_hash = plans.input_hash.clone();
        self.measured_evidence_hash = hash_bytes(evidence.canonical_json().map_err(evidence_error)?.as_bytes());
        self.plan_differences = before.holes.iter().zip(&after.holes).map(|(old, new)| {
            let executed = old.candidates.iter().find(|candidate| candidate.plan.current).ok_or_else(|| invalid("missing recorded incumbent"))?;
            Ok(PlanDifference {
                hole: old.hole, executed_order: executed.plan.order.clone(),
                prior_advised_order: old.candidates[old.selected].plan.order.clone(),
                measured_advised_order: new.candidates[new.selected].plan.order.clone(),
                prior_decision: old.decision, measured_decision: new.decision,
                admission_held_fixed: new.admission.as_ref().is_some_and(|a| a.held_fixed),
            })
        }).collect::<Result<_>>()?;
        self.bound_reviews = assess_bounds(self, after, plans, evidence)?;
        Ok(())
    }
    /// Deterministic count report. No elapsed times or hardware-fit coefficients
    /// are inferred from the operator counters.
    pub fn render(&self, evidence: &ev::DatasetEvidence) -> String {
        let mut text = format!("Calibration {:?}\nprogram={}\nconfiguration={}\ndataset={} -> {}\n{}\n", self.run.run_id,
            self.run.program_hash, self.run.configuration_hash, self.original_dataset_hash, self.measured_dataset_hash, self.runtime_adoption);
        for (name, relation) in &evidence.relations {
            if let Some(recursive) = &relation.recursive {
                text.push_str(&format!("relation {:?}: final={} N={} seed={} unconsumed={} [calibrated; {}]\n", name,
                    recursive.final_rows.value.unwrap(), recursive.rounds.value.unwrap(),
                    recursive.seed_rows.as_ref().and_then(|f| f.value).map_or_else(|| "unknown".into(), |n| n.to_string()),
                    recursive.unconsumed_final_delta_rows.as_ref().and_then(|f| f.value).map_or_else(|| "unknown".into(), |n| n.to_string()), recursive.final_rows.id));
            }
        }
        for series in &evidence.observations.rounds {
            for entry in &series.entries {
                text.push_str(&format!("delta {:?} round={} rows={} [calibrated; {}]\n", series.relation,
                    entry.round.value.unwrap(), entry.delta_rows.value.unwrap(), entry.delta_rows.id));
            }
        }
        text.push_str("hole / variant / round / instance | Physical -> VO | role | planned bound | observed rows | measured bound | fusion\n");
        for step in &self.physical_steps {
            let m = &step.measured;
            text.push_str(&format!("{} / {} / {} / {} | {:?} -> {:?} | {} | {} | {} [calibrated; {}] | {} | {}\n",
                m.hole, m.variant, m.round.as_ref().and_then(|r| r.value).map_or_else(|| "seed/key".into(), |r| r.to_string()), m.instance,
                m.physical_steps, m.vo_steps, step.bound_role, comparison(&step.planned_comparison),
                m.rows.value.unwrap(), m.rows.id, comparison(&m.comparison), if m.fused { "fused; counted once" } else { "single" }));
            for reason in step.planned_unknown.iter().chain(&m.unknown) { text.push_str(&format!("  unknown: {:?}\n", reason)); }
        }
        for step in &self.unobserved_physical_steps {
            text.push_str(&format!("hole {} / program:{} / {}: not row-observed (no exact zero is asserted)\n",
                step.hole, step.program, step.physical_step));
        }
        for change in &self.plan_differences {
            text.push_str(&format!("hole {}: executed {:?}; prior advice {:?}; measured advice {:?}; {:?}; admission held fixed={}\n",
                change.hole, change.executed_order, change.prior_advised_order, change.measured_advised_order, change.measured_decision, change.admission_held_fixed));
        }
        for review in &self.bound_reviews {
            text.push_str(&format!("bound review hole {}: {}; claimed ratio={}; incumbent looseness={}; calibrated fan-out re-cost=Unknown\n",
                review.hole, review.classification, display_ratio(&review.claimed_bound_ratio),
                review.incumbent.as_ref().map_or_else(|| "unknown".into(), |value| display_ratio(&value.ratio))));
            for step in &review.proposed_steps {
                text.push_str(&format!("  proposed clause {} key {:?}: looseness={}; {}\n", step.clause, step.key_columns,
                    step.comparison.as_ref().map_or_else(|| "unobserved".into(), |value| display_ratio(&value.ratio)), step.scope));
            }
            text.push_str(&format!("  {}\n", review.explanation));
        }
        text
    }
}
fn display_ratio(value: &Option<ev::Rational>) -> String {
    value.as_ref().map_or_else(|| "undefined/unknown".into(), |value| format!("{}/{}", value.numerator, value.denominator))
}
fn comparison(value: &Option<ev::PhysicalBoundComparison>) -> String {
    match value {
        Some(value) => {
            let n = value.computed.value.as_ref().unwrap();
            format!("{}/{} ({}) [{}]", n.numerator, n.denominator, if value.within_bound { "within" } else { "EXCEEDED" }, value.computed.id)
        }
        None => "unknown".into(),
    }
}

struct CheckedTrace {
    text: &'static str,
    hash: String,
    aliases: Vec<(&'static str, &'static str)>,
    provenance: TraceProvenance,
    coverage: BTreeMap<String, Vec<String>>,
    raw: BTreeMap<String, RawScope>,
    kinds: BTreeMap<String, String>,
    repeated_inputs: BTreeSet<String>,
    unknown_evaluation: BTreeMap<String, String>,
}
#[derive(Clone, Copy)]
enum RawScope { Input(usize), Unit, Stream }
fn checked_trace(text: &'static str, hole: &crate::export::Hole, context: &round::RoundContext,
    aliases: &[(&'static str, &'static str)]) -> Result<CheckedTrace> {
    let trace = Trace::parse_descriptor(text)?;
    let seed = trace.seed.as_ref().ok_or_else(|| invalid("physical trace lacks its VO producer"))?;
    let mut valuation = checked::<cq_lang_vo::Program>(&seed.valuation)?;
    let lower = checked::<cq_lang_ra::Program>(&seed.lowering)?;
    let steps = cq_relational_lowering::Steps(seed.steps.clone());
    cq_relational_lowering::check(&valuation, &lower, &steps)?;
    let mut expected: cq_lang_vo::Program = syn::parse_str(&hole.checked_vo)?;
    let clauses = crate::cost::enumerate::recorded_clause_indices(hole, &expected)?;
    valuation.cq.declared_evidence = None;
    expected.cq.declared_evidence = None;
    let semantic_vo_hash = hash_bytes(expected.to_token_stream().to_string().as_bytes());
    let hosts = context.relations.iter().flat_map(|relation|
        [&relation.next_canonical, &relation.next].into_iter().chain(relation.delta_host.iter()))
        .chain(context.layouts.iter().chain(&context.delta_layouts).map(|layout| &layout.host))
        .map(ToString::to_string).collect::<BTreeSet<_>>();
    require(aliases.iter().all(|(source, _)| hosts.contains(*source)), "trace alias is not a recorded compiler-owned host")?;
    expected = syn::parse2(cq_datalog_print::substitute_host_parameters(expected.to_token_stream(), aliases)?)?;
    require(valuation == expected, "executed VO differs from the supplied checked variant")?;
    let mut physical = None;
    let mut structures = None;
    if let Some(stage) = trace.stages.last() {
        if stage.is::<cq_lang_physical::Program>() {
            physical = Some(checked::<cq_lang_physical::Program>(stage)?);
        } else if stage.is::<cq_lang_structures::Program>() {
            let checked = checked::<cq_lang_structures::Program>(stage)?;
            structures = Some(checked.structures.entries);
            physical = Some(checked.physical);
        } else if stage.is::<cq_lang_sequence::Program>() {
            let checked = checked::<cq_lang_sequence::Program>(stage)?;
            structures = Some(checked.structures.structures.entries);
            physical = Some(checked.structures.physical);
        }
    }
    let physical = physical.ok_or_else(|| invalid("ST-Calibration: terminal requires actual final Physical definitions"))?;
    let mut coverage = trace.coverage(nodes)?.into_iter().map(|c| (c.node, c.origins)).collect::<BTreeMap<_, _>>();
    let mut raw = BTreeMap::new();
    let mut kinds = BTreeMap::new();
    use cq_lang_physical::Operator as Op;
    for definition in &physical.program.definitions {
        kinds.insert(cq_syntax::symbol_name(&definition.name),
            definition.operator.to_token_stream().to_string().split_whitespace().next().expect("operator keyword").to_uppercase());
        let unary = |name: &syn::Ident| raw.get(&cq_syntax::symbol_name(name)).copied().unwrap_or(RawScope::Stream);
        let scope = match &definition.operator {
            Op::Rename(_) => {
                match steps.0.get(&cq_syntax::symbol_name(&definition.name)).map(Vec::as_slice) {
                    Some([index]) => match &valuation.valuation.steps[*index] {
                        cq_lang_vo::Step::Atom(atom, _) if atom.clause.0.len() == 1 =>
                            RawScope::Input(atom.clause.clause()),
                        cq_lang_vo::Step::Negation(_, _) =>
                            clauses[*index].map(RawScope::Input).unwrap_or(RawScope::Stream),
                        _ => RawScope::Stream,
                    },
                    _ => RawScope::Stream,
                }
            }
            Op::Unit(_) => RawScope::Unit,
            Op::Select(select) | Op::Point(_, select) | Op::Range(_, select) => unary(&select.input),
            Op::Bind(bind) if bind.for_token.is_none() => unary(&bind.input),
            Op::Guard(guard) => unary(&guard.input),
            Op::Project(project) => unary(&project.input),
            Op::Group(group) if group.keys.is_empty() => RawScope::Unit,
            Op::Group(group) => unary(&group.input),
            _ => RawScope::Stream,
        };
        raw.insert(cq_syntax::symbol_name(&definition.name), scope);
    }
    require(raw.keys().eq(coverage.keys()), "trace coverage does not match actual Physical definitions")?;
    let (repeated_inputs, mut unknown_evaluation) = evaluation_scopes(&physical, structures.as_deref(), &coverage, &mut raw);
    for (name, scope) in &raw {
        if matches!(scope, RawScope::Stream) && !steps.0.contains_key(name) {
            unknown_evaluation.entry(name.clone()).or_insert_with(||
                "M6: sparse lowering Steps does not certify this Physical definition's stream-work scope".into());
        }
    }
    // ANSWER is the existing counter for Physical's explicit return, not a
    // fabricated definition or a storage enumeration count.
    let answer = format!("{}.answer", physical.program.returned);
    coverage.insert(answer.clone(), vec![cq_stage::calibration::OUTPUT.into()]);
    raw.insert(answer.clone(), RawScope::Stream);
    kinds.insert(answer, "ANSWER".into());
    let provenance = TraceProvenance {
        hole: hole.id, semantic_vo_hash,
        host_aliases: aliases.iter().map(|(source, target)| ((*source).into(), (*target).into())).collect(),
        descriptor: text.into(),
    };
    let hash = ev::canonical_hash(&provenance).map_err(evidence_error)?;
    Ok(CheckedTrace { text, hash, aliases: aliases.to_vec(), provenance, coverage, raw, kinds, repeated_inputs, unknown_evaluation })
}

fn current(report: &AdviceReport, hole: usize) -> Result<&CandidateRecord> {
    report.holes.get(hole).and_then(|h| h.candidates.iter().find(|c| c.plan.current)).ok_or_else(|| invalid("report lacks the executed incumbent"))
}
fn identity(origin: &Origin) -> Result<(usize, String)> {
    match origin {
        Origin::Rule { stratum, variant, seed, .. } => Ok((*stratum, format!("variant:{variant}/{}", if *seed { "seed" } else { "steady" }))),
        Origin::NegativeKey { stratum, key, .. } => Ok((*stratum, format!("key:{key}"))),
        Origin::Cq {} => Err(invalid("Datalog observation has a standalone CQ owner")),
    }
}
fn delta(program: &round::Program, evidence: &ev::DatasetEvidence, origin: &Origin, index: Option<usize>) -> Result<u64> {
    let Origin::Rule { seed: false, original_delta: Some(position), source_rule, .. } = origin else { return Ok(0); };
    let index = index.ok_or_else(|| invalid("steady variant lacks its round scope"))?;
    let source = &program.storage.incremental.strata.source;
    let cq_lang_cq::BodyItem::Positive { atom } = &source.rules[*source_rule].body[*position] else { return Err(invalid("delta owner is not a positive occurrence")); };
    let binding = source.declared_evidence.as_ref().and_then(|e| e.bindings.iter().find(|b| b.relation == atom.relation)).ok_or_else(|| invalid("measured delta lacks an explicit source binding"))?;
    evidence.observations.rounds.iter().find(|s| s.relation == binding.source)
        .and_then(|s| s.entries.get(index)).and_then(|e| e.delta_rows.value).ok_or_else(|| invalid("physical scope has no measured delta entry"))
}
fn supplied_ids(evidence: &ev::DatasetEvidence) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for relation in evidence.relations.values() {
        ids.extend(relation.rows.iter().map(|f| f.id.clone()));
        for column in relation.columns.values() {
            ids.extend(column.distinct.iter().chain(&column.max_degree).map(|f| f.id.clone()));
            ids.extend(column.min.iter().chain(&column.max).map(|f| f.id.clone()));
        }
        for key in &relation.key_sets { ids.extend(key.distinct.iter().chain(&key.max_degree).map(|f| f.id.clone())); }
        if let Some(recursive) = &relation.recursive {
            ids.insert(recursive.final_rows.id.clone()); ids.insert(recursive.rounds.id.clone());
            ids.extend(recursive.seed_rows.iter().chain(&recursive.unconsumed_final_delta_rows).map(|f| f.id.clone()));
        }
    }
    ids.extend(evidence.premises.iter().map(|premise| match premise {
        ev::Premise::DeltaSubset { id, .. } | ev::Premise::DeltaPartition { id, .. } => id.clone(),
    }));
    ids
}
fn compare(bound: &Bound, x: u64, rows: u64, id: String, hash: &str, run: &ev::RunIdentity, facts: &BTreeSet<String>) -> Result<Option<ev::PhysicalBoundComparison>> {
    let Some(value) = bound.at(x)? else { return Ok(None); };
    let within_bound = BigRational::from_integer(rows.into()) <= value.exact().map_err(evidence_error)?;
    let (fact_citations, compiler_citations) = bound.citations.iter().cloned().partition(|id| facts.contains(id));
    Ok(Some(ev::PhysicalBoundComparison {
        computed: ev::Fact {
            id: id.clone(), source: ev::Source::Calibrated, claim: ev::Claim::Observation,
            value: Some(value), lower: None, upper: None, dataset_hash: hash.into(), unit: ev::Unit::Rows,
            provenance: measured::provenance(run, id, "exact cost-tool calculation at the observed global delta cap for this operator instance; diagnostic comparison, not a cardinality certificate"),
        }, fact_citations, compiler_citations, within_bound,
    }))
}

pub(super) fn build(input: &PlannerInput, baseline: &ev::DatasetEvidence, plans: &PlannerInput, evidence: &mut ev::DatasetEvidence,
    before: &AdviceReport, operators: &[OperatorRecord], run: &ev::RunIdentity) -> Result<CalibrationReport> {
    let CheckedIr::Datalog { round: text } = &plans.checked_ir else { return Err(invalid("missing measured Round")); };
    let program: round::Program = syn::parse_str(text)?;
    let mut report = CalibrationReport {
        schema: REPORT_SCHEMA.into(), run: run.clone(), original_input_hash: input.input_hash.clone(),
        original_evidence_hash: hash_bytes(baseline.canonical_json().map_err(evidence_error)?.as_bytes()),
        measured_input_hash: String::new(), measured_evidence_hash: String::new(),
        original_dataset_hash: baseline.dataset.hash.clone(), measured_dataset_hash: evidence.dataset.hash.clone(),
        traces: BTreeMap::new(), physical_steps: Vec::new(), unexecuted_holes: Vec::new(),
        unobserved_physical_steps: Vec::new(),
        excluded_auxiliary_counters: Vec::new(), dropped_relative_facts: baseline.relative.iter().map(|r| r.id.clone()).collect(),
        plan_differences: Vec::new(), bound_reviews: Vec::new(),
        runtime_adoption: "No runtime advice adoption; measured orders are unvalidated proposals for a later checked compilation, not performance evidence.".into(),
    };
    let mut traces = BTreeMap::<usize, Vec<CheckedTrace>>::new();
    let mut program_traces = BTreeMap::<usize, (usize, usize)>::new();
    let mut groups = BTreeMap::<usize, Vec<&OperatorRecord>>::new();
    let records: BTreeMap<_, _> = operators.iter().map(|record| (record.id, record)).collect();
    for operator in operators.iter().filter(|o| o.calibration.is_some()) {
        let frame = operator.calibration.as_ref().unwrap();
        let hole = input.holes.get(frame.hole).ok_or_else(|| invalid("operator names an absent hole"))?;
        let text = frame.trace().ok_or_else(|| invalid("operator lacks compiler-produced Physical lineage"))?;
        let entries = traces.entry(frame.hole).or_default();
        let index = if let Some(&(owner, index)) = program_traces.get(&operator.program) {
            require(owner == frame.hole, "operator program crosses hole ownership")?;
            let trace = &entries[index];
            require((std::ptr::eq(trace.text, text) || trace.text == text) && trace.aliases == frame.host_aliases(),
                "operator program changed its recorded lineage or host bindings")?;
            index
        } else {
            let index = if let Some(index) = entries.iter().position(|trace|
                (std::ptr::eq(trace.text, text) || trace.text == text) && trace.aliases == frame.host_aliases())
            { index } else {
                let index = entries.len();
                entries.push(checked_trace(text, hole, &program.context, frame.host_aliases())?);
                index
            };
            program_traces.insert(operator.program, (frame.hole, index));
            index
        };
        let trace = &entries[index];
        if !trace.coverage.contains_key(operator.label.result) {
            require(operator.label.kind.ends_with("_BUILD")
                || matches!(operator.label.kind, "TASKS_BUILD_WALL" | "TASKS_QUERY_WALL"),
                "counter record is neither an actual Physical step nor a known auxiliary counter")?;
            report.excluded_auxiliary_counters.push(format!("program:{}/{}:{}", operator.program, operator.label.result, operator.label.kind));
            continue;
        }
        require(trace.kinds[operator.label.result] == operator.label.kind, "operator counter kind differs from its recorded Physical step")?;
        if !operator.row_observed {
            report.unobserved_physical_steps.push(UnobservedPhysicalStep {
                hole: frame.hole, program: operator.program,
                physical_step: operator.label.result.into(), trace_hash: trace.hash.clone(),
            });
            continue;
        }
        let mut root = operator;
        let mut visited = BTreeSet::new();
        while let Some(alias) = root.row_alias {
            require(visited.insert(root.id), "cyclic row-counter alias")?;
            root = records.get(&alias).copied().ok_or_else(|| invalid("dangling row-counter alias"))?;
            require(root.program == operator.program && root.calibration == operator.calibration, "row alias crosses an execution scope")?;
        }
        require(root.rows_out == operator.rows_out, "resolved row alias has inconsistent counts")?;
        groups.entry(root.id).or_default().push(operator);
    }
    let measured_model = crate::cost::bounds::BoundModel::new(plans, evidence, target_name(input.configuration.target))?;
    let mut catalogs = BTreeMap::new();
    for &hole in traces.keys() {
        let old = current(before, hole)?;
        let measured = CandidateRecord { plan: old.plan.clone(), cost: measured_model.cost(&plans.holes[hole], &old.plan, &BTreeSet::new())? };
        catalogs.insert(hole, (Catalog::new(&input.holes[hole], old, baseline)?, Catalog::new(&plans.holes[hole], &measured, evidence)?));
    }
    let old_ids = supplied_ids(baseline); let new_ids = supplied_ids(evidence);
    for (root, group) in groups {
        let first = group[0]; let frame = first.calibration.as_ref().unwrap();
        let hole = &input.holes[frame.hole];
        let (stratum, variant) = identity(&hole.origin)?;
        require(frame.stratum == Some(stratum), "Physical observation has the wrong Round component")?;
        let trace = &traces[&frame.hole][program_traces[&first.program].1];
        let physical_steps = group.iter().map(|o| o.label.result.to_string()).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        let vo_steps = physical_steps.iter().flat_map(|node| trace.coverage[node].iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        let raw_inputs = if physical_steps.iter().any(|node| matches!(trace.raw[node], RawScope::Stream)) {
            BTreeSet::new()
        } else {
            physical_steps.iter().filter_map(|node| match trace.raw[node] {
                RawScope::Input(clause) => Some(clause), _ => None,
            }).collect::<BTreeSet<_>>()
        };
        let unit = physical_steps.iter().all(|node| matches!(trace.raw[node], RawScope::Unit));
        let (old, new) = &catalogs[&frame.hole];
        let unsupported = physical_steps.iter().find_map(|name| trace.unknown_evaluation.get(name));
        let repeated = physical_steps.iter().any(|name| trace.repeated_inputs.contains(name));
        let old = if let Some(reason) = unsupported { Bound::unknown(reason.clone()) } else { old.combined(&vo_steps, &raw_inputs, unit)? };
        let new = if let Some(reason) = unsupported { Bound::unknown(reason.clone()) } else { new.combined(&vo_steps, &raw_inputs, unit)? };
        let x = delta(&program, evidence, &hole.origin, frame.round)?;
        let id = format!("calibration:{}:hole:{}:program:{}:operator:{root}", run.run_id, frame.hole, first.program);
        let rows = first.rows_out;
        let measurement = ev::PhysicalStepObservation {
            hole: format!("hole:{}", frame.hole), variant, instance: format!("program:{}:operator:{root}", first.program),
            stratum: stratum.to_string(), trace_hash: trace.hash.clone(),
            fused: physical_steps.len() > 1 || vo_steps.len() > 1, physical_steps, vo_steps,
            run: run.clone(), round: frame.round.map(|round| measured::count(format!("{id}:round"), round as u64 + 1, ev::Unit::Rounds, &evidence.dataset.hash, run, &id)),
            rows: measured::count(format!("{id}:rows"), rows, ev::Unit::Rows, &evidence.dataset.hash, run, &id),
            comparison: compare(&new, x, rows, format!("{id}:computed"), &evidence.dataset.hash, run, &new_ids)?, unknown: new.unknown,
        };
        let planned_comparison = compare(&old, x, rows, format!("{id}:planned"), &baseline.dataset.hash, run, &old_ids)?;
        report.physical_steps.push(PhysicalObservation {
            measured: measurement.clone(), bound_role: if unsupported.is_some() {
                "unmodeled repeated-input evaluation scope"
            } else if repeated {
                "keyless RHS replay: aggregate rows use the covered VO join-output cap"
            } else if unit {
                "singleton setup: Physical unit/empty-key group"
            } else if raw_inputs.is_empty() {
                "VO stream/preprocessing output"
            } else {
                "input/setup caps plus remaining covered VO caps"
            }.into(),
            planned_comparison, planned_unknown: old.unknown,
        });
        evidence.observations.physical_steps.push(measurement);
    }
    report.unexecuted_holes = input.holes.iter().filter(|h| !traces.contains_key(&h.id)).map(|h| h.id).collect();
    report.traces = traces.into_values().flatten().map(|trace| (trace.hash, trace.provenance)).collect();
    Ok(report)
}

/// A leaf on the right of a first-driven Cartesian probe is replayed once per
/// prefix row. Its aggregate counter equals the probe's pre-projection stream,
/// not one scan of the base relation. The equal VO coverage and sole consumer
/// are checked before using that stream cap. Other repeated scopes remain
/// explicitly unknown rather than assuming a materialization or call count.
fn evaluation_scopes(
    program: &cq_lang_physical::Program,
    structures: Option<&[cq_lang_structures::Entry]>,
    coverage: &BTreeMap<String, Vec<String>>,
    raw: &mut BTreeMap<String, RawScope>,
) -> (BTreeSet<String>, BTreeMap<String, String>) {
    use cq_lang_physical::{Choice, Mode, Operator};
    use cq_lang_structures::Structure;
    let definitions = program.program.definitions.iter()
        .map(|definition| (cq_syntax::symbol_name(&definition.name), definition)).collect::<BTreeMap<_, _>>();
    let mut uses = BTreeMap::<String, usize>::new();
    for definition in &program.program.definitions {
        let mut operands = definition.operator.operands().into_iter().map(cq_syntax::symbol_name).collect::<Vec<_>>();
        if let Operator::Union(union) = &definition.operator {
            if let Some(over) = union.origin.as_ref().and_then(|origin| origin.over.as_ref()) {
                let prefix = cq_syntax::symbol_name(&over.prefix);
                if !operands.contains(&prefix) { operands.push(prefix); }
            }
        }
        for operand in operands { *uses.entry(operand).or_default() += 1; }
    }
    let mut repeated = BTreeSet::new();
    let mut unknown = BTreeMap::new();
    for definition in &program.program.definitions {
        let (keyed, probe) = match &definition.operator {
            Operator::Probe(_, keyed) => (keyed, true),
            Operator::Semi(_, keyed) | Operator::Anti(_, keyed) => (keyed, false),
            _ => continue,
        };
        if !keyed.columns.is_empty() { continue; }
        let right = cq_syntax::symbol_name(&keyed.right);
        let input = definitions[&right];
        if matches!(input.mode, Mode::Let(_)) { continue; }
        let name = cq_syntax::symbol_name(&definition.name);
        let host_only = structures.is_some_and(|entries| {
            let mut present = false;
            for entry in entries.iter().filter(|entry| cq_syntax::symbol_name(&entry.name) == right) {
                present = true;
                if !matches!(entry.structure, Structure::Host { .. } | Structure::Native { .. }) { return false; }
            }
            present
        });
        let first = matches!(keyed.choice, None | Some(Choice::First(_)));
        if probe && first && matches!(input.mode, Mode::Pipe(_))
            && matches!(input.operator, Operator::Rename(_)) && host_only
            && uses.get(&right) == Some(&1) && coverage[&right] == coverage[&name]
        {
            raw.insert(right.clone(), RawScope::Stream);
            repeated.insert(right);
            continue;
        }
        let mut pending = vec![right];
        if !first { pending.push(cq_syntax::symbol_name(&keyed.left)); }
        let mut visited = BTreeSet::new();
        while let Some(node) = pending.pop() {
            if !visited.insert(node.clone()) { continue; }
            let input = definitions[&node];
            if matches!(input.mode, Mode::Let(_)) { continue; }
            unknown.insert(node.clone(), format!("Physical {node} is evaluated in an unsupported repeated-input scope of {name}; no single-scan cap is asserted"));
            pending.extend(input.operator.operands().into_iter().map(cq_syntax::symbol_name));
        }
    }
    (repeated, unknown)
}
