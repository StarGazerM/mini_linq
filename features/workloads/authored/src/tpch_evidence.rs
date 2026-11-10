//! Workload-boundary adapter, not a statistics producer. CPU planning uses the
//! checked-in neutral SF1 document; GPU planning imports its selected manifest.
//! Keys remain declared, not measured uniqueness certificates.
use cq_evidence as evidence;
use cq_syntax::{RelationDecl, declared_evidence::{ColumnBinding, DeclaredEvidence, EvidenceView, RelationBinding}};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, sync::Arc};

const SF1_DOCUMENT: &str = include_str!("../tpch-sf1-evidence.json");
const MAPPINGS: &str = include_str!("../tpch-evidence.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mappings {
    date: String,
    formulation: String,
    key_provenance: evidence::Provenance,
    bindings: BTreeMap<String, String>,
    tables: BTreeMap<String, Table>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    columns: BTreeMap<String, String>,
    keys: Vec<Vec<String>>,
}
#[derive(Deserialize)]
struct Manifest {
    relations: BTreeMap<String, ManifestRelation>,
}
#[derive(Deserialize)]
struct ManifestRelation {
    rows: u64,
    columns: Vec<ManifestColumn>,
}
#[derive(Deserialize)]
struct ManifestColumn {
    name: String,
}
fn error(error: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), error.to_string())
}
fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// The CPU and expected-VO fixtures share a portable checked-in SF1 document.
/// Including it relative to this crate tracks changes without a dataset lookup.
pub fn load_sf1(declarations: &[RelationDecl]) -> syn::Result<Option<DeclaredEvidence>> {
    let mappings: Mappings = serde_json::from_str(MAPPINGS).map_err(error)?;
    bind_sf1(declarations, canonical_bindings(declarations, &mappings)?)
}

/// Bind renamed inputs to the same checked-in document without a dataset path.
pub fn bind_sf1(
    declarations: &[RelationDecl],
    bindings: Vec<RelationBinding>,
) -> syn::Result<Option<DeclaredEvidence>> {
    if bindings.is_empty() {
        return Ok(None);
    }
    let declared = DeclaredEvidence::from_json(SF1_DOCUMENT, bindings)?;
    declared.check(declarations)?;
    Ok(Some(declared))
}

fn canonical_bindings(
    declarations: &[RelationDecl],
    mappings: &Mappings,
) -> syn::Result<Vec<RelationBinding>> {
    let mut bindings = Vec::new();
    for declaration in declarations {
        let Some(table) = mappings.bindings.get(&declaration.name.to_string()) else {
            continue;
        };
        let mapping = mappings.tables.get(table)
            .ok_or_else(|| error(format!("missing TPC-H source mapping for {table}")))?;
        let columns = declaration.columns.iter().map(|column| {
            let source = mapping.columns.get(&column.name.to_string())
                .ok_or_else(|| error(format!("missing TPC-H column mapping for {}.{}", declaration.name, column.name)))?;
            Ok(ColumnBinding { column: column.name.clone(), source: source.clone() })
        }).collect::<syn::Result<Vec<_>>>()?;
        bindings.push(RelationBinding {
            relation: declaration.name.clone(), source: table.clone(),
            view: EvidenceView::Full, columns,
        });
    }
    Ok(bindings)
}

/// Bind renamed inputs explicitly; callers supply the checked column/view
/// correspondence rather than relying on the workload's canonical bindings.
pub fn load_with_bindings(
    path: &Path,
    declarations: &[RelationDecl],
    bindings: Vec<RelationBinding>,
) -> syn::Result<Option<DeclaredEvidence>> {
    let mappings = serde_json::from_str(MAPPINGS).map_err(error)?;
    load_document(path, declarations, bindings, mappings)
}

fn load_document(
    path: &Path,
    declarations: &[RelationDecl],
    bindings: Vec<RelationBinding>,
    mappings: Mappings,
) -> syn::Result<Option<DeclaredEvidence>> {
    if bindings.is_empty() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|e| error(format!("{}: {e}", path.display())))?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(error)?;
    let manifest_hash = hash(&bytes);
    let mapping_hash = hash(MAPPINGS.as_bytes());
    // This identity certifies the imported metadata, not a scan of decoded rows.
    // Domains deliberately provide no scalar-order or encoding certificates.
    let domain = evidence::ScalarDomain::Unordered {
        limitation: "ordering-only source correspondence; scalar domain not certified".into(),
    };
    let mut schema = BTreeMap::new();
    for (name, table) in &mappings.tables {
        let relation = manifest.relations.get(name)
            .ok_or_else(|| error(format!("{}: missing table {name}", path.display())))?;
        let mut columns = BTreeMap::new();
        for column in table.columns.values() {
            if !relation.columns.iter().any(|c| &c.name == column) {
                return Err(error(format!("{}: missing column {name}.{column}", path.display())));
            }
            columns.insert(column.clone(), domain.clone());
        }
        schema.insert(name.clone(), evidence::ManifestRelation {
            semantics: evidence::RelationSemantics::Bag,
            columns, null_handling: "non-null dbgen input columns".into(),
            set_certification: None,
        });
    }
    let encoding_hash = evidence::canonical_hash(&"JSON metadata; no data-file certification").map_err(error)?;
    let files = [("manifest.json", manifest_hash.clone()), ("tpch-evidence.json", mapping_hash.clone())]
        .into_iter().map(|(name, content_hash)| (name.into(), evidence::ManifestFile {
            role: evidence::FileRole::Metadata, content_hash: Some(content_hash),
            encoding_hash: encoding_hash.clone(), missing_as_empty: false,
        })).collect();
    let identity = evidence::InputManifest {
        schema, formulation: mappings.formulation.clone(), files,
        dictionaries: BTreeMap::new(),
        loader_options: BTreeMap::from([
            ("hash_mode".into(), evidence::HASH_MODE.into()),
            ("identity_scope".into(), "imported metadata only; no decoded-row scan".into()),
        ]),
    };
    let dataset_hash = identity.dataset_hash().map_err(error)?;
    let mut key_provenance = mappings.key_provenance;
    key_provenance.artifact_hash = Some(mapping_hash);
    let row_provenance = evidence::Provenance {
        producer: "TPC-H manifest adapter".into(),
        method: "Import recorded base-table row count; no new measurement".into(),
        locator: path.display().to_string(), run: None,
        artifact_hash: Some(manifest_hash),
    };
    let relations = mappings.tables.into_iter().map(|(name, table)| {
        let columns = table.columns.into_values().map(|name| (name, evidence::ColumnEvidence {
            domain: domain.clone(), distinct: None, min: None, max: None, max_degree: None,
        })).collect();
        let rows = Some(evidence::Fact {
            id: format!("{name}.rows"), source: evidence::Source::Exact,
            claim: evidence::Claim::Equal, value: Some(manifest.relations[&name].rows),
            lower: None, upper: None, dataset_hash: dataset_hash.clone(),
            provenance: row_provenance.clone(), unit: evidence::Unit::Rows,
        });
        let keys = table.keys.into_iter().map(|columns| evidence::KeyDeclaration {
            columns, source: evidence::Source::Declared, dataset_hash: dataset_hash.clone(),
            provenance: key_provenance.clone(),
        }).collect();
        (name, evidence::RelationEvidence {
            semantics: evidence::RelationSemantics::Bag, scope: evidence::RelationScope::Input,
            columns, rows, key_sets: Vec::new(), keys, recursive: None,
        })
    }).collect();
    let document = evidence::DatasetEvidence {
        schema: evidence::SCHEMA.into(),
        dataset: evidence::DatasetIdentity {
            name: "TPC-H manifest ordering evidence".into(), hash: dataset_hash,
            date: mappings.date, schema_hash: identity.schema_hash().map_err(error)?,
            formulation: mappings.formulation, manifest: identity,
        },
        relations, premises: Vec::new(), relative: Vec::new(),
        targets: BTreeMap::new(), observations: evidence::Observations::default(),
    };
    let declared = DeclaredEvidence { document: Arc::new(document), bindings };
    declared.check(declarations)?;
    Ok(Some(declared))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_sf1_matches_origin_when_available() {
        let document = evidence::parse_json(SF1_DOCUMENT).unwrap();
        let origin = &document.relations.values().next().unwrap()
            .rows.as_ref().unwrap().provenance.locator;
        let bytes = match std::fs::read(origin) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("SKIP SF1 origin comparison: {origin} is not installed; checked-in document validation passed");
                return;
            }
            Err(error) => panic!("cannot read optional SF1 origin {origin}: {error}"),
        };
        let manifest: Manifest = serde_json::from_slice(&bytes).unwrap();
        let digest = hash(&bytes);
        assert_eq!(document.dataset.manifest.files["manifest.json"].content_hash.as_ref(), Some(&digest));
        assert_eq!(document.relations.len(), manifest.relations.len());
        for (name, relation) in &document.relations {
            let rows = relation.rows.as_ref().unwrap();
            assert_eq!(rows.value, Some(manifest.relations[name].rows), "{name}");
            assert_eq!(rows.provenance.artifact_hash.as_ref(), Some(&digest), "{name}");
            assert_eq!(&rows.provenance.locator, origin, "{name}");
            assert!(relation.keys.iter().all(|key| key.source == evidence::Source::Declared));
        }
    }
}
