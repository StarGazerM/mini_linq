//! Opt-in completed-run evidence. This module observes execution and invokes the
//! offline advisor only after the query returns; it never adopts advice at runtime.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use cq_lang_datalog_round as round;
use cq_load::stats_manifest::DatasetRequest;
use cq_syntax::evidence as ev;
use cq_syntax::declared_evidence::{ColumnBinding, DeclaredEvidence, EvidenceView, RelationBinding};
use quote::ToTokens;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use crate::export::{CheckedIr, PlannerInput, Target};
use crate::cost::AdviceReport;

mod capture;
mod measured;
mod report;
mod report_bounds;
mod review;
pub use review::{assess_bounds, BoundReview, Looseness, ProposedStepReview};
pub use report::{CalibrationReport, PhysicalObservation, PlanDifference, TraceProvenance, UnobservedPhysicalStep};
use capture::{Captured, InputCapture, Recorder, StratumCapture};
pub type Result<T> = crate::export::Result<T>;
fn invalid(message: impl Into<String>) -> crate::export::ExportError { crate::export::ExportError(message.into()) }
fn require(condition: bool, message: impl Into<String>) -> Result<()> { if condition { Ok(()) } else { Err(invalid(message)) } }
fn hash_bytes(bytes: &[u8]) -> String { format!("sha256:{:x}", Sha256::digest(bytes)) }
fn evidence_error(error: ev::EvidenceError) -> crate::export::ExportError { invalid(error.to_string()) }

/// The value is the unchanged program result. The emitted advice is a proposal
/// for a later compilation, not a modification of this completed invocation.
pub struct CalibrationResult<T> {
    pub value: T,
    pub evidence: ev::DatasetEvidence,
    pub plans: PlannerInput,
    pub advice: AdviceReport,
    pub report: CalibrationReport,
}

/// Execute exactly one synchronous Datalog invocation in an isolated counters
/// epoch. Load raw metadata inside `run`, forwarding its already-read bytes with
/// `cq_storage::counters::calibration::metadata`; input rows are observed at the
/// generated canonical-admission boundary. The request supplies explicit logical
/// data filenames/column order, never guessed from relation spelling.
pub fn try_measure<T>(
    input: &PlannerInput,
    baseline: &ev::DatasetEvidence,
    request: &DatasetRequest,
    run_id: &str,
    run: impl FnOnce() -> Result<T>,
) -> Result<CalibrationResult<T>> {
    input.check()?;
    baseline.check().map_err(evidence_error)?;
    request.check().map_err(|e| invalid(e.to_string()))?;
    require(!run_id.trim().is_empty(), "calibration needs an explicit run identity")?;
    require(input.configuration.target != Target::Gpu, "calibration uses host operator counters; a GPU device-counter producer is not enabled")?;
    let CheckedIr::Datalog { round: text } = &input.checked_ir else {
        return Err(invalid("round calibration requires a recorded Datalog program"));
    };
    let program: round::Program = syn::parse_str(text)?;
    require(program.schedule.progress != "relational", "schedule.progress=relational has no synchronous SCC round count; per-driver invocation counts require a different recorded cost model")?;
    crate::cost::bounds::validate_context(input, baseline, target_name(input.configuration.target))?;
    let before = crate::cost::analyze(input, baseline, target_name(input.configuration.target))?;
    let recorder = Arc::new(Recorder(std::sync::Mutex::new(prepare(&program, baseline, request)?)));
    let (value, operators, failures) = cq_storage::counters::TaskContext::default().run(|| {
        cq_storage::counters::calibration::with_observer(recorder.clone(), || {
            let value = run();
            (value, cq_storage::counters::operators(), cq_storage::counters::calibration::observer_failures())
        })
    });
    let value = value?;
    require(failures.is_empty(), format!("calibration metadata failures: {}", failures.join("; ")))?;
    require(!operators.iter().any(|operator| operator.calibration.is_some() && operator.row_overflow), "physical operator row counters overflowed")?;
    let recorder = Arc::try_unwrap(recorder).map_err(|_| invalid("calibration tasks remain active after query completion"))?;
    let captured = recorder.0.into_inner().map_err(|_| invalid("calibration observer was poisoned"))?;
    if let Some(error) = &captured.failure { return Err(invalid(error.clone())); }
    require(captured.completed, "incomplete execution cannot publish calibrated facts")?;
    require(captured.metadata.len() == captured.expected_metadata.len(), "not all raw metadata inputs were observed")?;
    let run_identity = ev::RunIdentity {
        program_hash: input.identity.program_hash.clone(),
        configuration_hash: input.identity.configuration_hash.clone(),
        admission: ev::Admission::InsertOnlySet, run_id: run_id.into(), completed: true,
    };
    let (mut evidence, attachment) = measured::finish(&program, baseline, request, captured, &run_identity)?;
    let plans = rebind(input, &program, attachment)?;
    require(plans.identity.program_hash == input.identity.program_hash, "evidence rebinding changed the semantic program")?;
    let mut report = report::build(input, baseline, &plans, &mut evidence, &before, &operators, &run_identity)?;
    evidence.check().map_err(evidence_error)?;
    let advice = crate::cost::analyze(&plans, &evidence, target_name(input.configuration.target))?;
    report.finish(&before, &advice, &plans, &evidence)?;
    Ok(CalibrationResult { value, evidence, plans, advice, report })
}

fn target_name(target: Target) -> &'static str {
    match target { Target::Cpu => "cpu", Target::CpuPar => "cpu-par", Target::Gpu => "gpu" }
}
fn erased_round(mut program: round::Program) -> String {
    program.storage.incremental.strata.source.declared_evidence = None;
    for hole in &mut program.holes {
        hole.cq.declared_evidence = None;
        hole.pipeline.calibration = None;
    }
    program.to_token_stream().to_string()
}
fn prepare(program: &round::Program, baseline: &ev::DatasetEvidence, request: &DatasetRequest) -> Result<Captured> {
    let source = &program.storage.incremental.strata.source;
    let attachment = source.declared_evidence.as_ref().ok_or_else(|| invalid("calibration requires explicit source-to-dataset bindings"))?;
    require(attachment.document.planning_facts() == baseline.planning_facts(), "baseline planning facts differ from the source's recorded evidence")?;
    require(request.name == baseline.dataset.name && request.formulation == baseline.dataset.formulation, "statistics request names a different dataset/formulation")?;
    let mut inputs = BTreeMap::new();
    let mut used = BTreeSet::new();
    for (index, declaration) in source.declarations.iter().enumerate().filter(|(_, d)| d.value.is_some()) {
        let binding = attachment.bindings.iter().find(|b| b.relation == declaration.name).ok_or_else(|| invalid(format!("unbound input {}", declaration.name)))?;
        require(binding.view == EvidenceView::Full, "input calibration requires the complete relation view")?;
        let requested = request.relations.iter().find(|r| r.relation.name == binding.source).ok_or_else(|| invalid(format!("missing explicit data request for {}", binding.source)))?;
        used.insert(binding.source.clone());
        let schema = baseline.dataset.manifest.schema.get(&binding.source).ok_or_else(|| invalid("input lacks dataset schema"))?;
        require(requested.relation.columns.iter().map(|c| (c.name.clone(), c.domain.clone())).collect::<BTreeMap<_, _>>() == schema.columns, "request differs from input scalar schema")?;
        let file = baseline.dataset.manifest.files.get(&requested.path).ok_or_else(|| invalid("request data file is absent from manifest"))?;
        require(file.role == ev::FileRole::Data, "input request names a non-data file")?;
        require(ev::canonical_hash(&(&requested.relation.columns, &requested.options)).map_err(evidence_error)? == file.encoding_hash,
            "request differs from the recorded decoder/column-order identity")?;
        require(requested.relation.columns.len() == declaration.columns.len(), "canonical input statistics require a complete column bijection")?;
        let mut mapping = Vec::new();
        for column in &requested.relation.columns {
            let alias = binding.columns.iter().find(|c| c.source == column.name).ok_or_else(|| invalid("request column lacks a source alias"))?;
            mapping.push(declaration.columns.iter().position(|c| c.name == alias.column).ok_or_else(|| invalid("source column alias is absent"))?);
        }
        require(mapping.iter().copied().collect::<BTreeSet<_>>().len() == declaration.columns.len(), "input column mapping is not a bijection")?;
        let mut spec = requested.relation.clone();
        spec.semantics = ev::RelationSemantics::Set;
        inputs.insert(index, InputCapture {
            dataset_relation: binding.source.clone(), path: requested.path.clone(), mapping,
            accumulator: Some(cq_load::stats::CanonicalAccumulator::new(spec).map_err(|e| invalid(e.to_string()))?),
            summary: None, content_hash: None, begun: false,
        });
    }
    require(used.len() == request.relations.len(), "statistics request includes inputs not observed by this program")?;
    let expected_metadata = baseline.dataset.manifest.files.iter().filter(|(_, f)| f.role != ev::FileRole::Data)
        .map(|(path, file)| {
            require(request.metadata.iter().any(|m| &m.path == path && m.role == file.role), "request omits recorded metadata input")?;
            Ok((path.clone(), file.content_hash.clone().ok_or_else(|| invalid("metadata lacks raw-byte identity"))?))
        }).collect::<Result<BTreeMap<_, _>>>()?;
    require(expected_metadata.len() == request.metadata.len(), "request includes metadata outside the recorded dataset manifest")?;
    Ok(Captured {
        inputs, strata: program.context.strata.iter().map(|_| StratumCapture::default()).collect(),
        members: program.context.strata.iter().map(|s| s.members.iter().copied().collect()).collect(),
        expected_round: erased_round(program.clone()), expected_metadata,
        metadata: BTreeSet::new(), begun: false, completed: false, failure: None,
    })
}

/// Replace recorded evidence only. Existing VOs, Storage, admission, layouts and
/// execution records are retained; this is not recompilation or replanning.
fn rebind(input: &PlannerInput, original: &round::Program, attachment: DeclaredEvidence) -> Result<PlannerInput> {
    let erased = round::erase(original)?;
    let mut program = original.clone();
    program.storage.incremental.strata.source.declared_evidence = Some(attachment);
    for (index, hole) in program.holes.iter_mut().enumerate() {
        let mut projected = round::evidence::for_hole(&program.storage, &program.context, index, &erased.holes[index].cq.declarations)?;
        if let Some(execution) = program.execution.iter().find(|entry| entry.hole == index) {
            let scan = program.context.strata.iter().flat_map(|s| s.seeds.iter().chain(&s.steps))
                .find(|rule| rule.hole == index).and_then(|rule| rule.scan.as_ref())
                .ok_or_else(|| invalid("execution certificate has no recorded source scan"))?;
            if let Some(evidence) = &mut projected {
                evidence.rename_relation(&scan.binding, &execution.binding);
            }
        }
        hole.cq.declared_evidence = projected;
    }
    let mut valuations = Vec::with_capacity(input.holes.len());
    for (record, hole) in input.holes.iter().zip(&program.holes) {
        let mut valuation: cq_lang_vo::Program = syn::parse_str(&record.checked_vo)?;
        valuation.cq.declared_evidence = hole.cq.declared_evidence.clone();
        valuations.push(valuation);
    }
    let source = program.storage.incremental.strata.source.to_token_stream().to_string();
    crate::export::export_datalog(&source, &program, &valuations, input.configuration.clone())
}

impl<T> CalibrationResult<T> {
    /// Compiler attachment with the same admitted facts and complete delta
    /// series, but without non-admissible per-operator diagnostic packets.
    pub fn planning_evidence(&self) -> ev::DatasetEvidence { self.evidence.planning_facts() }
}

impl<T> CalibrationResult<T> {
    /// Publish each artifact atomically into a fresh output directory. Existing
    /// files are never replaced. The full evidence includes diagnostics; the
    /// facts projection is the explicit compiler attachment used by plans.json.
    pub fn write_artifacts(&self, directory: &std::path::Path) -> Result<()> {
        std::fs::create_dir_all(directory).map_err(|e| invalid(e.to_string()))?;
        fn json<T: Serialize>(directory: &std::path::Path, name: &str, value: &T) -> Result<()> {
            let bytes = serde_json::to_vec_pretty(value)?;
            crate::artifacts::publish_new(&directory.join(name), &bytes).map_err(|e| invalid(e.to_string()))
        }
        json(directory, "calibration.evidence.json", &self.evidence)?;
        json(directory, "calibration.facts.json", &self.planning_evidence())?;
        json(directory, "calibration.plans.json", &self.plans)?;
        json(directory, "calibration.costs.json", &self.advice)?;
        json(directory, "calibration.report.json", &self.report)?;
        crate::artifacts::publish_new(&directory.join("calibration.orders.dl"), self.advice.advice.as_bytes())
            .map_err(|e| invalid(e.to_string()))?;
        crate::artifacts::publish_new(&directory.join("calibration.report.txt"), self.report.render(&self.evidence).as_bytes())
            .map_err(|e| invalid(e.to_string()))
    }
}

/// Infallible-query counterpart of [`try_measure`].
pub fn measure<T>(
    input: &PlannerInput,
    baseline: &ev::DatasetEvidence,
    request: &DatasetRequest,
    run_id: &str,
    run: impl FnOnce() -> T,
) -> Result<CalibrationResult<T>> {
    try_measure(input, baseline, request, run_id, || Ok(run()))
}
