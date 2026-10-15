use super::Source;
use cq_workloads::queries::typed::typed_calendar_shipping::run;

#[test]
fn csv_matches_duckdb_golden() {
    run(Source::Csv);
}

#[cfg(feature = "parquet")]
#[test]
fn parquet_matches_duckdb_golden() {
    run(Source::Parquet);
}
