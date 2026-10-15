//! Pairwise pipeline coverage with visible typed query cases.
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../matrix.rs"]
mod matrix;
#[path = "../../../src/queries/retail/retail_h03.rs"]
mod retail_h03;
#[path = "../../../src/queries/retail/retail_h05.rs"]
mod retail_h05;
#[path = "../../../src/queries/retail/retail_h07.rs"]
mod retail_h07;
#[path = "../../../src/queries/retail/retail_h08.rs"]
mod retail_h08;
#[path = "../../../src/queries/retail/retail_h09.rs"]
mod retail_h09;
#[path = "../../../src/queries/retail/retail_h21_positive.rs"]
mod retail_h21_positive;
#[path = "../../../src/queries/retail/retail_d19.rs"]
mod retail_d19;
#[path = "../../../src/queries/retail/retail_d27.rs"]
mod retail_d27;
#[path = "../../../src/queries/retail/retail_d72_inner.rs"]
mod retail_d72_inner;
#[path = "../../../src/queries/retail/retail_d85.rs"]
mod retail_d85;

#[test]
fn every_catalog_case_axis_value_and_pair_is_exercised() {
    let cases = [
        ("retail_h03", retail_h03::CONFIGURATIONS),
        ("retail_h05", retail_h05::CONFIGURATIONS),
        ("retail_h07", retail_h07::CONFIGURATIONS),
        ("retail_h08", retail_h08::CONFIGURATIONS),
        ("retail_h09", retail_h09::CONFIGURATIONS),
        ("retail_h21_positive", retail_h21_positive::CONFIGURATIONS),
        ("retail_d19", retail_d19::CONFIGURATIONS),
        ("retail_d27", retail_d27::CONFIGURATIONS),
        ("retail_d72_inner", retail_d72_inner::CONFIGURATIONS),
        ("retail_d85", retail_d85::CONFIGURATIONS),
    ];
    matrix::verify_catalog("retail", &cases.map(|(name, _)| name));
    let rows: Vec<_> = cases.into_iter().flat_map(|(_, rows)| rows.iter().copied()).collect();
    matrix::verify_cover(&rows);
}
