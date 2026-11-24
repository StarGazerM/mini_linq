//! Compression schemas shared by host build scripts and resident catalogs.

pub mod tpch;

/// Stored kind of one column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Dense u32 ID of the named domain.
    Id(String),
    /// Declared 32-bit integer, stored at that type.
    I32,
}

/// Which source columns share a domain, and every stored table's columns.
pub struct Schema {
    /// Domain name -> member source columns (`table.column`).
    pub domains: Vec<(String, Vec<String>)>,
    /// Table -> (column, kind), in stored column order.
    pub tables: Vec<(String, Vec<(String, Kind)>)>,
}

/// Select one row ID per complete projected tuple before encoding or upload.
/// `None` preserves the original columns when they are already a set.
/// Formal rule DR-ProjectedSet: canonicalize the actual projected rows, never a declared key.
pub fn unique_rows(
    columns: &[impl AsRef<[i64]>],
    rows: usize,
) -> Result<Option<Vec<u32>>, &'static str> {
    if rows >= u32::MAX as usize {
        return Err("relation exceeds device row-ID domain");
    }
    if columns.iter().any(|column| column.as_ref().len() != rows) {
        return Err("projected columns have different row counts");
    }
    let compare = |left: u32, right: u32| {
        columns
            .iter()
            .map(|column| {
                column.as_ref()[left as usize].cmp(&column.as_ref()[right as usize])
            })
            .find(|order| !order.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    let mut ordered = true;
    let mut unique = true;
    for row in 1..rows as u32 {
        match compare(row - 1, row) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => unique = false,
            std::cmp::Ordering::Greater => {
                ordered = false;
                break;
            }
        }
    }
    if ordered && unique {
        return Ok(None);
    }
    let mut indices: Vec<u32> = (0..rows as u32).collect();
    if !ordered {
        indices.sort_unstable_by(|&left, &right| compare(left, right));
    }
    indices.dedup_by(|left, right| compare(*left, *right).is_eq());
    Ok((indices.len() != rows).then_some(indices))
}

#[cfg(test)]
mod tests {
    use super::unique_rows;

    #[test]
    fn complete_projection_not_independent_columns_defines_equality() {
        let columns = [vec![2, 1, 2, 1, 2], vec![9, 8, 7, 8, 9]];
        let selected = unique_rows(&columns, 5).unwrap().unwrap();
        let rows: Vec<_> = selected
            .iter()
            .map(|&row| (columns[0][row as usize], columns[1][row as usize]))
            .collect();
        assert_eq!(rows, [(1, 8), (2, 7), (2, 9)]);
        let selected = unique_rows(&columns[..1], 5).unwrap().unwrap();
        assert_eq!(
            selected.iter().map(|&row| columns[0][row as usize]).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(unique_rows(&[vec![1, 1], vec![2, 3]], 2), Ok(None));
    }

    #[test]
    fn unique_rows_preserve_existing_storage_order() {
        assert_eq!(unique_rows(&[vec![1, 2, 3]], 3), Ok(None));
        assert_eq!(unique_rows(&[vec![3, 1, 2]], 3), Ok(None));
        assert_eq!(unique_rows(&[vec![1, 1, 2]], 3), Ok(Some(vec![0, 2])));
        assert_eq!(unique_rows(&[vec![]], 0), Ok(None));
        assert_eq!(unique_rows(&[vec![7]], 1), Ok(None));
    }

    #[test]
    fn empty_projection_has_at_most_one_tuple() {
        let columns: &[Vec<i64>] = &[];
        assert_eq!(unique_rows(columns, 0), Ok(None));
        assert_eq!(unique_rows(columns, 1), Ok(None));
        assert_eq!(unique_rows(columns, 3), Ok(Some(vec![0])));
    }

    #[test]
    fn inconsistent_lengths_and_row_id_overflow_are_rejected() {
        assert!(unique_rows(&[vec![1], vec![2, 3]], 2).is_err());
        assert!(unique_rows(&[vec![]], u32::MAX as usize).is_err());
    }
}
