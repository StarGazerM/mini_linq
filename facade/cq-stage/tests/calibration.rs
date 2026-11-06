use cq_stage::calibration::{Coverage, Node, Seed, Snapshot, Trace, Transition};
use cq_stage::{Language, Pipeline};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};

fn node(name: &str, inputs: &[&str]) -> Node {
    Node { name: name.into(), operator: format!("operator_{name}"),
        inputs: inputs.iter().map(|s| (*s).into()).collect() }
}
fn snapshot(language: &str, nodes: Vec<Node>) -> Snapshot {
    Snapshot { language: language.into(), program: language.into(), nodes }
}
fn trace() -> Trace {
    let mut trace = Trace::default();
    trace.initialize(Seed {
        valuation: snapshot("vo", (0..4).map(|i| node(&i.to_string(), &[])).collect()),
        lowering: snapshot("ra", vec![node("a", &[]), node("b", &["a"]),
            node("unrelated", &[]), node("c", &["b"])]),
        origins: ["a", "b", "unrelated", "c"].into_iter().enumerate()
            .map(|(i, n)| Coverage {node:n.into(), origins:vec![i.to_string()]}).collect(),
        elided: Vec::new(),
    }).unwrap();
    trace
}
#[test]
fn fusion_transfers_only_removed_dependency_cones() {
    let mut trace = trace();
    trace.advance(snapshot("physical", vec![node("a", &[]),
        node("unrelated", &[]), node("c", &["a"])])).unwrap();
    let coverage = trace.coverage().unwrap();
    assert_eq!(coverage[0].origins, ["0"]);
    assert_eq!(coverage[1].origins, ["2"]);
    assert_eq!(coverage[2].origins, ["1", "3"]);
    assert!(trace.transitions[0].elided.is_empty());
    let encoded = trace.descriptor().unwrap();
    assert_eq!(Trace::parse_descriptor(&encoded).unwrap(), trace);
}
#[test]
fn deleted_dead_work_is_explicit_not_fused_with_unrelated_nodes() {
    let mut trace = trace();
    trace.advance(snapshot("physical", vec![node("a", &[]), node("b", &["a"]),
        node("c", &["b"])])).unwrap();
    assert_eq!(trace.transitions[0].elided, ["unrelated"]);
    assert!(trace.coverage().unwrap().iter().all(|c| !c.origins.contains(&"2".into())));
}
#[test]
fn inserted_definition_anchors_to_its_actual_consumer() {
    let mut trace = trace();
    trace.advance(snapshot("physical", vec![node("a", &[]), node("b", &["a"]),
        node("unrelated", &[]), node("inserted", &["b"]), node("c", &["inserted"])])).unwrap();
    assert_eq!(trace.coverage().unwrap()[3].origins, ["3"]);
    assert!(Transition::between(trace.current().unwrap(), snapshot("physical", vec![node("unanchored", &[])])).is_err());
}
#[test]
fn moved_operators_follow_payload_identity_not_reused_result_names() {
    let mut trace = trace();
    let mut moved_b = node("b", &["a"]);
    moved_b.operator = "operator_c".into();
    let mut moved_c = node("c", &["b"]);
    moved_c.operator = "operator_b".into();
    trace.advance(snapshot("physical", vec![node("a", &[]), moved_b,
        node("unrelated", &[]), moved_c])).unwrap();
    let coverage = trace.coverage().unwrap();
    assert_eq!(coverage[1].origins, ["3"]);
    assert_eq!(coverage[3].origins, ["1"]);
    assert_eq!(coverage[2].origins, ["2"]);
}
#[test]
fn complete_alpha_renaming_keeps_exact_origins() {
    let mut trace = trace();
    let renamed = trace.current().unwrap().nodes.iter().map(|node| Node {
        name: format!("new_{}", node.name),
        operator: node.operator.clone(),
        inputs: node.inputs.iter().map(|input| format!("new_{input}")).collect(),
    }).collect();
    trace.advance(snapshot("physical", renamed)).unwrap();
    assert!(trace.transitions[0].elided.is_empty());
    for (index, coverage) in trace.coverage().unwrap().iter().enumerate() {
        assert_eq!(coverage.origins, [index.to_string()]);
        assert!(coverage.node.starts_with("new_"));
    }
}
#[test]
fn forged_mapping_and_unknown_vo_origins_are_rejected() {
    let mut trace = trace();
    trace.advance(snapshot("physical", vec![node("a", &[]), node("unrelated", &[]), node("c", &["a"])])).unwrap();
    trace.transitions[0].parents[2].origins.push("unrelated".into());
    assert!(Trace::parse_descriptor(&trace.to_token_stream().to_string()).is_err());
    let mut forged = self::trace();
    forged.seed.as_mut().unwrap().origins[0].origins[0] = "99".into();
    assert!(forged.check().is_err());
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
    fn calibration_snapshot(&self) -> Option<Snapshot> {
        let mut snapshot = snapshot("physical", vec![node("a", &[]), node("b", &["a"]),
            node("unrelated", &[]), node("c", &["b"])]);
        snapshot.program = self.to_token_stream().to_string();
        Some(snapshot)
    }
}
fn terminal_trace() -> Trace {
    let mut trace = trace();
    trace.advance(Payload(syn::parse_quote!(1)).calibration_snapshot().unwrap()).unwrap();
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
