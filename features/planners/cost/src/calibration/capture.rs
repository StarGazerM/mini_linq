use super::*;
use cq_load::stats::{CanonicalAccumulator, RelationSummary};
use cq_storage::counters::calibration::{CalibrationRow, Observer};
use std::sync::Mutex;

pub(super) struct InputCapture {
    pub dataset_relation: String,
    pub path: String,
    pub mapping: Vec<usize>,
    pub accumulator: Option<CanonicalAccumulator>,
    pub summary: Option<RelationSummary>,
    pub content_hash: Option<String>,
    pub begun: bool,
}
#[derive(Default)]
pub(super) struct StratumCapture {
    pub begun: bool,
    pub seed: Option<BTreeMap<usize, u64>>,
    pub deltas: Vec<BTreeMap<usize, u64>>,
    pub final_rows: Option<BTreeMap<usize, (u64, u64)>>,
}
pub(super) struct Captured {
    pub inputs: BTreeMap<usize, InputCapture>,
    pub strata: Vec<StratumCapture>,
    pub members: Vec<BTreeSet<usize>>,
    pub expected_round: String,
    pub expected_metadata: BTreeMap<String, String>,
    pub metadata: BTreeSet<String>,
    pub begun: bool,
    pub completed: bool,
    pub failure: Option<String>,
}
pub(super) struct Recorder(pub Mutex<Captured>);

impl Recorder {
    fn update(&self, operation: impl FnOnce(&mut Captured) -> Result<()>) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.failure.is_some() { return; }
        if let Err(error) = operation(&mut state) { state.failure = Some(error.to_string()); }
    }
}
fn count(value: usize) -> Result<u64> {
    u64::try_from(value).map_err(|_| invalid("measured cardinality exceeds u64"))
}
fn members(state: &Captured, stratum: usize, ids: impl Iterator<Item = usize>) -> Result<()> {
    let expected = state.members.get(stratum).ok_or_else(|| invalid("unknown measured stratum"))?;
    let ids = ids.collect::<Vec<_>>();
    let actual = ids.iter().copied().collect::<BTreeSet<_>>();
    require(ids.len() == actual.len() && &actual == expected, "round observation does not cover exactly its recorded members")
}
fn input(state: &mut Captured, relation: usize) -> Result<&mut InputCapture> {
    require(state.begun && !state.completed, "input observation outside its program invocation")?;
    state.inputs.get_mut(&relation).ok_or_else(|| invalid("unbound input relation observation"))
}
struct MappedRow<'a> {
    row: &'a dyn CalibrationRow,
    mapping: &'a [usize],
}
impl CalibrationRow for MappedRow<'_> {
    fn len(&self) -> usize { self.mapping.len() }
    fn cell(&self, index: usize) -> cq_storage::counters::calibration::ScalarValue<'_> {
        self.row.cell(self.mapping[index])
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
    fn program_end(&self) {
        self.update(|state| {
            require(state.begun && !state.completed, "unmatched program completion")?;
            require(state.inputs.values().all(|input| input.summary.is_some()), "program completion lacks input observations")?;
            require(state.strata.iter().all(|s| s.final_rows.is_some()), "program completion lacks a complete stratum")?;
            state.completed = true;
            Ok(())
        });
    }
    fn metadata(&self, path: &str, bytes: &[u8]) {
        self.update(|state| {
            let expected = state.expected_metadata.get(path).ok_or_else(|| invalid(format!("unexpected metadata input {path}")))?;
            require(expected == &hash_bytes(bytes), format!("raw metadata identity differs: {path}"))?;
            require(state.metadata.insert(path.into()), format!("metadata input observed more than once: {path}"))
        });
    }
    fn input_begin(&self, relation: usize) {
        self.update(|state| {
            let input = input(state, relation)?;
            require(!input.begun, "input traversal repeated")?;
            input.begun = true;
            Ok(())
        });
    }
    fn input_raw(&self, relation: usize, row: &dyn CalibrationRow) {
        self.update(|state| {
            let input = input(state, relation)?;
            require(input.begun, "input row precedes traversal")?;
            let row = MappedRow { row, mapping: &input.mapping };
            input.accumulator.as_mut().ok_or_else(|| invalid("row after completed input"))?.raw(&row).map_err(|e| invalid(e.to_string()))
        });
    }
    fn input_admitted(&self, relation: usize, row: &dyn CalibrationRow) {
        self.update(|state| {
            let input = input(state, relation)?;
            require(input.begun, "admission precedes input traversal")?;
            let row = MappedRow { row, mapping: &input.mapping };
            input.accumulator.as_mut().ok_or_else(|| invalid("admission after completed input"))?.admitted(&row).map_err(|e| invalid(e.to_string()))
        });
    }
    fn input_end(&self, relation: usize, canonical_rows: usize) {
        self.update(|state| {
            let input = input(state, relation)?;
            require(input.begun, "input completion precedes traversal")?;
            let accumulator = input.accumulator.take().ok_or_else(|| invalid("input traversal completed twice"))?;
            input.content_hash = Some(accumulator.content_hash());
            input.summary = Some(accumulator.finish(canonical_rows).map_err(|e| invalid(e.to_string()))?);
            Ok(())
        });
    }
    fn stratum_begin(&self, stratum: usize) {
        self.update(|state| {
            require(state.begun && !state.completed, "stratum outside program invocation")?;
            let current = state.strata.get_mut(stratum).ok_or_else(|| invalid("unknown stratum"))?;
            require(!current.begun, "stratum executed twice")?;
            current.begun = true;
            Ok(())
        });
    }
    fn seed_end(&self, stratum: usize, rows: &[(usize, usize)]) {
        self.update(|state| {
            members(state, stratum, rows.iter().map(|r| r.0))?;
            let current = &mut state.strata[stratum];
            require(current.begun && current.seed.is_none() && current.deltas.is_empty(), "seed observation outside its single seed phase")?;
            current.seed = Some(rows.iter().map(|&(r, n)| Ok((r, count(n)?))).collect::<Result<_>>()?);
            Ok(())
        });
    }
    fn round_begin(&self, stratum: usize, index: usize, rows: &[(usize, usize)]) {
        self.update(|state| {
            members(state, stratum, rows.iter().map(|r| r.0))?;
            let current = &mut state.strata[stratum];
            require(current.seed.is_some() && current.final_rows.is_none() && index == current.deltas.len(), "noncontiguous or out-of-scope round observation")?;
            require(rows.iter().any(|r| r.1 != 0), "terminal empty round must not be counted")?;
            let delta = rows.iter().map(|&(r, n)| Ok((r, count(n)?))).collect::<Result<BTreeMap<_, _>>>()?;
            if index == 0 { require(current.seed.as_ref() == Some(&delta), "first consumed delta differs from the published seed delta")?; }
            current.deltas.push(delta);
            Ok(())
        });
    }
    fn stratum_end(&self, stratum: usize, rounds: usize, rows: &[(usize, usize, usize)]) {
        self.update(|state| {
            members(state, stratum, rows.iter().map(|r| r.0))?;
            let current = &mut state.strata[stratum];
            require(current.seed.is_some() && current.final_rows.is_none() && rounds == current.deltas.len(), "completed N disagrees with executed rounds")?;
            let values = rows.iter().map(|&(r, full, tail)| Ok((r, (count(full)?, count(tail)?)))).collect::<Result<BTreeMap<_, _>>>()?;
            for (&relation, &(full, tail)) in &values {
                let consumed = current.deltas.iter().try_fold(0_u64, |sum, delta| sum.checked_add(delta[&relation]).ok_or_else(|| invalid("delta sum overflow")))?;
                require(consumed.checked_add(tail) == Some(full), "consumed deltas plus final unconsumed delta do not partition the final insert-only set")?;
                require(current.seed.as_ref().unwrap()[&relation] <= full, "seed cardinality exceeds final relation")?;
            }
            current.final_rows = Some(values);
            Ok(())
        });
    }
}
