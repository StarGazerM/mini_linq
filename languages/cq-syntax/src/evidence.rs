//! Versioned, declarative dataset evidence. This module never opens dataset files,
//! runs a query, estimates missing facts, or selects a physical plan.
//!
//! `parse_json` rejects duplicate object keys before deserialization. `check` is
//! also required after constructing or deserializing records through other APIs.
//! Canonical JSON sorts object keys recursively, preserves array order, and uses
//! compact serde_json string/integer encoding. Manifest hashes use those bytes.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use num_bigint::{BigInt, BigUint};
use num_rational::BigRational;
use num_traits::{One, ToPrimitive, Zero};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "mini-linq.planner-evidence.v1";
/// Semantic data-file identity; metadata files retain raw-byte SHA-256.
pub const HASH_MODE: &str = "typed-row-stream-v1";
pub const ROUND_CONVENTION: &str =
    "executed-steady-rounds-excluding-seed-and-terminal-empty";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceError(pub String);
impl fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for EvidenceError {}
impl From<serde_json::Error> for EvidenceError {
    fn from(error: serde_json::Error) -> Self { Self(error.to_string()) }
}
pub type Result<T, E = EvidenceError> = std::result::Result<T, E>;
fn ensure(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition { Ok(()) } else { Err(EvidenceError(message.into())) }
}
fn nonblank(s: &str) -> Result<()> { ensure(!s.trim().is_empty(), "blank identity/provenance") }
fn digest(s: &str) -> Result<()> {
    ensure(s.len() == 71 && s.starts_with("sha256:") &&
        s.as_bytes()[7..].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c)),
        "expected sha256:<64 lowercase hex digits>")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetEvidence {
    pub schema: String,
    pub dataset: DatasetIdentity,
    pub relations: BTreeMap<String, RelationEvidence>,
    #[serde(default)] pub premises: Vec<Premise>,
    #[serde(default)] pub relative: Vec<RelativeFact>,
    #[serde(default)] pub targets: BTreeMap<String, TargetEvidence>,
    #[serde(default)] pub observations: Observations,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetIdentity {
    pub name: String,
    pub hash: String,
    pub date: String,
    pub schema_hash: String,
    pub formulation: String,
    pub manifest: InputManifest,
}
/// The identity includes schema, loader policy, constants/metadata files, and
/// dictionaries. Paths are logical relative names, never machine-local paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputManifest {
    pub schema: BTreeMap<String, ManifestRelation>,
    pub formulation: String,
    pub files: BTreeMap<String, ManifestFile>,
    pub dictionaries: BTreeMap<String, String>,
    pub loader_options: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRelation {
    pub semantics: RelationSemantics,
    pub columns: BTreeMap<String, ScalarDomain>,
    pub null_handling: String,
    pub set_certification: Option<SetCertification>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetCertification { FullTupleDeduplicated, InputUnique }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub role: FileRole,
    pub content_hash: Option<String>,
    pub encoding_hash: String,
    pub missing_as_empty: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileRole { Data, Metadata, Constants, Dictionary }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationSemantics { Bag, Set }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationScope { Input, Frozen, RecursiveFinal }
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScalarDomain {
    SignedInteger,
    Decimal { scale: u32 },
    Date { encoding: String },
    Utf8,
    Symbol { dictionary_hash: String, ordered: bool },
    Unordered { limitation: String },
}
impl<'de> Deserialize<'de> for ScalarDomain {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Internally tagged unit variants otherwise discard unknown fields,
        // even with deny_unknown_fields. Empty struct variants enforce it.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            SignedInteger {},
            Decimal { scale: u32 },
            Date { encoding: String },
            Utf8 {},
            Symbol { dictionary_hash: String, ordered: bool },
            Unordered { limitation: String },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::SignedInteger {} => Self::SignedInteger,
            Wire::Decimal { scale } => Self::Decimal { scale },
            Wire::Date { encoding } => Self::Date { encoding },
            Wire::Utf8 {} => Self::Utf8,
            Wire::Symbol { dictionary_hash, ordered } => Self::Symbol { dictionary_hash, ordered },
            Wire::Unordered { limitation } => Self::Unordered { limitation },
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scalar {
    SignedInteger { value: i64 },
    Decimal { unscaled: String, scale: u32 },
    Date { value: i64, encoding: String },
    Utf8 { value: String },
    Symbol { code: u64, dictionary_hash: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationEvidence {
    pub semantics: RelationSemantics,
    pub scope: RelationScope,
    pub columns: BTreeMap<String, ColumnEvidence>,
    pub rows: Option<CountFact>,
    #[serde(default)] pub key_sets: Vec<KeySetEvidence>,
    #[serde(default)] pub keys: Vec<KeyDeclaration>,
    pub recursive: Option<RecursiveEvidence>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnEvidence {
    pub domain: ScalarDomain,
    pub distinct: Option<CountFact>,
    pub min: Option<ScalarFact>,
    pub max: Option<ScalarFact>,
    pub max_degree: Option<CountFact>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeySetEvidence {
    pub columns: Vec<String>,
    pub distinct: Option<CountFact>,
    pub max_degree: Option<CountFact>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source { Exact, Hll, Bound, Declared, Calibrated }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Claim { Equal, Upper, Lower, Interval, Observation }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Rows, Distinct, Degree, Rounds, Scalar, Ratio,
    #[serde(rename = "declared-row-work")]
    DeclaredRowWork,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact<T> {
    pub id: String,
    pub source: Source,
    pub claim: Claim,
    pub value: Option<T>,
    pub lower: Option<T>,
    pub upper: Option<T>,
    pub dataset_hash: String,
    pub provenance: Provenance,
    pub unit: Unit,
}
pub type CountFact = Fact<u64>;
pub type ScalarFact = Fact<Scalar>;
pub type RationalFact = Fact<Rational>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rational {
    pub numerator: String,
    pub denominator: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub producer: String,
    pub method: String,
    pub locator: String,
    pub run: Option<RunIdentity>,
    pub artifact_hash: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunIdentity {
    pub program_hash: String,
    pub configuration_hash: String,
    pub admission: Admission,
    pub run_id: String,
    pub completed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Admission { InsertOnlySet, Bag, Replacement, Deletion }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyDeclaration {
    pub columns: Vec<String>,
    pub source: Source,
    pub dataset_hash: String,
    pub provenance: Provenance,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecursiveEvidence {
    pub stratum: String,
    pub final_rows: CountFact,
    pub rounds: CountFact,
    pub round_convention: String,
    pub run: RunIdentity,
    pub seed_rows: Option<CountFact>,
    pub unconsumed_final_delta_rows: Option<CountFact>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Premise {
    DeltaSubset {
        id: String, relations: Vec<String>, source: Source,
        dataset_hash: String, provenance: Provenance,
    },
    DeltaPartition {
        id: String, relation: String, includes_seed_delta: bool,
        insert_only_set: bool, complete_consumption: bool,
        source: Source, dataset_hash: String, provenance: Provenance,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactSubject {
    pub relation: String,
    pub measure: RowMeasure,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowMeasure { Rows, FinalRows }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelativeOp { LeScaled }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelativeFact {
    pub id: String,
    pub left: FactSubject,
    pub op: RelativeOp,
    pub right: FactSubject,
    pub factor: RationalFact,
    pub source: Source,
    pub dataset_hash: String,
    pub provenance: Provenance,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetEvidence {
    pub unit: Unit,
    pub build_per_row_h: RationalFact,
    pub merge_per_row_m: RationalFact,
    pub configuration_hash: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observations {
    #[serde(default)] pub rounds: Vec<RoundObservations>,
    #[serde(default)] pub steps: Vec<StepObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physical_steps: Vec<PhysicalStepObservation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundObservations {
    pub relation: String,
    pub stratum: String,
    pub run: RunIdentity,
    pub complete: bool,
    pub entries: Vec<RoundObservation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundObservation {
    pub round: CountFact,
    pub delta_rows: CountFact,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepObservation {
    pub relation: String,
    pub hole: String,
    pub variant: String,
    pub step: String,
    pub run: RunIdentity,
    pub round: Option<CountFact>,
    pub rows: CountFact,
    pub bound: BoundCitation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundCitation {
    pub fact: String,
    pub premises: Vec<String>,
    pub dataset_hash: String,
    pub program_hash: String,
    pub variant: String,
}

/// Actual lowered operator-counter observations. Unlike a supplied row
/// certificate, a computed comparison can be unknown or falsified: neither
/// outcome is silently promoted into planning evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalStepObservation {
    pub hole: String,
    pub variant: String,
    pub instance: String,
    pub stratum: String,
    pub trace_hash: String,
    pub physical_steps: Vec<String>,
    pub vo_steps: Vec<String>,
    pub fused: bool,
    pub run: RunIdentity,
    pub round: Option<CountFact>,
    pub rows: CountFact,
    pub comparison: Option<PhysicalBoundComparison>,
    pub unknown: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalBoundComparison {
    /// Exact output of the bound calculation, not an admitted upper certificate.
    pub computed: RationalFact,
    pub fact_citations: Vec<String>,
    pub compiler_citations: Vec<String>,
    pub within_bound: bool,
}

/// A recursive JSON visitor, rather than `Value`'s last-key-wins parser.
struct UniqueJson(serde_json::Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("JSON without duplicate object keys") }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<Self::Value, E> { Err(E::custom("JSON floating-point numbers are not evidence integers")) }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_string<E: serde::de::Error>(self, v: String) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(serde_json::Value::Null)) }
            fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> { self.visit_unit() }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueJson(v)) = a.next_element()? { values.push(v); }
                Ok(UniqueJson(values.into()))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> std::result::Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if values.contains_key(&k) { return Err(serde::de::Error::custom(format!("duplicate JSON object key: {k}"))); }
                    let UniqueJson(v) = a.next_value()?;
                    values.insert(k, v);
                }
                Ok(UniqueJson(values.into()))
            }
        }
        de.deserialize_any(Visitor)
    }
}

pub fn parse_json(json: &str) -> Result<DatasetEvidence> {
    let UniqueJson(value) = serde_json::from_str(json)?;
    let evidence: DatasetEvidence = serde_json::from_value(value)?;
    evidence.check()?;
    Ok(evidence)
}
/// Byte-oriented front ends can preserve the invalid-UTF-8 error boundary.
pub fn parse_bytes(json: &[u8]) -> Result<DatasetEvidence> {
    parse_json(std::str::from_utf8(json).map_err(|e| EvidenceError(e.to_string()))?)
}
fn canonical<T: Serialize>(value: &T) -> Result<String> {
    fn sort(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let sorted: BTreeMap<_, _> = map.into_iter().map(|(k, v)| (k, sort(v))).collect();
                serde_json::Value::Object(sorted.into_iter().collect())
            }
            serde_json::Value::Array(a) => serde_json::Value::Array(a.into_iter().map(sort).collect()),
            v => v,
        }
    }
    Ok(serde_json::to_string(&sort(serde_json::to_value(value)?))?)
}
/// SHA-256 of the same canonical JSON encoding used by manifest identities.
pub fn canonical_hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("sha256:{:x}", Sha256::digest(canonical(value)?.as_bytes())))
}
impl InputManifest {
    pub fn dataset_hash(&self) -> Result<String> { canonical_hash(self) }
    pub fn schema_hash(&self) -> Result<String> { canonical_hash(&self.schema) }
}
fn unsigned(s: &str) -> Result<BigUint> {
    ensure(!s.is_empty() && (s == "0" || !s.starts_with('0')) && s.bytes().all(|c| c.is_ascii_digit()), "noncanonical unsigned decimal integer")?;
    s.parse().map_err(|_| EvidenceError("invalid unsigned decimal integer".into()))
}
fn signed(s: &str) -> Result<BigInt> {
    let magnitude = s.strip_prefix('-').unwrap_or(s);
    unsigned(magnitude)?;
    ensure(s != "-0", "negative zero is not canonical")?;
    s.parse().map_err(|_| EvidenceError("invalid signed decimal integer".into()))
}
impl Rational {
    pub fn exact(&self) -> Result<BigRational> {
        let n = unsigned(&self.numerator)?;
        let d = unsigned(&self.denominator)?;
        ensure(!d.is_zero(), "zero rational denominator")?;
        Ok(BigRational::new(n.into(), d.into()))
    }
}
impl RunIdentity {
    fn check(&self) -> Result<()> {
        digest(&self.program_hash)?;
        digest(&self.configuration_hash)?;
        nonblank(&self.run_id)?;
        ensure(self.completed, "evidence run must be completed")
    }
}
impl Provenance {
    fn check(&self) -> Result<()> {
        nonblank(&self.producer)?; nonblank(&self.method)?; nonblank(&self.locator)?;
        if let Some(run) = &self.run { run.check()?; }
        if let Some(hash) = &self.artifact_hash { digest(hash)?; }
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct Interval { lo: u64, hi: u64 }
impl Interval {
    const UNKNOWN: Self = Self { lo: 0, hi: u64::MAX };
    fn intersect(self, rhs: Self) -> Result<Self> {
        let result = Self { lo: self.lo.max(rhs.lo), hi: self.hi.min(rhs.hi) };
        ensure(result.lo <= result.hi, "empty feasible count interval")?;
        Ok(result)
    }
}
fn interval(fact: Option<&CountFact>) -> Interval {
    match fact {
        Some(f) => match f.claim {
            Claim::Equal => Interval { lo: f.value.unwrap(), hi: f.value.unwrap() },
            Claim::Upper => Interval { lo: 0, hi: f.value.unwrap() },
            Claim::Lower => Interval { lo: f.value.unwrap(), hi: u64::MAX },
            Claim::Interval => Interval { lo: f.lower.unwrap(), hi: f.upper.unwrap() },
            Claim::Observation => Interval::UNKNOWN,
        },
        None => Interval::UNKNOWN,
    }
}
fn payloads<T>(fact: &Fact<T>) -> impl Iterator<Item = &T> {
    fact.value.iter().chain(fact.lower.iter()).chain(fact.upper.iter())
}
struct Checker<'a> {
    document: &'a DatasetEvidence,
    ids: BTreeSet<&'a str>,
    counts: BTreeMap<&'a str, &'a CountFact>,
}
impl<'a> Checker<'a> {
    fn identity(&mut self, id: &'a str, source: Source, dataset: &str, provenance: &Provenance) -> Result<()> {
        nonblank(id)?;
        ensure(self.ids.insert(id), format!("duplicate fact id: {id}"))?;
        self.origin(source, dataset, provenance)
    }
    fn origin(&self, source: Source, dataset: &str, provenance: &Provenance) -> Result<()> {
        digest(dataset)?;
        ensure(dataset == self.document.dataset.hash, "fact dataset hash mismatch")?;
        provenance.check()?;
        if source == Source::Calibrated { ensure(provenance.run.is_some(), "calibrated fact lacks completed run")?; }
        Ok(())
    }
    fn fact<T>(&mut self, fact: &'a Fact<T>, unit: Unit) -> Result<()> {
        self.identity(&fact.id, fact.source, &fact.dataset_hash, &fact.provenance)?;
        ensure(fact.unit == unit, format!("wrong unit for {}", fact.id))?;
        ensure(match fact.claim {
            Claim::Interval => fact.value.is_none() && fact.lower.is_some() && fact.upper.is_some(),
            _ => fact.value.is_some() && fact.lower.is_none() && fact.upper.is_none(),
        }, format!("invalid value/interval envelope: {}", fact.id))?;
        ensure(fact.source != Source::Hll || fact.claim == Claim::Observation, "HLL is observational, never a certificate")?;
        ensure(fact.source != Source::Bound || matches!(fact.claim, Claim::Upper | Claim::Lower | Claim::Interval), "bound source requires a bound claim")?;
        Ok(())
    }
    fn count(&mut self, fact: &'a CountFact, unit: Unit, run: Option<&RunIdentity>) -> Result<()> {
        self.fact(fact, unit)?;
        ensure(fact.source != Source::Hll || unit == Unit::Distinct, "HLL only describes distinct observations")?;
        if fact.claim == Claim::Interval { ensure(fact.lower <= fact.upper, "inverted count interval")?; }
        if let Some(run) = run {
            ensure(fact.provenance.run.as_ref() == Some(run), "fact run/admission scope mismatch")?;
        }
        self.counts.insert(&fact.id, fact);
        Ok(())
    }
    fn rational(&mut self, fact: &'a RationalFact, unit: Unit) -> Result<()> {
        self.fact(fact, unit)?;
        ensure(fact.source != Source::Hll, "HLL is not a rational certificate")?;
        for value in payloads(fact) { value.exact()?; }
        if let (Some(lo), Some(hi)) = (&fact.lower, &fact.upper) {
            ensure(lo.exact()? <= hi.exact()?, "inverted rational interval")?;
        }
        Ok(())
    }
    fn scalar(&mut self, fact: &'a ScalarFact, domain: &ScalarDomain, run: Option<&RunIdentity>) -> Result<()> {
        self.fact(fact, Unit::Scalar)?;
        ensure(fact.source != Source::Hll, "HLL cannot describe extrema")?;
        if let Some(run) = run { ensure(fact.provenance.run.as_ref() == Some(run), "extremum run scope mismatch")?; }
        for value in payloads(fact) { scalar_value(value, domain)?; }
        if let (Some(lo), Some(hi)) = (&fact.lower, &fact.upper) {
            ensure(scalar_value(lo, domain)? <= scalar_value(hi, domain)?, "inverted scalar interval")?;
        }
        Ok(())
    }
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum OrderedScalar<'a> { Number(BigInt), Text(&'a str) }
fn scalar_value<'a>(value: &'a Scalar, domain: &ScalarDomain) -> Result<OrderedScalar<'a>> {
    use OrderedScalar::{Number, Text};
    Ok(match (value, domain) {
        (Scalar::SignedInteger { value }, ScalarDomain::SignedInteger) => Number((*value).into()),
        (Scalar::Decimal { unscaled, scale }, ScalarDomain::Decimal { scale: expected }) if scale == expected => Number(signed(unscaled)?),
        (Scalar::Date { value, encoding }, ScalarDomain::Date { encoding: expected }) if encoding == expected => Number((*value).into()),
        (Scalar::Utf8 { value }, ScalarDomain::Utf8) => Text(value),
        (Scalar::Symbol { code, dictionary_hash }, ScalarDomain::Symbol { dictionary_hash: expected, ordered: true }) if dictionary_hash == expected => Number((*code).into()),
        _ => return Err(EvidenceError("extremum encoding/dictionary/domain mismatch or unordered domain".into())),
    })
}
fn scalar_bounds(f: Option<&ScalarFact>) -> (Option<&Scalar>, Option<&Scalar>) {
    match f {
        Some(f) => match f.claim {
            Claim::Equal => (f.value.as_ref(), f.value.as_ref()),
            Claim::Upper => (None, f.value.as_ref()),
            Claim::Lower => (f.value.as_ref(), None),
            Claim::Interval => (f.lower.as_ref(), f.upper.as_ref()),
            Claim::Observation => (None, None),
        },
        None => (None, None),
    }
}
fn keys<'a>(columns: &'a [String], relation: &RelationEvidence) -> Result<BTreeSet<&'a str>> {
    ensure(!columns.is_empty(), "empty keys are unsupported")?;
    let mut names = BTreeSet::new();
    for name in columns {
        ensure(relation.columns.contains_key(name), format!("unknown key column: {name}"))?;
        ensure(names.insert(name.as_str()), "repeated key column")?;
    }
    Ok(names)
}
/// Feasibility of N rows, D nonempty buckets and largest bucket M. Endpoint
/// propagation uses necessary integer constraints, never compares loose upper
/// bounds as though they were exact values. Products are widened to u128.
fn cardinalities(mut rows: Interval, mut distinct: Interval, mut degree: Interval) -> Result<(Interval, Interval, Interval)> {
    loop {
        let old = (rows.lo, rows.hi, distinct.lo, distinct.hi, degree.lo, degree.hi);
        distinct.hi = distinct.hi.min(rows.hi);
        degree.hi = degree.hi.min(rows.hi);
        rows.lo = rows.lo.max(distinct.lo).max(degree.lo);
        if rows.lo > 0 { distinct.lo = distinct.lo.max(1); degree.lo = degree.lo.max(1); }
        if distinct.hi == 0 || degree.hi == 0 { rows.hi = 0; }
        if rows.hi == 0 { distinct.hi = 0; degree.hi = 0; }
        let product = u128::from(distinct.hi) * u128::from(degree.hi);
        rows.hi = rows.hi.min(product.min(u128::from(u64::MAX)) as u64);
        if degree.hi > 0 { distinct.lo = distinct.lo.max(rows.lo.div_ceil(degree.hi)); }
        if distinct.hi > 0 { degree.lo = degree.lo.max(rows.lo.div_ceil(distinct.hi)); }
        if distinct.lo > 0 && degree.lo > 0 {
            let minimum = u128::from(distinct.lo) + u128::from(degree.lo) - 1;
            ensure(minimum <= u128::from(rows.hi), "rows cannot contain distinct buckets and maximum degree")?;
            rows.lo = rows.lo.max(minimum as u64);
            degree.hi = degree.hi.min(rows.hi - distinct.lo + 1);
            distinct.hi = distinct.hi.min(rows.hi - degree.lo + 1);
        }
        ensure(rows.lo <= rows.hi && distinct.lo <= distinct.hi && degree.lo <= degree.hi, "infeasible rows/distinct/max_degree intervals")?;
        if old == (rows.lo, rows.hi, distinct.lo, distinct.hi, degree.lo, degree.hi) { return Ok((rows, distinct, degree)); }
    }
}
fn date(s: &str) -> Result<()> {
    let bytes = s.as_bytes();
    ensure(bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' && bytes.iter().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()), "date must be YYYY-MM-DD")?;
    let y: u32 = s[..4].parse().map_err(|_| EvidenceError("invalid year".into()))?;
    let m: usize = s[5..7].parse().map_err(|_| EvidenceError("invalid month".into()))?;
    let d: u32 = s[8..].parse().map_err(|_| EvidenceError("invalid day".into()))?;
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    ensure(y > 0 && (1..=12).contains(&m) && d > 0 && d <= days[m.saturating_sub(1).min(11)], "invalid calendar date")
}
impl DatasetEvidence {
    pub fn canonical_json(&self) -> Result<String> { self.check()?; canonical(self) }
    pub fn check(&self) -> Result<()> {
        ensure(self.schema == SCHEMA, "unsupported evidence schema version")?;
        self.check_manifest()?;
        let mut checker = Checker { document: self, ids: BTreeSet::new(), counts: BTreeMap::new() };
        let mut strata: BTreeMap<&str, (&RunIdentity, Interval)> = BTreeMap::new();
        let mut row_ranges = BTreeMap::new();
        for (name, relation) in &self.relations {
            nonblank(name)?;
            let schema = self.dataset.manifest.schema.get(name).ok_or_else(|| EvidenceError(format!("unknown manifest relation: {name}")))?;
            ensure(schema.semantics == relation.semantics, "bag/set semantics mismatch")?;
            ensure(schema.columns.len() == relation.columns.len(), "incomplete relation column schema")?;
            ensure((relation.scope == RelationScope::RecursiveFinal) == relation.recursive.is_some(), "recursive facts require recursive_final scope and vice versa")?;
            let run = relation.recursive.as_ref().map(|r| &r.run);
            let mut recorded_run = run;
            let provenance = relation.rows.iter().map(|f| &f.provenance)
                .chain(relation.columns.values().flat_map(|c| [
                    c.distinct.as_ref().map(|f| &f.provenance),
                    c.max_degree.as_ref().map(|f| &f.provenance),
                    c.min.as_ref().map(|f| &f.provenance),
                    c.max.as_ref().map(|f| &f.provenance),
                ].into_iter().flatten()))
                .chain(relation.key_sets.iter().flat_map(|k| k.distinct.iter().chain(k.max_degree.iter()).map(|f| &f.provenance)))
                .chain(relation.keys.iter().map(|k| &k.provenance));
            for provenance in provenance {
                if let Some(current) = &provenance.run {
                    ensure(recorded_run.is_none_or(|previous| previous == current), "conflicting run scopes within relation")?;
                    recorded_run = Some(current);
                }
            }
            if let Some(rows) = &relation.rows { checker.count(rows, Unit::Rows, run)?; }
            let mut rows = interval(relation.rows.as_ref());
            if let Some(recursive) = &relation.recursive {
                nonblank(&recursive.stratum)?;
                recursive.run.check()?;
                ensure(recursive.round_convention == ROUND_CONVENTION, "unsupported round convention")?;
                ensure(relation.semantics != RelationSemantics::Set || recursive.run.admission == Admission::InsertOnlySet, "set recursive evidence requires insert-only set admission")?;
                checker.count(&recursive.final_rows, Unit::Rows, run)?;
                checker.count(&recursive.rounds, Unit::Rounds, run)?;
                rows = rows.intersect(interval(Some(&recursive.final_rows)))?;
                for f in [&recursive.seed_rows, &recursive.unconsumed_final_delta_rows].into_iter().flatten() {
                    checker.count(f, Unit::Rows, run)?;
                    ensure(interval(Some(f)).lo <= rows.hi, "seed/unconsumed delta exceeds final rows")?;
                }
                let rounds = interval(Some(&recursive.rounds));
                if let Some((previous, range)) = strata.get_mut(recursive.stratum.as_str()) {
                    ensure(*previous == &recursive.run, "inconsistent run identity within stratum")?;
                    *range = range.intersect(rounds)?;
                } else { strata.insert(&recursive.stratum, (&recursive.run, rounds)); }
            }
            let mut declared_keys = BTreeSet::new();
            for key in &relation.keys {
                let names = keys(&key.columns, relation)?;
                ensure(declared_keys.insert(names), "repeated key declaration")?;
                ensure(matches!(key.source, Source::Exact | Source::Declared), "keys require exact or declared provenance")?;
                checker.origin(key.source, &key.dataset_hash, &key.provenance)?;
                if let Some(run) = run { ensure(key.provenance.run.as_ref() == Some(run), "key run scope mismatch")?; }
            }
            let mut statistics = BTreeMap::new();
            for (column_name, column) in &relation.columns {
                ensure(schema.columns.get(column_name) == Some(&column.domain), "column domain differs from manifest schema")?;
                for (fact, unit) in [(&column.distinct, Unit::Distinct), (&column.max_degree, Unit::Degree)] {
                    if let Some(fact) = fact { checker.count(fact, unit, run)?; }
                }
                for fact in [&column.min, &column.max].into_iter().flatten() { checker.scalar(fact, &column.domain, run)?; }
                let mut distinct = interval(column.distinct.as_ref());
                let (min_lo, min_hi) = scalar_bounds(column.min.as_ref());
                let (max_lo, max_hi) = scalar_bounds(column.max.as_ref());
                if min_lo.is_some() || min_hi.is_some() || max_lo.is_some() || max_hi.is_some() {
                    rows = rows.intersect(Interval { lo: 1, hi: u64::MAX })?;
                }
                if let (Some(lo), Some(hi)) = (min_lo, max_hi) {
                    let lo = scalar_value(lo, &column.domain)?;
                    let hi = scalar_value(hi, &column.domain)?;
                    ensure(lo <= hi, "minimum exceeds maximum")?;
                    if lo == hi { distinct = distinct.intersect(Interval { lo: 0, hi: 1 })?; }
                }
                if let (Some(lo), Some(hi)) = (max_lo, min_hi) {
                    if scalar_value(lo, &column.domain)? > scalar_value(hi, &column.domain)? {
                        distinct = distinct.intersect(Interval { lo: 2, hi: u64::MAX })?;
                    }
                }
                statistics.insert(BTreeSet::from([column_name.as_str()]), (distinct, interval(column.max_degree.as_ref())));
            }
            let mut key_sets = BTreeSet::new();
            for set in &relation.key_sets {
                let names = keys(&set.columns, relation)?;
                ensure(key_sets.insert(names.clone()), "repeated key-set statistic")?;
                for (fact, unit) in [(&set.distinct, Unit::Distinct), (&set.max_degree, Unit::Degree)] {
                    if let Some(fact) = fact { checker.count(fact, unit, run)?; }
                }
                let d = interval(set.distinct.as_ref());
                let m = interval(set.max_degree.as_ref());
                if let Some((old_d, old_m)) = statistics.get_mut(&names) {
                    *old_d = old_d.intersect(d)?; *old_m = old_m.intersect(m)?;
                } else { statistics.insert(names, (d, m)); }
            }
            for key in &declared_keys {
                statistics.entry(key.clone()).or_insert((Interval::UNKNOWN, Interval::UNKNOWN));
            }
            if relation.semantics == RelationSemantics::Set {
                statistics.entry(relation.columns.keys().map(String::as_str).collect()).or_insert((Interval::UNKNOWN, Interval::UNKNOWN));
            }
            loop {
                let old = (rows.lo, rows.hi);
                for (columns, (distinct, degree)) in &mut statistics {
                    if declared_keys.iter().any(|key| key.is_subset(columns)) ||
                        (relation.semantics == RelationSemantics::Set && columns.len() == relation.columns.len()) {
                        *degree = degree.intersect(Interval { lo: 0, hi: 1 })?;
                    }
                    (rows, *distinct, *degree) = cardinalities(rows, *distinct, *degree)?;
                }
                // A finer key has at least as many distinct values and at most
                // the degree of every coarser key; not the reverse.
                let snapshot: Vec<_> = statistics.iter().map(|(k, v)| (k.clone(), *v)).collect();
                let mut changed = false;
                for (fine, (d, m)) in &mut statistics {
                    let before = (d.lo, d.hi, m.lo, m.hi);
                    for (coarse, (cd, cm)) in &snapshot {
                        if coarse.is_subset(fine) {
                            *d = d.intersect(Interval { lo: cd.lo, hi: u64::MAX })?;
                            *m = m.intersect(Interval { lo: 0, hi: cm.hi })?;
                        }
                        if fine.is_subset(coarse) {
                            *d = d.intersect(Interval { lo: 0, hi: cd.hi })?;
                            *m = m.intersect(Interval { lo: cm.lo, hi: u64::MAX })?;
                        }
                    }
                    changed |= before != (d.lo, d.hi, m.lo, m.hi);
                }
                if old == (rows.lo, rows.hi) && !changed { break; }
            }
            for (name, column) in &relation.columns {
                ensure(rows.hi != 0 || (column.min.is_none() && column.max.is_none()), "extrema on certified empty relation")?;
                let (min_lo, min_hi) = scalar_bounds(column.min.as_ref());
                let (max_lo, max_hi) = scalar_bounds(column.max.as_ref());
                if let (Some(lo), Some(hi)) = (min_lo, max_hi) {
                    ensure(scalar_value(lo, &column.domain)? <= scalar_value(hi, &column.domain)?, "minimum exceeds maximum")?;
                }
                let (distinct, _) = statistics[&BTreeSet::from([name.as_str()])];
                if distinct.lo > 1 {
                    if let (Some(lo), Some(hi)) = (min_lo, max_hi) {
                        ensure(scalar_value(lo, &column.domain)? < scalar_value(hi, &column.domain)?, "multiple distinct values require different extrema")?;
                    }
                }
                if distinct.hi == 1 {
                    if let (Some(lo), Some(hi)) = (max_lo, min_hi) {
                        ensure(scalar_value(lo, &column.domain)? <= scalar_value(hi, &column.domain)?, "singleton distinct has incompatible extrema")?;
                    }
                }
            }
            if relation.semantics == RelationSemantics::Set && relation.scope != RelationScope::RecursiveFinal && schema.set_certification.is_none() {
                let facts = relation.rows.iter().chain(relation.columns.values().flat_map(|c| c.distinct.iter().chain(c.max_degree.iter()))).chain(relation.key_sets.iter().flat_map(|k| k.distinct.iter().chain(k.max_degree.iter())));
                for f in facts { ensure(matches!(f.claim, Claim::Upper | Claim::Observation), "uncertified bag accumulation cannot assert exact/lower set statistics")?; }
            }
            row_ranges.insert(name.as_str(), rows);
        }
        let partitions = self.check_premises(&mut checker)?;
        self.check_relative(&mut checker, &mut row_ranges)?;
        for (target, costs) in &self.targets {
            ensure(matches!(target.as_str(), "cpu" | "cpu-par" | "gpu"), "unknown target")?;
            ensure(costs.unit == Unit::DeclaredRowWork, "target unit must be declared_row_work")?;
            digest(&costs.configuration_hash)?;
            for f in [&costs.build_per_row_h, &costs.merge_per_row_m] {
                checker.rational(f, Unit::DeclaredRowWork)?;
                ensure(f.claim == Claim::Equal && matches!(f.source, Source::Declared | Source::Calibrated | Source::Exact), "target h/m must be supplied exact rational constants")?;
                if let Some(run) = &f.provenance.run { ensure(run.configuration_hash == costs.configuration_hash, "target calibration configuration mismatch")?; }
            }
        }
        self.check_observations(&mut checker, &partitions, &row_ranges, &strata)?;
        Ok(())
    }
    fn check_manifest(&self) -> Result<()> {
        let d = &self.dataset;
        nonblank(&d.name)?; nonblank(&d.formulation)?; date(&d.date)?;
        digest(&d.hash)?; digest(&d.schema_hash)?;
        let m = &d.manifest;
        ensure(m.formulation == d.formulation, "manifest formulation mismatch")?;
        ensure(!m.schema.is_empty() && !m.files.is_empty() && !m.loader_options.is_empty(), "incomplete manifest identity")?;
        ensure(m.loader_options.get("hash_mode").map(String::as_str) == Some(HASH_MODE),
            "missing or incompatible dataset hash mode")?;
        for (name, file) in &m.files {
            ensure(!name.is_empty() && !name.contains('\\') && !name.contains(':') && name.split('/').all(|p| !p.is_empty() && p != "." && p != ".."), "manifest file name must be a logical relative path")?;
            digest(&file.encoding_hash)?;
            ensure(file.content_hash.is_some() != file.missing_as_empty, "manifest file requires content digest or explicit missing-as-empty")?;
            if let Some(hash) = &file.content_hash { digest(hash)?; }
            if file.missing_as_empty {
                ensure(file.role == FileRole::Data && m.loader_options.get("missing_files").is_some_and(|v| v == "empty"), "loader does not permit missing-as-empty")?;
            }
        }
        for (name, hash) in &m.dictionaries { nonblank(name)?; digest(hash)?; }
        for (name, value) in &m.loader_options { nonblank(name)?; nonblank(value)?; }
        for (name, relation) in &m.schema {
            nonblank(name)?; nonblank(&relation.null_handling)?;
            ensure(!relation.columns.is_empty(), "manifest relation has no columns")?;
            ensure(relation.semantics == RelationSemantics::Set || relation.set_certification.is_none(), "bag schema cannot carry set certification")?;
            for (name, domain) in &relation.columns {
                nonblank(name)?;
                match domain {
                    ScalarDomain::Date { encoding } => nonblank(encoding)?,
                    ScalarDomain::Symbol { dictionary_hash, .. } => {
                        digest(dictionary_hash)?;
                        ensure(m.dictionaries.values().any(|h| h == dictionary_hash), "unknown dictionary identity")?;
                    }
                    ScalarDomain::Unordered { limitation } => nonblank(limitation)?,
                    _ => {}
                }
            }
        }
        ensure(m.schema_hash()? == d.schema_hash, "manifest schema hash mismatch")?;
        ensure(m.dataset_hash()? == d.hash, "manifest dataset hash mismatch")
    }
    fn recursive_relation(&self, name: &str) -> Result<(&RelationEvidence, &RecursiveEvidence)> {
        let relation = self.relations.get(name).ok_or_else(|| EvidenceError(format!("unknown relation: {name}")))?;
        let recursive = relation.recursive.as_ref().ok_or_else(|| EvidenceError(format!("nonrecursive relation: {name}")))?;
        Ok((relation, recursive))
    }
    fn check_premises<'a>(&'a self, checker: &mut Checker<'a>) -> Result<BTreeSet<&'a str>> {
        let mut partitions = BTreeSet::new();
        for premise in &self.premises {
            match premise {
                Premise::DeltaSubset { id, relations, source, dataset_hash, provenance } => {
                    checker.identity(id, *source, dataset_hash, provenance)?;
                    ensure(*source == Source::Declared && !relations.is_empty(), "subset premise requires a nonempty explicit declaration")?;
                    let mut seen = BTreeSet::new();
                    for name in relations {
                        ensure(seen.insert(name), "repeated subset relation")?;
                        let (relation, recursive) = self.recursive_relation(name)?;
                        ensure(relation.semantics == RelationSemantics::Set && recursive.run.admission == Admission::InsertOnlySet, "subset premise requires insert-only sets")?;
                        ensure(provenance.run.as_ref() == Some(&recursive.run), "subset premise run mismatch")?;
                    }
                }
                Premise::DeltaPartition { id, relation, includes_seed_delta, insert_only_set, complete_consumption, source, dataset_hash, provenance } => {
                    checker.identity(id, *source, dataset_hash, provenance)?;
                    ensure(*source == Source::Declared && *includes_seed_delta && *insert_only_set && *complete_consumption, "partition lacks seed/admission/complete-consumption premise")?;
                    ensure(partitions.insert(relation.as_str()), "repeated partition premise")?;
                    let (r, recursive) = self.recursive_relation(relation)?;
                    ensure(r.semantics == RelationSemantics::Set && recursive.run.admission == Admission::InsertOnlySet, "partition requires insert-only set admission")?;
                    ensure(provenance.run.as_ref() == Some(&recursive.run), "partition run mismatch")?;
                    if let Some(unconsumed) = &recursive.unconsumed_final_delta_rows {
                        ensure(interval(Some(unconsumed)).lo == 0, "partition contradicts unconsumed final delta")?;
                    }
                }
            }
        }
        Ok(partitions)
    }
    fn subject(&self, subject: &FactSubject) -> Result<&RelationEvidence> {
        let relation = self.relations.get(&subject.relation).ok_or_else(|| EvidenceError("dangling relative relation".into()))?;
        ensure(match subject.measure {
            RowMeasure::Rows => relation.scope != RelationScope::RecursiveFinal,
            RowMeasure::FinalRows => relation.scope == RelationScope::RecursiveFinal,
        }, "relative subject has incompatible input/final scope")?;
        Ok(relation)
    }
    fn check_relative<'a>(&'a self, checker: &mut Checker<'a>, rows: &mut BTreeMap<&str, Interval>) -> Result<()> {
        if self.relative.is_empty() { return Ok(()); }
        let subjects: BTreeSet<_> = self.relative.iter().flat_map(|fact| [fact.left.relation.as_str(), fact.right.relation.as_str()]).collect();
        let names: Vec<_> = subjects.into_iter().collect();
        let indices: BTreeMap<_, _> = names.iter().enumerate().map(|(i, n)| (*n, i)).collect();
        let n = names.len();
        // Exact multiplicative path closure. A product<1 cycle forces zero;
        // without a positive absolute lower anchor it is valid, not numeric data.
        let mut paths = vec![vec![None::<BigRational>; n]; n];
        for (i, row) in paths.iter_mut().enumerate() { row[i] = Some(BigRational::one()); }
        for fact in &self.relative {
            checker.identity(&fact.id, fact.source, &fact.dataset_hash, &fact.provenance)?;
            ensure(fact.source == Source::Declared, "relative predicates require declared source")?;
            let left = self.subject(&fact.left)?;
            let right = self.subject(&fact.right)?;
            ensure(left.semantics == right.semantics, "relative bag/set mismatch")?;
            if let (Some(l), Some(r)) = (&left.recursive, &right.recursive) { ensure(l.run == r.run, "relative run mismatch")?; }
            for recursive in [left.recursive.as_ref(), right.recursive.as_ref()].into_iter().flatten() {
                ensure(fact.provenance.run.as_ref() == Some(&recursive.run), "relative declaration run mismatch")?;
            }
            checker.rational(&fact.factor, Unit::Ratio)?;
            if let Some(run) = &fact.factor.provenance.run {
                ensure(fact.provenance.run.as_ref() == Some(run), "relative factor run mismatch")?;
            }
            ensure(fact.factor.claim == Claim::Equal, "relative factor must be exact, not observational or bounded")?;
            let factor = fact.factor.value.as_ref().unwrap().exact()?;
            ensure(factor > BigRational::zero(), "relative factor must be positive")?;
            let i = indices[fact.left.relation.as_str()];
            let j = indices[fact.right.relation.as_str()];
            if paths[i][j].as_ref().is_none_or(|old| &factor < old) { paths[i][j] = Some(factor); }
        }
        for k in 0..n {
            for i in 0..n {
                for j in 0..n {
                    if let (Some(a), Some(b)) = (&paths[i][k], &paths[k][j]) {
                        let candidate = a * b;
                        if paths[i][j].as_ref().is_none_or(|old| &candidate < old) { paths[i][j] = Some(candidate); }
                    }
                }
            }
        }
        for i in 0..n {
            let lo = BigRational::from_integer(rows[names[i]].lo.into());
            for j in 0..n {
                if let Some(factor) = &paths[i][j] {
                    let zero_cycle = paths[j][j].as_ref().is_some_and(|f| f < &BigRational::one());
                    let hi = if zero_cycle { BigRational::zero() } else { BigRational::from_integer(rows[names[j]].hi.into()) };
                    let upper = factor * hi;
                    ensure(lo <= upper, "relative inequalities contradict absolute facts")?;
                    if let Some(upper) = upper.to_integer().to_u64() {
                        let range = rows.get_mut(names[i]).unwrap();
                        range.hi = range.hi.min(upper);
                    }
                }
            }
        }
        Ok(())
    }
    fn check_observations<'a>(&'a self, checker: &mut Checker<'a>, partitions: &BTreeSet<&str>, rows: &BTreeMap<&str, Interval>, strata: &BTreeMap<&str, (&RunIdentity, Interval)>) -> Result<()> {
        // Observation packets cannot certify one another (including forward
        // references); citations resolve only to independently supplied facts.
        let certificate_ids: BTreeSet<_> = checker.counts.keys().copied().collect();
        let mut series = BTreeSet::new();
        for observed in &self.observations.rounds {
            ensure(series.insert(&observed.relation), "duplicate round-observation series")?;
            let (_, recursive) = self.recursive_relation(&observed.relation)?;
            ensure(observed.stratum == recursive.stratum && observed.run == recursive.run, "round observation stratum/run mismatch")?;
            let mut previous = 0;
            let mut total = 0u128;
            for entry in &observed.entries {
                checker.count(&entry.round, Unit::Rounds, Some(&observed.run))?;
                checker.count(&entry.delta_rows, Unit::Rows, Some(&observed.run))?;
                let round = measured(&entry.round)?;
                let delta = measured(&entry.delta_rows)?;
                ensure(round > previous && (!observed.complete || round == previous + 1), "round observations must be ordered, positive, and contiguous when complete")?;
                ensure(round <= strata[recursive.stratum.as_str()].1.hi, "observation exceeds executed steady rounds")?;
                total = total.checked_add(u128::from(delta)).ok_or_else(|| EvidenceError("observed delta sum overflow".into()))?;
                ensure(delta <= rows[observed.relation.as_str()].hi, "delta exceeds final rows")?;
                previous = round;
            }
            if observed.complete {
                let rounds = strata[recursive.stratum.as_str()].1;
                ensure(rounds.lo <= previous && previous <= rounds.hi, "complete observation round count mismatch")?;
            }
            if observed.run.admission == Admission::InsertOnlySet {
                let final_rows = rows[observed.relation.as_str()];
                let missing = recursive.unconsumed_final_delta_rows.as_ref().map(|f| interval(Some(f)).lo).unwrap_or(0);
                ensure(total + u128::from(missing) <= u128::from(final_rows.hi), "observed deltas plus unconsumed mass exceed final rows")?;
                if observed.complete && partitions.contains(observed.relation.as_str()) {
                    ensure(total >= u128::from(final_rows.lo), "complete delta partition does not cover final rows")?;
                }
            }
        }
        let premise_ids: BTreeSet<_> = self.premises.iter().map(|p| match p { Premise::DeltaSubset { id, .. } | Premise::DeltaPartition { id, .. } => id.as_str() }).collect();
        let mut steps = BTreeSet::new();
        for step in &self.observations.steps {
            nonblank(&step.hole)?; nonblank(&step.variant)?; nonblank(&step.step)?;
            let relation = self.relations.get(&step.relation).ok_or_else(|| EvidenceError("unknown step relation".into()))?;
            step.run.check()?;
            if let Some(recursive) = &relation.recursive { ensure(step.run == recursive.run, "step run mismatch")?; }
            checker.count(&step.rows, Unit::Rows, Some(&step.run))?;
            let value = measured(&step.rows)?;
            let round = if let Some(round) = &step.round {
                let recursive = relation.recursive.as_ref().ok_or_else(|| EvidenceError("steady-round observation requires recursive scope".into()))?;
                checker.count(round, Unit::Rounds, Some(&step.run))?;
                let n = measured(round)?;
                ensure(n > 0 && n <= strata[recursive.stratum.as_str()].1.hi, "step round outside steady execution")?;
                Some(n)
            } else { None };
            ensure(steps.insert((&step.hole, &step.variant, &step.step, round)), "duplicate step observation")?;
            ensure(step.bound.dataset_hash == self.dataset.hash && step.bound.program_hash == step.run.program_hash && step.bound.variant == step.variant, "bound citation dataset/program/variant mismatch")?;
            ensure(certificate_ids.contains(step.bound.fact.as_str()), "bound citation names an observation rather than a supplied certificate")?;
            let bound = checker.counts.get(step.bound.fact.as_str()).ok_or_else(|| EvidenceError("dangling bound citation".into()))?;
            ensure(bound.unit == Unit::Rows && matches!(bound.claim, Claim::Equal | Claim::Upper | Claim::Interval), "citation is not a row upper certificate")?;
            ensure(bound.id != step.rows.id && bound.provenance.run.as_ref().is_none_or(|r| r == &step.run), "self citation or bound run mismatch")?;
            ensure(value <= interval(Some(bound)).hi, "observed step exceeds cited bound")?;
            let mut seen = BTreeSet::new();
            for id in &step.bound.premises { ensure(premise_ids.contains(id.as_str()) && seen.insert(id), "dangling/repeated premise citation")?; }
            // A final-set bound cited for round work needs an explicit subset
            // premise (partition also proves subset); never infer it from names.
            if step.round.is_some() && relation.recursive.as_ref().is_some_and(|recursive| bound.id == recursive.final_rows.id || relation.rows.as_ref().is_some_and(|rows| rows.id == bound.id)) {
                ensure(self.premises.iter().any(|p| match p {
                    Premise::DeltaSubset { id, relations, .. } => step.bound.premises.contains(id) && relations.contains(&step.relation),
                    Premise::DeltaPartition { id, relation, .. } => step.bound.premises.contains(id) && relation == &step.relation,
                }), "final-to-delta bound citation omits subset premise")?;
            }
        }
        let mut physical = BTreeSet::new();
        for step in &self.observations.physical_steps {
            for label in [&step.hole, &step.variant, &step.instance, &step.stratum] { nonblank(label)?; }
            digest(&step.trace_hash)?;
            step.run.check()?;
            ensure(!step.physical_steps.is_empty() && !step.vo_steps.is_empty(), "physical observation lacks operator/VO coverage")?;
            for labels in [&step.physical_steps, &step.vo_steps] {
                let mut unique = BTreeSet::new();
                for label in labels { nonblank(label)?; ensure(unique.insert(label), "duplicate physical/VO coverage label")?; }
            }
            ensure(step.fused == (step.physical_steps.len() > 1 || step.vo_steps.len() > 1), "fused marker disagrees with recorded coverage")?;
            checker.count(&step.rows, Unit::Rows, Some(&step.run))?;
            let value = measured(&step.rows)?;
            let round = if let Some(round) = &step.round {
                checker.count(round, Unit::Rounds, Some(&step.run))?;
                let n = measured(round)?;
                let (run, rounds) = strata.get(step.stratum.as_str()).ok_or_else(|| EvidenceError("physical steady observation has no iterated component".into()))?;
                ensure(*run == &step.run && n > 0 && n <= rounds.hi, "physical observation round/run mismatch")?;
                Some(n)
            } else { None };
            ensure(physical.insert((&step.hole, &step.variant, &step.instance, &step.physical_steps, round)), "duplicate physical observation")?;
            if let Some(comparison) = &step.comparison {
                ensure(step.unknown.is_empty(), "partial physical bound cannot stand for full VO coverage")?;
                checker.rational(&comparison.computed, Unit::Rows)?;
                ensure(comparison.computed.source == Source::Calibrated
                    && comparison.computed.claim == Claim::Observation
                    && comparison.computed.provenance.run.as_ref() == Some(&step.run),
                    "computed physical comparison must remain a run-scoped observation")?;
                let bound = comparison.computed.value.as_ref().unwrap().exact()?;
                ensure(comparison.within_bound == (BigRational::from_integer(value.into()) <= bound), "physical comparison outcome disagrees with its numbers")?;
                let mut citations = BTreeSet::new();
                for id in &comparison.fact_citations {
                    ensure((certificate_ids.contains(id.as_str()) || premise_ids.contains(id.as_str())) && citations.insert(id), "physical bound cites an observation, duplicate, or absent fact")?;
                }
                for id in &comparison.compiler_citations { nonblank(id)?; ensure(citations.insert(id), "duplicate compiler citation")?; }
            } else {
                ensure(!step.unknown.is_empty(), "unknown physical bound requires a recorded reason")?;
                for reason in &step.unknown { nonblank(reason)?; }
            }
        }
        Ok(())
    }
}
fn measured(fact: &CountFact) -> Result<u64> {
    ensure(matches!(fact.claim, Claim::Equal | Claim::Observation) && matches!(fact.source, Source::Exact | Source::Calibrated), "run observation must be an exact/calibrated measured value")?;
    fact.value.ok_or_else(|| EvidenceError("measurement lacks value".into()))
}

impl DatasetEvidence {
    /// Explicit compiler/cost projection. Step diagnostics (including failed
    /// comparisons) cannot certify planning facts; complete delta series remain.
    /// Clone only retained fields, never the potentially large physical packets.
    pub fn planning_facts(&self) -> Self {
        Self {
            schema: self.schema.clone(), dataset: self.dataset.clone(),
            relations: self.relations.clone(), premises: self.premises.clone(),
            relative: self.relative.clone(), targets: self.targets.clone(),
            observations: Observations {
                rounds: self.observations.rounds.clone(),
                steps: Vec::new(), physical_steps: Vec::new(),
            },
        }
    }
}
