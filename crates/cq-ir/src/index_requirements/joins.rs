//! Logical join columns, owned by IX analysis and its validity contract.

use crate::relational_plan;

/// Join columns in the right result's sorted logical heading.
pub fn result_join_columns(
    left: &relational_plan::RelationInfo,
    right: &relational_plan::RelationInfo,
) -> Vec<usize> {
    right
        .heading
        .iter()
        .enumerate()
        .filter_map(|(column, attribute)| left.heading.contains(attribute).then_some(column))
        .collect()
}
