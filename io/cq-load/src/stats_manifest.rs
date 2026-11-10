//! One-pass dataset statistics with explicit loader and semantic hash identity.
//! Data files use the documented typed row stream; metadata uses raw bytes.
use crate::{Date, Error, Load, Result, stats::{DistinctMode, RelationAccumulator, RelationSpec}, stats_source::{self, SourceOptions}};
use cq_evidence::{self as evidence, DatasetEvidence, DatasetIdentity, FileRole, InputManifest, ManifestFile, ManifestRelation, Observations, Provenance, RelationSemantics, SetCertification, Source};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, fs::File, io::Read, path::{Path, Component}};

pub const REQUEST_SCHEMA: &str = "mini-linq.stats-input.v1";

/// Read checked evidence for a build script and register its canonical Cargo
/// dependency. Compiler-specific relation correspondence is supplied separately
/// by the caller after loading this neutral document.
pub fn load_build_input(path: &Path) -> Result<DatasetEvidence> {
    let path = path.canonicalize()
        .map_err(|error| Error::new(path, 0, "<evidence>", "", error.to_string()))?;
    let path_text = path.to_str().ok_or_else(|| {
        Error::new(&path, 0, "<evidence>", "", "evidence path must be UTF-8")
    })?;
    if path_text.contains('\n') || path_text.contains('\r') {
        return Err(Error::new(&path, 0, "<evidence>", "", "evidence path contains a line break"));
    }
    println!("cargo:rerun-if-changed={path_text}");
    let json = std::fs::read_to_string(&path)
        .map_err(|error| Error::new(&path, 0, "<evidence>", "", error.to_string()))?;
    evidence::parse_json(&json)
        .map_err(|error| Error::new(&path, 0, "<evidence>", "", error.to_string()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetRequest {
    pub schema: String,
    pub name: String,
    pub date: String,
    pub formulation: String,
    pub hash_mode: String,
    pub distinct: DistinctMode,
    pub relations: Vec<RelationInput>,
    pub metadata: Vec<MetadataInput>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationInput {
    pub relation: RelationSpec,
    /// Logical relative filename, resolved only against the caller's root.
    pub path: String,
    pub options: SourceOptions,
    pub missing_as_empty: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataInput {
    pub path: String,
    pub role: FileRole,
}

fn invalid(value: impl Into<String>, message: impl Into<String>) -> Error {
    Error::new(Path::new("<stats request>"), 0, "<schema>", value, message)
}
fn evidence_error(error: evidence::EvidenceError) -> Error { invalid("", error.to_string()) }
fn relative(path: &str) -> bool {
    !path.is_empty() && !path.contains('\\') && !path.contains(':')
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(path).components().all(|part| matches!(part, Component::Normal(_)))
}

impl DatasetRequest {
    /// Duplicate fields and unknown options are rejected by the typed grammar.
    /// Names/paths are vectors, not last-key-wins JSON maps.
    pub fn parse_json(json: &str) -> Result<Self> {
        let request: Self = serde_json::from_str(json).map_err(|error| invalid("JSON", error.to_string()))?;
        request.check()?;
        Ok(request)
    }

    pub fn check(&self) -> Result<()> {
        if self.schema != REQUEST_SCHEMA { return Err(invalid(&self.schema, "unsupported statistics request schema")); }
        if self.hash_mode != evidence::HASH_MODE { return Err(invalid(&self.hash_mode, "incompatible dataset hash mode")); }
        if self.name.trim().is_empty() || self.formulation.trim().is_empty() || self.relations.is_empty() {
            return Err(invalid("", "statistics request needs name, formulation and relations"));
        }
        Date::load(&self.date).map_err(|error| invalid(&self.date, error))?;
        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for input in &self.relations {
            if !names.insert(&input.relation.name) || !paths.insert(&input.path) || !relative(&input.path) {
                return Err(invalid(&input.path, "duplicate relation/file or nonrelative data filename"));
            }
            // Check the complete request before opening the first data file.
            input.relation.check(&self.distinct)?;
            let columns = input.relation.columns.iter().map(|column| column.name.as_str()).collect::<Vec<_>>();
            input.options.check(&columns)?;
        }
        for metadata in &self.metadata {
            if metadata.role == FileRole::Data || !relative(&metadata.path) || !paths.insert(&metadata.path) {
                return Err(invalid(&metadata.path, "metadata needs a distinct relative path and non-data role"));
            }
        }
        Ok(())
    }

    /// Collect every source once. Per-relation maps are released into compact
    /// summaries before the next relation, then bound to the final manifest hash.
    pub fn collect(&self, root: &Path) -> Result<DatasetEvidence> {
        self.check()?;
        let mut manifest = InputManifest {
            schema: BTreeMap::new(), formulation: self.formulation.clone(), files: BTreeMap::new(),
            dictionaries: BTreeMap::new(), loader_options: BTreeMap::from([
                ("hash_mode".into(), evidence::HASH_MODE.into()),
                ("missing_files".into(), if self.relations.iter().any(|r| r.missing_as_empty) { "empty" } else { "error" }.into()),
                ("nulls".into(), "reject".into()),
            ]),
        };
        let mut summaries = Vec::with_capacity(self.relations.len());
        for input in &self.relations {
            let path = root.join(&input.path);
            let mut accumulator = RelationAccumulator::new(input.relation.clone(), self.distinct.clone())?;
            let missing = match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => false,
                Ok(_) => return Err(Error::new(&path, 0, "<file>", "", "input is not a regular file")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && input.missing_as_empty => true,
                Err(error) => return Err(Error::new(&path, 0, "<file>", "", error.to_string())),
            };
            if !missing {
                let columns = input.relation.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>();
                stats_source::visit(&path, &columns, &input.options, |batch| accumulator.observe_batch(batch))?;
            }
            let digest = (!missing).then(|| accumulator.content_hash());
            let encoding_hash = evidence::canonical_hash(&(&input.relation.columns, &input.options))
                .map_err(evidence_error)?;
            manifest.files.insert(input.path.clone(), ManifestFile {
                role: FileRole::Data, content_hash: digest, encoding_hash, missing_as_empty: missing,
            });
            manifest.schema.insert(input.relation.name.clone(), ManifestRelation {
                semantics: input.relation.semantics,
                columns: input.relation.columns.iter().map(|c| (c.name.clone(), c.domain.clone())).collect(),
                null_handling: "reject".into(),
                set_certification: (input.relation.semantics == RelationSemantics::Set)
                    .then_some(SetCertification::FullTupleDeduplicated),
            });
            // Retain the declaration order as well as all effective loader
            // options; a BTreeMap of column domains alone loses that order.
            manifest.loader_options.insert(format!("relation:{}", input.relation.name),
                serde_json::to_string(input).map_err(|error| invalid("loader options", error.to_string()))?);
            summaries.push((input, accumulator.summarize()?));
        }
        for metadata in &self.metadata {
            let path = root.join(&metadata.path);
            let mut file = File::open(&path).map_err(|error| Error::new(&path, 0, "<file>", "", error.to_string()))?;
            let mut digest = Sha256::new();
            let mut bytes = [0u8; 64 * 1024];
            loop {
                let len = file.read(&mut bytes).map_err(|error| Error::new(&path, 0, "<file>", "", error.to_string()))?;
                if len == 0 { break; }
                digest.update(&bytes[..len]);
            }
            let hash = format!("sha256:{digest:x}", digest = digest.finalize());
            if metadata.role == FileRole::Dictionary {
                manifest.dictionaries.insert(metadata.path.clone(), hash.clone());
            }
            manifest.files.insert(metadata.path.clone(), ManifestFile {
                role: metadata.role, content_hash: Some(hash),
                encoding_hash: evidence::canonical_hash(&"raw-bytes-sha256-v1").map_err(evidence_error)?,
                missing_as_empty: false,
            });
        }
        let dataset_hash = manifest.dataset_hash().map_err(evidence_error)?;
        let schema_hash = manifest.schema_hash().map_err(evidence_error)?;
        let mut relations = BTreeMap::new();
        for (input, summary) in summaries {
            let provenance = Provenance {
                producer: "cq-load stats".into(),
                method: "complete decoded-source traversal; exact frequency maps; no sampling; typed-row-stream-v1 content identity".into(),
                locator: input.path.clone(), run: None,
                artifact_hash: manifest.files[&input.path].content_hash.clone(),
            };
            relations.insert(input.relation.name.clone(), summary.bind(&dataset_hash, Source::Exact, provenance)?);
        }
        let evidence = DatasetEvidence {
            schema: evidence::SCHEMA.into(),
            dataset: DatasetIdentity { name: self.name.clone(), hash: dataset_hash, date: self.date.clone(),
                schema_hash, formulation: self.formulation.clone(), manifest },
            relations, premises: Vec::new(), relative: Vec::new(), targets: BTreeMap::new(),
            observations: Observations::default(),
        };
        evidence.check().map_err(evidence_error)?;
        Ok(evidence)
    }
}
