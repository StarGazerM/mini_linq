use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use cq_load::stats::{ColumnSpec, DATE_ENCODING, DistinctMode, HASH_MODE, RelationAccumulator, RelationSpec};
use cq_load::stats_source::{SourceFormat, SourceOptions, visit};
use cq_evidence::{
    Admission, Claim, DatasetEvidence, DatasetIdentity, FileRole, InputManifest,
    ManifestFile, ManifestRelation, Provenance, RelationEvidence, RelationScope,
    RelationSemantics, RunIdentity, SCHEMA, Scalar, ScalarDomain, SetCertification,
    Source, Unit,
};

const HASH: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

struct Fixture(PathBuf);
impl Fixture {
    fn path(extension: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "cq-stats-accumulator-{}-{}.{}", std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed), extension,
        )))
    }

    fn csv(text: &str) -> Self {
        let fixture = Self::path("csv");
        std::fs::write(&fixture.0, text).unwrap();
        fixture
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

fn spec(domains: &[(&str, ScalarDomain)], semantics: RelationSemantics) -> RelationSpec {
    RelationSpec {
        name: "R".into(),
        columns: domains.iter().map(|(name, domain)| ColumnSpec { name: (*name).into(), domain: domain.clone() }).collect(),
        semantics,
        key_sets: Vec::new(),
    }
}

fn integer_spec() -> RelationSpec {
    spec(&[("a", ScalarDomain::SignedInteger)], RelationSemantics::Bag)
}

fn options(batch_size: usize) -> SourceOptions {
    SourceOptions {
        format: SourceFormat::Csv, delimiter: b',', headers: false,
        trailing_delimiter: false, batch_size,
    }
}

fn provenance() -> Provenance {
    Provenance {
        producer: "cq-load accumulator test".into(), method: "full stream".into(),
        locator: "input.csv".into(), run: None, artifact_hash: None,
    }
}

fn accumulate(spec: &RelationSpec, text: &str, mode: DistinctMode, batch_size: usize) -> RelationAccumulator {
    let fixture = Fixture::csv(text);
    let mut accumulator = RelationAccumulator::new(spec.clone(), mode).unwrap();
    let names: Vec<_> = spec.columns.iter().map(|c| c.name.as_str()).collect();
    visit(&fixture.0, &names, &options(batch_size), |batch| accumulator.observe_batch(batch)).unwrap();
    accumulator
}

// Check actual emitted envelopes against the shared checker, not a relaxed
// test-only model. The manifest hashes the loaded stream, not raw CSV bytes.
fn finish_checked(spec: &RelationSpec, accumulator: RelationAccumulator, source: Source, provenance: Provenance) -> RelationEvidence {
    let manifest = InputManifest {
        schema: BTreeMap::from([(spec.name.clone(), ManifestRelation {
            semantics: spec.semantics,
            columns: spec.columns.iter().map(|c| (c.name.clone(), c.domain.clone())).collect(),
            null_handling: "reject".into(),
            set_certification: (spec.semantics == RelationSemantics::Set).then_some(SetCertification::FullTupleDeduplicated),
        })]),
        formulation: "statistics-test".into(),
        files: BTreeMap::from([("input.csv".into(), ManifestFile {
            role: FileRole::Data, content_hash: Some(accumulator.content_hash()),
            encoding_hash: HASH.into(), missing_as_empty: false,
        })]),
        dictionaries: spec.columns.iter().filter_map(|c| match &c.domain {
            ScalarDomain::Symbol { dictionary_hash, .. } => Some((c.name.clone(), dictionary_hash.clone())),
            _ => None,
        }).collect(),
        loader_options: BTreeMap::from([("hash_mode".into(), HASH_MODE.into())]),
    };
    let dataset_hash = manifest.dataset_hash().unwrap();
    let relation = accumulator.finish(&dataset_hash, source, provenance).unwrap();
    let document = DatasetEvidence {
        schema: SCHEMA.into(),
        dataset: DatasetIdentity {
            name: "tiny-stream".into(), hash: dataset_hash, date: "2026-11-05".into(),
            schema_hash: manifest.schema_hash().unwrap(), formulation: manifest.formulation.clone(), manifest,
        },
        relations: BTreeMap::from([(spec.name.clone(), relation.clone())]),
        premises: Vec::new(), relative: Vec::new(), targets: BTreeMap::new(), observations: Default::default(),
    };
    document.check().unwrap();
    assert_eq!(cq_evidence::parse_json(&document.canonical_json().unwrap()).unwrap(), document);
    relation
}

fn exact(spec: &RelationSpec, text: &str) -> RelationEvidence {
    finish_checked(spec, accumulate(spec, text, DistinctMode::Exact, 2), Source::Exact, provenance())
}

#[test]
fn bag_and_set_counts_use_full_tuple_deduplication() {
    let mut schema = spec(&[("a", ScalarDomain::SignedInteger), ("b", ScalarDomain::Utf8)], RelationSemantics::Bag);
    schema.key_sets = vec![vec!["a".into(), "b".into()], vec!["a".into()]];
    let text = "1,x\n1,x\n1,y\n2,x\n2,z\n2,z\n";
    let bag_acc = accumulate(&schema, text, DistinctMode::Exact, 1);
    assert_eq!(bag_acc.source_rows(), 6);
    let hash = bag_acc.content_hash();
    let bag = finish_checked(&schema, bag_acc, Source::Exact, provenance());
    assert_eq!(bag.rows.as_ref().unwrap().value, Some(6));
    assert_eq!(bag.columns["a"].distinct.as_ref().unwrap().value, Some(2));
    assert_eq!(bag.columns["a"].max_degree.as_ref().unwrap().value, Some(3));
    assert_eq!(bag.columns["b"].distinct.as_ref().unwrap().value, Some(3));
    assert_eq!(bag.columns["b"].max_degree.as_ref().unwrap().value, Some(3));
    assert_eq!(bag.key_sets[0].distinct.as_ref().unwrap().value, Some(4));
    assert_eq!(bag.key_sets[0].max_degree.as_ref().unwrap().value, Some(2));
    assert_eq!(bag.key_sets[1].max_degree.as_ref().unwrap().value, Some(3));

    schema.semantics = RelationSemantics::Set;
    let set_acc = accumulate(&schema, text, DistinctMode::Exact, 3);
    assert_eq!(set_acc.source_rows(), 6);
    assert_eq!(set_acc.content_hash(), hash);
    let set = finish_checked(&schema, set_acc, Source::Exact, provenance());
    assert_eq!(set.rows.as_ref().unwrap().value, Some(4));
    assert_eq!(set.columns["a"].distinct.as_ref().unwrap().value, Some(2));
    assert_eq!(set.columns["a"].max_degree.as_ref().unwrap().value, Some(2));
    assert_eq!(set.columns["b"].max_degree.as_ref().unwrap().value, Some(2));
    assert_eq!(set.key_sets[0].distinct.as_ref().unwrap().value, Some(4));
    assert_eq!(set.key_sets[0].max_degree.as_ref().unwrap().value, Some(1));
    assert_eq!(set.key_sets[1].max_degree.as_ref().unwrap().value, Some(2));
    assert_eq!(set.semantics, RelationSemantics::Set);
    assert_eq!(set.scope, RelationScope::Input);
    assert!(set.recursive.is_none());
    assert!(set.keys.is_empty());
}

#[test]
fn empty_stream_has_zero_counts_and_no_invented_extrema() {
    for semantics in [RelationSemantics::Bag, RelationSemantics::Set] {
        for mode in [DistinctMode::Exact, DistinctMode::Hll { precision: 4 }] {
            let mut schema = integer_spec();
            schema.semantics = semantics;
            schema.key_sets = vec![vec!["a".into()]];
            let accumulator = accumulate(&schema, "", mode, 2);
            assert_eq!(accumulator.source_rows(), 0);
            let relation = finish_checked(&schema, accumulator, Source::Exact, provenance());
            assert_eq!(relation.rows.unwrap().value, Some(0));
            let column = &relation.columns["a"];
            assert_eq!(column.distinct.as_ref().unwrap().value, Some(0));
            assert_eq!(column.max_degree.as_ref().unwrap().value, Some(0));
            assert!(column.min.is_none() && column.max.is_none());
            assert_eq!(relation.key_sets[0].distinct.as_ref().unwrap().value, Some(0));
            assert_eq!(relation.key_sets[0].max_degree.as_ref().unwrap().value, Some(0));
        }
    }
}

#[test]
fn extrema_use_typed_order_and_exact_load_conversions() {
    let schema = spec(&[
        ("integer", ScalarDomain::SignedInteger),
        ("decimal", ScalarDomain::Decimal { scale: 2 }),
        ("day", ScalarDomain::Date { encoding: DATE_ENCODING.into() }),
        ("text", ScalarDomain::Utf8),
        ("symbol", ScalarDomain::Symbol { dictionary_hash: HASH.into(), ordered: true }),
    ], RelationSemantics::Bag);
    let relation = exact(&schema, "-9223372036854775808,-2.5000,1969-12-31,é,18446744073709551615\n9223372036854775807,+10.0,1970-01-02,,0\n+0001,10.00,1970-01-01,aa,1\n");
    let c = &relation.columns;
    assert_eq!(c["integer"].min.as_ref().unwrap().value, Some(Scalar::SignedInteger { value: i64::MIN }));
    assert_eq!(c["integer"].max.as_ref().unwrap().value, Some(Scalar::SignedInteger { value: i64::MAX }));
    assert_eq!(c["decimal"].min.as_ref().unwrap().value, Some(Scalar::Decimal { unscaled: "-250".into(), scale: 2 }));
    assert_eq!(c["decimal"].max.as_ref().unwrap().value, Some(Scalar::Decimal { unscaled: "1000".into(), scale: 2 }));
    assert_eq!(c["decimal"].distinct.as_ref().unwrap().value, Some(2));
    assert_eq!(c["decimal"].max_degree.as_ref().unwrap().value, Some(2));
    assert_eq!(c["day"].min.as_ref().unwrap().value, Some(Scalar::Date { value: -1, encoding: DATE_ENCODING.into() }));
    assert_eq!(c["day"].max.as_ref().unwrap().value, Some(Scalar::Date { value: 1, encoding: DATE_ENCODING.into() }));
    assert_eq!(c["text"].min.as_ref().unwrap().value, Some(Scalar::Utf8 { value: "".into() }));
    assert_eq!(c["text"].max.as_ref().unwrap().value, Some(Scalar::Utf8 { value: "é".into() }));
    assert_eq!(c["symbol"].min.as_ref().unwrap().value, Some(Scalar::Symbol { code: 0, dictionary_hash: HASH.into() }));
    assert_eq!(c["symbol"].max.as_ref().unwrap().value, Some(Scalar::Symbol { code: u64::MAX, dictionary_hash: HASH.into() }));
}

#[test]
fn every_supported_decimal_scale_uses_exact_scaled_coefficients() {
    for scale in 0..=18 {
        let schema = spec(&[("d", ScalarDomain::Decimal { scale })], RelationSemantics::Bag);
        let relation = exact(&schema, "1\n+1.0000000000000000000\n-1\n");
        let column = &relation.columns["d"];
        assert_eq!(column.distinct.as_ref().unwrap().value, Some(2));
        assert_eq!(column.max_degree.as_ref().unwrap().value, Some(2));
        assert_eq!(column.min.as_ref().unwrap().value, Some(Scalar::Decimal { unscaled: (-10_i64.pow(scale)).to_string(), scale }));
    }
}

#[test]
fn unordered_symbols_have_counts_but_never_extrema() {
    let schema = spec(&[("s", ScalarDomain::Symbol { dictionary_hash: HASH.into(), ordered: false })], RelationSemantics::Bag);
    let relation = exact(&schema, "2\n1\n2\n");
    let column = &relation.columns["s"];
    assert_eq!(column.distinct.as_ref().unwrap().value, Some(2));
    assert_eq!(column.max_degree.as_ref().unwrap().value, Some(2));
    assert!(column.min.is_none() && column.max.is_none());
}

#[test]
fn skew_and_composite_keys_preserve_correlations() {
    let mut schema = spec(&[("a", ScalarDomain::SignedInteger), ("b", ScalarDomain::Utf8)], RelationSemantics::Bag);
    schema.key_sets = vec![vec!["b".into(), "a".into()]];
    let mut text = "7,repeat\n".repeat(257);
    text.push_str("7,other\n8,repeat\n9,third\n");
    let relation = exact(&schema, &text);
    assert_eq!(relation.rows.unwrap().value, Some(260));
    assert_eq!(relation.columns["a"].max_degree.as_ref().unwrap().value, Some(258));
    assert_eq!(relation.columns["b"].max_degree.as_ref().unwrap().value, Some(258));
    assert_eq!(relation.key_sets[0].max_degree.as_ref().unwrap().value, Some(257));
    assert_eq!(relation.key_sets[0].distinct.as_ref().unwrap().value, Some(4));
}

#[test]
fn canonical_stream_digest_has_a_pinned_cross_language_fixture() {
    let schema = spec(&[("a", ScalarDomain::SignedInteger), ("b", ScalarDomain::SignedInteger)], RelationSemantics::Bag);
    let accumulator = accumulate(&schema, "1,1\n1,2\n2,3\n2,4\n", DistinctMode::Exact, 2);
    assert_eq!(accumulator.content_hash(), "sha256:44b7ca5a9a60a2a3b7b52e6291304e7f150553472ca54de2507c0a62a25fdb10");
    assert_eq!(accumulator.content_hash(), accumulator.content_hash());
    assert_eq!(accumulator.source_rows(), 4);
}

#[test]
fn content_digest_is_loaded_typed_ordered_and_length_framed() {
    let schema = integer_spec();
    let hash = |spec: &RelationSpec, text: &str, size| accumulate(spec, text, DistinctMode::Exact, size).content_hash();
    assert_eq!(hash(&schema, "1\n2\n", 1), hash(&schema, "+001\n02\n", 50));
    assert_ne!(hash(&schema, "1\n2\n", 1), hash(&schema, "2\n1\n", 1));
    assert_ne!(hash(&schema, "1\n2\n", 1), hash(&schema, "1\n1\n2\n", 1));
    let text_schema = spec(&[("a", ScalarDomain::Utf8)], RelationSemantics::Bag);
    assert_ne!(hash(&schema, "1\n", 1), hash(&text_schema, "1\n", 1));
    let decimal = spec(&[("a", ScalarDomain::Decimal { scale: 0 })], RelationSemantics::Bag);
    assert_ne!(hash(&schema, "1\n", 1), hash(&decimal, "1\n", 1));
    assert_eq!(hash(&decimal, "1.000\n", 1), hash(&decimal, "+01\n", 1));
    let decimal2 = spec(&[("a", ScalarDomain::Decimal { scale: 2 })], RelationSemantics::Bag);
    assert_ne!(hash(&decimal, "1\n", 1), hash(&decimal2, "0.01\n", 1));
    let date = spec(&[("a", ScalarDomain::Date { encoding: DATE_ENCODING.into() })], RelationSemantics::Bag);
    assert_ne!(hash(&schema, "0\n", 1), hash(&date, "1970-01-01\n", 1));
    let symbol = spec(&[("a", ScalarDomain::Symbol { dictionary_hash: HASH.into(), ordered: true })], RelationSemantics::Bag);
    assert_ne!(hash(&schema, "1\n", 1), hash(&symbol, "1\n", 1));
    let text_pair = spec(&[("a", ScalarDomain::Utf8), ("b", ScalarDomain::Utf8)], RelationSemantics::Bag);
    assert_ne!(hash(&text_pair, "ab,c\n", 1), hash(&text_pair, "a,bc\n", 1));
    assert_ne!(hash(&schema, "", 1), hash(&text_pair, "", 1));
    // Column names/dictionary identity are manifest schema, not stream payload.
    let mut renamed = schema.clone();
    renamed.columns[0].name = "renamed".into();
    assert_eq!(hash(&schema, "1\n", 1), hash(&renamed, "1\n", 1));
}

#[test]
fn content_hash_snapshot_does_not_finalize_or_reset_the_stream() {
    let schema = integer_spec();
    let first = Fixture::csv("1\n2\n");
    let second = Fixture::csv("3\n");
    let mut accumulator = RelationAccumulator::new(schema.clone(), DistinctMode::Exact).unwrap();
    visit(&first.0, &["a"], &options(1), |b| accumulator.observe_batch(b)).unwrap();
    let prefix = accumulator.content_hash();
    assert_eq!(prefix, accumulate(&schema, "1\n2\n", DistinctMode::Exact, 2).content_hash());
    visit(&second.0, &["a"], &options(1), |b| accumulator.observe_batch(b)).unwrap();
    assert_ne!(prefix, accumulator.content_hash());
    assert_eq!(accumulator.content_hash(), accumulate(&schema, "1\n2\n3\n", DistinctMode::Exact, 3).content_hash());
}

#[test]
fn hll_is_deterministic_full_stream_observation_with_exact_degrees() {
    let mut schema = spec(&[("a", ScalarDomain::SignedInteger), ("b", ScalarDomain::Utf8)], RelationSemantics::Bag);
    schema.key_sets = vec![vec!["a".into(), "b".into()]];
    let mut text = "7,repeated\n".repeat(200);
    for n in 0..1000 { text.push_str(&format!("{n},v{n}\n")); }
    let mode = DistinctMode::Hll { precision: 12 };
    let one = finish_checked(&schema, accumulate(&schema, &text, mode, 1), Source::Exact, provenance());
    let many = finish_checked(&schema, accumulate(&schema, &text, mode, 127), Source::Exact, provenance());
    assert_eq!(one, many);
    assert_eq!(one.rows.as_ref().unwrap().value, Some(1200));
    assert_eq!(one.columns["a"].max_degree.as_ref().unwrap().value, Some(201));
    assert_eq!(one.columns["b"].max_degree.as_ref().unwrap().value, Some(200));
    assert_eq!(one.key_sets[0].max_degree.as_ref().unwrap().value, Some(200));
    assert_eq!(one.columns["a"].max_degree.as_ref().unwrap().source, Source::Exact);
    assert_eq!(one.columns["a"].min.as_ref().unwrap().source, Source::Exact);
    for observation in one.columns.values().map(|c| c.distinct.as_ref().unwrap()).chain(one.key_sets.iter().map(|k| k.distinct.as_ref().unwrap())) {
        assert_eq!(observation.source, Source::Hll);
        assert_eq!(observation.claim, Claim::Observation);
        assert_eq!(observation.unit, Unit::Distinct);
        assert!(observation.lower.is_none() && observation.upper.is_none());
        assert!(observation.provenance.method.contains("algorithm=hyperloglog-v1"));
        assert!(observation.provenance.method.contains("precision=12"));
        assert!(observation.provenance.method.contains("hash_seed=0"));
        assert!(observation.provenance.method.contains("stream=full-loaded-row-stream"));
        assert!((850..=1150).contains(&observation.value.unwrap()));
    }
    assert!(one.keys.is_empty());
    let reversed = text.lines().rev().collect::<Vec<_>>().join("\n");
    let reversed = finish_checked(&schema, accumulate(&schema, &reversed, mode, 51), Source::Exact, provenance());
    assert_eq!(one.columns["a"].distinct.as_ref().unwrap().value, reversed.columns["a"].distinct.as_ref().unwrap().value);
    assert_eq!(one.key_sets[0].distinct.as_ref().unwrap().value, reversed.key_sets[0].distinct.as_ref().unwrap().value);
    schema.semantics = RelationSemantics::Set;
    let set = finish_checked(&schema, accumulate(&schema, &text, mode, 37), Source::Exact, provenance());
    assert_eq!(set.rows.as_ref().unwrap().value, Some(1001));
    assert_eq!(set.columns["a"].max_degree.as_ref().unwrap().value, Some(2));
    assert_eq!(set.key_sets[0].max_degree.as_ref().unwrap().value, Some(1));
    assert_eq!(one.columns["a"].distinct.as_ref().unwrap().value, set.columns["a"].distinct.as_ref().unwrap().value);
    assert_eq!(one.key_sets[0].distinct.as_ref().unwrap().value, set.key_sets[0].distinct.as_ref().unwrap().value);
}

#[test]
fn every_numeric_fact_carries_unique_identity_source_and_provenance() {
    let mut schema = spec(&[("a", ScalarDomain::SignedInteger), ("b", ScalarDomain::Utf8)], RelationSemantics::Bag);
    schema.name = "R:column:0:rows".into();
    schema.key_sets = vec![vec!["b".into(), "a".into()]];
    let mut origin = provenance();
    origin.run = Some(RunIdentity {
        program_hash: HASH.into(), configuration_hash: HASH.into(), admission: Admission::Bag,
        run_id: "calibration-1".into(), completed: true,
    });
    let relation = finish_checked(&schema, accumulate(&schema, "1,a\n2,b\n", DistinctMode::Exact, 1), Source::Calibrated, origin.clone());
    let json = serde_json::to_value(&relation).unwrap();
    let dataset_hash = relation.rows.as_ref().unwrap().dataset_hash.as_str();
    fn inspect(value: &serde_json::Value, ids: &mut BTreeSet<String>, hash: &str, origin: &Provenance) {
        if let Some(object) = value.as_object() {
            if let Some(id) = object.get("id") {
                assert!(ids.insert(id.as_str().unwrap().to_owned()));
                assert_eq!(object["dataset_hash"], hash);
                assert_eq!(object["source"], "calibrated");
                assert_eq!(object["claim"], "equal");
                assert_eq!(object["provenance"], serde_json::to_value(origin).unwrap());
                assert!(object["lower"].is_null() && object["upper"].is_null());
            }
            for child in object.values() { inspect(child, ids, hash, origin); }
        } else if let Some(array) = value.as_array() {
            for child in array { inspect(child, ids, hash, origin); }
        }
    }
    let mut ids = BTreeSet::new();
    inspect(&json, &mut ids, dataset_hash, &origin);
    assert_eq!(ids.len(), 11); // rows + four facts per column + two per key set
    assert!(relation.keys.is_empty());
}

#[test]
fn invalid_schema_and_unknown_json_fields_are_rejected() {
    let base = integer_spec();
    base.check(&DistinctMode::Exact).unwrap();
    base.check(&DistinctMode::Hll { precision: 4 }).unwrap();
    base.check(&DistinctMode::Hll { precision: 18 }).unwrap();
    let mut invalid = Vec::new();
    let mut s = base.clone(); s.name = " ".into(); invalid.push(s);
    let mut s = base.clone(); s.columns.clear(); invalid.push(s);
    let mut s = base.clone(); s.columns[0].name = "".into(); invalid.push(s);
    let mut s = base.clone(); s.columns.push(s.columns[0].clone()); invalid.push(s);
    let mut s = base.clone(); s.key_sets = vec![vec![]]; invalid.push(s);
    let mut s = base.clone(); s.key_sets = vec![vec!["missing".into()]]; invalid.push(s);
    let mut s = base.clone(); s.key_sets = vec![vec!["a".into(), "a".into()]]; invalid.push(s);
    let mut s = base.clone(); s.key_sets = vec![vec!["a".into()], vec!["a".into()]]; invalid.push(s);
    let mut s = spec(&[("a", ScalarDomain::SignedInteger), ("b", ScalarDomain::SignedInteger)], RelationSemantics::Bag);
    s.key_sets = vec![vec!["a".into(), "b".into()], vec!["b".into(), "a".into()]]; invalid.push(s);
    for domain in [
        ScalarDomain::Decimal { scale: 19 },
        ScalarDomain::Date { encoding: "unix-milliseconds".into() },
        ScalarDomain::Symbol { dictionary_hash: "unknown".into(), ordered: true },
        ScalarDomain::Unordered { limitation: "IEEE NaN is not an equality key".into() },
    ] {
        invalid.push(spec(&[("a", domain)], RelationSemantics::Bag));
    }
    for s in invalid {
        assert!(s.check(&DistinctMode::Exact).is_err());
        assert!(RelationAccumulator::new(s, DistinctMode::Exact).is_err());
    }
    for precision in [0, 3, 19, 64, 255] {
        assert!(base.check(&DistinctMode::Hll { precision }).is_err());
        assert!(RelationAccumulator::new(base.clone(), DistinctMode::Hll { precision }).is_err());
    }
    let mut json = serde_json::to_value(&base).unwrap();
    json["unexpected"] = true.into();
    assert!(serde_json::from_value::<RelationSpec>(json).is_err());
    let mut json = serde_json::to_value(&base).unwrap();
    json["columns"][0]["unexpected"] = true.into();
    assert!(serde_json::from_value::<RelationSpec>(json).is_err());
}

#[test]
fn invalid_loaded_values_poison_statistics_instead_of_certifying_prefixes() {
    let cases = [
        (ScalarDomain::SignedInteger, "1\n9223372036854775808\n"),
        (ScalarDomain::SignedInteger, "1\n 2\n"),
        (ScalarDomain::SignedInteger, "1\nNaN\n"),
        (ScalarDomain::Decimal { scale: 2 }, "1\n1.001\n"),
        (ScalarDomain::Decimal { scale: 2 }, "1\n9223372036854775808\n"),
        (ScalarDomain::Date { encoding: DATE_ENCODING.into() }, "1970-01-01\n2025-02-29\n"),
        (ScalarDomain::Symbol { dictionary_hash: HASH.into(), ordered: false }, "1\n-1\n"),
    ];
    for (domain, text) in cases {
        let schema = spec(&[("a", domain)], RelationSemantics::Bag);
        let fixture = Fixture::csv(text);
        let mut accumulator = RelationAccumulator::new(schema, DistinctMode::Exact).unwrap();
        let error = visit(&fixture.0, &["a"], &options(1), |b| accumulator.observe_batch(b)).unwrap_err();
        assert_eq!(error.column, "a");
        assert_eq!(error.line, 2);
        assert!(accumulator.finish(HASH, Source::Exact, provenance()).is_err());
    }
}

#[test]
fn mismatched_batch_dimensions_and_names_are_rejected() {
    for (names, text) in [(vec!["a", "b"], "1,2\n"), (vec!["wrong"], "1\n")] {
        let fixture = Fixture::csv(text);
        let mut accumulator = RelationAccumulator::new(integer_spec(), DistinctMode::Exact).unwrap();
        let error = visit(&fixture.0, &names, &options(1), |b| accumulator.observe_batch(b)).unwrap_err();
        assert!(error.message.contains("batch schema"));
        assert!(accumulator.finish(HASH, Source::Exact, provenance()).is_err());
    }
}

#[test]
fn invalid_fact_origins_cannot_finish() {
    let fresh = || RelationAccumulator::new(integer_spec(), DistinctMode::Exact).unwrap();
    for source in [Source::Hll, Source::Bound, Source::Declared, Source::Calibrated] {
        assert!(fresh().finish(HASH, source, provenance()).is_err());
    }
    assert!(fresh().finish("bad-hash", Source::Exact, provenance()).is_err());
    let mut origin = provenance(); origin.method.clear();
    assert!(fresh().finish(HASH, Source::Exact, origin).is_err());
    let mut origin = provenance(); origin.artifact_hash = Some("bad-hash".into());
    assert!(fresh().finish(HASH, Source::Exact, origin).is_err());
    let mut origin = provenance(); origin.run = Some(RunIdentity {
        program_hash: HASH.into(), configuration_hash: HASH.into(), admission: Admission::Bag,
        run_id: "incomplete".into(), completed: false,
    });
    assert!(fresh().finish(HASH, Source::Calibrated, origin).is_err());
}

#[cfg(feature = "parquet")]
mod parquet_cases {
    use super::*;
    use arrow_array::{ArrayRef, Date32Array, Decimal128Array, Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    fn parquet(fields: Vec<Field>, arrays: Vec<ArrayRef>) -> Fixture {
        let fixture = Fixture::path("parquet");
        let schema = Arc::new(Schema::new(fields));
        let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
        let mut writer = ArrowWriter::try_new(std::fs::File::create(&fixture.0).unwrap(), schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        fixture
    }

    #[test]
    fn native_arrow_conversion_matches_csv_values_and_digest() {
        let schema = spec(&[
            ("i", ScalarDomain::SignedInteger),
            ("d", ScalarDomain::Decimal { scale: 2 }),
            ("day", ScalarDomain::Date { encoding: DATE_ENCODING.into() }),
            ("s", ScalarDomain::Utf8),
        ], RelationSemantics::Bag);
        // Physical column order deliberately differs from declaration order.
        let fixture = parquet(vec![
            Field::new("s", DataType::Utf8, false),
            Field::new("day", DataType::Date32, false),
            Field::new("d", DataType::Decimal128(18, 3), false),
            Field::new("i", DataType::Int64, false),
        ], vec![
            Arc::new(StringArray::from(vec!["same", "", "same"])),
            Arc::new(Date32Array::from(vec![-1, 0, 1])),
            Arc::new(Decimal128Array::from(vec![1230, -2500, 1230]).with_precision_and_scale(18, 3).unwrap()),
            Arc::new(Int64Array::from(vec![1, -2, 1])),
        ]);
        let names: Vec<_> = schema.columns.iter().map(|c| c.name.as_str()).collect();
        let mut accumulator = RelationAccumulator::new(schema.clone(), DistinctMode::Exact).unwrap();
        let mut config = options(1); config.format = SourceFormat::Parquet;
        visit(&fixture.0, &names, &config, |b| accumulator.observe_batch(b)).unwrap();
        let csv = accumulate(&schema, "1,1.23,1969-12-31,same\n-2,-2.5,1970-01-01,\n1,1.230,1970-01-02,same\n", DistinctMode::Exact, 2);
        assert_eq!(accumulator.content_hash(), csv.content_hash());
        assert_eq!(finish_checked(&schema, accumulator, Source::Exact, provenance()), finish_checked(&schema, csv, Source::Exact, provenance()));
    }

    #[test]
    fn nulls_and_lossy_decimal_rescaling_are_errors() {
        let fixtures = [
            (integer_spec(), parquet(vec![Field::new("a", DataType::Int64, true)], vec![Arc::new(Int64Array::from(vec![Some(1), None]))])),
            (spec(&[("a", ScalarDomain::Utf8)], RelationSemantics::Bag), parquet(vec![Field::new("a", DataType::Utf8, true)], vec![Arc::new(StringArray::from(vec![Some("ok"), None]))])),
            (spec(&[("a", ScalarDomain::Decimal { scale: 2 })], RelationSemantics::Bag), parquet(vec![Field::new("a", DataType::Decimal128(18, 3), false)], vec![Arc::new(Decimal128Array::from(vec![1000, 1001]).with_precision_and_scale(18, 3).unwrap())])),
        ];
        for (schema, fixture) in fixtures {
            let mut accumulator = RelationAccumulator::new(schema, DistinctMode::Exact).unwrap();
            let mut config = options(1); config.format = SourceFormat::Parquet;
            assert!(visit(&fixture.0, &["a"], &config, |b| accumulator.observe_batch(b)).is_err());
            assert!(accumulator.finish(HASH, Source::Exact, provenance()).is_err());
        }
    }
}

#[test]
fn compact_summaries_bind_only_the_final_dataset_identity() {
    let mut schema = spec(&[("a", ScalarDomain::SignedInteger), ("b", ScalarDomain::Utf8)], RelationSemantics::Set);
    schema.key_sets = vec![vec!["a".into(), "b".into()]];
    let text = "1,x\n1,x\n1,y\n2,z\n";
    for mode in [DistinctMode::Exact, DistinctMode::Hll { precision: 8 }] {
        let direct = accumulate(&schema, text, mode, 1).finish(HASH, Source::Exact, provenance()).unwrap();
        let summary = accumulate(&schema, text, mode, 3).summarize().unwrap();
        let bound = summary.bind(HASH, Source::Exact, provenance()).unwrap();
        assert_eq!(direct, bound);
        assert_eq!(bound.rows.as_ref().unwrap().value, Some(3));
        assert_eq!(bound.key_sets[0].max_degree.as_ref().unwrap().value, Some(1));
    }
    let summary = accumulate(&schema, text, DistinctMode::Exact, 2).summarize().unwrap();
    assert!(summary.bind("not-a-dataset-hash", Source::Exact, provenance()).is_err());
}

#[cfg(feature = "planner-calibration")]
mod canonical {
    use super::*;
    use cq_load::stats::CanonicalAccumulator;

    #[test]
    fn compiler_admission_matches_loader_stats_and_raw_content_identity() {
        let mut spec = spec(
            &[("a", ScalarDomain::SignedInteger), ("b", ScalarDomain::Utf8)],
            RelationSemantics::Set,
        );
        spec.key_sets = vec![vec!["a".into(), "b".into()]];
        let source = [(1_i32, "x"), (1, "x"), (1, "y"), (2, "x")];
        let mut canonical = CanonicalAccumulator::new(spec.clone()).unwrap();
        let mut actual_set = BTreeSet::new();
        for row in source {
            canonical.raw(&row).unwrap();
            if actual_set.insert(row) { canonical.admitted(&row).unwrap(); }
        }
        let ordinary = accumulate(&spec, "1,x\n1,x\n1,y\n2,x\n", DistinctMode::Exact, 2);
        assert_eq!(canonical.content_hash(), ordinary.content_hash());
        assert_eq!(canonical.source_rows(), 4);
        let observed = canonical.finish(actual_set.len()).unwrap().bind(HASH, Source::Exact, provenance()).unwrap();
        let expected = ordinary.finish(HASH, Source::Exact, provenance()).unwrap();
        assert_eq!(observed, expected);
    }

    #[test]
    fn canonical_empty_stream_is_exact() {
        let spec = spec(&[("a", ScalarDomain::SignedInteger)], RelationSemantics::Set);
        let canonical = CanonicalAccumulator::new(spec.clone()).unwrap();
        let ordinary = accumulate(&spec, "", DistinctMode::Exact, 1);
        assert_eq!(canonical.content_hash(), ordinary.content_hash());
        assert_eq!(
            canonical.finish(0).unwrap().bind(HASH, Source::Exact, provenance()).unwrap(),
            ordinary.finish(HASH, Source::Exact, provenance()).unwrap(),
        );
    }

    #[test]
    fn canonical_observation_errors_cannot_publish_partial_statistics() {
        let make = || CanonicalAccumulator::new(spec(
            &[("a", ScalarDomain::SignedInteger)], RelationSemantics::Set,
        )).unwrap();
        let mut repeated = make();
        repeated.raw(&(1_i32,)).unwrap();
        repeated.admitted(&(1_i32,)).unwrap();
        assert!(repeated.admitted(&(1_i32,)).is_err());
        assert!(repeated.finish(1).is_err());
        let mut wrong = make();
        wrong.raw(&(1_i32,)).unwrap();
        assert!(wrong.admitted(&(2_i32,)).is_err());
        assert!(wrong.finish(1).is_err());
        let mut missing = make();
        missing.raw(&(1_i32,)).unwrap();
        assert!(missing.finish(1).is_err());
        let mut unsupported = make();
        assert!(unsupported.raw(&(vec![1_i64],)).is_err());
        assert!(unsupported.finish(0).is_err());
        let mut domain = make();
        assert!(domain.raw(&("not an integer",)).is_err());
        assert!(domain.finish(0).is_err());
    }
}
