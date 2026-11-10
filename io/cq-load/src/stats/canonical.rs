//! Statistics piggybacking on the checked DL-InsertNew admission boundary.
//!
//! Raw rows feed the canonical content digest. Only the compiler's successful
//! full-tuple admission callback increments frequencies: no second tuple set or
//! extra traversal is necessary. Finish checks the actual canonical cardinality.
use super::*;
use cq_storage::counters::calibration::{CalibrationCell, CalibrationRow, ScalarValue};

pub struct CanonicalAccumulator {
    accumulator: RelationAccumulator,
    row: Vec<usize>,
    key: Vec<usize>,
    pending: bool,
}

impl CanonicalAccumulator {
    pub fn new(spec: RelationSpec) -> Result<Self> {
        if spec.semantics != RelationSemantics::Set {
            return Err(error("canonical admission statistics require set semantics"));
        }
        let width = spec.columns.len();
        let mut accumulator = RelationAccumulator::new(spec, DistinctMode::Exact)?;
        // Uniqueness is supplied by the existing canonical insertion, not by an
        // independently maintained full-tuple set in the observer.
        accumulator.tuples = None;
        Ok(Self { accumulator, row: vec![0; width], key: Vec::with_capacity(width), pending: false })
    }

    /// Observe the original decoded row BEFORE canonical insertion. Duplicates
    /// and input order remain part of typed-row-stream-v1 identity.
    pub fn raw(&mut self, row: &dyn CalibrationRow) -> Result<()> {
        self.observe(row, false)
    }

    /// Invoke exactly once inside DL-InsertNew's successful admission callback,
    /// with the same borrowed declaration-order row passed to `raw`.
    pub fn admitted(&mut self, row: &dyn CalibrationRow) -> Result<()> {
        self.observe(row, true)
    }

    fn observe(&mut self, row: &dyn CalibrationRow, admitted: bool) -> Result<()> {
        if self.accumulator.failed { return Err(error("canonical statistics have a failed observation")); }
        let result = self.observe_checked(row, admitted);
        if result.is_err() { self.accumulator.failed = true; }
        result
    }

    fn observe_checked(&mut self, row: &dyn CalibrationRow, admitted: bool) -> Result<()> {
        if row.len() != self.row.len() { return Err(error("canonical row width differs from evidence schema")); }
        if admitted && !self.pending { return Err(error("admission lacks its raw row or repeats an admission")); }
        if !admitted {
            increment(&mut self.accumulator.source_rows)?;
            self.accumulator.digest.update([0x52]);
        }
        for (index, (state, spec)) in self.accumulator.columns.iter_mut().zip(&self.accumulator.spec.columns).enumerate() {
            let value = decoded(row.cell(index), &spec.domain)?;
            if admitted {
                let same = match (&state.values[self.row[index]], value) {
                    (Value::Signed(a), Decoded::Signed(b)) => *a == b,
                    (Value::Symbol(a), Decoded::Symbol(b)) => *a == b,
                    (Value::Text(a), Decoded::Text(b)) => a.as_ref() == b,
                    _ => false,
                };
                if !same { return Err(error("admitted row differs from its raw observation")); }
            } else {
                let id = match value {
                    Decoded::Signed(value) => state.number(value),
                    Decoded::Symbol(value) => state.symbol(value),
                    Decoded::Text(value) => state.text(value),
                };
                self.row[index] = id;
                state.values[id].content_hash_into(&mut self.accumulator.digest, &spec.domain);
            }
        }
        if admitted {
            self.accumulator.count_row(&self.row, &mut self.key)?;
            self.pending = false;
        } else {
            self.pending = true;
        }
        Ok(())
    }

    pub fn content_hash(&self) -> String { self.accumulator.content_hash() }
    pub fn source_rows(&self) -> u64 { self.accumulator.source_rows() }

    pub fn finish(self, canonical_rows: usize) -> Result<RelationSummary> {
        let canonical_rows = u64::try_from(canonical_rows).map_err(|_| error("canonical row count exceeds u64"))?;
        if self.accumulator.rows != canonical_rows || self.accumulator.source_rows < canonical_rows {
            return Err(error("admission observations disagree with the actual canonical cardinality"));
        }
        self.accumulator.summarize()
    }
}

enum Decoded<'a> { Signed(i64), Symbol(u64), Text(&'a str) }

fn decoded<'a>(value: ScalarValue<'a>, domain: &ScalarDomain) -> Result<Decoded<'a>> {
    match (value, domain) {
        (ScalarValue::Signed(value), ScalarDomain::SignedInteger) => Ok(Decoded::Signed(value)),
        (ScalarValue::Unsigned(value), ScalarDomain::SignedInteger) => {
            Ok(Decoded::Signed(i64::try_from(value).map_err(|_| error("unsigned value exceeds signed evidence domain"))?))
        }
        (ScalarValue::Decimal { unscaled, scale }, ScalarDomain::Decimal { scale: expected }) if scale == *expected => Ok(Decoded::Signed(unscaled)),
        (ScalarValue::Date(value), ScalarDomain::Date { encoding }) if encoding == DATE_ENCODING => Ok(Decoded::Signed(value)),
        (ScalarValue::Utf8(value), ScalarDomain::Utf8) => Ok(Decoded::Text(value)),
        (ScalarValue::Symbol(value), ScalarDomain::Symbol { .. }) => Ok(Decoded::Symbol(value)),
        (ScalarValue::Unsupported(reason), _) => Err(error(reason)),
        _ => Err(error("decoded scalar does not match its declared evidence domain/encoding")),
    }
}

impl CalibrationCell for Date {
    fn calibration_value(&self) -> ScalarValue<'_> { ScalarValue::Date(i64::from(self.days_since_epoch())) }
}
impl<const SCALE: u32> CalibrationCell for Decimal<SCALE> {
    fn calibration_value(&self) -> ScalarValue<'_> { ScalarValue::Decimal { unscaled: self.scaled(), scale: SCALE } }
}
impl CalibrationCell for crate::StringCode {
    fn calibration_value(&self) -> ScalarValue<'_> { ScalarValue::Symbol(u64::from(self.0)) }
}
