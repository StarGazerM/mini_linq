//! Feature-local canonical CSV encoding and strict byte comparison.
use crate::Result;
#[cfg(test)]
use std::collections::BTreeSet;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

pub fn canonical_bytes(rows: &[Vec<i32>], arity: usize) -> Result<Vec<u8>> {
    if rows.iter().any(|row| row.len() != arity) || rows.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("oracle rows must have the declared width and be sorted and distinct".into());
    }
    let mut text = String::new();
    for row in rows {
        text.push_str(&row.iter().map(i32::to_string).collect::<Vec<_>>().join(","));
        text.push('\n');
    }
    Ok(text.into_bytes())
}

pub fn compare(path: &Path, rows: &[Vec<i32>], arity: usize) -> Result<()> {
    let expected = canonical_bytes(rows, arity)?;
    let actual = std::fs::read(path)?;
    if actual != expected {
        let offset = actual.iter().zip(&expected).position(|(a, b)| a != b)
            .unwrap_or(actual.len().min(expected.len()));
        return Err(format!("{} differs at byte {offset} (file {}, oracle {} bytes)",
            path.display(), actual.len(), expected.len()).into());
    }
    Ok(())
}

#[cfg(test)]
pub fn inventory(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() { files.extend(inventory(&entry.path())?); }
        else { files.insert(entry.path()); }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_encoding() {
        assert_eq!(canonical_bytes(&[], 1).unwrap(), b"");
        assert_eq!(canonical_bytes(&[vec![-2, 3]], 2).unwrap(), b"-2,3\n");
        assert!(canonical_bytes(&[vec![1]], 2).is_err());
        assert!(canonical_bytes(&[vec![1], vec![1]], 1).is_err());
        assert!(canonical_bytes(&[vec![2], vec![1]], 1).is_err());
    }
}
