//! Per-query, checked pipeline advice. Query bodies always come from `source`.
//! The JSON override uses ordinary pass arguments and Sequence Advice syntax;
//! reports contain the resolved invocations, not a second executable query.
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pipeline {
    #[serde(default)]
    pub storage: String,
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub physical: Vec<String>,
    #[serde(default)]
    pub structures: Vec<String>,
    /// A replay artifact already records every grade and output decision.
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub sequence: Vec<String>,
}

impl Pipeline {
    pub fn baseline() -> Self {
        Self {
            storage: String::new(),
            plan: String::new(),
            physical: Vec::new(),
            structures: Vec::new(),
            sequence: Vec::new(),
            complete: false,
        }
    }

    pub fn h05() -> Self {
        Self {
            storage: "tpch_domains".into(),
            plan: "no_reduce, order(Region, Nation, Customer, Orders, Lineitem, Supplier)".into(),
            physical: vec!["build_host_reads()".into()],
            structures: Vec::new(),
            sequence: Vec::new(),
            complete: false,
        }
    }

    pub fn h09() -> Self {
        Self {
            plan: "order(Part, Lineitem, PartSupp, Orders, Supplier, Nation)".into(),
            ..Self::baseline()
        }
    }

    pub fn negation(query: &str) -> Self {
        let order = match query {
            "h04" => "Lineitem, Orders",
            "h04_not_exists" => "Orders",
            "h16" => "Part, Partsupp",
            "h22" => "Customer",
            _ => unreachable!("negation advice requires a registered negation core"),
        };
        Self {
            plan: format!("no_reduce, order({order})"),
            ..Self::baseline()
        }
    }
}

#[derive(Default)]
pub struct Plan {
    no_reduce: bool,
    order: Vec<Ident>,
    reduce: Vec<(Ident, Ident)>,
    skip: Vec<(Ident, Ident)>,
}

impl Parse for Plan {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut plan = Self::default();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            if key == "no_reduce" {
                plan.no_reduce = true;
            } else {
                let arguments;
                syn::parenthesized!(arguments in input);
                if key == "order" {
                    plan.order = arguments
                        .parse_terminated(Ident::parse, Token![,])?
                        .into_iter()
                        .collect();
                } else if key == "reduce" || key == "skip" {
                    let target = arguments.parse()?;
                    let with: Ident = arguments.parse()?;
                    if with != "with" {
                        return Err(syn::Error::new_spanned(with, "expected with"));
                    }
                    let support = arguments.parse()?;
                    if !arguments.is_empty() {
                        return Err(arguments.error("unexpected reduction argument"));
                    }
                    if key == "reduce" {
                        plan.reduce.push((target, support));
                    } else {
                        plan.skip.push((target, support));
                    }
                } else {
                    return Err(syn::Error::new_spanned(
                        key,
                        "expected order, no_reduce, reduce, or skip",
                    ));
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(plan)
    }
}

pub fn valuation(
    source: &cq_lang_cq::Program,
    arguments: &str,
) -> syn::Result<cq_lang_vo::Program> {
    let plan: Plan = syn::parse_str(arguments)?;
    let ordered = if plan.order.is_empty() {
        let options = cq_order::Options::default();
        let ordered = cq_order::compile(source, &options)?;
        cq_order::check(source, &ordered, &options)?;
        ordered
    } else {
        cq_order::compile_explicit(source, &plan.order)?
    };
    let options = cq_reduce::Options {
        semijoins: if plan.no_reduce {
            cq_reduce::Reductions::None
        } else {
            cq_reduce::Reductions::Masks
        },
    };
    let mut reduced = cq_reduce::compile(&ordered, &options)?;
    cq_reduce::check(&ordered, &reduced, &options)?;
    if plan.skip.is_empty() && plan.reduce.is_empty() {
        return Ok(reduced);
    }
    let analysis = cq_lang_vo::contract::check(&reduced)?;
    let graph = cq_lang_cq::hypergraph::Analysis::of(source);
    let occurrence = |name: &Ident| -> syn::Result<usize> {
        let matches: Vec<_> = source
            .query
            .body
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                matches!(item, cq_lang_cq::BodyItem::Positive { atom } if atom.relation == *name)
                    .then_some(i)
            })
            .collect();
        if matches.len() != 1 {
            return Err(syn::Error::new_spanned(
                name,
                "reduction advice needs one top-level occurrence",
            ));
        }
        Ok(matches[0])
    };
    let step = |target: &Ident, support: &Ident| -> syn::Result<cq_lang_vo::Step> {
        let (t, s) = (occurrence(target)?, occurrence(support)?);
        if t == s || graph.shared(t, s).is_empty() {
            return Err(syn::Error::new_spanned(
                target,
                "reduce advice needs two atoms sharing a variable",
            ));
        }
        Ok(cq_reduce::semijoin(&analysis, &graph, t, s))
    };
    for (target, support) in plan.skip {
        let skipped = step(&target, &support)?;
        if !reduced.valuation.steps.contains(&skipped) {
            return Err(syn::Error::new_spanned(
                target,
                "skip advice names no Masks reduction",
            ));
        }
        reduced
            .valuation
            .steps
            .retain(|candidate| *candidate != skipped);
    }
    let extra = plan
        .reduce
        .iter()
        .map(|(t, s)| step(t, s))
        .collect::<syn::Result<Vec<_>>>()?;
    let first = reduced
        .valuation
        .steps
        .iter()
        .position(|step| {
            matches!(
                step,
                cq_lang_vo::Step::Atom(..) | cq_lang_vo::Step::Block(..)
            )
        })
        .unwrap_or(reduced.valuation.steps.len());
    reduced.valuation.steps.splice(first..first, extra);
    cq_lang_vo::contract::check(&reduced)?;
    Ok(reduced)
}

struct Invocation {
    name: Ident,
    arguments: TokenStream,
}
impl Parse for Invocation {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        let arguments;
        syn::parenthesized!(arguments in input);
        Ok(Self {
            name,
            arguments: arguments.parse()?,
        })
    }
}

pub fn physical(
    source: &cq_lang_physical::Program,
    invocations: &[String],
) -> syn::Result<cq_lang_physical::Program> {
    let mut current = source.clone();
    for invocation in invocations {
        let Invocation { name, arguments } = syn::parse_str(invocation)?;
        current = match name.to_string().as_str() {
            "retain_streams" => cq_rewrite_passes::retain_streams(&current, arguments)?,
            "hoist" => cq_rewrite_passes::hoist(&current, arguments)?,
            "canon_prefixes" => cq_rewrite_passes::canon_prefixes(&current, arguments)?,
            "smallest_intersections" => {
                cq_rewrite_passes::smallest_intersections(&current, arguments)?
            }
            "build_host_reads" => cq_rewrite_passes::build_host_reads(&current, arguments)?,
            _ => return Err(syn::Error::new_spanned(name, "unsupported Physical advice")),
        };
    }
    Ok(current)
}

pub fn structures(
    source: &cq_lang_structures::Program,
    invocations: &[String],
) -> syn::Result<cq_lang_structures::Program> {
    let mut current = source.clone();
    for invocation in invocations {
        let Invocation { name, arguments } = syn::parse_str(invocation)?;
        current = match name.to_string().as_str() {
            "sorted_indexes" => cq_rewrite_passes::sorted_indexes(&current, arguments)?,
            "retype" => cq_rewrite_passes::retype(&current, arguments)?,
            "membership" => cq_rewrite_passes::membership(&current, arguments)?,
            _ => {
                return Err(syn::Error::new_spanned(
                    name,
                    "unsupported Structures advice",
                ));
            }
        };
    }
    Ok(current)
}

pub fn provider_layout(program: &cq_lang_physical::Program, relation: &str) -> syn::Result<String> {
    for definition in &program.program.definitions {
        let (cq_lang_physical::Operator::Semi(_, key) | cq_lang_physical::Operator::Probe(_, key)) =
            &definition.operator
        else {
            continue;
        };
        if relation_of(program, &key.right).is_some_and(|name| name == relation) {
            let provider = &key.right;
            return Ok(quote!(membership(#provider)).to_string());
        }
    }
    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        format!("no membership provider for {relation}"),
    ))
}

/// Resolve a semantic relation selector to its existing lowered producer. This
/// follows unary projections/filters, never creates or duplicates an operator.
fn relation_of<'a>(program: &'a cq_lang_physical::Program, name: &Ident) -> Option<&'a Ident> {
    let definition = program
        .program
        .definitions
        .iter()
        .find(|d| d.name == *name)?;
    match &definition.operator {
        cq_lang_physical::Operator::Rename(rename) => Some(&rename.relation),
        cq_lang_physical::Operator::Select(_)
        | cq_lang_physical::Operator::Point(_, _)
        | cq_lang_physical::Operator::Range(_, _)
        | cq_lang_physical::Operator::Project(_)
        | cq_lang_physical::Operator::Guard(_) => {
            relation_of(program, definition.operator.input()?)
        }
        _ => None,
    }
}

pub fn hoist_inputs(program: &cq_lang_physical::Program) -> String {
    let names = program
        .program
        .definitions
        .iter()
        .filter(|definition| relation_of(program, &definition.name).is_some())
        .map(|definition| &definition.name);
    quote!(hoist(#(#names),*)).to_string()
}

pub fn joined_prefix(program: &cq_lang_physical::Program, relation: &str) -> syn::Result<Ident> {
    let matches: Vec<_> = program
        .program
        .definitions
        .iter()
        .filter_map(|definition| {
            let cq_lang_physical::Operator::Probe(_, key) = &definition.operator else {
                return None;
            };
            relation_of(program, &key.right)
                .is_some_and(|name| name == relation)
                .then(|| definition.name.clone())
        })
        .collect();
    if matches.len() != 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("expected one expansion of {relation}"),
        ));
    }
    Ok(matches[0].clone())
}

/// Retain a host predicate result whose checked carrier is numeric. The
/// logical heading may still contain dead strings; only Structures' proven
/// layout, never an assumed projection, determines the uploaded carrier.
pub fn filtered_leaf(
    program: &cq_lang_structures::Program,
    relation: &str,
) -> syn::Result<Option<Ident>> {
    let facts = cq_lang_structures::contract::check(program)?;
    for definition in &program.physical.program.definitions {
        if !matches!(
            definition.operator,
            cq_lang_physical::Operator::Project(_) | cq_lang_physical::Operator::Guard(_)
        ) || !relation_of(&program.physical, &definition.name)
            .is_some_and(|name| name == relation)
        {
            continue;
        }
        let name = definition.name.to_string();
        if facts.layouts[&name].iter().any(|column| {
            !matches!(
                facts.physical.rows[&name].types[column]
                    .to_token_stream()
                    .to_string()
                    .as_str(),
                "i32" | "i64" | "u32" | "u64"
            )
        }) {
            continue;
        }
        return Ok(matches!(definition.mode, cq_lang_physical::Mode::Pipe(_))
            .then(|| definition.name.clone()));
    }
    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        format!(
            "no numeric filtered leaf for {relation}; keep its predicate before joins in the source"
        ),
    ))
}

/// H05's authored R schedule, expressed only as checked rewrites of the generic
/// lowering. Placement is shared with the planner; decomposition is authored.
pub fn h05_schedule(source: &cq_lang_sequence::Program) -> syn::Result<cq_sequence_plan::Planned> {
    use cq_sequence_plan::Advice;
    let suggested = cq_sequence_plan::decompose(
        source,
        &cq_gpu_sequence::machine(),
        &cq_gpu_sequence::catalog(),
    )?;
    let mut planned = cq_sequence_plan::Planned {
        program: source.clone(),
        trace: Vec::new(),
    };
    for step in suggested.trace {
        if matches!(
            step,
            Advice::Tile { .. } | Advice::Partition { .. } | Advice::Segment { .. }
        ) {
            continue;
        }
        planned.program = step.apply(&planned.program)?;
        planned.trace.push(step);
    }
    let output = planned
        .program
        .sequence
        .definitions
        .last()
        .unwrap()
        .name
        .clone();
    let mut schedule = vec![Advice::Partition {
        name: output,
        n: 256,
        items: None,
    }];
    for relation in ["Orders", "Lineitem"] {
        let physical = &source.structures.physical;
        let prefix = joined_prefix(physical, relation)?;
        let definition = source
            .sequence
            .definitions
            .iter()
            .find(|d| d.name == prefix)
            .ok_or_else(|| {
                syn::Error::new_spanned(&prefix, "expansion has no Sequence definition")
            })?;
        if !matches!(definition.term, cq_lang_sequence::Term::Flatten(_)) {
            return Err(syn::Error::new_spanned(
                &prefix,
                "expansion is not a named flatten",
            ));
        }
        schedule.push(Advice::Segment {
            name: prefix,
            n: 1024,
            items: Some(4),
        });
    }
    schedule.push(Advice::Output(cq_lang_sequence::OutputProtocol::Count));
    for step in schedule {
        planned.program = step.apply(&planned.program)?;
        planned.trace.push(step);
    }
    Ok(planned)
}

pub fn apply_sequence(
    source: &cq_lang_sequence::Program,
    invocations: &[String],
) -> syn::Result<cq_sequence_plan::Planned> {
    let mut planned = cq_sequence_plan::Planned {
        program: source.clone(),
        trace: Vec::new(),
    };
    for invocation in invocations {
        let step: cq_sequence_plan::Advice = syn::parse_str(invocation)?;
        planned.program = step.apply(&planned.program)?;
        planned.trace.push(step);
    }
    Ok(planned)
}

pub fn trace_tokens(trace: &[cq_sequence_plan::Advice]) -> String {
    quote!(#(#trace);*).to_token_stream().to_string()
}
