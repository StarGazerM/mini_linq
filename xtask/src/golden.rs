#[cfg(feature = "oracle")]
use std::collections::BTreeSet;
use std::path::Path;

use crate::Result;

/// Compare the serialized answer too: parsing would hide CRLF, leading zeroes,
/// a missing final newline, or other drift in a checked-in golden.
pub(super) fn compare(path: &Path, rows: &[Vec<i32>], arity: usize) -> Result<()> {
    let expected = canonical_bytes(rows, arity)?;
    compare_bytes(path, &expected)
}

pub(super) fn compare_bytes(path: &Path, expected: &[u8]) -> Result<()> {
    let tracked = std::fs::read(path)?;
    if tracked != expected {
        let position = tracked
            .iter()
            .zip(expected)
            .position(|(a, b)| a != b)
            .unwrap_or(tracked.len().min(expected.len()));
        return Err(format!(
            "{} differs from DuckDB at byte {position} (tracked {} bytes, DuckDB {} bytes)",
            path.display(),
            tracked.len(),
            expected.len()
        )
        .into());
    }
    Ok(())
}

fn canonical_bytes(rows: &[Vec<i32>], arity: usize) -> Result<Vec<u8>> {
    if rows.iter().any(|row| row.len() != arity) || rows.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("DuckDB result must have the declared width and sorted distinct rows".into());
    }
    let mut text = String::new();
    for row in rows {
        text.push_str(&row.iter().map(i32::to_string).collect::<Vec<_>>().join(","));
        text.push('\n');
    }
    Ok(text.into_bytes())
}

#[cfg(feature = "oracle")]
pub(super) fn check_inventory(
    directory: &Path,
    checked: &BTreeSet<std::path::PathBuf>,
) -> Result<()> {
    let root = directory
        .parent()
        .and_then(Path::parent)
        .ok_or("golden directory has no repository root")?;
    // Include newly authored, not-yet-committed fixtures. Ignore generated
    // medium/large artifacts according to the repository's own .gitignore.
    let output = std::process::Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "workloads/golden",
        ])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err("could not inventory checked-in goldens with git ls-files".into());
    }
    let actual: BTreeSet<_> = String::from_utf8(output.stdout)?
        .lines()
        .map(|path| root.join(path))
        .collect();
    if actual != *checked {
        return Err(format!(
            "golden inventory differs: not recomputed={:?}; missing={:?}",
            actual.difference(checked).collect::<Vec<_>>(),
            checked.difference(&actual).collect::<Vec<_>>()
        )
        .into());
    }
    println!("verified all {} golden files byte for byte", checked.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_comparison_rejects_equivalent_noncanonical_csv() {
        let path = std::env::temp_dir().join(format!("cq-golden-{}.csv", std::process::id()));
        for text in ["01,2\n", "1,2\r\n", "1,2", "1,2\n\n"] {
            std::fs::write(&path, text).unwrap();
            assert!(compare(&path, &[vec![1, 2]], 2).is_err(), "{text:?}");
        }
        std::fs::write(&path, "1,2\n").unwrap();
        compare(&path, &[vec![1, 2]], 2).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn oracle_answers_must_be_canonical() {
        assert!(canonical_bytes(&[vec![1]], 2).is_err());
        assert!(canonical_bytes(&[vec![1], vec![1]], 1).is_err());
        assert!(canonical_bytes(&[vec![2], vec![1]], 1).is_err());
        assert_eq!(canonical_bytes(&[], 1).unwrap(), b"");
        assert_eq!(canonical_bytes(&[vec![-2, 3]], 2).unwrap(), b"-2,3\n");
    }
}
