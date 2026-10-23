use serde::Deserialize;
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
pub struct Relation {
    pub rows: usize,
    pub columns: Vec<Column>,
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

impl HostValue for String {
    fn read(manifest: &Manifest, table: &str, column: &str) -> Result<Vec<Self>> {
        let dictionary = manifest.dictionary(manifest.column(table, column)?)?;
        manifest
            .read_stored_column(table, column)?
            .into_iter()
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
        let mut manifest: Self = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
        manifest.root = root.to_owned();
        if manifest.schema_version != 2 || manifest.byte_order != "little" {
            return Err("requires schema_version 2 and little-endian columns".into());
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
        Ok(values)
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
