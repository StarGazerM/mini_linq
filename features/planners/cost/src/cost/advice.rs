//! Static advice uses the same authored grammar and pre-Storage compiler entry.
//! This module prints recorded selections; it never chooses an order.
use super::{AdmissionHint, AdmissionOwner, AdviceReport, Candidate, Decision, FixedAdmission, IndexDemand, IndexVersion};
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
/// A valuation as authored selectors. An atom order names each reorderable
/// occurrence once; otherwise every read of one is named in step order, a
/// lookup or bind with the source variables it retains.
fn selectors(query: &round::cq::Query, plan: &Candidate) -> Result<Vec<OrderAtom>> {
    if plan.is_atom_order() {
        return plan.order.iter().map(|&position| selector(query, position)).collect();
    }
    let mut order = Vec::new();
    for read in plan.steps.iter().filter_map(|step| step.read.as_ref()) {
        if !plan.order.contains(&read.source_occurrence) { continue; }
        let mut atom = selector(query, read.source_occurrence)?;
        if read.partial {
            let Some(round::cq::BodyItem::Positive { atom: source }) = query.body.iter().nth(read.source_occurrence) else {
                return Err(error("advice selector has no original positive source occurrence"));
            };
            atom.columns = Some(read.columns.iter().filter(|column| column.variable.is_some()).map(|column| {
                source.args.iter().nth(column.source_column).and_then(cq_syntax::rust::variable).cloned()
                    .ok_or_else(|| error("a retained column is not a variable of its source occurrence"))
            }).collect::<Result<Vec<_>>>()?);
        }
        order.push(atom);
    }
    Ok(order)
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

fn expected_admission(program: &round::Program, origin: &Origin) -> Result<FixedAdmission> {
    let Origin::Rule { variant, admission, .. } = origin else {
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
/// `program` is the input's recorded Round, parsed; `None` for a CQ.
pub(crate) fn record_admission_constraints(input: &PlannerInput, program: Option<&round::Program>, report: &mut AdviceReport) -> Result<()> {
    check_report(input, report)?;
    let Some(program) = program else { return Ok(()); };
    if !report.holes.iter().any(|hole| hole.decision == Decision::Changed) { return Ok(()); }
    for choice in &mut report.holes {
        if choice.decision == Decision::Changed && matches!(choice.origin, Origin::Rule { .. }) {
            choice.admission = Some(expected_admission(program, &choice.origin)?);
            choice.citations.insert(format!("compiler:hole:{}:admission-held-fixed", choice.hole));
        }
    }
    Ok(())
}

fn check_constraints(program: &round::Program, report: &AdviceReport) -> Result<()> {
    for choice in &report.holes {
        let expected = if choice.decision == Decision::Changed && matches!(choice.origin, Origin::Rule { .. }) {
            Some(expected_admission(program, &choice.origin)?)
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
    datalog_plans_checked(input, &recorded(input)?, report)
}

/// The directive of one variant: a selected valuation with its admission
/// constraint, or else the variant's own authored directives, if any.
fn variant_plan(program: &round::Program, index: usize, replacement: Option<(&Candidate, Option<Known>)>) -> Result<Option<RulePlan>> {
    let source = &program.storage.incremental.strata.source;
    let variant = &program.storage.incremental.variants[index];
    if replacement.is_none() && !variant.authored && variant.known.is_none() { return Ok(None); }
    let label = source.labels[variant.rule].clone().ok_or_else(|| error("cost-changing advice requires a source rule label"))?;
    if source.invariants.iter().any(|entry| entry.rule == variant.rule) {
        return Err(error("authored advice cannot replace an invariant-hoisted rule"));
    }
    let query = &source.rules[variant.rule];
    let mut order = Vec::new();
    if let Some((plan, _)) = replacement {
        // The valuation owns lookup and bind phases; intersection sites
        // are a Physical decision this tool does not cost.
        if !variant.smallest.is_empty() {
            return Err(error("cost advice cannot discard authored intersection decisions"));
        }
        order = selectors(query, plan)?;
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
    Ok(Some(RulePlan {
        rule: label, delta: variant.delta.map(|position| selector(query, position)).transpose()?,
        order, smallest, known: replacement.map_or(variant.known, |(_, known)| known),
    }))
}

/// The recorded Round program of a Datalog input.
pub(crate) fn recorded(input: &PlannerInput) -> Result<round::Program> {
    let CheckedIr::Datalog { round: recorded } = &input.checked_ir else { return Err(error("Datalog advice needs a recorded Round")); };
    Ok(syn::parse_str(recorded)?)
}

/// `check` of every item, each alone, in the items' order: on threads, each
/// with its own parse of the recorded program (a parsed program stays on
/// its thread), parsed once per thread.
pub(crate) fn each_alone<T: Sync, R: Send>(input: &PlannerInput, items: &[T], check: impl Fn(&round::Program, &T) -> Result<R> + Sync) -> Result<Vec<R>> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, usize::from).min(items.len());
    let mut done = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads).map(|_| scope.spawn(|| -> Result<Vec<(usize, R)>> {
            let program = recorded(input)?;
            let mut done = Vec::new();
            loop {
                let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(item) = items.get(at) else { break };
                done.push((at, check(&program, item)?));
            }
            Ok(done)
        })).collect();
        workers.into_iter().map(|worker| worker.join().expect("an advice check panicked")).collect::<Result<Vec<_>>>()
    })?.into_iter().flatten().collect::<Vec<_>>();
    done.sort_by_key(|(at, _)| *at);
    Ok(done.into_iter().map(|(_, result)| result).collect())
}

/// The directives of every variant: `replace` names the valuation the hole
/// of a variant adopts, if any.
fn plans_with<'a>(input: &PlannerInput, program: &round::Program, replace: impl Fn(usize) -> Result<Option<(&'a Candidate, Option<Known>)>>) -> Result<Vec<RulePlan>> {
    let holes = input.holes.iter().enumerate().filter_map(|(at, hole)| match hole.origin {
        Origin::Rule { variant, .. } => Some((variant, at)), _ => None,
    }).collect::<BTreeMap<_, _>>();
    let mut plans = Vec::new();
    for index in 0..program.storage.incremental.variants.len() {
        let hole = *holes.get(&index).ok_or_else(|| error("the checked input lost an Incremental variant"))?;
        plans.extend(variant_plan(program, index, replace(hole)?)?);
    }
    Ok(plans)
}

fn datalog_plans_checked(input: &PlannerInput, program: &round::Program, report: &AdviceReport) -> Result<Vec<RulePlan>> {
    check_report(input, report)?;
    check_constraints(program, report)?;
    let choices = report.holes.iter().map(|choice| (choice.hole, choice)).collect::<BTreeMap<_, _>>();
    plans_with(input, program, |hole| {
        let choice = choices[&hole];
        if choice.decision != Decision::Changed { return Ok(None); }
        let admission = choice.admission.as_ref().ok_or_else(|| error("changed variant lacks its fixed admission constraint"))?;
        Ok(Some((&choice.candidates[choice.selected].plan, Some(known(admission.hint)))))
    })
}

/// The directives that adopt the `chosen` valuations (by hole), each with
/// its variant's fixed admission constraint, every other variant keeping
/// its own directives, compiled through production: for valuations planned
/// outside a report. The caller has checked the input and parsed `program`
/// from it.
pub(crate) fn compile_plans_for(input: &PlannerInput, program: &round::Program, chosen: &BTreeMap<usize, &Candidate>) -> Result<Vec<RulePlan>> {
    let plans = plans_with(input, program, |hole| match chosen.get(&hole) {
        Some(plan) => Ok(Some((*plan, Some(known(expected_admission(program, &input.holes[hole].origin)?.hint))))),
        None => Ok(None),
    })?;
    compile_datalog_advice_checked(program, &plans)?;
    Ok(plans)
}

/// Whether production provisions the reads of a sub-atom valuation when it
/// replaces the directives of `hole`'s variant: the advice must pass the
/// production checkers and Storage must hold every index the valuation
/// demands. `Err` names the gap; nothing is adopted on the tool's word alone.
/// A CQ hole has no Storage and is realized by its adopting target.
pub(crate) fn provisioned(program: &round::Program, hole: &export::Hole, plan: &Candidate) -> Result<std::result::Result<(), String>> {
    let Origin::Rule { variant: target, .. } = &hole.origin else { return Ok(Ok(())) };
    let mut plans = Vec::new();
    for index in 0..program.storage.incremental.variants.len() {
        let replacement = (index == *target).then(|| (plan, program.storage.incremental.variants[index].known));
        match variant_plan(program, index, replacement) {
            Ok(directive) => plans.extend(directive),
            Err(gap) if index == *target => return Ok(Err(gap.0)),
            Err(other) => return Err(other),
        }
    }
    let planned = match compile_datalog_advice_checked(program, &plans) {
        Ok(planned) => planned,
        Err(rejection) => return Ok(Err(format!("production rejects its reads: {}", rejection.0))),
    };
    let actual = demanded(&planned);
    Ok(match plan.demands.iter().find(|demand| !actual.contains(*demand)) {
        Some(demand) => Err(format!("Storage provisions no index on {} {:?} {:?} for step advice", demand.logical_relation, demand.version, demand.key)),
        None => Ok(()),
    })
}

/// Compile advice before Storage, using the recorded logical driver and shared
/// production checkers. This receipt does not claim GPU/custom physical pipeline
/// realization; those front ends consume the same directives on adoption.
pub fn compile_datalog_advice(input: &PlannerInput, plans: &[RulePlan]) -> Result<round::Program> {
    input.check()?;
    compile_datalog_advice_checked(&recorded(input)?, plans)
}

fn compile_datalog_advice_checked(original: &round::Program, plans: &[RulePlan]) -> Result<round::Program> {
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
        // Only advice is checked: an unchanged hole keeps what production
        // already compiled, whatever the tool predicted for its demands.
        if hole.decision != Decision::Changed { continue; }
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
    let program = recorded(input)?;
    let expected = datalog_plans_checked(input, &program, report)?;
    if plans != expected {
        return Err(error("adopted advice differs from the recorded orders or fixed admission constraints"));
    }
    let planned = compile_datalog_advice_checked(&program, plans)?;
    check_compiled(report, &planned)?;
    Ok(planned)
}

/// The emitted advice, parsed back through the compiler-owned RulePlan
/// parser and compiled through the actual pre-Storage entry: `Err` for
/// production's rejection. One compilation both checks what Storage
/// provisions and validates what is printed.
pub(crate) type Compiled = (Vec<RulePlan>, round::Program);
pub(crate) fn compile_report(input: &PlannerInput, program: &round::Program, report: &AdviceReport) -> Result<std::result::Result<Compiled, String>> {
    let plans = datalog_plans_checked(input, program, report)?;
    let reparsed: AdviceList = syn::parse2(AdviceList(plans).to_token_stream())?;
    Ok(match compile_datalog_advice_checked(program, &reparsed.0) {
        Ok(planned) => Ok((reparsed.0, planned)),
        Err(rejection) => Err(format!("production rejects its reads: {}", rejection.0)),
    })
}

/// The indexes a compiled program's Storage provisions.
pub(crate) fn provisions(planned: &round::Program) -> BTreeSet<IndexDemand> { demanded(planned) }

/// The advice text of a report, with the compilation of its Datalog advice.
pub(crate) fn render(input: &PlannerInput, report: &AdviceReport, compiled: Option<&Compiled>) -> Result<(String, String)> {
    check_report(input, report)?;
    let mut text = format!("// static advice; dataset {}\n// input {}; evidence {}; target {}\n", report.dataset_hash, report.input_hash, report.evidence_hash, report.target);
    for fact in [&report.target_constants.build_per_row_h, &report.target_constants.merge_per_row_m].into_iter()
        .chain(&report.target_constants.index_per_group_g).chain(&report.target_constants.scan_per_row_s) {
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
                let selectors = selectors(&source.query, &choice.candidates[choice.selected].plan)?;
                let selected = selectors.iter().map(|atom| {
                    let relation = &atom.relation;
                    let position = syn::Index::from(atom.occurrence.expect("positional selector"));
                    let columns = atom.columns.as_ref().map(|columns| quote!((#(#columns),*)));
                    quote!(#relation @ #position #columns)
                }).collect::<Vec<_>>();
                let options = cq_planning::PlanOptions { order: Some(selectors), ..Default::default() };
                let (normal, planned) = cq_planning::prefix_valuation(&source, &options)?;
                cq_lang_vo::contract::check(&normal, &planned)?;
                text.push_str(&quote!(plan(order(#(#selected),*));).to_string()); text.push('\n');
            }
            Ok((text, "shared production CQ planning and VO-Order; target realization required on adoption".into()))
        }
        CheckedIr::Datalog { .. } => {
            let (plans, planned) = compiled.ok_or_else(|| error("Datalog advice is rendered from its compilation"))?;
            check_compiled(report, planned)?;
            text.push_str("// Complete replacement for existing authored directives; never append duplicate rule/variant advice.\n");
            for plan in plans { text.push_str(&quote!(plan(#plan);).to_string()); text.push('\n'); }
            Ok((text, "production Source/Incremental/Storage/Fork/CQ/VO-Order with recorded driver; target-specific realization required on adoption".into()))
        }
    }
}
