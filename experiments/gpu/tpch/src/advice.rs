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

    /// Group valuations retain the authored atom order; reductions are left
    /// explicit rather than introducing CPU-oriented choices into this route.
    pub fn grouped(source: &TokenStream) -> syn::Result<Self> {
        let program: cq_lang_cq::Program = syn::parse2(source.clone())?;
        let program = cq_lang_cq::normal::normalize(&program)?;
        let order = program
            .query
            .body
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                cq_lang_cq::BodyItem::Positive { atom } => {
                    let repeated = program.query.body.iter().filter(|item| matches!(item,
                        cq_lang_cq::BodyItem::Positive { atom: other } if other.relation == atom.relation
                    )).count() > 1;
                    Some(if repeated { format!("{}@{index}", atom.relation) } else { atom.relation.to_string() })
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(", ");
        Ok(Self {
            plan: format!("no_reduce, order({order})"),
            ..Self::baseline()
        })
    }

    /// Stage-local orders measured with DuckDB; see the Box 14b tables in
    /// compare/tpch_statistics.md. Publications are ordinary relation inputs.
    pub fn staged(query: &str, stage: usize) -> Self {
        let order = match (query, stage) {
            ("h02", 0) => "Region, Nation, Supplier, Partsupp",
            ("h02", 1) => "Part, Partsupp, MinimumCost, Supplier, Nation, Region",
            ("h11", 0) => "Nation, Supplier, Partsupp",
            ("h11", 1 | 2) => "Stock",
            ("h11", 3) => "TotalValue, PartValue",
            ("h13", 0) => "Customer, QualifiedOrders",
            ("h13", 1) => "Customer",
            ("h15", 0) | ("h18", 0) | ("h20", 0) => "Lineitem",
            ("h15", 1) => "Revenue",
            ("h15", 2) => "Maximum, Revenue, Supplier",
            ("h18", 1) => "Quantity, Orders, Customer",
            ("h20", 1) => "Part, Partsupp, Shipped, Supplier, Nation",
            _ => unreachable!("unregistered staged query"),
        };
        // These are semantic keys: grouped publications have one row per
        // group; Stock preserves dbgen's Partsupp key, and QualifiedOrders
        // filters/project the declared Orders key. Counts are not key proofs.
        let key: Option<(&str, &[&str])> = match (query, stage) {
            ("h02", 1) => Some(("MinimumCost", &["part_key"])),
            ("h11", 1 | 2) => Some(("Stock", &["part_key", "supplier_key"])),
            ("h11", 3) => Some(("PartValue", &["part_key"])),
            ("h13", _) => Some(("QualifiedOrders", &["order_key"])),
            ("h15", 1 | 2) => Some(("Revenue", &["supplier_key"])),
            ("h18", 1) => Some(("Quantity", &["order_key"])),
            ("h20", 1) => Some(("Shipped", &["part_key", "supplier_key"])),
            _ => None,
        };
        let sequence = key
            .into_iter()
            .map(|(relation, columns)| {
                let evidence = cq_lang_sequence::Evidence {
                    facts: vec![cq_lang_sequence::EvidenceFact::Key {
                        subject: quote::format_ident!("{relation}"),
                        columns: columns
                            .iter()
                            .map(|column| quote::format_ident!("{column}"))
                            .collect(),
                    }],
                };
                cq_sequence_plan::Advice::Evidence(evidence)
                    .to_token_stream()
                    .to_string()
            })
            .collect();
        Self {
            plan: format!("no_reduce, order({order})"),
            storage: if matches!(query, "h11" | "h15" | "h18") {
                "tpch_dictionary".into()
            } else {
                String::new()
            },
            sequence,
            ..Self::baseline()
        }
    }

    // ASIA/date prefixes SF1: 1,5,30183,46008,184082,7243; SF10:
    // 1,5,300270,456771,1825856,72985. This beats Orders-root prefix sums
    // 920124 / 9183369. Preserve authored R's compressed IDs, customer store,
    // segmented Orders/Lineitem, full-key Supplier membership and count output.
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

    /// Hand-estimated orders; full predicate/key/prefix tables and SQL are in
    /// compare/tpch_statistics.md. Counts below are SF1 / SF10, not estimates
    /// inferred from table names. Selective forward provider filters are explicit.
    pub fn hand(query: &str) -> Option<Self> {
        let order = match query {
            // Selected Lineitem valuations: 5,916,591 / 59,142,609 -> four groups.
            // Nearly all input survives; no join expansion justifies a segment.
            "h01" => "Lineitem",
            // Date+discount+quantity: 114,160 / 1,139,264 -> one exact sum.
            "h06" => "Lineitem",
            // 30,988 / 310,803 qualifying lines; Orders is a unique lookup.
            "h12" => "Lineitem, Orders",
            // BUILDING Customer -> pre-cutoff Orders -> post-cutoff Lineitem:
            // SF1 prefixes 30,142 -> 147,126 -> 30,519 (11,620 distinct orders).
            // SF10: 300,276 -> 1,461,923 -> 302,114.
            // The final Lineitem contributes no payload, only existence.
            "h03" => "Customer, Orders, Lineitem",
            // Customer Nation@6 and Supplier Nation@5 form the exact country
            // pair before nation-indexed Customer; use the block's measured
            // forward traversal without replacing the authored disjunction.
            "h07" => "Nation@6, Nation@5, Customer, Orders, Lineitem, Supplier",
            // The two country pairs restrict Customer to 12,008 at SF1, then
            // Orders 121,324 -> dated Lineitem 148,370 -> opposite Supplier 5,924.
            // SF10: 2 -> 120,469 -> 1,205,808 -> 1,463,770 -> 58,365.
            // Seed the whole block, carrying the exact opposite-country pair.
            "h07_block" => "block@5, Customer, Orders, Lineitem, Supplier",
            // US Nation -> Customer -> quarter Orders -> Lineitem -> bound PKs:
            // SF1 prefixes 1, 5,983, 2,231, 8,963, 8,963, 8,963.
            // SF10: 1, 60,006, 22,579, 90,015, 90,015, 90,015.
            // Extra forward providers measured slower (20.698 vs 19.324 ms
            // at SF10); retain this accepted unmasked traversal.
            "d19" => "Nation, Customer, Orders, Lineitem, Part, Supplier",
            // US Nation -> Supplier -> Lineitem -> quarter Orders -> Brand#12:
            // SF1 prefixes 1, 393, 236,346, 8,969, 357.
            // SF10: 1, 4,004, 2,402,516, 90,158, 3,575. Expand after the
            // country filter; check the 3.7% date window before the brand.
            // Forward provider filters narrow Lineitem to 357 / 3,575 rows;
            // the numbers above are the unmaterialized candidate prefixes.
            "d27" => "Nation, Supplier, Lineitem, Orders, Part",
            // Customer Nation@7 is the selected US role, not Supplier Nation@10.
            // SF1 prefixes 1, 5,983, 2,231, then 8,963 through all FK checks.
            // SF10: 1, 60,006, 22,579, then 90,015 through all FK checks.
            "d85" => "Nation@7, Customer, Orders, Lineitem, Part, PartSupp, Supplier, Nation@10",
            // Orders-root minimizes logical prefixes (202087 / 2024242), but
            // publishing its late-line provider costs twice the accepted fact
            // scan. Keep Lineitem -> quarter Orders (52523 / 526040 outputs).
            "h04" => "Lineitem, Orders",
            // Same quarter root, anti membership leaves 4,695 / 47,631 keys.
            "h04_not_exists" => "Orders",
            // Filtered Part -> exactly four Partsupp rows, then anti membership;
            // only 4 / 56 bad supplier keys. Anti keys cannot seed the domain.
            "h16" => "Part, Partsupp",
            // Phone-filtered customer is the only positive root; Orders is an
            // anti lookup by customer, yielding 14,086 / 140,489 keys.
            "h22" => "Customer",
            // Green Part: 10,664 / 108,782. PartSupp-first minimizes prefix
            // positions (777,440 / 7,937,392), but builds the entire composite
            // Lineitem index. Its selective forward-provider attempt is rejected
            // at opaque Part publication. Keep the accepted single-key baseline;
            // the report records both the smaller-prefix candidate and its cost.
            "h09" => "Part, Lineitem, PartSupp, Orders, Supplier, Nation",
            // Type Part 1,451 / 13,452 -> lines 43,693 / 403,487 ->
            // dated orders 13,389 / 122,404 -> America 2,603 / 24,254.
            // Part-root sum 93,120 / 856,913 beats region-root 494,037 / 4,921,493.
            "h08" => "Part, Lineitem, Orders, Customer, Nation@6, Region, Supplier, Nation@8",
            // Saudi Nation -> suppliers 411 / 4,010 -> late lines
            // 156,739 / 1,522,366 -> finalized 75,871 / 734,523 ->
            // other-supplier lines 302,383 / 2,930,383. Occurrences matter.
            // Build late adjacency after Saudi support, not from all 38M late rows.
            "h21_positive" => "Nation, Supplier, Lineitem@1, Orders, Lineitem@0",
            // Nation -> Supplier -> window+quantity Lineitem -> Orders ->
            // Customer closes the nation cycle (3.94% / 4.00% survives).
            // Prefix sum 297,333 / 2,959,481; fact-root 428,290 / 4,262,201.
            "d72_inner" => "Nation, Supplier, Lineitem, Orders, Customer, PartSupp, Part",
            _ => return None,
        };
        Some(Self {
            // Only constant equality/unused string columns are dictionary-coded.
            // H09 substring, H16 string methods and H22 phone methods stay host.
            storage: if matches!(
                query,
                "h03" | "h07" | "h07_block" | "h08" | "h21_positive" | "d19" | "d27" | "d85"
            ) {
                "tpch_dictionary".into()
            } else {
                String::new()
            },
            plan: match query {
                // Filter adjacency before indexing: selected roots retain
                // 4% nation support or 3.8% quarter orders before indexing.
                "h21_positive" => format!(
                    "no_reduce, order({order}), reduce(Supplier with Nation), reduce(Lineitem@1 with Supplier)"
                ),
                "d27" => format!(
                    "no_reduce, order({order}), reduce(Supplier with Nation), reduce(Lineitem with Supplier), reduce(Lineitem with Orders), reduce(Lineitem with Part)"
                ),
                _ => format!("no_reduce, order({order})"),
            },
            physical: if matches!(query, "h01" | "h06" | "h12" | "h04") {
                Vec::new()
            } else {
                vec!["build_host_reads()".into()]
            },
            ..Self::baseline()
        })
    }
}

#[derive(Default)]
pub struct Plan {
    no_reduce: bool,
    order: Vec<cq_order::OrderAtom>,
    reduce: Vec<(cq_order::OrderAtom, cq_order::OrderAtom)>,
    skip: Vec<(cq_order::OrderAtom, cq_order::OrderAtom)>,
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
                        .parse_terminated(cq_order::OrderAtom::parse, Token![,])?
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
    let step = |target: &cq_order::OrderAtom,
                support: &cq_order::OrderAtom|
     -> syn::Result<cq_lang_vo::Step> {
        let (t, s) = (
            target.positive_clause(source)?,
            support.positive_clause(source)?,
        );
        if t == s || graph.shared(t, s).is_empty() {
            return Err(syn::Error::new_spanned(
                &target.relation,
                "reduce advice needs two atoms sharing a variable",
            ));
        }
        Ok(cq_reduce::semijoin(&analysis, &graph, t, s))
    };
    for (target, support) in plan.skip {
        let skipped = step(&target, &support)?;
        if !reduced.valuation.steps.contains(&skipped) {
            return Err(syn::Error::new_spanned(
                &target.relation,
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
        cq_lang_physical::Operator::Semi(_, key) | cq_lang_physical::Operator::Anti(_, key) => {
            relation_of(program, &key.left)
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

/// Materialize only the measured forward stream, never a reverse reduction.
pub fn hand_physical(
    program: &cq_lang_physical::Program,
    query: &str,
    pipeline: &mut Pipeline,
) -> syn::Result<()> {
    if matches!(query, "h01" | "h06" | "h12" | "h04") {
        return Ok(());
    }
    if matches!(
        query,
        "h08" | "h21_positive" | "d72_inner" | "h04" | "h04_not_exists"
    ) {
        pipeline.physical.push(hoist_inputs(program));
    }
    let prefix = match query {
        "h21_positive" | "d72_inner" => Some("Supplier"),
        "d19" | "d85" | "h07" | "h07_block" => Some("Customer"),
        "d27" => Some("Supplier"),
        _ => None,
    };
    if let Some(relation) = prefix {
        let prefix = joined_prefix(program, relation)?;
        pipeline
            .physical
            .insert(0, quote!(retain_streams(#prefix)).to_string());
    }
    if query == "h21_positive" {
        // A 411/4010-supplier root underfills the device. Publish the measured
        // 156739/1522366 late-line prefix globally before Orders/other-line work.
        let prefix = program
            .program
            .definitions
            .iter()
            .find_map(|definition| {
                let cq_lang_physical::Operator::Probe(_, key) = &definition.operator else {
                    return None;
                };
                (relation_of(program, &key.right).is_some_and(|relation| relation == "Lineitem")
                    && key
                        .columns
                        .iter()
                        .any(|column| column == "waiting_supplier"))
                .then_some(&definition.name)
            })
            .ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "missing late Lineitem expansion",
                )
            })?;
        pipeline
            .physical
            .insert(0, quote!(retain_streams(#prefix)).to_string());
    }
    Ok(())
}

/// Choose full-key providers only when the checked live layout has no payload.
/// This includes cycle-closing Probe operators as well as explicit semijoins.
pub fn hand_membership(program: &cq_lang_structures::Program) -> syn::Result<Vec<String>> {
    let facts = cq_lang_structures::contract::check(program)?;
    let mut providers = std::collections::BTreeSet::new();
    for definition in &program.physical.program.definitions {
        let (cq_lang_physical::Operator::Semi(_, key)
        | cq_lang_physical::Operator::Anti(_, key)
        | cq_lang_physical::Operator::Probe(_, key)) = &definition.operator
        else {
            continue;
        };
        let right = key.right.to_string();
        // Current filtered-storage certificates reject constant-bearing host
        // renames (notably H08 Region and H21 finalized Orders). Keep their
        // accepted RangeProvider rather than pretending membership was proved.
        if program.physical.program.definitions.iter().any(|d| {
            d.name == key.right
                && matches!(&d.operator, cq_lang_physical::Operator::Rename(rename)
                if rename.constants.is_some())
        }) {
            continue;
        }
        let built = program.structures.entries.iter().any(|entry| {
            entry.name == key.right
                && matches!(&entry.structure, cq_lang_structures::Structure::Built { keyed, .. }
                if keyed.levels.len() == 1)
        });
        if built
            && facts.layouts[&right]
                .iter()
                .all(|column| key.columns.iter().any(|key| key.to_string() == *column))
        {
            providers.insert(right);
        }
    }
    Ok(providers
        .into_iter()
        .map(|provider| format!("membership({provider})"))
        .collect())
}

/// Share only legal placement/catalog discovery with decomposition. The root,
/// expansion boundaries, tile/items and output protocol are authored here.
pub fn hand_schedule(
    source: &cq_lang_sequence::Program,
    query: &str,
) -> syn::Result<cq_sequence_plan::Planned> {
    use cq_sequence_plan::Advice;
    let suggested = cq_sequence_plan::decompose(
        source,
        &cq_gpu_sequence::machine(),
        &cq_gpu_sequence::catalog(),
    )?;
    // Keep checked group-input publication boundaries. Group extraction can
    // consume a flatten, so it must not subsequently receive join segmentation.
    if matches!(
        query,
        "h01"
            | "h06"
            | "h12"
            | "h04"
            | "h02.0"
            | "h13.0"
            | "h11.1"
            | "h11.2"
            | "h15.0"
            | "h15.1"
            | "h18.0"
            | "h20.0"
    ) {
        return Ok(suggested);
    }
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
    // Partition every independent device-side numeric filter. Tiling only join
    // expansion leaves a 6M/60M-row publication as a scalar producer.
    for definition in &source.structures.physical.program.definitions {
        let device = planned.trace.iter().any(|step| {
            matches!(step,
            Advice::Grade { name, level, .. }
                if *name == definition.name && level.iter().any(|level| level == "device"))
        });
        if device
            && matches!(definition.mode, cq_lang_physical::Mode::Let(_))
            && matches!(
                definition.operator,
                cq_lang_physical::Operator::Select(_)
                    | cq_lang_physical::Operator::Semi(..)
                    | cq_lang_physical::Operator::Anti(..)
            )
        {
            schedule.push(Advice::Partition {
                name: definition.name.clone(),
                n: 256,
                items: Some(4),
            });
        }
    }
    if matches!(query, "d72_inner" | "d85" | "h03") {
        // The host filtered-range builder accepts one half-open column only.
        // Publish numeric filtered leaves on device instead: shipdate+quantity
        // and availability are checked map predicates, not host index bounds.
        for definition in &source.structures.physical.program.definitions {
            if matches!(definition.mode, cq_lang_physical::Mode::Let(_))
                && matches!(definition.operator, cq_lang_physical::Operator::Range(..))
            {
                let name = &definition.name;
                let mut input = definition.operator.input().unwrap();
                while let Some(parent) = source
                    .structures
                    .physical
                    .program
                    .definitions
                    .iter()
                    .find(|d| d.name == *input)
                {
                    if matches!(parent.operator, cq_lang_physical::Operator::Rename(_)) {
                        schedule.push(Advice::Store(parent.name.clone()));
                        break;
                    }
                    schedule.push(Advice::Grade {
                        machine: syn::parse_quote!(gpu),
                        name: parent.name.clone(),
                        level: vec![
                            syn::parse_quote!(host),
                            syn::parse_quote!(device),
                            syn::parse_quote!(block),
                        ],
                    });
                    input = parent.operator.input().unwrap();
                }
                let producer = quote::format_ident!("__hand_filter_{name}");
                schedule.push(Advice::Publish {
                    name: name.clone(),
                    producer: producer.clone(),
                });
                schedule.push(Advice::Grade {
                    machine: syn::parse_quote!(gpu),
                    name: producer,
                    level: vec![
                        syn::parse_quote!(host),
                        syn::parse_quote!(device),
                        syn::parse_quote!(block),
                    ],
                });
                schedule.push(Advice::Partition {
                    name: name.clone(),
                    n: 256,
                    items: None,
                });
            }
        }
    }
    // Observed fanouts: h09 Lineitem 7.49; h08 Lineitem 30;
    // h21 Supplier 411/4010 and late Lineitem 381/380 (other line ~4);
    // d72 Supplier 400/4000 and filtered Lineitem 13.56/13.49.
    let expanded: &[&str] = match query {
        "h09" | "h08" | "d27" | "d19" | "d85" => &["Lineitem"],
        "h03" | "h07" | "h07_block" => &["Orders"],
        "h21_positive" | "d72_inner" => &["Supplier", "Lineitem"],
        "h11.0" => &["Supplier", "Partsupp"],
        _ => &[],
    };
    for definition in &source.structures.physical.program.definitions {
        let cq_lang_physical::Operator::Probe(_, key) = &definition.operator else {
            continue;
        };
        if !relation_of(&source.structures.physical, &key.right)
            .is_some_and(|relation| expanded.iter().any(|name| relation == *name))
        {
            continue;
        }
        let name = &definition.name;
        if source
            .sequence
            .definitions
            .iter()
            .any(|d| d.name == *name && matches!(d.term, cq_lang_sequence::Term::Flatten(_)))
        {
            schedule.push(Advice::Segment {
                name: name.clone(),
                n: 1024,
                items: Some(4),
            });
        }
    }
    schedule.push(Advice::Output(cq_lang_sequence::OutputProtocol::Count));
    for step in schedule {
        planned.program = step.apply(&planned.program)?;
        planned.trace.push(step);
    }
    Ok(planned)
}
