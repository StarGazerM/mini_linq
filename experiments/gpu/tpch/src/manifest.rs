use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

#[derive(Clone, Debug, Deserialize)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub logical_type: String,
    pub encoding: String,
    pub file: String,
    pub dictionary: Option<String>,
    pub dictionary_index_base: Option<i64>,
    pub dictionary_entries: Option<usize>,
    pub epoch: Option<String>,
    pub unit: Option<String>,
    pub scale: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VerifiedKey {
    pub set: String,
    pub columns: BTreeMap<String, String>,
    pub types: BTreeMap<String, String>,
    pub rows: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Relation {
    pub rows: usize,
    pub columns: Vec<Column>,
    #[serde(default)]
    pub verified_keys: Vec<VerifiedKey>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub sf: u32,
    pub byte_order: String,
    pub relations: BTreeMap<String, Relation>,
    #[serde(skip)]
    pub root: PathBuf,
}

/// Borrowed adapter for an explicitly declared, checked sorted dictionary.
/// CQ's shared domain normalization owns operand capture and comparison rules.
pub struct DictionaryDomain<'a>(pub &'a [&'a str]);

impl cq_storage::CodeDomain for DictionaryDomain<'_> {
    type Value = str;
    type Code = u32;

    fn exact(&self, value: &str) -> u32 {
        self.0
            .binary_search(&value)
            .ok()
            .map(|index| u32::try_from(index).expect("validated dictionary size"))
            .unwrap_or(u32::MAX)
    }
}

impl cq_storage::OrderedCodeDomain for DictionaryDomain<'_> {
    fn lower(&self, value: &str) -> u32 {
        u32::try_from(self.0.partition_point(|entry| *entry < value))
            .expect("validated dictionary size")
    }

    fn upper(&self, value: &str) -> u32 {
        u32::try_from(self.0.partition_point(|entry| *entry <= value))
            .expect("validated dictionary size")
    }
}

/// Representation conversion selected by a declared domain column, never by
/// inspection of its Rust type. Opaque host producers retain their own carrier.
pub trait DictionaryCode: Sized {
    fn from_code(code: u32) -> Self;
    fn into_code(self) -> u32;
}

impl DictionaryCode for cq_load::StringCode {
    fn from_code(code: u32) -> Self {
        Self(code)
    }
    fn into_code(self) -> u32 {
        self.0
    }
}

impl DictionaryCode for u32 {
    fn from_code(code: u32) -> Self {
        code
    }
    fn into_code(self) -> u32 {
        self
    }
}

/// Host values follow the query declaration, not the on-disk representation.
pub trait HostValue: Sized {
    fn read(manifest: &Manifest, table: &str, column: &str) -> Result<Vec<Self>>;
}

impl HostValue for i64 {
    fn read(manifest: &Manifest, table: &str, column: &str) -> Result<Vec<Self>> {
        manifest.read_column(table, column)
    }
}

impl HostValue for i32 {
    fn read(manifest: &Manifest, table: &str, column: &str) -> Result<Vec<Self>> {
        manifest
            .read_column(table, column)?
            .into_iter()
            .map(|value| i32::try_from(value).map_err(Into::into))
            .collect()
    }
}

impl HostValue for u32 {
    fn read(manifest: &Manifest, table: &str, column: &str) -> Result<Vec<Self>> {
        manifest
            .read_column(table, column)?
            .into_iter()
            .map(|value| u32::try_from(value).map_err(Into::into))
            .collect()
    }
}

impl HostValue for cq_load::StringCode {
    fn read(manifest: &Manifest, table: &str, column: &str) -> Result<Vec<Self>> {
        manifest
            .read_column(table, column)?
            .into_iter()
            .map(|value| u32::try_from(value).map(Self).map_err(Into::into))
            .collect()
    }
}

impl HostValue for cq_load::Date {
    fn read(manifest: &Manifest, table: &str, column: &str) -> Result<Vec<Self>> {
        // DECODE ?: Date HostValue reconstruction serves typed host inputs; caller rust_reads versus reference-only use determines necessity.
        manifest
            .read_column(table, column)?
            .into_iter()
            .map(|value| Ok(Self::from_days(i32::try_from(value)?)))
            .collect()
    }
}

impl<const SCALE: u32> HostValue for cq_load::Decimal<SCALE> {
    fn read(manifest: &Manifest, table: &str, column: &str) -> Result<Vec<Self>> {
        if manifest.column(table, column)?.scale != Some(SCALE) {
            return Err(
                format!("{table}.{column}: decimal scale differs from loader contract").into(),
            );
        }
        // DECODE ?: Decimal HostValue reconstructs a whole typed input column; requires caller rust_reads evidence, not its type alone.
        manifest
            .read_column(table, column)?
            .into_iter()
            .map(|value| Self::from_scaled(value).map_err(Into::into))
            .collect()
    }
}

impl HostValue for String {
    fn read(manifest: &Manifest, table: &str, column: &str) -> Result<Vec<Self>> {
        let dictionary = manifest.dictionary(manifest.column(table, column)?)?;
        manifest
            .read_stored_column(table, column)?
            .into_iter()
            // DECODE ?: String HostValue resolves dictionary entries for typed host inputs; classify each caller as opaque Rust or premature relational decode.
            .map(|code| {
                usize::try_from(code)
                    .ok()
                    .and_then(|index| dictionary.get(index))
                    .cloned()
                    .ok_or_else(|| format!("{table}.{column}: out-of-range dictionary code").into())
            })
            .collect()
    }
}

impl Manifest {
    pub fn load(root: &Path) -> Result<Self> {
        let path = std::env::var_os("TPCH_GPU_MANIFEST")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("manifest.json"));
        Self::load_with_manifest(root, &path)
    }

    pub fn load_with_manifest(root: &Path, path: &Path) -> Result<Self> {
        let mut manifest: Self = serde_json::from_slice(&fs::read(path)?)?;
        manifest.root = root.to_owned();
        if !matches!(manifest.schema_version, 2 | 3) || manifest.byte_order != "little" {
            return Err("requires schema_version 2 or 3 and little-endian columns".into());
        }
        for (table, relation) in &manifest.relations {
            for column in &relation.columns {
                let valid = match column.encoding.as_str() {
                    "plain" => matches!(column.ty.as_str(), "i32" | "i64"),
                    "dictionary" => {
                        column.ty == "i32"
                            && column.logical_type == "String"
                            && column.dictionary.is_some()
                            && column.dictionary_index_base == Some(0)
                    }
                    "days_since_epoch" => {
                        column.ty == "i32"
                            && column.logical_type == "Date"
                            && column.epoch.as_deref() == Some("1970-01-01")
                            && column.unit.as_deref() == Some("day")
                    }
                    "scaled_integer" => {
                        column.ty == "i64"
                            && column.logical_type == "Decimal"
                            && column.scale == Some(2)
                    }
                    _ => false,
                };
                if !valid {
                    return Err(format!(
                        "unsupported manifest encoding for {table}.{}: {column:?}",
                        column.name
                    )
                    .into());
                }
            }
        }
        Ok(manifest)
    }

    pub fn column(&self, table: &str, column: &str) -> Result<&Column> {
        self.relations
            .get(table)
            .and_then(|r| r.columns.iter().find(|c| c.name == column))
            .ok_or_else(|| format!("missing manifest column {table}.{column}").into())
    }

    pub fn dictionary(&self, column: &Column) -> Result<Vec<String>> {
        let file = column
            .dictionary
            .as_ref()
            .ok_or("column has no dictionary")?;
        let entries: Vec<_> = fs::read_to_string(self.root.join(file))?
            .lines()
            .map(str::to_owned)
            .collect();
        if column.dictionary_entries != Some(entries.len()) {
            return Err(format!("dictionary length mismatch: {file}").into());
        }
        Ok(entries)
    }

    pub fn text_id(&self, table: &str, column: &str, text: &str) -> Result<i32> {
        let column = self.column(table, column)?;
        let values = self.dictionary(column)?;
        // An absent literal must remain a nonmatching literal, not a new plan.
        // -1 is outside the checked nonnegative dictionary-code domain.
        Ok(values
            .iter()
            .position(|value| value == text)
            .map(i32::try_from)
            .transpose()?
            .unwrap_or(-1))
    }

    fn read_stored_column(&self, table: &str, name: &str) -> Result<Vec<i64>> {
        self.read_stored_column_checked(table, name, false).map(|(values, _)| values)
    }

    pub fn read_column_checked(
        &self, table: &str, name: &str, check_key: bool,
    ) -> Result<(Vec<i64>, Option<String>)> {
        let column = self.column(table, name)?;
        if column.encoding == "dictionary" {
            return self.read_column(table, name).map(|values| (values, None));
        }
        self.read_stored_column_checked(table, name, check_key)
    }

    fn read_stored_column_checked(
        &self, table: &str, name: &str, check_key: bool,
    ) -> Result<(Vec<i64>, Option<String>)> {
        let column = self.column(table, name)?;
        let bytes = fs::read(self.root.join(&column.file))?;
        let width = match column.ty.as_str() {
            "i32" => 4,
            "i64" => 8,
            other => return Err(format!("unsupported stored type {other}").into()),
        };
        let rows = self.relations[table].rows;
        if bytes.len() != rows.checked_mul(width).ok_or("column size overflow")? {
            return Err(format!("{table}.{name}: byte count does not match manifest").into());
        }
        let hash = check_key.then(|| format!("{:x}", Sha256::digest(&bytes)));
        let values: Vec<_> = bytes
            .chunks_exact(width)
            .map(|bytes| {
                if width == 4 {
                    i64::from(i32::from_le_bytes(bytes.try_into().unwrap()))
                } else {
                    i64::from_le_bytes(bytes.try_into().unwrap())
                }
            })
            .collect();
        Ok((values, hash))
    }

    pub fn read_column(&self, table: &str, name: &str) -> Result<Vec<i64>> {
        let column = self.column(table, name)?;
        let values = self.read_stored_column(table, name)?;
        if column.encoding == "dictionary" {
            let count = self.dictionary(column)?.len();
            if values
                .iter()
                .any(|&value| value < 0 || value as usize >= count)
            {
                return Err(format!("{table}.{name}: out-of-range dictionary code").into());
            }
        }
        Ok(values)
    }
}

#[cfg(test)]
mod dictionary_domain_tests {
    use super::{DictionaryCode, DictionaryDomain};
    use cq_storage::{CodeDomain, OrderedCodeDomain};

    #[test]
    fn exact_and_ordered_operands_follow_the_declared_sorted_domain() {
        let dictionary = DictionaryDomain(&["green part", "red"]);
        assert_eq!(dictionary.exact("green part"), 0);
        assert_eq!(dictionary.exact("red"), 1);
        assert_eq!(dictionary.exact("absent"), u32::MAX);
        assert_eq!(dictionary.lower("purple"), 1);
        assert_eq!(dictionary.upper("purple"), 1);
        assert_eq!(dictionary.lower("red"), 1);
        assert_eq!(dictionary.upper("red"), 2);
        assert_eq!(dictionary.lower("z"), 2);
        assert_eq!(cq_storage::domain::exact(&dictionary, &"red".to_owned()), 1);
        assert_eq!(cq_storage::domain::exact(&dictionary, &{ "red" }), 1);
        assert_eq!(cq_load::StringCode::from_code(1).into_code(), 1);
        assert_eq!(
            cq_load::StringCode::from_code(u32::MAX).into_code(),
            u32::MAX
        );
    }

    #[test]
    fn override_hashes_the_exact_decoded_bytes_and_keeps_legacy_inputs() -> super::Result<()> {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!("tpch-key-manifest-{}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        let bytes: Vec<_> = [3_i32, 1, 2].into_iter().flat_map(i32::to_le_bytes).collect();
        std::fs::write(root.join("key.bin"), &bytes)?;
        let path = root.join("override.json");
        let expected = format!("{:x}", Sha256::digest(&bytes));
        std::fs::write(&path, serde_json::to_vec(&serde_json::json!({
            "schema_version": 3, "sf": 1, "byte_order": "little",
            "relations": {"r": {"rows": 3, "columns": [{
                "name": "id", "type": "i32", "logical_type": "Integer",
                "encoding": "plain", "file": "key.bin"
            }], "verified_keys": [{"set": "verified", "rows": 3,
                "types": {"id": "i32"}, "columns": {"id": expected}}]}}
        }))?)?;
        let manifest = super::Manifest::load_with_manifest(&root, &path)?;
        let (values, hash) = manifest.read_column_checked("r", "id", true)?;
        assert_eq!(values, vec![3, 1, 2]);
        assert_eq!(hash.as_deref(), Some(expected.as_str()));
        let changed: Vec<_> = [3_i32, 3, 2].into_iter().flat_map(i32::to_le_bytes).collect();
        std::fs::write(root.join("key.bin"), changed)?;
        assert_ne!(manifest.read_column_checked("r", "id", true)?.1, hash);
        assert_eq!(manifest.read_column_checked("r", "id", false)?.1, None);
        let legacy: super::Relation = serde_json::from_str(r#"{"rows":0,"columns":[]}"#)?;
        assert!(legacy.verified_keys.is_empty());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
