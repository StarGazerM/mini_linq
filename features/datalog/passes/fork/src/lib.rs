//! Record complete CQ holes, immutable round contexts, and their invocation queues.
use cq_datalog_support::{Fresh, columns, ensure, filters};
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
        print = ::mini_linq::passes::print_pull;
    })
}
pub fn default_pipeline() -> cq_stage::Pipeline {
    cq_stage::Configuration::default()
        .resolve(cq_defaults())
        .expect("complete CQ defaults")
}
pub fn compile(source: &storage::Program) -> syn::Result<Program> {
    construct(
        source,
        default_pipeline(),
        PullPrinters::default(),
        syn::parse_quote!(none),
    )
}
struct Arguments {
    pipeline: cq_stage::Pipeline,
    pull_printers: PullPrinters,
    timing: Ident,
}
impl syn::parse::Parse for Arguments {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut pipeline = None;
        let mut pull_printers = None;
        let mut timing = None;
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            if name == "cq" && pipeline.is_none() {
                pipeline = Some(input.parse()?);
            } else if name == "timing" && timing.is_none() {
                timing = Some(input.parse()?);
            } else if name == "pull_printers" && pull_printers.is_none() {
                pull_printers = Some(input.parse()?);
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
            pull_printers: pull_printers.unwrap_or_default(),
            timing: timing.unwrap_or_else(|| syn::parse_quote!(none)),
        })
    }
}
pub fn compile_with(source: &storage::Program, args: TokenStream) -> syn::Result<Program> {
    let Arguments {
        pipeline,
        pull_printers,
        timing,
    } = syn::parse2(args)?;
    construct(source, pipeline, pull_printers, timing)
}
/// Formal rule CK-DL-Fork: independently check storage embedding and every hole.
pub fn check(source: &storage::Program, target: &Program) -> syn::Result<()> {
    storage::contract::check(source)?;
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
    round::check_storage(source, target)
}

fn guard(atom: &cq_syntax::Atom, key: &KeySet) -> BodyItem {
    let name = &key.name;
    if key.columns.is_empty() {
        syn::parse_quote!(#[total] if !#name.contains(&((),)))
    } else {
        let args = key.columns.iter().map(|&i| &atom.args[i]);
        syn::parse_quote!(#[total] if !#name.contains(&(#(#args,)*)))
    }
}

fn key_representation(source: &storage::Program) -> syn::Result<KeyRepresentation> {
    match Some(symbol_name(&source.policy.negation_keys)).as_deref() {
        Some("hash_set_borrowed") => Ok(KeyRepresentation::HashSetBorrowed),
        _ => Err(syn::Error::new_spanned(
            source,
            "fork requires negation_keys hash_set_borrowed",
        )),
    }
}

/// Lower variants and key projections, retaining the recorded schedule.
fn construct(
    source: &storage::Program,
    pipeline: cq_stage::Pipeline,
    pull_printers: PullPrinters,
    timing: Ident,
) -> syn::Result<Program> {
    let analysis = storage::contract::check(source)?;
    let variants = &source.incremental.variants;
    let representation = key_representation(source)?;
    let mut fresh = Fresh::new(source);
    let mut context = RoundContext {
        relations: source
            .incremental
            .strata
            .source
            .declarations
            .iter()
            .map(|declaration| Relation {
                declaration: declaration.clone(),
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
            })
            .collect(),
        strata: Vec::new(),
        outputs: source
            .incremental
            .strata
            .source
            .outputs
            .iter()
            .map(|id| analysis.positions[&symbol_name(id)])
            .collect(),
        observer: source.incremental.strata.source.observer.clone(),
        observer_name: fresh.id("observer"),
        round_name: fresh.id("round"),
    };
    let declarations = inputs(&context);
    let mut holes = Vec::new();
    for members in &analysis.components {
        let mut stratum = Stratum {
            members: members.clone(),
            keys: Vec::new(),
            seeds: Vec::new(),
            steps: Vec::new(),
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
                let planned = source
                    .negations
                    .iter()
                    .find(|entry| {
                        entry.component == context.strata.len()
                            && entry.relation == relation
                            && entry.columns == projection
                            && entry.filters == selections
                    })
                    .expect("checked negative read plan");
                let projection = planned.columns.clone();
                let selections = planned.filters.clone();
                let key_index = if let Some(index) = stratum.keys.iter().position(|key| {
                    key.relation == relation
                        && key.columns == projection
                        && key.filters == selections
                }) {
                    index
                } else {
                    let key = KeySet {
                        relation,
                        columns: projection,
                        filters: selections,
                        name: fresh.id("keys"),
                        representation,
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
                                .map(|&i| variables[i].to_token_stream())
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
                    // Keep owned constant expressions in post-scan predicates.
                    // An atom expression becomes an owned CQ prefix binding;
                    // carrying that binding through scan fanout can move it out
                    // of an FnMut closure. Comparison borrows both operands.
                    let selections = key.filters.iter().map(|(column, expression)| {
                        let variable = &variables[*column];
                        quote!(if *#variable == (#expression))
                    });
                    holes.push(syn::parse2(quote!(struct #name; #(#declarations)* relation #name(#(#output_columns),*); #name(#(#selected),*) :- #source_name(#(#variables),*) #(, #selections)*;))?);
                    stratum.keys.push(key);
                    stratum.keys.len() - 1
                };
                *item = guard(&negative.atom, &stratum.keys[key_index]);
            }
            let scan = scan_source(&query, &context).map(|source| ScanBinding {
                source,
                binding: fresh.id("scan"),
            });
            let scan_declaration = scan
                .as_ref()
                .map(|scan| use_scan(&mut query, &context, scan));
            let name = fresh.id("hole");
            query.head.relation = name.clone();
            let mut output = source.incremental.strata.source.declarations[head].clone();
            output.name = name.clone();
            let hole = holes.len();
            holes.push(syn::parse2(
                quote!(struct #name; #(#declarations)* #scan_declaration #output #query),
            )?);
            let rule = RuleHole {
                variant: variant_index,
                head,
                hole,
                scan,
            };
            if variant.delta.is_some() {
                stratum.steps.push(rule);
            } else {
                stratum.seeds.push(rule);
            }
        }
        context.strata.push(stratum);
    }
    let holes = holes
        .into_iter()
        .map(|cq| Hole {
            cq,
            pipeline: pipeline.clone(),
            interface: HoleInterface::Pull,
        })
        .collect();
    let target = Program {
        storage: source.clone(),
        context,
        holes,
        pull_printers,
        execution: Vec::new(),
        policy: RoundPolicy {
            phases: syn::parse_quote!(evaluate_admit_publish),
            admission: syn::parse_quote!(written_order),
            termination: syn::parse_quote!(any_delta),
            observer: syn::parse_quote!(before_step),
            outputs: syn::parse_quote!(owned_iterators),
            timing,
        },
    };
    check(source, &target)?;
    Ok(target)
}
