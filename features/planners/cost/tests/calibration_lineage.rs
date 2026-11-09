#![cfg(feature = "planner-calibration")]
use cq_cost::calibration::lineage::{checked_sequence, checked_structures, checked_valuation, nodes};
use cq_stage::calibration::{Seed, Stage, Trace};
use quote::ToTokens;

#[test]
fn actual_checked_boundaries_preserve_local_origins() {
    let source = syn::parse_quote! {
        struct Q; relation R(x:i32,y:i32); relation S(y:i32);
        answer(x) :- R(x,y), where x >= 1, S(y), if accept(x);
    };
    let options: cq_planning::PlanOptions = syn::parse_quote!(no_reduce);
    let (lowered, normal, valuation, steps) =
        cq_planning::compile_prefix_with_steps(&source, &options).unwrap();
    let valuation_stage = Stage::annotation(&valuation, vec![normal.to_token_stream().to_string()]);
    let checked = checked_valuation(&valuation_stage).unwrap();
    assert_eq!(checked.cq, normal);
    assert_eq!(checked.valuation, valuation);
    assert!(nodes(&Stage::of(&valuation)).is_err());
    let mut unrelated = valuation_stage.clone();
    unrelated.bases[0] = "struct Wrong; relation Missing(x:i32); answer(x) :- Missing(x);".into();
    assert!(nodes(&unrelated).is_err());
    let mut trace = Trace::default();
    trace.initialize(Seed {
        valuation: valuation_stage,
        lowering: Stage::of(&lowered),
        steps: steps.0,
    }).unwrap();
    let original_coverage = trace.coverage(nodes).unwrap();
    let indexed = cq_index_policy::compile(&lowered).unwrap();
    trace.advance(Stage::of(&indexed));
    let physical = cq_physical_lowering::compile(&indexed).unwrap();
    let physical_nodes = nodes(&Stage::of(&physical)).unwrap();
    assert_eq!(nodes(trace.current().unwrap()).unwrap(), physical_nodes);
    trace.advance(Stage::of(&physical));
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let structures_stage = Stage::annotation(
        &structures, vec![physical.to_token_stream().to_string()],
    );
    let checked = checked_structures(&structures_stage).unwrap();
    assert_eq!(checked.physical.to_token_stream().to_string(), structures_stage.bases[0]);
    assert_eq!(checked.structures.to_token_stream().to_string(), structures_stage.program);
    assert!(nodes(&Stage::of(&structures)).is_err());
    trace.advance(structures_stage);
    assert_eq!(trace.coverage(nodes).unwrap(), original_coverage);
    assert_eq!(nodes(trace.current().unwrap()).unwrap().len(), physical.program.definitions.len());
    let sequence = cq_sequence_lowering::compile(&physical, &structures).unwrap();
    let sequence_stage = Stage::annotation(&sequence, vec![
        physical.to_token_stream().to_string(), structures.to_token_stream().to_string(),
    ]);
    let checked = checked_sequence(&sequence_stage).unwrap();
    assert_eq!(checked.physical.to_token_stream().to_string(), sequence_stage.bases[0]);
    assert_eq!(checked.structures.to_token_stream().to_string(), sequence_stage.bases[1]);
    assert_eq!(checked.sequence.to_token_stream().to_string(), sequence_stage.program);
    assert!(nodes(&Stage::of(&sequence)).is_err());
    let mut missing_base = sequence_stage.clone();
    missing_base.bases.pop();
    assert!(nodes(&missing_base).is_err());
    let mut malformed_base = sequence_stage.clone();
    malformed_base.bases[1] = "not_a_structures_annotation".into();
    assert!(nodes(&malformed_base).is_err());
    trace.advance(sequence_stage);
    assert_eq!(trace.coverage(nodes).unwrap(), original_coverage);
    let descriptor = trace.descriptor().unwrap();
    let restored = Trace::parse_descriptor(&descriptor).unwrap();
    assert_eq!(restored, trace);
    assert_eq!(restored.coverage(nodes).unwrap(), original_coverage);
}

#[test]
fn every_optional_planner_records_its_actual_pre_rewrite_lowering() {
    use cq_planners::{Planner, compile, compile_with_steps};
    let source = syn::parse_quote! {
        struct Q;
        relation R(x:i32,y:i32); relation S(y:i32,z:i32); relation T(z:i32);
        answer(x) :- R(x,y), S(y,z), T(z), where z >= 1;
    };
    for planner in Planner::ALL {
        let (target, normal, valuation, lowering, steps) = compile_with_steps(&source, planner).unwrap();
        assert_eq!(target, compile(&source, planner).unwrap(), "{planner:?}");
        cq_relational_lowering::check(&normal, &valuation, &lowering, &steps).unwrap();
        let mut trace = Trace::default();
        trace.initialize(Seed {
            valuation: Stage::annotation(&valuation, vec![normal.to_token_stream().to_string()]),
            lowering: Stage::of(&lowering),
            steps: steps.0,
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
