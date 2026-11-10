//! Streaming evidence over the same bounded, typed batches used by relation loaders.
//!
//! Exact frequencies retain distinct values, not a second row dataset. Set input
//! additionally retains distinct full tuples of column-local IDs: deduplicating a
//! key projection would incorrectly reduce degrees. The content digest observes
//! loaded rows before that deduplication, including source order and duplicates.
//! Unsupported unordered domains are rejected rather than assigned text equality.
//! Nulls follow `Load` (rejected); empty UTF-8 strings remain ordinary values.
//! Signed integers use `i64::load`, decimals use `Decimal<0..=18>::load`,
//! and dates use `Date::load` with the sole encoding `days_since_unix_epoch`.
//! Symbols load `u64` codes under an explicit dictionary identity; unordered
//! symbol domains permit exact equality counts but deliberately omit extrema.
//! UTF-8 conversion borrows the same source text as `String::load`, allocating
//! owned text only for a previously unseen value. Native Arrow conversions and
//! errors remain those of the existing column loaders.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use cq_evidence::{
    Claim, ColumnEvidence, Fact, KeySetEvidence, Provenance, RelationEvidence,
    RelationScope, RelationSemantics, Scalar, ScalarDomain, Source, Unit,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Batch, Date, Decimal, Error, Result};

#[cfg(feature = "planner-calibration")]
mod canonical;
#[cfg(feature = "planner-calibration")]
pub use canonical::CanonicalAccumulator;

/// The canonical content digest is independent of source formatting/batch size.
pub use cq_evidence::HASH_MODE;
pub const DATE_ENCODING: &str = "days_since_unix_epoch";
const HLL_SEED: &[u8] = b"cq-load-hll-sha256-seed-0-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnSpec {
    pub name: String,
    pub domain: ScalarDomain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationSpec {
    pub name: String,
    pub columns: Vec<ColumnSpec>,
    pub semantics: RelationSemantics,
    pub key_sets: Vec<Vec<String>>,
}

impl RelationSpec {
    /// Validate an authored schema and estimator configuration without allocating
    /// any frequency maps, dictionaries, HLL registers, or tuple state.
    pub fn check(&self, mode: &DistinctMode) -> Result<()> {
        if self.name.trim().is_empty() || self.columns.is_empty() {
            return Err(error("relation name and columns must be nonempty"));
        }
        if let DistinctMode::Hll { precision } = mode {
            if !(4..=18).contains(precision) { return Err(error("HLL precision must be in 4..=18")); }
        }
        let mut names = BTreeSet::new();
        for column in &self.columns {
            if column.name.trim().is_empty() || !names.insert(column.name.as_str()) {
                return Err(error("blank or duplicate column name"));
            }
            match &column.domain {
                ScalarDomain::Decimal { scale } if *scale > 18 => return Err(error("Decimal scale must be at most 18")),
                ScalarDomain::Date { encoding } if encoding != DATE_ENCODING => return Err(error(format!("unsupported date encoding: expected {DATE_ENCODING}"))),
                ScalarDomain::Symbol { dictionary_hash, .. } if !valid_digest(dictionary_hash) => return Err(error("symbol requires a sha256 dictionary identity")),
                ScalarDomain::Unordered { .. } => return Err(error("unordered domain has no supported typed equality decoder")),
                _ => {}
            }
        }
        let mut seen = BTreeSet::new();
        for columns in &self.key_sets {
            let unique: BTreeSet<_> = columns.iter().collect();
            if columns.is_empty() || unique.len() != columns.len() || !seen.insert(unique) {
                return Err(error("empty, repeated, or duplicate composite key set"));
            }
            for name in columns {
                if !names.contains(name.as_str()) {
                    return Err(error(format!("unknown key column: {name}")));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DistinctMode {
    Exact,
    Hll { precision: u8 },
}

fn error(message: impl Into<String>) -> Error {
    Error::new(Path::new("<statistics>"), 0, "<schema>", "", message)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71 && value.starts_with("sha256:")
        && value.as_bytes()[7..].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
}

fn increment(value: &mut u64) -> Result<()> {
    *value = value.checked_add(1).ok_or_else(|| error("statistics count overflow"))?;
    Ok(())
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Value {
    Signed(i64),
    Symbol(u64),
    Text(Arc<str>),
}

impl Value {
    fn hash_into(&self, hash: &mut Sha256) {
        match self {
            Self::Signed(value) => frame(hash, &value.to_le_bytes()),
            Self::Symbol(value) => frame(hash, &value.to_le_bytes()),
            Self::Text(value) => frame(hash, value.as_bytes()),
        }
    }

    fn content_hash_into(&self, hash: &mut Sha256, domain: &ScalarDomain) {
        match (self, domain) {
            (Self::Signed(value), ScalarDomain::SignedInteger) => {
                hash.update([0x01]);
                hash.update(value.to_be_bytes());
            }
            (Self::Signed(value), ScalarDomain::Decimal { scale }) => {
                hash.update([0x02]);
                hash.update(scale.to_be_bytes());
                hash.update(value.to_be_bytes());
            }
            (Self::Signed(value), ScalarDomain::Date { .. }) => {
                hash.update([0x03]);
                hash.update(value.to_be_bytes());
            }
            (Self::Text(value), ScalarDomain::Utf8) => {
                hash.update([0x04]);
                hash.update((value.len() as u64).to_be_bytes());
                hash.update(value.as_bytes());
            }
            (Self::Symbol(value), ScalarDomain::Symbol { .. }) => {
                hash.update([0x05]);
                hash.update(value.to_be_bytes());
            }
            _ => unreachable!("validated domain and its decoder disagree"),
        }
    }

    fn scalar(&self, domain: &ScalarDomain) -> Scalar {
        match (self, domain) {
            (Self::Signed(value), ScalarDomain::SignedInteger) => Scalar::SignedInteger { value: *value },
            (Self::Signed(value), ScalarDomain::Decimal { scale }) => Scalar::Decimal {
                unscaled: value.to_string(), scale: *scale,
            },
            (Self::Signed(value), ScalarDomain::Date { encoding }) => Scalar::Date {
                value: *value, encoding: encoding.clone(),
            },
            (Self::Symbol(code), ScalarDomain::Symbol { dictionary_hash, .. }) => Scalar::Symbol {
                code: *code, dictionary_hash: dictionary_hash.clone(),
            },
            (Self::Text(value), ScalarDomain::Utf8) => Scalar::Utf8 { value: value.to_string() },
            _ => unreachable!("validated domain and its decoder disagree"),
        }
    }
}

struct ColumnState {
    // Numeric keys use their lossless bits; one column never mixes numeric types.
    numbers: HashMap<u64, usize>,
    strings: HashMap<Arc<str>, usize>,
    values: Vec<Value>,
    frequencies: Vec<u64>,
    cell_prefix: Vec<u8>,
    hashes: Vec<[u8; 32]>,
    hll: Option<Hll>,
}

impl ColumnState {
    fn new(spec: &ColumnSpec, mode: DistinctMode) -> Result<Self> {
        let domain = serde_json::to_vec(&spec.domain).map_err(|e| error(e.to_string()))?;
        let mut cell_prefix = Vec::new();
        for part in [spec.name.as_bytes(), domain.as_slice()] {
            cell_prefix.extend_from_slice(&(part.len() as u64).to_le_bytes());
            cell_prefix.extend_from_slice(part);
        }
        Ok(Self {
            numbers: HashMap::new(), strings: HashMap::new(), values: Vec::new(),
            frequencies: Vec::new(), cell_prefix, hashes: Vec::new(), hll: Hll::new(mode),
        })
    }

    fn push(&mut self, value: Value) -> usize {
        let id = self.values.len();
        if self.hll.is_some() {
            let mut hash = Sha256::new();
            hash.update(HLL_SEED);
            hash.update(&self.cell_prefix);
            value.hash_into(&mut hash);
            self.hashes.push(hash.finalize().into());
        }
        self.values.push(value);
        self.frequencies.push(0);
        id
    }

    fn number(&mut self, value: i64) -> usize {
        if let Some(&id) = self.numbers.get(&(value as u64)) { return id; }
        let id = self.push(Value::Signed(value));
        self.numbers.insert(value as u64, id);
        id
    }

    fn symbol(&mut self, value: u64) -> usize {
        if let Some(&id) = self.numbers.get(&value) { return id; }
        let id = self.push(Value::Symbol(value));
        self.numbers.insert(value, id);
        id
    }

    fn text(&mut self, value: &str) -> usize {
        if let Some(&id) = self.strings.get(value) { return id; }
        let owned: Arc<str> = Arc::from(value);
        let id = self.push(Value::Text(Arc::clone(&owned)));
        self.strings.insert(owned, id);
        id
    }

    fn decode(&mut self, batch: &Batch<'_>, index: usize, domain: &ScalarDomain) -> Result<Vec<usize>> {
        match domain {
            ScalarDomain::SignedInteger => Ok(batch.column::<i64>(index)?.into_iter().map(|v| self.number(v)).collect()),
            ScalarDomain::Date { .. } => Ok(batch.column::<Date>(index)?.into_iter().map(|v| self.number(i64::from(v.days_since_epoch()))).collect()),
            ScalarDomain::Symbol { .. } => Ok(batch.column::<u64>(index)?.into_iter().map(|v| self.symbol(v)).collect()),
            ScalarDomain::Utf8 => batch.strings(index, |v| self.text(v)),
            ScalarDomain::Decimal { scale } => {
                // Dispatch to the existing const-generic Load implementation,
                // including its native Arrow rescaling and precision checks.
                macro_rules! scales {
                    ($($scale:literal),*) => { match scale {
                        $($scale => self.decimals::<$scale>(batch, index),)*
                        _ => Err(error("Decimal scale must be at most 18")),
                    } };
                }
                scales!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18)
            }
            ScalarDomain::Unordered { .. } => Err(error("unordered domain has no supported typed equality decoder")),
        }
    }

    fn decimals<const SCALE: u32>(&mut self, batch: &Batch<'_>, index: usize) -> Result<Vec<usize>> {
        Ok(batch.column::<Decimal<SCALE>>(index)?.into_iter().map(|v| self.number(v.scaled())).collect())
    }
}

struct KeyState {
    indices: Vec<usize>,
    frequencies: HashMap<Vec<usize>, u64>,
    hll: Option<Hll>,
}

/// HLL is deliberately observational. Frequencies are still exact in this mode.
struct Hll {
    precision: u8,
    registers: Vec<u8>,
}

impl Hll {
    fn new(mode: DistinctMode) -> Option<Self> {
        match mode {
            DistinctMode::Exact => None,
            DistinctMode::Hll { precision } => Some(Self {
                precision, registers: vec![0; 1_usize << precision],
            }),
        }
    }

    fn observe(&mut self, digest: &[u8; 32]) {
        let hash = u64::from_le_bytes(digest[..8].try_into().expect("eight digest bytes"));
        let index = (hash >> (64 - self.precision)) as usize;
        let rank = ((hash << self.precision).leading_zeros() + 1).min(65 - u32::from(self.precision)) as u8;
        self.registers[index] = self.registers[index].max(rank);
    }

    fn estimate(&self) -> Result<u64> {
        let m = self.registers.len() as f64;
        let alpha = match self.registers.len() {
            16 => 0.673, 32 => 0.697, 64 => 0.709, _ => 0.7213 / (1.0 + 1.079 / m),
        };
        let raw = alpha * m * m / self.registers.iter().map(|&r| 2_f64.powi(-i32::from(r))).sum::<f64>();
        let zeros = self.registers.iter().filter(|&&r| r == 0).count();
        let estimate = if raw <= 2.5 * m && zeros != 0 { m * (m / zeros as f64).ln() } else { raw };
        // Do not silently saturate an out-of-range floating-point conversion.
        if !estimate.is_finite() || estimate.round() >= u64::MAX as f64 {
            return Err(error("HLL estimate exceeds u64 range"));
        }
        Ok(estimate.round() as u64)
    }
}

/// Compact, unbound statistics. No source values, frequency maps, HLL registers,
/// or full-tuple deduplication sets survive summarization. Bind only after the
/// complete dataset manifest has its final identity.
pub struct RelationSummary {
    spec: RelationSpec,
    rows: u64,
    columns: Vec<ColumnSummary>,
    keys: Vec<FrequencySummary>,
}

struct ColumnSummary {
    frequencies: FrequencySummary,
    min: Option<Scalar>,
    max: Option<Scalar>,
}

struct FrequencySummary {
    distinct: u64,
    max_degree: u64,
    hll: Option<HllEstimate>,
}

struct HllEstimate {
    precision: u8,
    value: u64,
}

impl FrequencySummary {
    fn new(distinct: usize, max_degree: u64, hll: Option<Hll>) -> Result<Self> {
        Ok(Self {
            distinct: u64::try_from(distinct).map_err(|_| error("distinct count exceeds u64 range"))?,
            max_degree,
            hll: match hll {
                Some(hll) => Some(HllEstimate { precision: hll.precision, value: hll.estimate()? }),
                None => None,
            },
        })
    }
}

pub struct RelationAccumulator {
    spec: RelationSpec,
    columns: Vec<ColumnState>,
    keys: Vec<KeyState>,
    tuples: Option<HashSet<Vec<usize>>>,
    digest: Sha256,
    source_rows: u64,
    rows: u64,
    failed: bool,
}

impl RelationAccumulator {
    pub fn new(spec: RelationSpec, mode: DistinctMode) -> Result<Self> {
        spec.check(&mode)?;
        let keys = spec.key_sets.iter().map(|columns| {
            let indices = columns.iter().map(|name| {
                spec.columns.iter().position(|column| &column.name == name)
                    .expect("validated key column")
            }).collect();
            KeyState { indices, frequencies: HashMap::new(), hll: Hll::new(mode) }
        }).collect();
        let columns = spec.columns.iter().map(|c| ColumnState::new(c, mode)).collect::<Result<Vec<_>>>()?;
        let mut digest = Sha256::new();
        digest.update(b"mini-linq:typed-row-stream-v1\0");
        digest.update((columns.len() as u64).to_be_bytes());
        let tuples = (spec.semantics == RelationSemantics::Set).then(HashSet::new);
        Ok(Self { spec, columns, keys, tuples, digest, source_rows: 0, rows: 0, failed: false })
    }

    /// Failed observations poison the accumulator; partial data can never finish
    /// as exact evidence. A fresh accumulator is required after any error.
    pub fn observe_batch(&mut self, batch: &Batch<'_>) -> Result<()> {
        if self.failed { return Err(error("statistics accumulator has a failed observation")); }
        let result = self.observe(batch);
        if result.is_err() { self.failed = true; }
        result
    }

    fn observe(&mut self, batch: &Batch<'_>) -> Result<()> {
        if batch.columns.len() != self.columns.len() || batch.mapping.len() != self.columns.len()
            || batch.columns.iter().zip(&self.spec.columns).any(|(name, column)| *name != column.name)
        {
            return Err(error("batch schema does not match statistics relation columns"));
        }
        let decoded = self.columns.iter_mut().zip(&self.spec.columns).enumerate()
            .map(|(index, (state, spec))| state.decode(batch, index, &spec.domain))
            .collect::<Result<Vec<_>>>()?;
        if decoded.iter().any(|column| column.len() != batch.len()) {
            return Err(error("decoded column length does not match batch rows"));
        }
        // Scratch tuple/key buffers are reused. Maps copy only previously unseen
        // tuples; repeated values and repeated composite keys allocate nothing.
        let mut tuple = vec![0; self.columns.len()];
        let mut key = Vec::with_capacity(self.columns.len());
        for row in 0..batch.len() {
            increment(&mut self.source_rows)?;
            self.digest.update([0x52]);
            for (index, state) in self.columns.iter_mut().enumerate() {
                let id = decoded[index][row];
                tuple[index] = id;
                state.values[id].content_hash_into(&mut self.digest, &self.spec.columns[index].domain);
                if let Some(hll) = &mut state.hll { hll.observe(&state.hashes[id]); }
            }
            for state in &mut self.keys {
                if let Some(hll) = &mut state.hll {
                    let mut hash = Sha256::new();
                    hash.update(HLL_SEED);
                    hash.update((state.indices.len() as u64).to_le_bytes());
                    for &index in &state.indices { hash.update(self.columns[index].hashes[tuple[index]]); }
                    hll.observe(&hash.finalize().into());
                }
            }
            if let Some(tuples) = &mut self.tuples {
                if tuples.contains(tuple.as_slice()) { continue; }
                tuples.insert(tuple.clone());
            }
            self.count_row(&tuple, &mut key)?;
        }
        Ok(())
    }

    fn count_row(&mut self, tuple: &[usize], key: &mut Vec<usize>) -> Result<()> {
        increment(&mut self.rows)?;
        for (state, &id) in self.columns.iter_mut().zip(tuple) {
            increment(&mut state.frequencies[id])?;
        }
        for state in &mut self.keys {
            key.clear();
            key.extend(state.indices.iter().map(|&index| tuple[index]));
            if let Some(count) = state.frequencies.get_mut(key.as_slice()) {
                increment(count)?;
            } else {
                state.frequencies.insert(key.clone(), 1);
            }
        }
        Ok(())
    }

    pub fn content_hash(&self) -> String {
        let mut digest = self.digest.clone();
        digest.update([0x45]);
        digest.update(self.source_rows.to_be_bytes());
        format!("sha256:{:x}", digest.finalize())
    }

    pub fn source_rows(&self) -> u64 { self.source_rows }

    /// Release all distinct-value and full-tuple state before the dataset's
    /// aggregate manifest hash is available.
    pub fn summarize(self) -> Result<RelationSummary> {
        if self.failed { return Err(error("cannot summarize statistics after a failed observation")); }
        let mut columns = Vec::with_capacity(self.columns.len());
        for (spec, state) in self.spec.columns.iter().zip(self.columns) {
            let ordered = !matches!(spec.domain, ScalarDomain::Symbol { ordered: false, .. });
            let extrema = |maximum: bool| {
                if !ordered { return None; }
                let value = if maximum { state.values.iter().max() } else { state.values.iter().min() }?;
                Some(value.scalar(&spec.domain))
            };
            let min = extrema(false);
            let max = extrema(true);
            columns.push(ColumnSummary {
                frequencies: FrequencySummary::new(
                    state.values.len(), state.frequencies.iter().copied().max().unwrap_or(0), state.hll,
                )?,
                min, max,
            });
        }
        let keys = self.keys.into_iter().map(|state| {
            FrequencySummary::new(
                state.frequencies.len(), state.frequencies.values().copied().max().unwrap_or(0), state.hll,
            )
        }).collect::<Result<Vec<_>>>()?;
        Ok(RelationSummary { spec: self.spec, rows: self.rows, columns, keys })
    }

    pub fn finish(self, dataset_hash: &str, source: Source, provenance: Provenance) -> Result<RelationEvidence> {
        self.summarize()?.bind(dataset_hash, source, provenance)
    }
}

impl RelationSummary {
    pub fn bind(self, dataset_hash: &str, source: Source, provenance: Provenance) -> Result<RelationEvidence> {
        validate_origin(dataset_hash, source, &provenance)?;
        let facts = Facts { relation: &self.spec.name, dataset_hash, source, provenance: &provenance };
        let mut columns = BTreeMap::new();
        for (index, (spec, summary)) in self.spec.columns.into_iter().zip(self.columns).enumerate() {
            let prefix = format!("column:{index}");
            columns.insert(spec.name, ColumnEvidence {
                domain: spec.domain,
                distinct: Some(facts.distinct(&prefix, &summary.frequencies)),
                min: summary.min.map(|v| facts.fact(&format!("{prefix}:min"), Unit::Scalar, v)),
                max: summary.max.map(|v| facts.fact(&format!("{prefix}:max"), Unit::Scalar, v)),
                max_degree: Some(facts.fact(&format!("{prefix}:max_degree"), Unit::Degree, summary.frequencies.max_degree)),
            });
        }
        let mut key_sets = Vec::with_capacity(self.keys.len());
        for (index, (names, summary)) in self.spec.key_sets.into_iter().zip(self.keys).enumerate() {
            let prefix = format!("key_set:{index}");
            key_sets.push(KeySetEvidence {
                columns: names,
                distinct: Some(facts.distinct(&prefix, &summary)),
                max_degree: Some(facts.fact(&format!("{prefix}:max_degree"), Unit::Degree, summary.max_degree)),
            });
        }
        Ok(RelationEvidence {
            semantics: self.spec.semantics, scope: RelationScope::Input, columns,
            rows: Some(facts.fact("rows", Unit::Rows, self.rows)), key_sets,
            // Cardinality observations never become key declarations implicitly.
            keys: Vec::new(), recursive: None,
        })
    }
}

fn validate_origin(dataset_hash: &str, source: Source, provenance: &Provenance) -> Result<()> {
    if !valid_digest(dataset_hash) { return Err(error("expected sha256 dataset hash")); }
    if !matches!(source, Source::Exact | Source::Calibrated) {
        return Err(error("measured statistics require exact or calibrated source"));
    }
    if [&provenance.producer, &provenance.method, &provenance.locator].iter().any(|s| s.trim().is_empty()) {
        return Err(error("blank statistics provenance"));
    }
    if provenance.artifact_hash.as_deref().is_some_and(|h| !valid_digest(h)) {
        return Err(error("invalid provenance artifact hash"));
    }
    if source == Source::Calibrated && provenance.run.is_none() {
        return Err(error("calibrated statistics require a completed run"));
    }
    if let Some(run) = &provenance.run {
        if !run.completed || run.run_id.trim().is_empty() || !valid_digest(&run.program_hash) || !valid_digest(&run.configuration_hash) {
            return Err(error("invalid or incomplete statistics run identity"));
        }
    }
    Ok(())
}

struct Facts<'a> {
    relation: &'a str,
    dataset_hash: &'a str,
    source: Source,
    provenance: &'a Provenance,
}

impl Facts<'_> {
    fn fact<T>(&self, suffix: &str, unit: Unit, value: T) -> Fact<T> {
        Fact {
            // Length framing prevents punctuation in relation names from
            // colliding with another relation's generated fact namespace.
            id: format!("stats:{}:{}:{suffix}", self.relation.len(), self.relation),
            source: self.source, claim: Claim::Equal, value: Some(value), lower: None, upper: None,
            dataset_hash: self.dataset_hash.to_owned(), provenance: self.provenance.clone(), unit,
        }
    }

    fn distinct(&self, prefix: &str, summary: &FrequencySummary) -> Fact<u64> {
        let mut fact = self.fact(&format!("{prefix}:distinct"), Unit::Distinct, summary.distinct);
        if let Some(hll) = &summary.hll {
            fact.source = Source::Hll;
            fact.claim = Claim::Observation;
            fact.value = Some(hll.value);
            fact.provenance.method = format!(
                "{}; algorithm=hyperloglog-v1; precision={}; hash=sha256; hash_seed=0; hash_domain=cq-load-hll-sha256-seed-0-v1; stream=full-loaded-row-stream; correction=small-range-linear-counting",
                fact.provenance.method, hll.precision,
            );
        }
        fact
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_counters_do_not_wrap() {
        let mut count = u64::MAX;
        assert!(increment(&mut count).is_err());
        assert_eq!(count, u64::MAX);
    }

    #[test]
    fn overflowing_source_rows_poison_the_accumulator() {
        let spec = RelationSpec {
            name: "R".into(),
            columns: vec![ColumnSpec { name: "a".into(), domain: ScalarDomain::SignedInteger }],
            semantics: RelationSemantics::Bag, key_sets: Vec::new(),
        };
        let mut accumulator = RelationAccumulator::new(spec, DistinctMode::Exact).unwrap();
        accumulator.source_rows = u64::MAX;
        let records = [csv::StringRecord::from(vec!["1"])];
        let batch = Batch {
            file: Path::new("overflow.csv"), columns: &["a"], mapping: &[0],
            data: crate::BatchData::Csv(&records),
        };
        assert!(accumulator.observe_batch(&batch).is_err());
        assert!(accumulator.observe_batch(&batch).is_err());
        assert!(accumulator.summarize().is_err());
    }
}
