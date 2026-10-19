//! Explicitly reviewable snapshot capture; ordinary tests never update fixtures.
use cq_planners::{Planner, compile, compile_valuation};
use cq_testkit::SCENARIOS;
use quote::ToTokens;

fn main() {
    let selected = std::env::args().nth(1);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::create_dir_all(&root).unwrap();
    for (fixture, scenario_name) in [("triangle", "triangle"), ("chain", "oriented_chain")] {
        let source = SCENARIOS
            .iter()
            .find(|s| s.name == scenario_name)
            .unwrap()
            .parse_cq()
            .unwrap();
        for planner in Planner::ALL {
            if selected.as_ref().is_some_and(|name| name != planner.name()) {
                continue;
            }
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
    for fixture in ["retail_h05", "retail_h09", "retail_d72_inner", "block"] {
        if selected.as_ref().is_some_and(|name| name != "le") {
            continue;
        }
        let stem = format!("le_{fixture}");
        let source =
            syn::parse_str(&std::fs::read_to_string(root.join(format!("{stem}.cq"))).unwrap())
                .unwrap();
        let valuation = compile_valuation(&source, Planner::Le).unwrap();
        let algebra = compile(&source, Planner::Le).unwrap();
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
