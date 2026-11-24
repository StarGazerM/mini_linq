//! Trusted preparation certificates for immutable Parquet files, not planner facts.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::{Path, PathBuf}};

pub const SCHEMA: &str = "mini-linq.parquet-set.v1";

/// Native source domains on which a decoder preserves distinct values.
/// Custom decoders default to no certificate support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEncoding { Integer, String, Boolean, Date, Decimal }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub files: Vec<CertifiedFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedFile {
    pub file: String,
    pub sha256: String,
    /// Only the explicit preparation marker `verified` authorizes a shortcut.
    pub set: String,
    pub verified_keys: Vec<Vec<String>>,
}

/// Adjacent certificates are used unless CQ_LOAD_MANIFEST selects one manifest
/// containing entries for all desired files. A missing override never falls back
/// to a different certificate.
pub fn manifest_path(file: &Path) -> PathBuf {
    std::env::var_os("CQ_LOAD_MANIFEST").map(PathBuf::from)
        .unwrap_or_else(|| adjacent_path(file))
}

pub fn adjacent_path(file: &Path) -> PathBuf {
    let mut path = file.as_os_str().to_owned();
    path.push(".set.json");
    PathBuf::from(path)
}

pub fn sha256(reader: &mut impl Read) -> std::io::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 { break; }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn file_sha256(path: &Path) -> std::io::Result<String> {
    sha256(&mut File::open(path)?)
}

#[cfg(feature = "parquet")]
impl KeyEncoding {
    pub(crate) fn matches(self, ty: &arrow_schema::DataType) -> bool {
        use arrow_schema::DataType as T;
        match self {
            Self::Integer => matches!(ty, T::Int8 | T::Int16 | T::Int32 | T::Int64 | T::UInt8 | T::UInt16 | T::UInt32 | T::UInt64),
            Self::String => matches!(ty, T::Utf8 | T::LargeUtf8 | T::Utf8View),
            Self::Boolean => matches!(ty, T::Boolean),
            Self::Date => matches!(ty, T::Date32),
            Self::Decimal => matches!(ty, T::Decimal128(_, _)),
        }
    }
}

#[cfg(feature = "parquet")]
pub(crate) fn certified(
    manifest: &Path,
    path: &Path,
    input: &mut File,
    schema: &arrow_schema::Schema,
    columns: &[(&str, Option<KeyEncoding>)],
) -> bool {
    use std::io::{Seek, SeekFrom};
    let Some(manifest) = std::fs::read(manifest).ok()
        .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok()) else { return false; };
    if manifest.schema != SCHEMA { return false; }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return false; };
    let mut files = manifest.files.iter().filter(|file| file.file == name);
    let Some(file) = files.next() else { return false; };
    if files.next().is_some() || file.set != "verified" { return false; }
    let eligible = file.verified_keys.iter().any(|key| {
        !key.is_empty() && key.iter().enumerate().all(|(index, name)| {
            !key[..index].contains(name) && columns.iter().any(|(column, encoding)| {
                *column == name && encoding.is_some_and(|encoding| {
                    schema.field_with_name(name).is_ok_and(|field| encoding.matches(field.data_type()))
                })
            })
        })
    });
    if !eligible { return false; }
    if input.seek(SeekFrom::Start(0)).is_err() { return false; }
    let digest = sha256(input).ok();
    let rewound = input.seek(SeekFrom::Start(0)).is_ok();
    rewound && digest.as_deref() == Some(file.sha256.as_str())
}
