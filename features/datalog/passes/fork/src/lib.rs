//! Record complete CQ holes, immutable round contexts, and their invocation queues.
use cq_datalog_support::{Fresh, columns, ensure, equalities, filters};
use cq_lang_datalog_round::cq::BodyItem;
use cq_lang_datalog_round::{self as round, *};
use cq_lang_datalog_storage as storage;
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::Ident;

pub fn cq_defaults() -> cq_stage::Configuration {
    syn::parse_quote!(pipeline {
        plan = ::mini_linq::passes::plan;
        ix = ::mini_linq::passes::index_demands;
        lower = ::mini_linq::passes::lower;
        select = ::mini_linq::passes::select;
        represent = ::mini_linq::passes::represent;
        stage = ::mini_linq::passes::stage;
        sequence = ::mini_linq::passes::sequence;
        grade = ::mini_linq::passes::sequential;
        print = ::mini_linq::passes::translate_pull;
    })
}
/// Resolve authored CQ phases over the fact-aware Datalog defaults.
pub fn resolve_cq(configuration: cq_stage::Configuration) -> syn::Result<cq_stage::Pipeline> {
    configuration.resolve(cq_defaults())
}
pub fn default_pipeline() -> cq_stage::Pipeline {
    resolve_cq(cq_stage::Configuration::default()).expect("complete CQ defaults")
}
pub fn compile(source: &storage::Program) -> syn::Result<Program> {
    construct(
        source,
        default_pipeline(),
        TerminalRegistry::default(),
        syn::parse_quote!(none),
        Some(2),
        Some(0),
    )
}
struct Arguments {
    pipeline: cq_stage::Pipeline,
    terminals: TerminalRegistry,
    timing: Ident,
    lower: Option<usize>,
    plan: Option<usize>,
}
impl syn::parse::Parse for Arguments {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut pipeline = None;
        let mut terminals = None;
        let mut timing = None;
        let mut lower = None;
        let mut plan = None;
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            if name == "cq" && pipeline.is_none() {
                pipeline = Some(input.parse()?);
            } else if name == "timing" && timing.is_none() {
                timing = Some(input.parse()?);
            } else if name == "terminals" && terminals.is_none() {
                terminals = Some(input.parse()?);
            } else if name == "lower" && lower.is_none() {
                lower = Some(input.parse::<syn::LitInt>()?.base10_parse()?);
            } else if name == "plan" && plan.is_none() {
                plan = Some(input.parse::<syn::LitInt>()?.base10_parse()?);
            } else {
                return Err(syn::Error::new_spanned(
                    name,
                    "unknown or duplicate fork argument",
                ));
            }
            input.parse::<syn::Token![;]>()?;
        }
        Ok(Self {
            plan: plan.or_else(|| pipeline.is_none().then_some(0)),
            lower: lower.or_else(|| pipeline.is_none().then_some(2)),
            pipeline: pipeline.unwrap_or_else(default_pipeline),
            terminals: terminals.unwrap_or_default(),
            timing: timing.unwrap_or_else(|| syn::parse_quote!(none)),
        })
    }
}
pub fn compile_with(source: &storage::Program, args: TokenStream) -> syn::Result<Program> {
    let Arguments {
        pipeline,
        terminals,
        timing,
        lower,
        plan,
    } = syn::parse2(args)?;
    construct(source, pipeline, terminals, timing, lower, plan)
}
/// Formal rule CK-DL-Fork: independently check storage embedding and every hole.
pub fn check(source: &storage::Program, target: &Program) -> syn::Result<()> {
    round::contract::check(target)?;
    ensure(
        source == &target.storage,
        target,
        "fork changed the storage program",
    )?;
    ensure(
        target.execution.is_empty(),
        target,
        "fork must not introduce execution certificates",
    )?;
    Ok(())
}

fn negation(atom: &cq_syntax::Atom, key: &KeySet) -> BodyItem {
    let name = &key.name;
    if key.columns.is_empty() {
        syn::parse_quote!(!#name(_))
    } else {
        let args = key.columns.iter().map(|&i| &atom.args[i]);
        syn::parse_quote!(!#name(#(#args),*))
    }
}
/// The synthetic in-plan admission read `!C(x̄)`: the head relation's committed
/// canonical layout over the complete head key, in canonical column order.
fn admission_negation(context: &RoundContext, head: usize, args: &[syn::Expr]) -> BodyItem {
    let canonical = &context.layouts[context.relations[head].providers.canonical];
    let name = &canonical.name;
    let args = canonical.columns.iter().map(|&column| &args[column]);
    syn::parse_quote!(!#name(#(#args),*))
}
// Layouts change only positional host representation, never Rust-step order.
fn use_layouts(
    query: &mut round::cq::Query,
    context: &RoundContext,
    storage: &storage::Program,
    variant: usize,
) -> syn::Result<()> {
    for read in storage.reads.iter().filter(|read| read.variant == variant) {
        let Some(BodyItem::Positive { atom }) = query.body.iter_mut().nth(read.position) else {
            return Err(syn::Error::new_spanned(
                &*query,
                "recorded read is not a positive occurrence",
            ));
        };
        if read.kind == "direct" {
            continue;
        }
        let layouts = if read.kind == "delta" {
            &context.delta_layouts
        } else {
            &context.layouts
        };
        let layout = layouts
            .get(read.index)
            .ok_or_else(|| syn::Error::new_spanned(&*atom, "missing recorded layout"))?;
        atom.args = read.columns.iter().map(|&i| atom.args[i].clone()).collect();
        atom.relation = layout.name.clone();
    }
    Ok(())
}

fn scan_source(query: &round::cq::Query, context: &RoundContext) -> Option<InputSource> {
    let BodyItem::Positive { atom } = query.body.first()? else {
        return None;
    };
    if let Some(index) = context
        .relations
        .iter()
        .position(|relation| relation.delta.as_ref() == Some(&atom.relation))
    {
        return Some(InputSource::Delta(index));
    }
    context
        .layouts
        .iter()
        .position(|layout| layout.name == atom.relation && layout.key == 0)
        .map(InputSource::Layout)
}

/// Formal rule FK-Known: decide where a rule hole discharges its head's
/// committed anti-join, returning the pinned in-plan level. Advice
/// `known(at(level k))` places it in plan after the k-th positive read (k must
/// not precede the read binding the last head variable); `known(admit)` keeps
/// admission. Without advice, a hole whose queue records a planning invocation
/// is in plan at the head-binding level iff a later read is recorded
/// `multiply` (DL-Fanout): an early anti-join saves the expansion of known
/// heads there, while before reads that only filter it would probe every
/// intermediate row (the unconditional structural default measured DOOP xalan
/// `mini` 2.36x and batik `mini-par` 2.95x slower). Device storage realizes
/// `admit` only.
fn known_admission(
    query: &round::cq::Query,
    advice: Option<incremental::Known>,
    device: bool,
    planned: bool,
    fanout: &[Fanout],
) -> syn::Result<Option<usize>> {
    let (last, reads) = round::head_level(query);
    match advice {
        Some(incremental::Known::At(_)) if device => Err(syn::Error::new_spanned(
            query,
            "GPU Datalog admission is at admit only (ADM-InPlan GPU follow-up)",
        )),
        _ if device => Ok(None),
        Some(incremental::Known::At(level)) => {
            ensure(
                level >= last.max(1) && level <= reads,
                query,
                &format!(
                    "known(at(level {level})) must lie between the read binding the last head variable (level {}) and the last positive read ({reads})",
                    last.max(1)
                ),
            )?;
            Ok(Some(level))
        }
        Some(incremental::Known::Admit) => Ok(None),
        None => Ok((planned && round::fanout_follows(fanout, last)).then_some(last.max(1))),
    }
}

fn scan_input(
    query: &mut round::cq::Query,
    context: &RoundContext,
    scan: &ScanBinding,
) -> cq_syntax::RelationDecl {
    let (relation, columns) = match scan.source {
        InputSource::Delta(index) => (index, None),
        InputSource::Layout(index) => {
            let layout = &context.layouts[index];
            (layout.relation, Some(&layout.columns))
        }
    };
    let mut declaration = context.relations[relation].declaration.clone();
    if let Some(columns) = columns {
        declaration.columns = columns
            .iter()
            .map(|&column| declaration.columns[column].clone())
            .collect();
    }
    let binding = &scan.binding;
    declaration.name = binding.clone();
    declaration.value = Some(syn::parse_quote!(= (#binding)));
    // A scan is a whole, unkeyed input (a delta or key-zero layout bucket).
    declaration.hosts = Vec::new();
    let BodyItem::Positive { atom } = &mut query.body[0] else {
        unreachable!("scan source is a positive read")
    };
    atom.relation = binding.clone();
    declaration
}
// Each hole owns only the CQ inputs it reads, including completed borrowed keys.
// Layout and key-set inputs declare their token's host reads on their recorded
// key (IF-HostDecl); delta row buffers declare none. Key sets collected by
// non-batch key holes store borrowed key columns (`borrowed_keys`).
fn hole_inputs<'a>(
    tokens: &[storage::provider::Token],
    context: &RoundContext,
    keys: &[KeySet],
    borrowed_keys: bool,
    names: impl Iterator<Item = &'a Ident>,
) -> syn::Result<Vec<cq_syntax::RelationDecl>> {
    let names: std::collections::BTreeSet<_> = names.map(symbol_name).collect();
    let mut inputs = Vec::new();
    for relation in &context.relations {
        if let Some(delta) = &relation.delta {
            if names.contains(&symbol_name(delta)) {
                let mut declaration = relation.declaration.clone();
                declaration.name = delta.clone();
                let host = relation.delta_host.as_ref().expect("checked delta host");
                declaration.value = Some(syn::parse_quote!(= (&#host)));
                declaration.hosts = Vec::new();
                inputs.push(declaration);
            }
        }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        if names.contains(&symbol_name(&layout.name)) {
            let mut declaration = context.relations[layout.relation].declaration.clone();
            declaration.name = layout.name.clone();
            declaration.columns = layout
                .columns
                .iter()
                .map(|&column| declaration.columns[column].clone())
                .collect();
            let host = &layout.host;
            declaration.value = Some(syn::parse_quote!(= (&#host)));
            declaration.hosts =
                host_decls(tokens, &layout.provider, &declaration, layout.key, false)?;
            inputs.push(declaration);
        }
    }
    for key in keys {
        if names.contains(&symbol_name(&key.name)) {
            let mut declaration = context.relations[key.relation].declaration.clone();
            declaration.name = key.name.clone();
            declaration.columns = if key.columns.is_empty() {
                let column = &declaration.columns[0].name;
                syn::parse_quote!(#column: ())
            } else {
                key.columns
                    .iter()
                    .map(|&column| declaration.columns[column].clone())
                    .collect()
            };
            let host = &key.name;
            declaration.value = Some(syn::parse_quote!(= (#host)));
            // A unit key (no projected columns) stores an owned `()`.
            let borrowed = borrowed_keys && !key.columns.is_empty();
            let arity = declaration.columns.len();
            declaration.hosts = host_decls(tokens, &key.provider, &declaration, arity, borrowed)?;
            inputs.push(declaration);
        }
    }
    Ok(inputs)
}
/// The token's declared host reads on the leading `key` columns of `declaration`.
fn host_decls(
    tokens: &[storage::provider::Token],
    provider: &storage::Provider,
    declaration: &cq_syntax::RelationDecl,
    key: usize,
    borrowed: bool,
) -> syn::Result<Vec<cq_syntax::HostDecl>> {
    let token = storage::provider::lookup(tokens, &provider.token)?;
    let key: Vec<Ident> = declaration
        .columns
        .iter()
        .take(key)
        .map(|column| column.name.clone())
        .collect();
    Ok(storage::provider::host_decl(token, &key, borrowed)
        .into_iter()
        .collect())
}
/// Lower variants and key projections, retaining the recorded schedule.
fn construct(
    source: &storage::Program,
    pipeline: cq_stage::Pipeline,
    terminals: TerminalRegistry,
    timing: Ident,
    lower: Option<usize>,
    plan: Option<usize>,
) -> syn::Result<Program> {
    let analysis = storage::contract::check(source)?;
    terminals.check()?;
    let interface = terminals.interface(&pipeline)?;
    let lower = lower.ok_or_else(|| {
        syn::Error::new_spanned(source, "custom fork queues require lower = <ordinal>")
    })?;
    ensure(
        lower < pipeline.passes.len(),
        source,
        "lower invocation ordinal is out of range",
    )?;
    // Only checked, per-hole facts may enter this invocation.
    let authored: cq_stage::Context = syn::parse2(pipeline.passes[lower].arguments.clone())?;
    ensure(
        authored.is_empty(),
        source,
        "fork derives lower facts; do not author a separate lower fact packet",
    )?;
    let variants = &source.incremental.variants;
    let mut fresh = Fresh::new(source);
    let mut context = RoundContext {
        relations: source
            .incremental
            .strata
            .source
            .declarations
            .iter()
            .zip(&source.providers)
            .map(|(declaration, providers)| Relation {
                declaration: declaration.clone(),
                providers: providers.clone(),
                next_canonical: fresh.id("next_canonical"),
                next: fresh.id("next"),
                delta: analysis
                    .deltas
                    .get(&symbol_name(&declaration.name))
                    .cloned(),
                delta_host: analysis
                    .deltas
                    .contains_key(&symbol_name(&declaration.name))
                    .then(|| fresh.id("delta")),
                admit: Vec::new(),
            })
            .collect(),
        layouts: source
            .layouts
            .iter()
            .map(|layout| Layout {
                relation: analysis.positions[&symbol_name(&layout.relation)],
                columns: layout.columns.clone(),
                key: layout.key,
                name: fresh.id("layout"),
                host: fresh.id("host"),
                provider: layout.provider.clone(),
                hasher: layout.hasher,
                maintenance: layout.maintenance.clone(),
                protocol: layout.protocol,
            })
            .collect(),
        delta_layouts: source
            .delta_layouts
            .iter()
            .map(|layout| Layout {
                relation: analysis.positions[&symbol_name(&layout.relation)],
                columns: layout.columns.clone(),
                key: layout.key,
                name: fresh.id("delta_layout"),
                host: fresh.id("delta_host"),
                provider: layout.provider.clone(),
                hasher: layout.hasher,
                maintenance: layout.maintenance.clone(),
                protocol: layout.protocol,
            })
            .collect(),
        strata: Vec::new(),
        outputs: source
            .incremental
            .strata
            .source
            .outputs
            .iter()
            .map(|id| {
                let relation = analysis.positions[&symbol_name(id)];
                Output {
                    relation,
                    conversion: if source.layouts[source.providers[relation].canonical].protocol
                        == storage::LayoutProtocol::Bulk
                    {
                        syn::parse_quote!(borrowed_cloned_into_iter)
                    } else {
                        syn::parse_quote!(owned_into_iter)
                    },
                }
            })
            .collect(),
        observer: source.incremental.strata.source.observer.clone(),
        observer_name: fresh.id("observer"),
        round_name: fresh.id("round"),
    };
    let mut holes: Vec<round::cq::Program> = Vec::new();
    let mut planning = Vec::new();
    for members in &analysis.components {
        let mut stratum = Stratum {
            members: members.clone(),
            keys: Vec::new(),
            seeds: Vec::new(),
            steps: Vec::new(),
            phases: vec![
                syn::parse_quote!(evaluate),
                syn::parse_quote!(admit),
                syn::parse_quote!(publish),
            ],
            publication: members
                .iter()
                .map(|&relation| {
                    let full: Vec<_> = context
                        .layouts
                        .iter()
                        .enumerate()
                        .filter(|(_, layout)| layout.relation == relation)
                        .map(|(index, _)| index)
                        .collect();
                    let delta: Vec<_> = context
                        .delta_layouts
                        .iter()
                        .enumerate()
                        .filter(|(_, layout)| layout.relation == relation)
                        .map(|(index, _)| index)
                        .collect();
                    // The canonical layout is a full layout, so this is never empty.
                    let mut operations = vec![PublicationOp {
                        kind: syn::parse_quote!(full_layouts),
                        layouts: full,
                    }];
                    operations.push(PublicationOp {
                        kind: syn::parse_quote!(delta_swap),
                        layouts: Vec::new(),
                    });
                    if !delta.is_empty() {
                        operations.push(PublicationOp {
                            kind: syn::parse_quote!(delta_rebuild),
                            layouts: delta,
                        });
                    }
                    Publication {
                        relation,
                        operations,
                    }
                })
                .collect(),
            termination: members.clone(),
            termination_test: syn::parse_quote!(sum_delta_lengths),
            observer: syn::parse_quote!(before_step),
            key_scope: syn::parse_quote!(component),
            loop_form: syn::parse_quote!(while_nonempty),
        };
        for (variant_index, variant) in variants.iter().enumerate() {
            let head = analysis.positions[&symbol_name(&variant.query.head.relation)];
            if !members.contains(&head) {
                continue;
            }
            let mut query = variant.query.clone();
            use_layouts(&mut query, &context, source, variant_index)?;
            for item in &mut query.body {
                let BodyItem::Negation(negative) = item else {
                    continue;
                };
                let relation = analysis.positions[&symbol_name(&negative.atom.relation)];
                let projection = columns(&negative.atom);
                let selections = filters(&negative.atom);
                let repeated = equalities(&negative.atom);
                let planned = source
                    .negations
                    .iter()
                    .find(|entry| {
                        entry.component == context.strata.len()
                            && entry.relation == relation
                            && entry.columns == projection
                            && entry.filters == selections
                            && entry.equalities == repeated
                    })
                    .expect("checked negative read plan");
                let projection = planned.columns.clone();
                let selections = planned.filters.clone();
                let repeated = planned.equalities.clone();
                let key_index = if let Some(index) = stratum.keys.iter().position(|key| {
                    key.relation == relation
                        && key.columns == projection
                        && key.filters == selections
                        && key.equalities == repeated
                }) {
                    index
                } else {
                    let key = KeySet {
                        relation,
                        columns: projection,
                        filters: selections,
                        equalities: repeated,
                        name: fresh.id("keys"),
                        provider: planned.provider.clone(),
                        hasher: planned.hasher,
                        hole: holes.len(),
                    };
                    let name = fresh.id("key_hole");
                    // Completed relations are scanned through their canonical layout.
                    let canonical = &context.layouts[source.providers[relation].canonical];
                    let source_name = &canonical.name;
                    let variables: Vec<_> = source.incremental.strata.source.declarations[relation]
                        .columns
                        .iter()
                        .map(|_| fresh.id("column"))
                        .collect();
                    let (selected, output_columns): (Vec<_>, Vec<_>) = if key.columns.is_empty() {
                        let column = &variables[0];
                        (vec![quote!(())], vec![quote!(#column: ())])
                    } else {
                        (
                            // Key columns are projected variables on every
                            // interface; a batch arena stores them as values.
                            key.columns
                                .iter()
                                .map(|&i| {
                                    let variable = &variables[i];
                                    quote!(#variable)
                                })
                                .collect(),
                            key.columns
                                .iter()
                                .map(|&i| {
                                    let variable = &variables[i];
                                    let column = &source.incremental.strata.source.declarations
                                        [relation]
                                        .columns[i];
                                    let (ty, domain) = (&column.ty, &column.domain);
                                    // Batch arenas own their scalar key columns;
                                    // host key sets retain borrowed projections.
                                    // A key column keeps its authored code domain.
                                    if interface == HoleInterface::Batch {
                                        quote!(#variable: #ty #domain)
                                    } else {
                                        quote!(#variable: &#ty #domain)
                                    }
                                })
                                .collect(),
                        )
                    };
                    // Every key filter compares after scanning, so owned
                    // values cannot move through an FnMut fanout closure.
                    let arguments: Vec<syn::Expr> = variables
                        .iter()
                        .map(|variable| syn::parse_quote!(#variable))
                        .collect();
                    let mut selections = Vec::new();
                    for (column, expression) in &key.filters {
                        let variable = &variables[*column];
                        selections.push(quote!(where #variable == (#expression)));
                    }
                    for &(first, repeated) in &key.equalities {
                        let first = &variables[first];
                        let repeated = &variables[repeated];
                        selections.push(quote!(where #first == #repeated));
                    }
                    let arguments = canonical.columns.iter().map(|&column| &arguments[column]);
                    let declarations = hole_inputs(
                        &source.tokens,
                        &context,
                        &[],
                        false,
                        std::iter::once(source_name),
                    )?;
                    holes.push(syn::parse2(quote!(struct #name; #(#declarations)* relation #name(#(#output_columns),*); #name(#(#selected),*) :- #source_name(#(#arguments),*) #(, #selections)*;))?);
                    planning.push(None);
                    stratum.keys.push(key);
                    stratum.keys.len() - 1
                };
                *item = negation(&negative.atom, &stratum.keys[key_index]);
            }
            let scan = scan_source(&query, &context).map(|source| ScanBinding {
                source,
                binding: fresh.id("scan"),
            });
            let scan_declaration = scan
                .as_ref()
                .map(|scan| scan_input(&mut query, &context, scan));
            let name = fresh.id("hole");
            query.head.relation = name.clone();
            let mut output = source.incremental.strata.source.declarations[head].clone();
            output.name = name.clone();
            // Formal rule DL-Fanout over the variant's source relations and
            // the program's recorded EDB keys.
            let strata_source = &source.incremental.strata.source;
            let fanout = round::fanout(&variant.query, &|relation| strata_source.keys(relation));
            let level = known_admission(
                &query,
                variant.known,
                source.device.is_some(),
                plan.is_some(),
                &fanout,
            )?;
            let known = if level.is_some() {
                Admission::InPlan
            } else {
                Admission::Admit
            };
            if level.is_some() {
                // Formal rule ADM-InPlan: name computed head terms by ordinary
                // bindings, then append the committed anti-join `!C(x̄)` over the
                // head's canonical layout, pinned after the FK-Known level.
                let declaration = &source.incremental.strata.source.declarations[head];
                for (position, argument) in query.head.args.iter_mut().enumerate() {
                    if cq_syntax::rust::variable(argument).is_none() {
                        let variable = fresh.id("known");
                        let ty = &declaration.columns[position].ty;
                        let binding: BodyItem = syn::parse_quote!(let #variable: #ty = #argument);
                        query.body.push(binding);
                        *argument = syn::parse_quote!(#variable);
                    }
                }
                let args: Vec<syn::Expr> = query.head.args.iter().cloned().collect();
                query.body.push(admission_negation(&context, head, &args));
            }
            let declarations = hole_inputs(
                &source.tokens,
                &context,
                &stratum.keys,
                interface != HoleInterface::Batch,
                query.body.iter().filter_map(|item| match item {
                    BodyItem::Positive { atom } => Some(&atom.relation),
                    BodyItem::Negation(negative) => Some(&negative.atom.relation),
                    _ => None,
                }),
            )?;
            let hole = holes.len();
            holes.push(syn::parse2(
                quote!(struct #name; #(#declarations)* #scan_declaration #output #query),
            )?);
            planning.push(Some(Planning {
                order: variant
                    .query
                    .body
                    .iter()
                    .enumerate()
                    .filter_map(|(position, item)| {
                        matches!(item, BodyItem::Positive { .. }).then_some(variant.order[position])
                    })
                    .collect(),
                authored: variant.authored,
                steps: variant
                    .steps
                    .iter()
                    .map(|step| PlanningStep {
                        occurrence: step.occurrence,
                        columns: step.columns.clone(),
                    })
                    .collect(),
                smallest: variant.smallest.clone(),
                known: level,
            }));
            context.relations[head].admit.push(hole);
            let rule = RuleHole {
                variant: variant_index,
                head,
                hole,
                scan,
                known,
                fanout,
            };
            if variant.delta.is_some() {
                stratum.steps.push(rule);
            } else {
                stratum.seeds.push(rule);
            }
        }
        context.strata.push(stratum);
    }
    let mut certified_hosts = std::collections::BTreeSet::new();
    for certificate in &source.certificates {
        let name = match symbol_name(&certificate.kind).as_str() {
            "delta" => context.relations[certificate.index].delta.as_ref(),
            "layout" => Some(&context.layouts[certificate.index].name),
            "delta_layout" => Some(&context.delta_layouts[certificate.index].name),
            _ => None,
        };
        if let Some(name) = name {
            certified_hosts.insert(symbol_name(name));
        }
    }
    for rule in context
        .strata
        .iter()
        .flat_map(|s| s.seeds.iter().chain(&s.steps))
    {
        if let Some(scan) = &rule.scan {
            let source_name = match scan.source {
                InputSource::Delta(index) => context.relations[index].delta.as_ref(),
                InputSource::Layout(index) => Some(&context.layouts[index].name),
            };
            if source_name.is_some_and(|name| certified_hosts.contains(&symbol_name(name))) {
                certified_hosts.insert(symbol_name(&scan.binding));
            }
        }
    }
    /// Formal rule DL-PlanningOwner: authored order targets only its recorded planning invocation.
    fn check_planning_owner(
        pipeline: &cq_stage::Pipeline,
        plan: Option<usize>,
        lower: usize,
        authored: bool,
    ) -> syn::Result<()> {
        if let Some(plan) = plan {
            ensure(
                plan < lower && plan < pipeline.passes.len(),
                pipeline,
                "planning invocation must precede lowering",
            )?;
            if authored {
                ensure(
                    pipeline.passes[plan].arguments.is_empty(),
                    pipeline,
                    "authored Datalog order conflicts with planning invocation arguments",
                )?;
            }
        } else {
            ensure(
                !authored,
                pipeline,
                "authored Datalog order requires a recorded planning invocation: plan = <ordinal>",
            )?;
        }
        Ok(())
    }
    check_planning_owner(
        &pipeline,
        plan,
        lower,
        planning
            .iter()
            .flatten()
            .any(|facts| facts.authored || facts.known.is_some()),
    )?;
    debug_assert_eq!(holes.len(), planning.len());
    let holes = holes
        .into_iter()
        .zip(planning)
        .map(|(cq, planning)| {
            let facts = cq_stage::Context {
                consumer_canonical: true,
                discharged_host_sets: cq
                    .query
                    .body
                    .iter()
                    .filter_map(|item| match item {
                        BodyItem::Positive { atom }
                            if certified_hosts.contains(&symbol_name(&atom.relation)) =>
                        {
                            Some(symbol_name(&atom.relation))
                        }
                        _ => None,
                    })
                    .collect(),
            };
            let mut pipeline = pipeline.clone();
            pipeline.passes[lower].arguments = facts.to_token_stream();
            Hole {
                answer: round::answer_ownership(&cq, interface),
                cq,
                pipeline,
                interface,
                facts,
                lower,
                plan,
                planning,
                binding: None,
            }
        })
        .collect();
    let schedule = cq_lang_datalog_round::Schedule::whole(source, &analysis, &context)?;
    let target = Program {
        storage: source.clone(),
        context,
        holes,
        terminals,
        execution: Vec::new(),
        policy: RoundPolicy { timing },
        schedule,
    };
    check(source, &target)?;
    Ok(target)
}
