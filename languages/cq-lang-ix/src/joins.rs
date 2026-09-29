//! Logical join columns, in the right result's sorted heading.
pub fn result_join_columns(left: &[String], right: &[String]) -> Vec<usize> {
    right
        .iter()
        .enumerate()
        .filter_map(|(column, attribute)| left.contains(attribute).then_some(column))
        .collect()
}
