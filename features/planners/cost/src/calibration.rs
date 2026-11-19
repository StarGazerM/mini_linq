//! PS-After: the statistics of a completed run. A calibration run evaluates
//! as a plain run; when its program has its result, the program offers each
//! relation once and the requested ones are passed over into the runtime's
//! degree accumulator. The offline advisor is invoked only after the query
//! returns; no advice is adopted at run time.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use cq_evidence as ev;
use cq_lang_datalog_round as round;
use cq_storage::statistics::collection::{Age, CalibrationRow, Observer, RelationRows, ScalarValue};
use cq_syntax::declared_evidence::EvidenceView;
use quote::ToTokens;
use crate::cost::AdviceReport;
use crate::export::{CheckedIr, PlannerInput, Target};

pub type Result<T, E = crate::export::ExportError> = std::result::Result<T, E>;
fn invalid(message: impl Into<String>) -> crate::export::ExportError { crate::export::ExportError(message.into()) }
fn require(condition: bool, message: impl Into<String>) -> Result<()> { if condition { Ok(()) } else { Err(invalid(message)) } }
fn evidence_error(error: ev::EvidenceError) -> crate::export::ExportError { invalid(error.to_string()) }

pub struct CalibrationResult<T> {
    pub value: T,
    /// The supplied evidence with the degree facts of the run's relations.
    pub evidence: ev::DatasetEvidence,
    pub advice: AdviceReport,
}

fn target_name(target: Target) -> &'static str {
    match target { Target::Cpu => "cpu", Target::CpuPar => "cpu-par", Target::Gpu => "gpu" }
}
/// The program without its evidence attachments: what a run must equal.
fn erased_round(mut program: round::Program) -> String {
    program.storage.incremental.strata.source.declared_evidence = None;
    for hole in &mut program.holes { hole.cq.declared_evidence = None; }
    program.to_token_stream().to_string()
}

struct Captured {
    expected_round: String,
    begun: bool,
    completed: bool,
    failure: Option<String>,
    requests: Vec<ev::DegreeRequest>,
    degrees: cq_storage::statistics::Degrees,
    /// Per relation, the rows the program holds, where it reported or passed them.
    rows: Vec<Option<u64>>,
    /// Per relation, whether a request compares the rounds of its rows.
    rounds: Vec<bool>,
    cells: Vec<i64>,
    /// A code per text value: degrees compare values for equality only.
    text: HashMap<String, i64>,
}
impl Captured {
    fn new(program: &round::Program, requests: Vec<ev::DegreeRequest>) -> Result<Self> {
        let declarations = &program.storage.incremental.strata.source.declarations;
        let names: Vec<String> = declarations.iter().map(|declaration| declaration.name.to_string()).collect();
        let relations: Vec<(&str, usize)> = names.iter().map(String::as_str).zip(declarations.iter().map(|declaration| declaration.columns.len())).collect();
        let degrees = cq_load::stats::admitted_degrees(&requests, &relations).map_err(|e| invalid(e.to_string()))?;
        let mut rounds = vec![false; relations.len()];
        let mut compared = |name: &str| if let Some(relation) = names.iter().position(|known| known == name) { rounds[relation] = true; };
        for request in &requests {
            match request {
                ev::DegreeRequest::Joined { staged: true, reach, binder, read, .. } => {
                    compared(&read.relation);
                    compared(&binder.relation);
                    if let Some(reach) = reach { compared(&reach.changing.relation); }
                }
                ev::DegreeRequest::Visible { changing, relation } => { compared(changing); compared(relation); }
                ev::DegreeRequest::Rounds { relation } => compared(relation),
                _ => {}
            }
        }
        Ok(Self {
            expected_round: erased_round(program.clone()), begun: false, completed: false, failure: None,
            requests, degrees, rows: vec![None; relations.len()], rounds, cells: Vec::new(), text: HashMap::new(),
        })
    }
    /// The cells of a row as the degrees take them.
    fn cells(&mut self, row: &dyn CalibrationRow) -> Result<()> {
        self.cells.clear();
        for index in 0..row.len() {
            self.cells.push(match row.cell(index) {
                ScalarValue::Signed(value) | ScalarValue::Date(value) | ScalarValue::Decimal { unscaled: value, .. } => value,
                // The bits identify the value.
                ScalarValue::Unsigned(value) | ScalarValue::Symbol(value) => value as i64,
                ScalarValue::Utf8(text) => match self.text.get(text) {
                    Some(code) => *code,
                    None => { let code = self.text.len() as i64; self.text.insert(text.to_owned(), code); code }
                },
                ScalarValue::Unsupported(reason) => return Err(invalid(format!("degree statistics over an unsupported column: {reason}"))),
            });
        }
        Ok(())
    }
    /// PS-Heavy: a row of a relation's first pass.
    fn observe(&mut self, relation: usize, row: &dyn CalibrationRow) -> Result<()> {
        self.cells(row)?;
        self.degrees.observe(relation, &self.cells);
        Ok(())
    }
    fn admit(&mut self, relation: usize, age: Age, row: &dyn CalibrationRow) -> Result<()> {
        let stamp = |at: usize| u32::try_from(at).map_err(|_| invalid("a program published more than 2^32 times"));
        let (published, horizon) = (stamp(age.published)?, stamp(age.horizon)?);
        self.cells(row)?;
        self.degrees.admit(relation, published, horizon, &self.cells);
        Ok(())
    }
    /// The supplied evidence with this run's degree facts. The run is bound
    /// to its inputs by what is free: every input relation whose rows are
    /// known holds the rows its evidence states.
    fn finish(self, program: &round::Program, baseline: &ev::DatasetEvidence, run: &ev::RunIdentity) -> Result<ev::DatasetEvidence> {
        let source = &program.storage.incremental.strata.source;
        let attachment = source.declared_evidence.as_ref().ok_or_else(|| invalid("calibration requires explicit source-to-dataset bindings"))?;
        for (index, declaration) in source.declarations.iter().enumerate().filter(|(_, declaration)| declaration.value.is_some()) {
            let Some(binding) = attachment.bindings.iter().find(|binding| binding.relation == declaration.name && binding.view == EvidenceView::Full) else { continue };
            let stated = baseline.relations.get(&binding.source).and_then(|relation| relation.rows.as_ref())
                .filter(|fact| fact.claim == ev::Claim::Equal).and_then(|fact| fact.value);
            if let (Some(held), Some(stated)) = (self.rows[index], stated) {
                require(held == stated, format!("input {} holds {held} rows; its evidence states {stated}", declaration.name))?;
            }
        }
        let provenance = ev::Provenance {
            producer: "cq-cost planner calibration".into(),
            method: "degree statistics of the relations a completed run holds: distinct counts by sketch, matched degrees over a sample of keys chosen by hash".into(),
            locator: "relations".into(), run: Some(run.clone()), artifact_hash: None,
        };
        let values = self.requests.into_iter().zip(self.degrees.finish()).map(|(request, (numerator, denominator))| (request, numerator, denominator)).collect();
        let mut evidence = baseline.clone();
        evidence.degrees = cq_load::stats::bind_degrees(values, &baseline.dataset.hash, ev::Source::Calibrated, ev::Claim::Observation, provenance);
        evidence.check().map_err(evidence_error)?;
        Ok(evidence)
    }
}

struct Recorder(Mutex<Captured>);
impl Recorder {
    fn update(&self, operation: impl FnOnce(&mut Captured) -> Result<()>) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.failure.is_some() { return; }
        if let Err(error) = operation(&mut state) { state.failure = Some(error.to_string()); }
    }
}
impl Observer for Recorder {
    fn program_begin(&self, descriptor: &'static str) {
        self.update(|state| {
            require(!state.begun, "calibration requires exactly one complete program invocation")?;
            let program: round::Program = syn::parse_str(descriptor)?;
            round::contract::check(&program)?;
            require(erased_round(program) == state.expected_round, "executed Round differs from the supplied checked plans")?;
            state.begun = true;
            Ok(())
        });
    }
    fn needs_rounds(&self, relation: usize) -> bool {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).rounds.get(relation).copied().unwrap_or(false)
    }
    fn relation(&self, relation: usize, rows: Option<usize>, pass: RelationRows<'_>) {
        self.update(|state| {
            require(state.begun && !state.completed, "a relation outside its program invocation")?;
            require(relation < state.rows.len() && state.rows[relation].is_none(), "a relation the program does not declare, or offered twice")?;
            let mut held = rows.map(|rows| rows as u64);
            if state.degrees.wants(relation) {
                let (mut passed, mut failure) = (0u64, None);
                // PS-Heavy: the relation's heavy keys first, then its rows.
                if state.degrees.observes(relation) {
                    pass(&mut |_, row| if failure.is_none() { failure = state.observe(relation, row).err(); });
                    state.degrees.observed(relation);
                }
                pass(&mut |age, row| if failure.is_none() { passed += 1; failure = state.admit(relation, age, row).err(); });
                if let Some(failure) = failure { return Err(failure); }
                require(held.is_none_or(|held| held == passed), "a relation passed other rows than it holds")?;
                held = Some(passed);
            }
            state.rows[relation] = held;
            Ok(())
        });
    }
    fn program_end(&self) {
        self.update(|state| {
            require(state.begun && !state.completed, "unmatched program completion")?;
            state.completed = true;
            Ok(())
        });
    }
}

/// Run `run` once under a statistics observer, bind the degree facts of the
/// relations it leaves to `baseline`, and advise from them.
pub fn try_measure<T>(
    input: &PlannerInput,
    baseline: &ev::DatasetEvidence,
    run_id: &str,
    run: impl FnOnce() -> Result<T>,
) -> Result<CalibrationResult<T>> {
    input.check()?;
    baseline.check().map_err(evidence_error)?;
    require(!run_id.trim().is_empty(), "calibration needs an explicit run identity")?;
    require(input.configuration.target != Target::Gpu, "calibration passes over host relations; a device collector is not built")?;
    let CheckedIr::Datalog { round: text } = &input.checked_ir else {
        return Err(invalid("calibration requires a recorded Datalog program"));
    };
    let program: round::Program = syn::parse_str(text)?;
    let target = target_name(input.configuration.target);
    crate::cost::bounds::validate_context(input, baseline, target)?;
    // PS-Derive: what the variable planner can ask for, at the baseline's constants.
    let requests = crate::cost::variables::Planner::new(input, baseline)?.requests()?;
    let recorder = Arc::new(Recorder(Mutex::new(Captured::new(&program, requests)?)));
    let value = cq_storage::statistics::collection::with_observer(recorder.clone(), run)?;
    let recorder = Arc::try_unwrap(recorder).map_err(|_| invalid("the calibration observer is still held after the query"))?;
    let captured = recorder.0.into_inner().map_err(|_| invalid("calibration observer was poisoned"))?;
    if let Some(error) = &captured.failure { return Err(invalid(error.clone())); }
    require(captured.completed, "incomplete execution cannot publish calibrated facts")?;
    let run_identity = ev::RunIdentity {
        program_hash: input.identity.program_hash.clone(),
        configuration_hash: input.identity.configuration_hash.clone(),
        admission: ev::Admission::InsertOnlySet, run_id: run_id.into(), completed: true,
    };
    let evidence = captured.finish(&program, baseline, &run_identity)?;
    let advice = crate::cost::analyze(input, &evidence, target)?;
    Ok(CalibrationResult { value, evidence, advice })
}

/// Infallible-query counterpart of [`try_measure`].
pub fn measure<T>(input: &PlannerInput, baseline: &ev::DatasetEvidence, run_id: &str, run: impl FnOnce() -> T) -> Result<CalibrationResult<T>> {
    try_measure(input, baseline, run_id, || Ok(run()))
}

impl<T> CalibrationResult<T> {
    /// Publish each artifact atomically into a fresh output directory;
    /// existing files are never replaced. The facts are the supplied
    /// evidence with the run's degree facts; the costs and the orders are
    /// the advice from them.
    pub fn write_artifacts(&self, directory: &std::path::Path) -> Result<()> {
        std::fs::create_dir_all(directory).map_err(|e| invalid(e.to_string()))?;
        let publish = |name: &str, bytes: &[u8]| crate::artifacts::publish_new(&directory.join(name), bytes).map_err(|e| invalid(e.to_string()));
        publish("calibration.facts.json", &serde_json::to_vec_pretty(&self.evidence)?)?;
        publish("calibration.costs.json", &serde_json::to_vec_pretty(&self.advice)?)?;
        publish("calibration.orders.dl", self.advice.advice.as_bytes())
    }
}
