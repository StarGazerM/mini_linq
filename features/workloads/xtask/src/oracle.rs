//! Schema adapter to the public independent testkit SQL oracle.
use crate::Result;
use cq_workloads::{QueryCase, dataset::Dataset};

pub fn evaluate(case: &QueryCase, dataset: &Dataset) -> Result<Vec<Vec<i32>>> {
    dataset.validate(&case.module()?)?;
    evaluate_sql(&dataset.relations, case.sql)
}

pub fn evaluate_sql(
    relations: &[cq_workloads::dataset::RelationData],
    sql: &str,
) -> Result<Vec<Vec<i32>>> {
    let inputs = relations
        .iter()
        .map(|r| cq_testkit::duckdb::RelationData {
            name: r.name.clone(),
            arity: r.arity,
            rows: r.rows.clone(),
        })
        .collect::<Vec<_>>();
    cq_testkit::duckdb::evaluate_sql(&inputs, sql).map_err(crate::testkit_error)
}
