use cq_planners::{Planner, compile, compile_with_origins};
use cq_stage::{Language, calibration::{Coverage, Seed, Trace}};

#[test]
fn every_optional_planner_records_its_actual_pre_rewrite_lowering() {
    let source = syn::parse_quote! {
        struct Q;
        relation R(x:i32,y:i32); relation S(y:i32,z:i32); relation T(z:i32);
        answer(x) :- R(x,y), S(y,z), T(z), where z >= 1;
    };
    for planner in Planner::ALL {
        let (target, valuation, lowering, origins) = compile_with_origins(&source, planner).unwrap();
        assert_eq!(target, compile(&source, planner).unwrap(), "{planner:?}");
        cq_relational_lowering::check_origins(&valuation, &lowering, &origins).unwrap();
        let mut trace = Trace::default();
        trace.initialize(Seed {
            valuation: valuation.calibration_snapshot().unwrap(),
            lowering: lowering.calibration_snapshot().unwrap(),
            origins: origins.definitions.into_iter().map(|(node, origins)| Coverage {node, origins}).collect(),
            elided: origins.elided,
        }).unwrap();
        trace.advance(target.calibration_snapshot().unwrap()).unwrap();
        let coverage = trace.coverage().unwrap();
        assert_eq!(coverage.len(), target.plan.definitions.len());
        for (definition, coverage) in target.plan.definitions.iter().zip(coverage) {
            assert_eq!(definition.result.to_string(), coverage.node);
            assert!(!coverage.origins.is_empty());
        }
    }
}
