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
        None,
    )
}
struct Arguments {
    pipeline: cq_stage::Pipeline,
    terminals: TerminalRegistry,
    timing: Ident,
    lower: Option<usize>,
}
impl syn::parse::Parse for Arguments {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut pipeline = None;
        let mut terminals = None;
        let mut timing = None;
        let mut lower = None;
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
            } else {
                return Err(syn::Error::new_spanned(
                    name,
                    "unknown or duplicate fork argument",
                ));
            }
            input.parse::<syn::Token![;]>()?;
        }
        Ok(Self {
            pipeline: pipeline.unwrap_or_else(default_pipeline),
            terminals: terminals.unwrap_or_default(),
            timing: timing.unwrap_or_else(|| syn::parse_quote!(none)),
            lower,
        })
    }
}
pub fn compile_with(source: &storage::Program, args: TokenStream) -> syn::Result<Program> {
    let Arguments {
        pipeline,
        terminals,
        timing,
        lower,
    } = syn::parse2(args)?;
    construct(source, pipeline, terminals, timing, lower)
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
    let BodyItem::Positive { atom } = &mut query.body[0] else {
        unreachable!("scan source is a positive read")
    };
    atom.relation = binding.clone();
    declaration
}
// Each hole owns only the CQ inputs it reads, including completed borrowed keys.
fn hole_inputs<'a>(
    context: &RoundContext,
    keys: &[KeySet],
    names: impl Iterator<Item = &'a Ident>,
) -> Vec<cq_syntax::RelationDecl> {
    let names: std::collections::BTreeSet<_> = names.map(symbol_name).collect();
    let mut inputs = Vec::new();
    for relation in &context.relations {
        if names.contains(&symbol_name(&relation.declaration.name)) {
            let mut declaration = relation.declaration.clone();
            let host = &relation.all;
            declaration.value = Some(if declaration.value.is_some() {
                syn::parse_quote!(= (#host))
            } else {
                syn::parse_quote!(= (&#host))
            });
            inputs.push(declaration);
        }
        if let Some(delta) = &relation.delta {
            if names.contains(&symbol_name(delta)) {
                let mut declaration = relation.declaration.clone();
                declaration.name = delta.clone();
                let host = relation.delta_host.as_ref().expect("checked delta host");
                declaration.value = Some(syn::parse_quote!(= (&#host)));
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
            inputs.push(declaration);
        }
    }
    inputs
}
/// Lower variants and key projections, retaining the recorded schedule.
fn construct(
    source: &storage::Program,
    pipeline: cq_stage::Pipeline,
    terminals: TerminalRegistry,
    timing: Ident,
    lower: Option<usize>,
) -> syn::Result<Program> {
    let analysis = storage::contract::check(source)?;
    terminals.check()?;
    let interface = terminals.interface(&pipeline)?;
    let lower = if let Some(lower) = lower {
        ensure(
            lower < pipeline.passes.len(),
            source,
            "lower invocation ordinal is out of range",
        )?;
        lower
    } else {
        let candidates: Vec<_> = pipeline
            .passes
            .iter()
            .enumerate()
            .filter(|(_, pass)| {
                pass.path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "lower")
            })
            .map(|(index, _)| index)
            .collect();
        ensure(
            candidates.len() == 1,
            source,
            "fork requires exactly one named lower invocation; supply lower = <ordinal> for a custom alias",
        )?;
        candidates[0]
    };
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
                all: fresh.id("all"),
                seen: fresh.id("seen"),
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
                maintenance: layout.maintenance.clone(),
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
                maintenance: layout.maintenance.clone(),
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
                    conversion: if source.incremental.strata.source.declarations[relation]
                        .value
                        .is_some()
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
                    let mut operations = Vec::new();
                    if !full.is_empty() {
                        operations.push(PublicationOp {
                            kind: syn::parse_quote!(full_layouts),
                            layouts: full,
                        });
                    }
                    operations.push(PublicationOp {
                        kind: syn::parse_quote!(all_extend),
                        layouts: Vec::new(),
                    });
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
                        provider: syn::parse_quote!(::std::collections::HashSet<_, ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>),
                        hole: holes.len(),
                    };
                    let name = fresh.id("key_hole");
                    let source_name = &source.incremental.strata.source.declarations[relation].name;
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
                                    let ty = &source.incremental.strata.source.declarations
                                        [relation]
                                        .columns[i]
                                        .ty;
                                    quote!(#variable: &#ty)
                                })
                                .collect(),
                        )
                    };
                    // Literals stay in the filtered occurrence. Nonliteral
                    // expressions compare after scanning so owned values cannot
                    // move through an FnMut fanout closure.
                    let mut arguments: Vec<syn::Expr> = variables
                        .iter()
                        .map(|variable| syn::parse_quote!(#variable))
                        .collect();
                    let mut selections = Vec::new();
                    for (column, expression) in &key.filters {
                        if cq_syntax::equality::literal(cq_syntax::rust::unparen(expression)) {
                            arguments[*column] = cq_syntax::rust::unparen(expression).clone();
                        } else {
                            let variable = &variables[*column];
                            selections.push(quote!(if *#variable == (#expression)));
                        }
                    }
                    for &(first, repeated) in &key.equalities {
                        let first = &variables[first];
                        let repeated = &variables[repeated];
                        selections.push(quote!(if #first == #repeated));
                    }
                    let declarations = hole_inputs(&context, &[], std::iter::once(source_name));
                    holes.push(syn::parse2(quote!(struct #name; #(#declarations)* relation #name(#(#output_columns),*); #name(#(#selected),*) :- #source_name(#(#arguments),*) #(, #selections)*;))?);
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
            let declarations = hole_inputs(
                &context,
                &stratum.keys,
                query.body.iter().filter_map(|item| match item {
                    BodyItem::Positive { atom } => Some(&atom.relation),
                    BodyItem::Negation(negative) => Some(&negative.atom.relation),
                    _ => None,
                }),
            );
            let hole = holes.len();
            holes.push(syn::parse2(
                quote!(struct #name; #(#declarations)* #scan_declaration #output #query),
            )?);
            context.relations[head].admit.push(hole);
            let rule = RuleHole {
                variant: variant_index,
                head,
                hole,
                scan,
                insertion: syn::parse_quote!(insert),
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
            "all" => Some(&context.relations[certificate.index].declaration.name),
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
    let holes = holes
        .into_iter()
        .map(|cq| {
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
                cq,
                pipeline,
                interface,
                facts,
                lower,
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
