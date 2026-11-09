//! Static advice uses the same authored grammar and pre-Storage compiler entry.
//! This module prints recorded selections; it never chooses an order.
use super::{AdmissionHint, AdmissionOwner, AdviceReport, Decision, FixedAdmission, HoleAdvice, IndexDemand, IndexVersion};
use crate::export::{self, CheckedIr, Origin, PlannerInput, Result};
use cq_datalog_incremental::{Known, OrderAtom, RulePlan};
use cq_lang_datalog_round as round;
use cq_syntax::symbol_name;
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Token, parse::{Parse, ParseStream}};

fn error(message: impl Into<String>) -> export::ExportError { export::ExportError(message.into()) }

/// Outer directive transport; the RulePlan body parser is shared with macros.
#[derive(Clone, Debug)]
pub struct AdviceList(pub Vec<RulePlan>);
impl Parse for AdviceList {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut plans = Vec::new();
        while !input.is_empty() {
            let name: syn::Ident = input.parse()?;
            if name != "plan" { return Err(syn::Error::new_spanned(name, "expected plan(...) advice")); }
            let body;
            syn::parenthesized!(body in input);
            plans.push(body.parse()?);
            input.parse::<Token![;]>()?;
        }
        Ok(Self(plans))
    }
}
impl ToTokens for AdviceList {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        for plan in &self.0 { tokens.extend(quote!(plan(#plan);)); }
    }
}
fn selector(query: &round::cq::Query, position: usize) -> Result<OrderAtom> {
    let Some(round::cq::BodyItem::Positive { atom }) = query.body.iter().nth(position) else {
        return Err(error("advice selector has no original positive source occurrence"));
    };
    Ok(OrderAtom { relation: atom.relation.clone(), occurrence: Some(position), columns: None })
}
fn prefix(query: &round::cq::Query) -> std::ops::Range<usize> {
    let first = query.body.iter().position(|i| matches!(i, round::cq::BodyItem::Positive { .. })).unwrap_or(query.body.len());
    first..first + query.body.iter().skip(first).take_while(|i| matches!(i, round::cq::BodyItem::Positive { .. })).count()
}

fn check_report(input: &PlannerInput, report: &AdviceReport) -> Result<()> {
    if report.input_hash != input.input_hash || report.identity != input.identity
        || report.holes.len() != input.holes.len()
    {
        return Err(error("static advice belongs to a different checked compiler input"));
    }
    let mut seen = BTreeSet::new();
    for choice in &report.holes {
        let original = input.holes.get(choice.hole).ok_or_else(|| error("advice references an absent hole"))?;
        if !seen.insert(choice.hole) || choice.origin != original.origin
            || choice.candidates.get(choice.selected).is_none()
        {
            return Err(error("advice hole identity, coverage or selected candidate is invalid"));
        }
    }
    Ok(())
}

fn expected_admission(program: &round::Program, choice: &HoleAdvice) -> Result<FixedAdmission> {
    let Origin::Rule { variant, admission, .. } = &choice.origin else {
        return Err(error("only a rule variant has an admission owner"));
    };
    let source = &program.storage.incremental.strata.source;
    let variant_record = program.storage.incremental.variants.get(*variant)
        .ok_or_else(|| error("admission constraint references an absent variant"))?;
    let owner = match admission.as_str() {
        "admit" => AdmissionOwner::Admit,
        "in_plan" => AdmissionOwner::InPlan,
        _ => return Err(error("admission constraint has no checked input owner")),
    };
    let hint = match variant_record.known {
        Some(Known::Admit) => AdmissionHint::Admit {},
        Some(Known::At(level)) => AdmissionHint::At { level },
        None if owner == AdmissionOwner::Admit => AdmissionHint::Admit {},
        None => {
            // The original automatic InPlan fact proves the head is available
            // before a later positive read. Naming the complete positive level
            // is a checked ready boundary for every prefix permutation; I2
            // still drains the committed filter at its earliest actual binding.
            let level = source.rules[variant_record.rule].body.iter()
                .filter(|item| matches!(item, round::cq::BodyItem::Positive { .. })).count();
            if level == 0 { return Err(error("InPlan owner has no positive ready level")); }
            AdmissionHint::At { level }
        }
    };
    if (owner == AdmissionOwner::Admit) != matches!(hint, AdmissionHint::Admit {}) {
        return Err(error("recorded known advice disagrees with the hole's admission owner"));
    }
    Ok(FixedAdmission {
        held_fixed: true, owner, hint, source_variant: *variant,
        alternative_owner: "not compared: a different owner may be cheaper, but is not a static-order choice".into(),
    })
}

/// Produce an explicit constraint by copying the checked owner, never by
/// selecting a cheaper alternative owner. This runs before text emission.
pub(crate) fn record_admission_constraints(input: &PlannerInput, report: &mut AdviceReport) -> Result<()> {
    check_report(input, report)?;
    let CheckedIr::Datalog { round: recorded } = &input.checked_ir else { return Ok(()); };
    if !report.holes.iter().any(|hole| hole.decision == Decision::Changed) { return Ok(()); }
    let program: round::Program = syn::parse_str(recorded)?;
    for choice in &mut report.holes {
        if choice.decision == Decision::Changed && matches!(choice.origin, Origin::Rule { .. }) {
            choice.admission = Some(expected_admission(&program, choice)?);
            choice.citations.insert(format!("compiler:hole:{}:admission-held-fixed", choice.hole));
        }
    }
    Ok(())
}

fn check_constraints(program: &round::Program, report: &AdviceReport) -> Result<()> {
    for choice in &report.holes {
        let expected = if choice.decision == Decision::Changed && matches!(choice.origin, Origin::Rule { .. }) {
            Some(expected_admission(program, choice)?)
        } else { None };
        if choice.admission != expected {
            return Err(error(format!("hole {} admission constraint does not preserve its recorded owner/ready level", choice.hole)));
        }
    }
    Ok(())
}

/// Independently verify the admission constraints without replaying cost choice.
pub fn check_admission_constraints(input: &PlannerInput, report: &AdviceReport) -> Result<()> {
    input.check()?;
    check_report(input, report)?;
    match &input.checked_ir {
        CheckedIr::Datalog { round } => check_constraints(&syn::parse_str(round)?, report),
        CheckedIr::Cq {} if report.holes.iter().all(|hole| hole.admission.is_none()) => Ok(()),
        CheckedIr::Cq {} => Err(error("CQ advice cannot invent a Datalog admission owner")),
    }
}

fn known(hint: AdmissionHint) -> Known {
    match hint { AdmissionHint::Admit {} => Known::Admit, AdmissionHint::At { level } => Known::At(level) }
}

/// Preserve all existing authored phase/site/admission directives while replacing
/// only selected whole-occurrence orders. The result is a complete replacement
/// directive set, not something to append beside conflicting old advice.
pub fn datalog_plans(input: &PlannerInput, report: &AdviceReport) -> Result<Vec<RulePlan>> {
    input.check()?;
    datalog_plans_checked(input, report)
}

fn datalog_plans_checked(input: &PlannerInput, report: &AdviceReport) -> Result<Vec<RulePlan>> {
    check_report(input, report)?;
    let CheckedIr::Datalog { round: recorded } = &input.checked_ir else { return Err(error("Datalog advice needs a recorded Round")); };
    let program: round::Program = syn::parse_str(recorded)?;
    check_constraints(&program, report)?;
    let source = &program.storage.incremental.strata.source;
    let choices = report.holes.iter().filter_map(|hole| match hole.origin {
        Origin::Rule { variant, .. } => Some((variant, hole)), _ => None,
    }).collect::<BTreeMap<_, _>>();
    let mut plans = Vec::new();
    for (index, variant) in program.storage.incremental.variants.iter().enumerate() {
        let choice = choices.get(&index).ok_or_else(|| error("advice report lost an Incremental variant"))?;
        let changed = choice.decision == Decision::Changed;
        if !changed && !variant.authored && variant.known.is_none() { continue; }
        let label = source.labels[variant.rule].clone().ok_or_else(|| error("cost-changing advice requires a source rule label"))?;
        if source.invariants.iter().any(|entry| entry.rule == variant.rule) {
            return Err(error("authored advice cannot replace an invariant-hoisted rule"));
        }
        let query = &source.rules[variant.rule];
        let mut order = Vec::new();
        if changed {
            if !variant.steps.is_empty() || !variant.smallest.is_empty() {
                return Err(error("whole-order cost advice cannot discard authored phase/intersection decisions"));
            }
            for &position in &choice.candidates[choice.selected].plan.order { order.push(selector(query, position)?); }
        } else if !variant.steps.is_empty() {
            for step in &variant.steps {
                let mut atom = selector(query, step.occurrence)?;
                atom.columns = step.columns.clone();
                order.push(atom);
            }
        } else if variant.authored {
            let allowed = prefix(query);
            for &position in &variant.order {
                if allowed.contains(&position) { order.push(selector(query, position)?); }
            }
        }
        let smallest = variant.smallest.iter().map(|site| site.iter().map(|&position| selector(query, position)).collect())
            .collect::<Result<Vec<Vec<_>>>>()?;
        plans.push(RulePlan {
            rule: label, delta: variant.delta.map(|position| selector(query, position)).transpose()?,
            order, smallest, known: if changed {
                Some(known(choice.admission.as_ref().ok_or_else(|| error("changed variant lacks its fixed admission constraint"))?.hint))
            } else { variant.known },
        });
    }
    Ok(plans)
}

/// Compile advice before Storage, using the recorded logical driver and shared
/// production checkers. This receipt does not claim GPU/custom physical pipeline
/// realization; those front ends consume the same directives on adoption.
pub fn compile_datalog_advice(input: &PlannerInput, plans: &[RulePlan]) -> Result<round::Program> {
    input.check()?;
    compile_datalog_advice_checked(input, plans)
}

fn compile_datalog_advice_checked(input: &PlannerInput, plans: &[RulePlan]) -> Result<round::Program> {
    let CheckedIr::Datalog { round: recorded } = &input.checked_ir else { return Err(error("expected Datalog input")); };
    let original: round::Program = syn::parse_str(recorded)?;
    let incremental = cq_datalog_incremental::compile_with_advice(
        &original.storage.incremental.strata, original.storage.incremental.driver, plans,
    )?;
    let storage = cq_datalog_storage::compile(&incremental)?;
    let planned = cq_datalog_fork::compile(&storage)?;
    for hole in &planned.holes {
        let pipeline = hole.invocation_pipeline()?;
        let at = hole.plan.ok_or_else(|| error("advised hole lacks its planning invocation"))?;
        let options: cq_planning::PlanOptions = syn::parse2(pipeline.passes[at].arguments.clone())?;
        let (normal, plan) = cq_planning::prefix_valuation(&hole.cq, &options)?;
        cq_lang_vo::contract::check(&normal, &plan)?;
    }
    Ok(planned)
}

fn demanded(program: &round::Program) -> BTreeSet<IndexDemand> {
    let source = &program.storage.incremental.strata.source;
    let mut result = BTreeSet::new();
    for (version, layouts) in [(IndexVersion::Full, &program.context.layouts), (IndexVersion::Delta, &program.context.delta_layouts)] {
        for layout in layouts {
            let mut key = layout.columns[..layout.key].to_vec(); key.sort_unstable();
            result.insert(IndexDemand { logical_relation: symbol_name(&source.declarations[layout.relation].name), version, key });
        }
    }
    result
}

fn check_compiled(report: &AdviceReport, planned: &round::Program) -> Result<()> {
    let actual = demanded(planned);
    for hole in &report.holes {
        if let Some(constraint) = &hole.admission {
            let actual_owner = planned.context.strata.iter().flat_map(|s| s.seeds.iter().chain(&s.steps))
                .find(|rule| rule.variant == constraint.source_variant)
                .ok_or_else(|| error("advised program lost a constrained variant"))?.known;
            let expected_owner = match constraint.owner {
                AdmissionOwner::Admit => round::Admission::Admit,
                AdmissionOwner::InPlan => round::Admission::InPlan,
            };
            if actual_owner != expected_owner {
                return Err(error("compiled admission owner differs from the costed hole's fixed constraint"));
            }
        }
        for demand in &hole.candidates[hole.selected].plan.demands {
            if !actual.contains(demand) {
                return Err(error(format!("hole {} {:?}: advised Storage does not realize recorded candidate demand {demand:?}", hole.hole, hole.origin)));
            }
        }
    }
    Ok(())
}

/// Check adopted directives against the costed hole constraints before compiling.
/// Removing/changing a known constraint cannot silently select another owner.
pub fn check_datalog_advice(input: &PlannerInput, report: &AdviceReport, plans: &[RulePlan]) -> Result<round::Program> {
    input.check()?;
    let expected = datalog_plans_checked(input, report)?;
    if plans != expected {
        return Err(error("adopted advice differs from the recorded orders or fixed admission constraints"));
    }
    let planned = compile_datalog_advice_checked(input, plans)?;
    check_compiled(report, &planned)?;
    Ok(planned)
}

pub(crate) fn validate_and_render(input: &PlannerInput, report: &AdviceReport) -> Result<(String, String)> {
    check_report(input, report)?;
    let mut text = format!("// static advice; dataset {}\n// input {}; evidence {}; target {}\n", report.dataset_hash, report.input_hash, report.evidence_hash, report.target);
    for fact in [&report.target_constants.build_per_row_h, &report.target_constants.merge_per_row_m] {
        text.push_str(&format!("// declared unit cost {}: {:?}; provenance {}\n",
            fact.id.escape_debug(), fact.value, fact.provenance.method.escape_debug()));
    }
    for hole in &report.holes {
        text.push_str(&format!("// hole {} {:?}: {}; candidates={}\n", hole.hole, hole.decision, hole.reason.escape_debug(), hole.candidates.len()));
        if !hole.citations.is_empty() {
            text.push_str("// citations: "); text.push_str(&hole.citations.iter().map(|id| id.escape_debug().to_string()).collect::<Vec<_>>().join(", ")); text.push('\n');
        }
        if let Some(admission) = &hole.admission {
            text.push_str(&format!("// admission held fixed: {:?}; hint {:?}; {}\n",
                admission.owner, admission.hint, admission.alternative_owner.escape_debug()));
        }
    }
    match &input.checked_ir {
        CheckedIr::Cq {} => {
            let choice = report.holes.first().ok_or_else(|| error("CQ report lacks its hole"))?;
            if choice.decision == Decision::Changed {
                let source: round::cq::Program = syn::parse_str(&input.source)?;
                let selectors = choice.candidates[choice.selected].plan.order.iter().map(|&position| selector(&source.query, position)).collect::<Result<Vec<_>>>()?;
                let options = cq_planning::PlanOptions { order: Some(selectors), ..Default::default() };
                let (normal, planned) = cq_planning::prefix_valuation(&source, &options)?;
                cq_lang_vo::contract::check(&normal, &planned)?;
                let selected = choice.candidates[choice.selected].plan.order.iter().map(|&position| {
                    let round::cq::BodyItem::Positive { atom } = &source.query.body[position] else { unreachable!("checked selector") };
                    let relation = &atom.relation; let position = syn::Index::from(position);
                    quote!(#relation @ #position)
                });
                text.push_str(&quote!(plan(order(#(#selected),*));).to_string()); text.push('\n');
            }
            Ok((text, "shared production CQ planning and VO-Order; target realization required on adoption".into()))
        }
        CheckedIr::Datalog { .. } => {
            let plans = datalog_plans_checked(input, report)?;
            // Parse exactly the syntax the tool emits through the compiler-owned
            // RulePlan parser, then exercise the actual pre-Storage entry.
            let syntax = AdviceList(plans).to_token_stream();
            let reparsed: AdviceList = syn::parse2(syntax.clone())?;
            let planned = compile_datalog_advice_checked(input, &reparsed.0)?;
            check_compiled(report, &planned)?;
            text.push_str("// Complete replacement for existing authored directives; never append duplicate rule/variant advice.\n");
            for plan in reparsed.0 { text.push_str(&quote!(plan(#plan);).to_string()); text.push('\n'); }
            Ok((text, "production Source/Incremental/Storage/Fork/CQ/VO-Order with recorded driver; target-specific realization required on adoption".into()))
        }
    }
}
