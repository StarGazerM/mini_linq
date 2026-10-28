//! Per-query, checked pipeline advice. Query bodies always come from `source`.
//! The JSON override uses ordinary pass arguments and Sequence Advice syntax;
//! reports contain the resolved invocations, not a second executable query.
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::Ident;

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
    #[serde(default)]
    pub target: Target,
    #[serde(default)]
    pub publication: Placement,
    #[serde(default)]
    pub recipe: Recipe,
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    #[default]
    Device,
    Host,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Host,
    Device,
}

/// Explicit catalog facts; query labels never reach recipe execution.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Recipe {
    pub automatic: bool,
    pub retained_prefixes: Vec<String>,
    pub filtered_leaves: Vec<String>,
    pub retained_probe: Option<(String, String)>,
    pub hoist_inputs: bool,
    pub membership: bool,
    pub membership_relations: Vec<String>,
    pub schedule: Option<Schedule>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    pub expansions: Vec<String>,
    pub partition_filters: bool,
    pub publish_ranges: bool,
    pub partition: usize,
    pub segment: usize,
    pub items: usize,
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
            target: Target::Device,
            publication: Placement::Host,
            recipe: Recipe::default(),
        }
    }

    pub fn automatic() -> Self {
        Self {
            recipe: Recipe { automatic: true, ..Recipe::default() },
            ..Self::baseline()
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
            recipe: catalog_recipe(&format!("{query}.{stage}")),
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
            recipe: catalog_recipe("h05"),
            ..Self::baseline()
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
            // Shipping guard: 214,377 / 2,141,904 lines -> unique Part probes
            // -> 121 / 1,134 aggregate inputs. Keep the mixed branch guard
            // intact; Part-first needs a larger Lineitem adjacency provider.
            "h19" => "Lineitem, Part",
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
            physical: if matches!(query, "h01" | "h06" | "h12" | "h19" | "h04") {
                Vec::new()
            } else {
                vec!["build_host_reads()".into()]
            },
            recipe: catalog_recipe(query),
            ..Self::baseline()
        })
    }
}

/// Catalog registration resolves workload labels to printable input facts.
/// Formal rule DR-Recipe: Execute only explicit recipe facts, never query labels.
fn catalog_recipe(query: &str) -> Recipe {
    let simple = matches!(query, "h01" | "h06" | "h12" | "h19" | "h04"
        | "h02.0" | "h13.0" | "h11.1" | "h11.2" | "h15.0"
        | "h15.1" | "h18.0" | "h20.0");
    let expansions: &[&str] = match query {
        "h05" => &["Orders", "Lineitem"],
        "h09" | "h08" | "d27" | "d19" | "d85" => &["Lineitem"],
        "h03" | "h07" | "h07_block" => &["Orders"],
        "h21_positive" | "d72_inner" => &["Supplier", "Lineitem"],
        "h11.0" => &["Supplier", "Partsupp"],
        _ => &[],
    };
    let prefixes: &[&str] = match query {
        "h05" | "d19" | "d85" | "h07" | "h07_block" => &["Customer"],
        "h21_positive" | "d72_inner" | "d27" => &["Supplier"],
        _ => &[],
    };
    Recipe {
        retained_prefixes: prefixes.iter().map(|s| (*s).into()).collect(),
        filtered_leaves: if query == "h09" { vec!["Part".into()] } else { Vec::new() },
        retained_probe: (query == "h21_positive").then(|| ("Lineitem".into(), "waiting_supplier".into())),
        hoist_inputs: matches!(query, "h05" | "h08" | "h21_positive" | "d72_inner" | "h04_not_exists"),
        membership: query != "h04" && query != "h05",
        membership_relations: if query == "h05" { vec!["Supplier".into()] } else { Vec::new() },
        schedule: (!simple).then(|| Schedule {
            expansions: expansions.iter().map(|s| (*s).into()).collect(),
            partition_filters: query != "h05",
            publish_ranges: matches!(query, "d72_inner" | "d85" | "h03"),
            partition: 256, segment: 1024, items: 4,
        }),
        ..Recipe::default()
    }
}

pub fn valuation(
    source: &cq_lang_cq::Program,
    arguments: &str,
) -> syn::Result<cq_lang_vo::Program> {
    let options: cq_planning::PlanOptions = syn::parse_str(arguments)?;
    cq_planning::prefix_valuation(source, &options)
}

pub fn physical(
    source: &cq_lang_physical::Program,
    invocations: &[String],
) -> syn::Result<cq_lang_physical::Program> {
    let mut current = source.clone();
    for invocation in invocations {
        let invocation: cq_stage::Invocation = syn::parse_str(invocation)?;
        current = cq_rewrite_passes::invoke_physical(&current, &invocation.path, invocation.arguments)?;
    }
    Ok(current)
}

pub fn structures(
    source: &cq_lang_structures::Program,
    invocations: &[String],
) -> syn::Result<cq_lang_structures::Program> {
    let mut current = source.clone();
    for invocation in invocations {
        let invocation: cq_stage::Invocation = syn::parse_str(invocation)?;
        current = cq_rewrite_passes::invoke_structures(&current, &invocation.path, invocation.arguments)?;
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
    recipe: &Recipe,
    pipeline: &mut Pipeline,
) -> syn::Result<()> {
    if recipe.hoist_inputs {
        pipeline.physical.push(hoist_inputs(program));
    }
    for relation in &recipe.retained_prefixes {
        let prefix = joined_prefix(program, relation)?;
        pipeline
            .physical
            .insert(0, quote!(retain_streams(#prefix)).to_string());
    }
    if let Some((relation, column)) = &recipe.retained_probe {
        let prefix = program
            .program
            .definitions
            .iter()
            .find_map(|definition| {
                let cq_lang_physical::Operator::Probe(_, key) = &definition.operator else {
                    return None;
                };
                (relation_of(program, &key.right).is_some_and(|name| name == relation)
                    && key
                        .columns
                        .iter()
                        .any(|name| name == column))
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
    recipe: &Schedule,
) -> syn::Result<cq_sequence_plan::Planned> {
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
        n: recipe.partition,
        items: None,
    }];
    // Partition every independent device-side numeric filter. Tiling only join
    // expansion leaves a 6M/60M-row publication as a scalar producer.
    for definition in source.structures.physical.program.definitions.iter().filter(|_| recipe.partition_filters) {
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
                n: recipe.partition,
                items: Some(recipe.items),
            });
        }
    }
    if recipe.publish_ranges {
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
                    n: recipe.partition,
                    items: None,
                });
            }
        }
    }
    for definition in &source.structures.physical.program.definitions {
        let cq_lang_physical::Operator::Probe(_, key) = &definition.operator else {
            continue;
        };
        if !relation_of(&source.structures.physical, &key.right)
            .is_some_and(|relation| recipe.expansions.iter().any(|name| relation == name))
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
                n: recipe.segment,
                items: Some(recipe.items),
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
