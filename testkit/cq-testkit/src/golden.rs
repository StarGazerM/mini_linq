//! Strict golden CSV encoding, byte comparison, and complete Git inventory.
//!
//! Inputs must be sorted, distinct `i32` tuples of the declared width. Comparing
//! bytes catches equivalent but noncanonical CSV. No golden files are rewritten.

use std::collections::BTreeSet;
use std::path::Path;

use crate::Result;

/// Compare the serialized answer too: parsing would hide CRLF, leading zeroes,
/// a missing final newline, or other drift in a checked-in golden.
pub fn compare(path: &Path, rows: &[Vec<i32>], arity: usize) -> Result<()> {
    let expected = canonical_bytes(rows, arity)?;
    compare_bytes(path, &expected)
}

/// Compare an existing file byte for byte against an independently computed result.
pub fn compare_bytes(path: &Path, expected: &[u8]) -> Result<()> {
    let tracked = std::fs::read(path)?;
    if tracked != expected {
        let position = tracked
            .iter()
            .zip(expected)
            .position(|(a, b)| a != b)
            .unwrap_or(tracked.len().min(expected.len()));
        return Err(format!(
            "{} differs from oracle at byte {position} (tracked {} bytes, oracle {} bytes)",
            path.display(),
            tracked.len(),
            expected.len()
        )
        .into());
    }
    Ok(())
}

/// Encode sorted distinct tuples as CSV with a final LF for each tuple.
pub fn canonical_bytes(rows: &[Vec<i32>], arity: usize) -> Result<Vec<u8>> {
    if rows.iter().any(|row| row.len() != arity) || rows.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("oracle result must have the declared width and sorted distinct rows".into());
    }
    let mut text = String::new();
    for row in rows {
        text.push_str(&row.iter().map(i32::to_string).collect::<Vec<_>>().join(","));
        text.push('\n');
    }
    Ok(text.into_bytes())
}

/// Check every tracked or unignored new golden under a repository directory.
///
/// `directory` must be relative to `repository`; `checked` contains paths
/// formed by joining Git's repository-relative paths to `repository`. Ignored
/// generated files are excluded using the repository's own Git ignore rules.
pub fn check_inventory(
    repository: &Path,
    directory: &Path,
    checked: &BTreeSet<std::path::PathBuf>,
) -> Result<()> {
    if directory.is_absolute()
        || directory
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("golden directory must be a repository-relative path".into());
    }
    // Include newly authored, not-yet-committed fixtures. Ignore generated
    // medium/large artifacts according to the repository's own .gitignore.
    let output = std::process::Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
        ])
        .arg(directory)
        .current_dir(repository)
        .output()?;
    if !output.status.success() {
        return Err("could not inventory checked-in goldens with git ls-files".into());
    }
    let actual: BTreeSet<_> = String::from_utf8(output.stdout)?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| repository.join(path))
        .collect();
    if actual != *checked {
        return Err(format!(
            "golden inventory differs: not recomputed={:?}; missing={:?}",
            actual.difference(checked).collect::<Vec<_>>(),
            checked.difference(&actual).collect::<Vec<_>>()
        )
        .into());
    }
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

    #[test]
    fn inventory_includes_new_goldens_excludes_ignored_files_and_rejects_omissions() {
        struct Temporary(std::path::PathBuf);
        impl Drop for Temporary {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = Temporary(
            std::env::temp_dir().join(format!("cq-testkit-inventory-{}", std::process::id())),
        );
        std::fs::create_dir(&root.0).unwrap();
        let git = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&root.0)
            .output()
            .unwrap();
        assert!(
            git.status.success(),
            "{}",
            String::from_utf8_lossy(&git.stderr)
        );
        let directory = std::path::Path::new("goldens");
        std::fs::create_dir(root.0.join(directory)).unwrap();
        std::fs::write(root.0.join(".gitignore"), "goldens/generated.csv\n").unwrap();
        let tracked = root.0.join("goldens/tracked.csv");
        let new = root.0.join("goldens/new\nname.csv");
        std::fs::write(&tracked, "1\n").unwrap();
        std::fs::write(&new, "2\n").unwrap();
        std::fs::write(root.0.join("goldens/generated.csv"), "3\n").unwrap();
        let git = std::process::Command::new("git")
            .args(["add", "goldens/tracked.csv"])
            .current_dir(&root.0)
            .output()
            .unwrap();
        assert!(git.status.success());
        let checked = BTreeSet::from([tracked.clone(), new]);
        check_inventory(&root.0, directory, &checked).unwrap();
        assert!(check_inventory(&root.0, directory, &BTreeSet::from([tracked])).is_err());
        assert!(check_inventory(&root.0, std::path::Path::new("../goldens"), &checked).is_err());
    }
}
