use cq_load::{stats::{ColumnSpec, DistinctMode, RelationSpec}, stats_manifest::{DatasetRequest, MetadataInput, RelationInput, REQUEST_SCHEMA}, stats_source::SourceOptions};
use cq_syntax::evidence::{self, Claim, FileRole, RelationSemantics, ScalarDomain, Source};
use std::{path::PathBuf, sync::atomic::{AtomicUsize, Ordering}};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("cq-stats-manifest-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("R.csv"), "1,1\n1,2\n2,3\n2,4\n").unwrap();
        std::fs::write(path.join("meta.json"), "{\"constant\":7}\n").unwrap();
        Self(path)
    }
}
impl Drop for Fixture { fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); } }
fn request() -> DatasetRequest {
    DatasetRequest {
        schema: REQUEST_SCHEMA.into(), name: "tiny".into(), date: "2026-11-05".into(),
        formulation: "test-query".into(), hash_mode: evidence::HASH_MODE.into(), distinct: DistinctMode::Exact,
        relations: vec![RelationInput {
            relation: RelationSpec { name: "R".into(), semantics: RelationSemantics::Bag,
                columns: ["a", "b"].into_iter().map(|name| ColumnSpec { name: name.into(), domain: ScalarDomain::SignedInteger }).collect(),
                key_sets: vec![vec!["a".into(), "b".into()]], },
            path: "R.csv".into(), options: SourceOptions::default(), missing_as_empty: false,
        }],
        metadata: vec![MetadataInput { path: "meta.json".into(), role: FileRole::Metadata }],
    }
}

#[test]
fn collected_file_checks_with_exact_statistics_and_pinned_typed_identity() {
    let files = Fixture::new();
    let result = request().collect(&files.0).unwrap();
    let parsed = evidence::parse_json(&result.canonical_json().unwrap()).unwrap();
    assert_eq!(parsed, result);
    let r = &result.relations["R"];
    assert_eq!(r.rows.as_ref().unwrap().value, Some(4));
    assert_eq!(r.columns["a"].distinct.as_ref().unwrap().value, Some(2));
    assert_eq!(r.columns["a"].max_degree.as_ref().unwrap().value, Some(2));
    assert_eq!(r.columns["b"].max_degree.as_ref().unwrap().value, Some(1));
    assert_eq!(r.key_sets[0].max_degree.as_ref().unwrap().value, Some(1));
    assert_eq!(result.dataset.manifest.loader_options["hash_mode"], evidence::HASH_MODE);
    assert_eq!(result.dataset.manifest.files["R.csv"].content_hash.as_deref(), Some("sha256:44b7ca5a9a60a2a3b7b52e6291304e7f150553472ca54de2507c0a62a25fdb10"));
    assert!(r.recursive.is_none(), "input stats do not invent a fixpoint");
    assert!(result.targets.is_empty(), "stats do not invent hardware unit costs");
}

#[test]
fn content_identity_uses_decoded_values_and_binds_metadata_order_and_duplicates() {
    let files = Fixture::new();
    let request = request();
    let original = request.collect(&files.0).unwrap();
    std::fs::write(files.0.join("R.csv"), "01,1\r\n1,02\r\n2,3\r\n2,4\r\n").unwrap();
    assert_eq!(original.dataset.hash, request.collect(&files.0).unwrap().dataset.hash);
    std::fs::write(files.0.join("R.csv"), "2,4\n1,1\n1,2\n2,3\n").unwrap();
    assert_ne!(original.dataset.hash, request.collect(&files.0).unwrap().dataset.hash);
    std::fs::write(files.0.join("R.csv"), "1,1\n1,2\n2,3\n2,4\n1,1\n").unwrap();
    let mut set = request.clone(); set.relations[0].relation.semantics = RelationSemantics::Set;
    let duplicate = set.collect(&files.0).unwrap();
    assert_eq!(duplicate.relations["R"].rows.as_ref().unwrap().value, Some(4));
    std::fs::write(files.0.join("R.csv"), "1,1\n1,2\n2,3\n2,4\n").unwrap();
    let unique = set.collect(&files.0).unwrap();
    assert_ne!(unique.dataset.hash, duplicate.dataset.hash, "dedup cannot erase source identity");
    std::fs::write(files.0.join("meta.json"), "{\"constant\":8}\n").unwrap();
    assert_ne!(unique.dataset.hash, set.collect(&files.0).unwrap().dataset.hash);
}

#[test]
fn hll_is_observational_and_does_not_replace_exact_degrees() {
    let files = Fixture::new();
    let mut request = request(); request.distinct = DistinctMode::Hll { precision: 10 };
    let result = request.collect(&files.0).unwrap();
    for column in result.relations["R"].columns.values() {
        let distinct = column.distinct.as_ref().unwrap();
        assert_eq!(distinct.source, Source::Hll); assert_eq!(distinct.claim, Claim::Observation);
        assert_eq!(column.max_degree.as_ref().unwrap().source, Source::Exact);
    }
    result.check().unwrap();
}

#[test]
fn missing_files_are_empty_only_under_the_recorded_policy() {
    let files = Fixture::new();
    let mut request = request(); request.relations[0].path = "absent.csv".into();
    assert!(request.collect(&files.0).is_err());
    request.relations[0].missing_as_empty = true;
    let result = request.collect(&files.0).unwrap();
    assert_eq!(result.relations["R"].rows.as_ref().unwrap().value, Some(0));
    assert!(result.relations["R"].columns["a"].min.is_none());
    assert!(result.dataset.manifest.files["absent.csv"].missing_as_empty);
    assert!(result.dataset.manifest.files["absent.csv"].content_hash.is_none());
}

#[test]
fn invalid_requests_stop_before_data_acquisition() {
    for mutation in 0..8 {
        let mut r = request();
        match mutation {
            0 => r.hash_mode = "raw-file-sha256".into(),
            1 => r.relations[0].path = "../R.csv".into(),
            2 => r.relations.push(r.relations[0].clone()),
            3 => r.metadata[0].path = "R.csv".into(),
            4 => r.relations[0].options.batch_size = 0,
            5 => r.relations[0].options.delimiter = b'\n',
            6 => r.metadata[0].role = FileRole::Data,
            7 => r.date = "2026-02-30".into(),
            _ => unreachable!(),
        }
        assert!(r.check().is_err(), "accepted mutation {mutation}");
    }
    let encoded = serde_json::to_string(&request()).unwrap();
    assert!(DatasetRequest::parse_json(&encoded.replacen("\"name\":\"tiny\"", "\"name\":\"tiny\",\"name\":\"tiny\"", 1)).is_err());
    let mut unknown = serde_json::to_value(request()).unwrap(); unknown["unexpected"] = true.into();
    assert!(DatasetRequest::parse_json(&unknown.to_string()).is_err());
}

#[test]
fn command_publishes_only_complete_validated_evidence() {
    let files = Fixture::new();
    let request_path = files.0.join("request.json");
    let output = files.0.join("evidence.json");
    std::fs::write(&request_path, serde_json::to_string(&request()).unwrap()).unwrap();
    let invoke = |extra: &[&str]| std::process::Command::new(env!("CARGO_BIN_EXE_cq-stats"))
        .args(["stats", "--request"]).arg(&request_path).arg("--dataset").arg(&files.0)
        .arg("--out").arg(&output).args(extra).output().unwrap();
    let first = invoke(&[]); assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    let bytes = std::fs::read(&output).unwrap(); evidence::parse_bytes(&bytes).unwrap();
    let checked = std::process::Command::new(env!("CARGO_BIN_EXE_cq-stats"))
        .args(["check", "--evidence"]).arg(&output).output().unwrap();
    assert!(checked.status.success());
    assert!(!invoke(&["--expect-dataset-hash", "sha256:wrong"]).status.success());
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
    std::fs::write(files.0.join("R.csv"), "not-an-integer,1\n").unwrap();
    assert!(!invoke(&[]).status.success());
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
}
