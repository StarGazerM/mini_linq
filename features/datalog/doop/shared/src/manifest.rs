//! IO-only certificates shared by the raw and prepared-integer DOOP loaders.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::File, io::{self, Read}, path::Path};

pub(crate) const DEFAULT_MANIFEST: &str = "doop-set-manifest.json";

#[derive(Default, Deserialize)]
pub(crate) struct Manifest {
    version: u32,
    relations: BTreeMap<String, Relation>,
}

#[derive(Deserialize)]
struct Relation {
    types: Vec<String>,
    verified_keys: Vec<Key>,
}

#[derive(Deserialize)]
struct Key {
    columns: Vec<usize>,
    set: String,
    sha256: String,
}

impl Manifest {
    pub(crate) fn load(directory: &Path, path: Option<&Path>) -> Self {
        let default = directory.join(DEFAULT_MANIFEST);
        File::open(path.unwrap_or(&default)).ok()
            .and_then(|file| serde_json::from_reader(file).ok())
            .unwrap_or_default()
    }

    pub(crate) fn expected_hash(&self, path: &Path, types: &[&str], projection: &[usize]) -> Option<[u8; 32]> {
        if self.version != 1 { return None; }
        let relation = self.relations.get(path.file_name()?.to_str()?)?;
        if relation.types.iter().map(String::as_str).ne(types.iter().copied()) { return None; }
        // Formal rule LD-DoopVerifiedKey: only a verified key retained by the
        // loaded heading can discharge its IO set boundary, after hashing.
        relation.verified_keys.iter().find_map(|key| {
            if key.set != "verified" || key.columns.is_empty()
                || key.columns.iter().any(|column| *column >= types.len() || !projection.contains(column))
                || key.columns.iter().enumerate().any(|(i, column)| key.columns[..i].contains(column))
                || key.sha256.len() != 64 {
                return None;
            }
            let mut hash = [0; 32];
            for (out, pair) in hash.iter_mut().zip(key.sha256.as_bytes().chunks_exact(2)) {
                let text = std::str::from_utf8(pair).ok()?;
                *out = u8::from_str_radix(text, 16).ok()?;
            }
            Some(hash)
        })
    }
}

/// Hash only when a usable certificate exists, while reading the exact bytes
/// decoded by the caller (including newlines and blank lines).
pub(crate) struct CertifiedReader<R> {
    reader: R,
    certificate: Option<([u8; 32], Sha256)>,
}

impl<R> CertifiedReader<R> {
    pub(crate) fn new(reader: R, hash: Option<[u8; 32]>) -> Self {
        Self { reader, certificate: hash.map(|hash| (hash, Sha256::new())) }
    }

    pub(crate) fn verified(self) -> bool {
        self.certificate.is_some_and(|(expected, hash)| {
            let actual: [u8; 32] = hash.finalize().into();
            actual == expected
        })
    }
}

impl<R: Read> Read for CertifiedReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let length = self.reader.read(bytes)?;
        if let Some((_, hash)) = &mut self.certificate { hash.update(&bytes[..length]); }
        Ok(length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificates_require_the_decoder_and_a_retained_key() {
        let hash = format!("{:x}", Sha256::digest(b"2\t7\n1\t9\n"));
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "version": 1, "relations": {"r.csv": {"types": ["i32", "i32"],
            "verified_keys": [{"columns": [0, 1], "set": "verified", "sha256": hash}]}}
        })).unwrap();
        let path = Path::new("r.csv");
        assert!(manifest.expected_hash(path, &["i32", "i32"], &[0, 1]).is_some());
        assert!(manifest.expected_hash(path, &["i32", "i32"], &[0]).is_none());
        assert!(manifest.expected_hash(path, &["string", "i32"], &[0, 1]).is_none());
        assert!(manifest.expected_hash(Path::new("other.csv"), &["i32", "i32"], &[0, 1]).is_none());
    }

    #[test]
    fn hash_is_bound_to_consumed_bytes() {
        let bytes = b"2\t7\n1\t9\n";
        let expected = Some(Sha256::digest(bytes).into());
        for (input, matches) in [(bytes.as_slice(), true), (b"2\t7\n2\t7\n".as_slice(), false)] {
            let mut reader = CertifiedReader::new(input, expected);
            io::copy(&mut reader, &mut io::sink()).unwrap();
            assert_eq!(reader.verified(), matches);
        }
        assert!(!CertifiedReader::new(bytes.as_slice(), None).verified());
    }
}
