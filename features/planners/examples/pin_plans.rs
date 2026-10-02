//! Explicitly reviewable snapshot capture; ordinary tests never update fixtures.
use cq_planners::{Planner, compile, compile_valuation};
use cq_testkit::SCENARIOS;
use quote::ToTokens;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::create_dir_all(&root).unwrap();
    for (fixture, scenario_name) in [
        ("triangle", "triangle"),
        ("chain", "oriented_chain"),
        ("fine_triangle", "fine_triangle"),
    ] {
        let source = SCENARIOS
            .iter()
            .find(|s| s.name == scenario_name)
            .unwrap()
            .parse_cq()
            .unwrap();
        for planner in Planner::ALL {
            let stem = format!("{}_{fixture}", planner.name());
            let valuation = compile_valuation(&source, planner).unwrap();
            let algebra = compile(&source, planner).unwrap();
            std::fs::write(
                root.join(format!("{stem}.vo")),
                format!("{}\n", valuation.to_token_stream()),
            )
            .unwrap();
            std::fs::write(
                root.join(format!("{stem}.ra")),
                format!("{}\n", algebra.to_token_stream()),
            )
            .unwrap();
        }
    }
}
