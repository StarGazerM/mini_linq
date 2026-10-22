//! Pairwise pipeline coverage with visible typed query cases.
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../../../src/queries/classic/cartesian_product.rs"]
mod cartesian_product;
#[path = "../../../src/queries/classic/claw.rs"]
mod claw;
#[path = "../../../src/queries/classic/fact_star.rs"]
mod fact_star;
#[path = "../../../src/queries/classic/intersection.rs"]
mod intersection;
#[path = "../matrix.rs"]
mod matrix;
#[path = "../../../src/queries/classic/oriented_chain.rs"]
mod oriented_chain;
#[macro_use]
#[path = "../../../src/queries/classic/triangle.rs"]
mod triangle_case;
mod triangle {
    case_configurations!(triangle, run_triangle);
}
#[path = "../../../src/queries/classic/barbell.rs"]
mod barbell;
#[path = "../../../src/queries/classic/cycle_4.rs"]
mod cycle_4;
#[path = "../../../src/queries/classic/loomis_whitney_4.rs"]
mod loomis_whitney_4;
#[path = "../../../src/queries/classic/spade.rs"]
mod spade;

#[test]
fn every_catalog_case_axis_value_and_pair_is_exercised() {
    let cases = [
        ("intersection", intersection::CONFIGURATIONS),
        ("cartesian-product", cartesian_product::CONFIGURATIONS),
        ("oriented-chain", oriented_chain::CONFIGURATIONS),
        ("claw", claw::CONFIGURATIONS),
        ("fact-star", fact_star::CONFIGURATIONS),
        ("triangle", triangle::CONFIGURATIONS),
        ("cycle-4", cycle_4::CONFIGURATIONS),
        ("spade", spade::CONFIGURATIONS),
        ("barbell", barbell::CONFIGURATIONS),
        ("loomis-whitney-4", loomis_whitney_4::CONFIGURATIONS),
    ];
    matrix::verify_catalog("classic", &cases.map(|(name, _)| name));
    let rows: Vec<_> = cases
        .into_iter()
        .flat_map(|(_, rows)| rows.iter().copied())
        .collect();
    matrix::verify_cover(&rows);
}
