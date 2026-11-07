use std::collections::BTreeMap;
use cq_stage::calibration::{Node, Seed, Snapshot, Stage, Trace, Transition};
use cq_stage::{Language, Pipeline};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};

fn node(name: &str, inputs: &[&str]) -> Node {
    Node { name: name.into(), operator: format!("operator_{name}"),
        inputs: inputs.iter().map(|s| (*s).into()).collect() }
}
fn stage(program: &str) -> Stage {
    Stage { language: "test".into(), program: program.into(), bases: Vec::new() }
}
/// Calibrator-supplied graphs, keyed by recorded program.
#[derive(Default)]
struct Graphs(BTreeMap<String, Vec<Node>>);
impl Graphs {
    fn record(&mut self, program: &str, nodes: Vec<Node>) -> Stage {
        self.0.insert(program.into(), nodes);
        stage(program)
    }
    fn nodes(&self) -> impl FnMut(&Stage) -> syn::Result<Vec<Node>> + '_ {
        |stage| Ok(self.0[&stage.program].clone())
    }
}
fn ra() -> Vec<Node> {
    vec![node("a", &[]), node("b", &["a"]), node("unrelated", &[]), node("c", &["b"])]
}
fn trace(graphs: &mut Graphs) -> Trace {
    let mut trace = Trace::default();
    trace.initialize(Seed {
        valuation: graphs.record("vo", (0..4).map(|i| node(&i.to_string(), &[])).collect()),
        lowering: graphs.record("ra", ra()),
        steps: ["a", "b", "unrelated", "c"].into_iter().enumerate()
            .map(|(i, n)| (n.into(), vec![i])).collect(),
    }).unwrap();
    trace
}
#[test]
fn fusion_transfers_only_removed_dependency_cones() {
    let mut graphs = Graphs::default();
    let mut trace = trace(&mut graphs);
    trace.advance(graphs.record("physical", vec![node("a", &[]),
        node("unrelated", &[]), node("c", &["a"])]));
    let coverage = trace.coverage(graphs.nodes()).unwrap();
    assert_eq!(coverage[0].origins, ["0"]);
    assert_eq!(coverage[1].origins, ["2"]);
    assert_eq!(coverage[2].origins, ["1", "3"]);
    assert!(trace.transitions(graphs.nodes()).unwrap().1[0].elided.is_empty());
    let encoded = trace.descriptor().unwrap();
    assert_eq!(Trace::parse_descriptor(&encoded).unwrap(), trace);
}

#[test]
fn annotation_descriptor_preserves_bases_without_changing_selected_identity() {
    let program = quote!(structures {});
    let bases = vec!["physical {}".to_owned()];
    let selected = Stage::of(&program);
    let annotated = Stage::annotation(&program, bases);
    assert!(selected.bases.is_empty());
    assert_eq!(Snapshot::new(&selected, Vec::new()), Snapshot::new(&annotated, Vec::new()));
    let mut graphs = Graphs::default();
    let mut trace = trace(&mut graphs);
    trace.advance(annotated.clone());
    let seed = trace.seed.as_ref().unwrap();
    let language = &annotated.language;
    let printed = &annotated.program;
    let expected = quote!(calibration_trace { 4; [#seed]; [(#language, #printed, ["physical {}"])] });
    assert_eq!(trace.descriptor().unwrap(), expected.to_string());
    assert_eq!(Trace::parse_descriptor(&expected.to_string()).unwrap(), trace);
    trace.check_source(&annotated).unwrap();
    let mut changed_base = annotated;
    changed_base.bases = vec!["physical { changed }".into()];
    assert!(trace.check_source(&changed_base).is_err());
    trace.advance(changed_base.clone());
    assert_eq!(trace.stages.len(), 2);
    assert_eq!(trace.current(), Some(&changed_base));
}

#[test]
fn annotation_descriptor_requires_version_four_and_explicit_bases() {
    assert!(Trace::parse_descriptor("calibration_trace { 2; []; [] }").is_err());
    assert!(Trace::parse_descriptor("calibration_trace { 3; []; [] }").is_err());
    assert!(syn::parse_str::<Stage>(r#"("test", "structures {}")"#).is_err());
    let stage: Stage = syn::parse_str(r#"("test", "sequence {}", ["physical {}", "structures {}"])"#).unwrap();
    assert_eq!(stage.bases, ["physical {}", "structures {}"]);
}
#[test]
fn deleted_dead_work_is_explicit_not_fused_with_unrelated_nodes() {
    let mut graphs = Graphs::default();
    let mut trace = trace(&mut graphs);
    trace.advance(graphs.record("physical", vec![node("a", &[]), node("b", &["a"]),
        node("c", &["b"])]));
    assert_eq!(trace.transitions(graphs.nodes()).unwrap().1[0].elided, ["unrelated"]);
    assert!(trace.coverage(graphs.nodes()).unwrap().iter().all(|c| !c.origins.contains(&"2".into())));
}
#[test]
fn inserted_definition_anchors_to_its_actual_consumer() {
    let mut graphs = Graphs::default();
    let mut trace = trace(&mut graphs);
    trace.advance(graphs.record("physical", vec![node("a", &[]), node("b", &["a"]),
        node("unrelated", &[]), node("inserted", &["b"]), node("c", &["inserted"])]));
    assert_eq!(trace.coverage(graphs.nodes()).unwrap()[3].origins, ["3"]);
    let current = trace.transitions(graphs.nodes()).unwrap().1.pop().unwrap().target;
    let unanchored = Snapshot::new(&stage("unanchored"), vec![node("unanchored", &[])]);
    assert!(Transition::between(&current, unanchored).is_err());
}
#[test]
fn moved_operators_follow_payload_identity_not_reused_result_names() {
    let mut graphs = Graphs::default();
    let mut trace = trace(&mut graphs);
    let mut moved_b = node("b", &["a"]);
    moved_b.operator = "operator_c".into();
    let mut moved_c = node("c", &["b"]);
    moved_c.operator = "operator_b".into();
    trace.advance(graphs.record("physical", vec![node("a", &[]), moved_b,
        node("unrelated", &[]), moved_c]));
    let coverage = trace.coverage(graphs.nodes()).unwrap();
    assert_eq!(coverage[1].origins, ["3"]);
    assert_eq!(coverage[3].origins, ["1"]);
    assert_eq!(coverage[2].origins, ["2"]);
}
#[test]
fn complete_alpha_renaming_keeps_exact_origins() {
    let mut graphs = Graphs::default();
    let mut trace = trace(&mut graphs);
    let renamed = ra().iter().map(|node| Node {
        name: format!("new_{}", node.name),
        operator: node.operator.clone(),
        inputs: node.inputs.iter().map(|input| format!("new_{input}")).collect(),
    }).collect();
    trace.advance(graphs.record("physical", renamed));
    assert!(trace.transitions(graphs.nodes()).unwrap().1[0].elided.is_empty());
    for (index, coverage) in trace.coverage(graphs.nodes()).unwrap().iter().enumerate() {
        assert_eq!(coverage.origins, [index.to_string()]);
        assert!(coverage.node.starts_with("new_"));
    }
}
#[test]
fn unknown_vo_origins_and_unseeded_stages_are_rejected() {
    let mut graphs = Graphs::default();
    let mut forged = trace(&mut graphs);
    forged.seed.as_mut().unwrap().steps.insert("a".into(), vec![99]);
    assert!(forged.coverage(graphs.nodes()).is_err());
    let unseeded = Trace { seed: None, stages: vec![stage("physical")] };
    assert!(Trace::parse_descriptor(&unseeded.to_token_stream().to_string()).is_err());
}

#[test]
fn sparse_steps_follow_dependencies_without_inventing_vo_sites() {
    let mut graphs = Graphs::default();
    let mut trace = trace(&mut graphs);
    let seed = trace.seed.as_mut().unwrap();
    seed.steps.remove("b");
    seed.steps.remove("c");
    let coverage = trace.coverage(graphs.nodes()).unwrap();
    assert_eq!(coverage[1].origins, ["0"]);
    assert_eq!(coverage[3].origins, ["0"]);
}

struct Payload(syn::LitInt);
impl Parse for Payload {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> { input.parse().map(Self) }
}
impl ToTokens for Payload {
    fn to_tokens(&self, tokens: &mut TokenStream) { self.0.to_tokens(tokens); }
}
impl Language for Payload {
    fn check(&self) -> syn::Result<()> { Ok(()) }
    fn check_rewrite(_: &Self, _: &Self) -> syn::Result<()> { Ok(()) }
}
fn terminal_trace() -> Trace {
    let mut trace = trace(&mut Graphs::default());
    trace.advance(Stage::of(&Payload(syn::parse_quote!(1))));
    trace
}
#[test]
fn disabled_continuation_and_terminal_emit_identical_original_tokens() {
    let pipeline = Pipeline::default();
    assert!(pipeline.calibration.is_none());
    assert_eq!(pipeline.to_token_stream().to_string(), quote!(pipeline {}).to_string());
    let expression: syn::Expr = syn::parse_quote!({ original() });
    let result = cq_stage::pass::<Payload, syn::Expr>(quote!(pipeline {}; 1), |_, _| Ok(expression.clone()));
    assert_eq!(result.to_string(), expression.to_token_stream().to_string());
}
#[test]
fn registration_is_selected_before_the_unchanged_terminal_expression() {
    let trace = terminal_trace();
    let expression: syn::Expr = syn::parse_quote!({ original() });
    let result = cq_stage::pass::<Payload, syn::Expr>(quote!(pipeline {} #trace; 1), |_, _| Ok(expression.clone()));
    let descriptor = trace.descriptor().unwrap();
    assert_eq!(result.to_string(), quote!(::cq_storage::calibration_select! {
        plain { #expression }
        calibration {{ ::cq_storage::counters::calibration::register_plan(#descriptor); #expression }}
    }).to_string());
}
#[test]
fn source_tampering_is_rejected_before_transform() {
    let trace = terminal_trace();
    let result = cq_stage::pass::<Payload, syn::Expr>(quote!(pipeline {} #trace; 2), |_, _| panic!("forged source reached terminal"));
    assert!(result.to_string().contains("continuation does not match checked source"));
}
#[test]
fn enablement_without_actual_lowering_producer_is_rejected() {
    let mut pipeline = Pipeline::default();
    pipeline.enable_calibration();
    let result = cq_stage::pass::<Payload, syn::Expr>(quote!(#pipeline; 1), |_, _| Ok(syn::parse_quote!(original())));
    assert!(result.to_string().contains("no lowering origin producer"));
}
