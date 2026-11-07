#![cfg(feature = "planner-calibration")]
use cq_cost::calibration::lineage::nodes;
use cq_stage::calibration::{Coverage, Seed, Stage, Trace};

#[test]
fn actual_checked_boundaries_preserve_local_origins() {
    let source = syn::parse_quote! {
        struct Q; relation R(x:i32,y:i32); relation S(y:i32);
        answer(x) :- R(x,y), where x >= 1, S(y), if accept(x);
    };
    let options: cq_planning::PlanOptions = syn::parse_quote!(no_reduce);
    let (lowered, valuation, origins) =
        cq_planning::compile_prefix_with_origins(&source, &options).unwrap();
    let mut trace = Trace::default();
    trace.initialize(Seed {
        valuation: Stage::of(&valuation),
        lowering: Stage::of(&lowered),
        origins: origins.definitions.into_iter().map(|(node, origins)|
            Coverage {node, origins}).collect(),
        elided: origins.elided,
    }).unwrap();
    let original_coverage = trace.coverage(nodes).unwrap();
    let indexed = cq_index_policy::compile(&lowered).unwrap();
    trace.advance(Stage::of(&indexed));
    let physical = cq_physical_lowering::compile(&indexed).unwrap();
    let physical_nodes = nodes(&Stage::of(&physical)).unwrap();
    assert_eq!(nodes(trace.current().unwrap()).unwrap(), physical_nodes);
    trace.advance(Stage::of(&physical));
    let structures = cq_structure_selection::compile(&physical).unwrap();
    trace.advance(Stage::of(&structures));
    assert_eq!(trace.coverage(nodes).unwrap(), original_coverage);
    assert_eq!(nodes(trace.current().unwrap()).unwrap().len(), physical.program.definitions.len());
    let sequence = cq_sequence_lowering::compile(&structures).unwrap();
    trace.advance(Stage::of(&sequence));
    assert_eq!(trace.coverage(nodes).unwrap(), original_coverage);
}

#[test]
fn every_optional_planner_records_its_actual_pre_rewrite_lowering() {
    use cq_planners::{Planner, compile, compile_with_origins};
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
            valuation: Stage::of(&valuation),
            lowering: Stage::of(&lowering),
            origins: origins.definitions.into_iter().map(|(node, origins)| Coverage {node, origins}).collect(),
            elided: origins.elided,
        }).unwrap();
        trace.advance(Stage::of(&target));
        let coverage = trace.coverage(nodes).unwrap();
        assert_eq!(coverage.len(), target.plan.definitions.len());
        for (definition, coverage) in target.plan.definitions.iter().zip(coverage) {
            assert_eq!(definition.result.to_string(), coverage.node);
            assert!(!coverage.origins.is_empty());
        }
    }
}
