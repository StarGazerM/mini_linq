#[path = "src/advice.rs"]
mod advice;
mod build_stages;
#[path = "src/manifest.rs"]
#[allow(dead_code)]
mod manifest;
#[path = "src/source.rs"]
mod source;
#[path = "src/stages.rs"]
mod stages;

use manifest::{Manifest, Result};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use serde_json::json;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

struct Rejected {
    phase: &'static str,
    error: String,
    term: String,
}

type CompileResult<T> = std::result::Result<T, Rejected>;

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct Publication {
    placement: advice::Placement,
    nullable: Vec<bool>,
    domains: Vec<Option<(String, String)>>,
}

impl Publication {
    fn resident(&self) -> bool {
        self.placement == advice::Placement::Device
    }
}

fn nullable_inputs(
    program: &cq_lang_cq::Program,
    inputs: Option<&std::collections::BTreeMap<String, Publication>>,
) -> std::collections::BTreeSet<String> {
    let mut nullable = std::collections::BTreeSet::new();
    for item in &program.query.body {
        let cq_lang_cq::BodyItem::Positive { atom } = item else { continue };
        if let Some(input) = inputs.and_then(|inputs| inputs.get(&atom.relation.to_string())) {
            for (argument, valid) in atom.args.iter().zip(&input.nullable) {
                if *valid && let syn::Expr::Path(path) = argument {
                    nullable.insert(path.path.to_token_stream().to_string());
                }
            }
        }
    }
    nullable
}

/// Formal rule DR-Publication: Check explicit placement against recorded semantic validity before generation.
fn publication(
    program: &cq_lang_cq::Program,
    pipeline: &advice::Pipeline,
    inputs: Option<&std::collections::BTreeMap<String, Publication>>,
) -> syn::Result<Publication> {
    let mut nullable = nullable_inputs(program, inputs);
    if let Some(group) = &program.query.group {
        for aggregate in &group.aggregates {
            if matches!(aggregate.kind, cq_syntax::algebra::AggregateKind::Min | cq_syntax::algebra::AggregateKind::Max) {
                nullable.insert(aggregate.name.to_string());
            }
        }
    }
    let nullable: Vec<_> = program.query.head.args.iter()
        .map(|argument| matches!(argument, syn::Expr::Path(path)
            if nullable.contains(&path.path.to_token_stream().to_string())))
        .collect();
    let placement = pipeline.publication.resolve(pipeline.target, nullable.iter().any(|value| *value));
    if placement == advice::Placement::Device
        && (matches!(pipeline.target, advice::Target::Host) || nullable.iter().any(|value| *value))
    {
        return Err(syn::Error::new_spanned(program,
            "device publication requires a device target and nonnullable columns; select host publication"));
    }
    Ok(Publication { placement, nullable, domains: Vec::new() })
}

fn checked<T>(
    phase: &'static str,
    term: &impl ToTokens,
    result: syn::Result<T>,
) -> CompileResult<T> {
    result.map_err(|error| Rejected {
        phase,
        error: error
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        term: term.to_token_stream().to_string(),
    })
}

fn compile(
    kernel: &source::Kernel,
    manifest: &Manifest,
    pipeline: &advice::Pipeline,
) -> CompileResult<(TokenStream, serde_json::Value)> {
    compile_stage(kernel, manifest, pipeline, None)
}

fn compile_stage(
    kernel: &source::Kernel,
    manifest: &Manifest,
    pipeline: &advice::Pipeline,
    publications: Option<&std::collections::BTreeMap<String, Publication>>,
) -> CompileResult<(TokenStream, serde_json::Value)> {
    let cpu_source = kernel.cpu_query();
    let host = matches!(pipeline.target, advice::Target::Host);
    let storage_schema = match pipeline.storage.as_str() {
        "" | "tpch_dictionary" => None,
        "tpch_domains" => Some(gpu_compress_host::tpch::schema()),
        other => {
            return Err(Rejected {
                phase: "loader profile",
                error: format!("unknown storage profile {other}"),
                term: cpu_source.to_string(),
            });
        }
    };
    let mut stored_types = kernel.scalar_codec_types(manifest).map_err(|error| Rejected {
        phase: "loader representation", error: error.to_string(), term: cpu_source.to_string(),
    })?;
    let mut domains = std::collections::BTreeMap::new();
    if let Some(schema) = &storage_schema {
        for relation in &kernel.relations {
            let columns = schema
                .tables
                .iter()
                .find(|(name, _)| name == &relation.table.to_string())
                .map(|(_, columns)| columns)
                .ok_or_else(|| Rejected {
                    phase: "loader profile",
                    error: format!("storage profile has no table {}", relation.table),
                    term: cpu_source.to_string(),
                })?;
            if columns.len() != relation.columns.len()
                || columns
                    .iter()
                    .zip(&relation.columns)
                    .any(|((name, _), (column, _))| name != &column.to_string())
            {
                return Err(Rejected {
                    phase: "loader profile",
                    error: format!("storage profile projection differs for {}", relation.table),
                    term: cpu_source.to_string(),
                });
            }
            for (column, kind) in columns {
                let key = (relation.table.to_string(), column.clone());
                let ty = match kind {
                    gpu_compress_host::Kind::Id(domain) => {
                        domains.insert(key.clone(), domain.clone());
                        syn::parse_quote!(u32)
                    }
                    gpu_compress_host::Kind::I32 => syn::parse_quote!(i32),
                };
                stored_types.insert(key, ty);
            }
        }
    }
    // Dictionary IDs are an input representation, independent of the schedule
    // or whether the authored query contains opaque Rust reads.
    stored_types.extend(kernel.dictionary_types(manifest).map_err(|error| Rejected {
        phase: "loader representation",
        error: error.to_string(),
        term: cpu_source.to_string(),
    })?);
    let setup_bindings = kernel.setup_bindings(manifest).map_err(|error| Rejected {
        phase: "loader captures", error: error.to_string(), term: cpu_source.to_string(),
    })?;
    let encoded = kernel
        .encoded_query_with_types(manifest, &stored_types)
        .map_err(|error| Rejected {
            phase: "loader",
            error: error.to_string(),
            term: cpu_source.to_string(),
        })?;
    let cq: cq_lang_cq::Program = checked("CQ parse", &encoded, syn::parse2(encoded.clone()))?;
    let normal = checked("CQ normalize", &cq, cq_lang_cq::normal::normalize(&cq))?;
    let mut publication = checked("publication contract", &normal, publication(&normal, pipeline, publications))?;
    publication.domains = kernel.answer_columns().map_err(|error| Rejected {
        phase: "publication domains", error: error.to_string(), term: cpu_source.to_string(),
    })?.into_iter().map(|origin| origin.filter(|(table, column)|
        manifest.column(table, column).is_ok_and(|metadata| metadata.encoding == "dictionary")
    )).collect();
    let reduced = checked(
        "CQ plan advice",
        &normal,
        advice::valuation(&cq, &pipeline.plan),
    )?;
    let ra = checked("CQ elaborate", &reduced, cq_elaborate::compile(&reduced))?;
    checked(
        "CQ elaborate check",
        &ra,
        cq_elaborate::check(&reduced, &ra),
    )?;
    let ix = checked("index demands", &ra, cq_index_policy::compile(&ra))?;
    let mut pipeline = pipeline.clone();
    pipeline.publication = publication.placement;
    let physical = checked("Physical lowering", &ix, cq_physical_lowering::compile(&ix))?;
    let recipe = pipeline.recipe.clone();
    for relation in &recipe.filtered_leaves {
        let selected = checked(
            "filtered Part layout",
            &physical,
            cq_structure_selection::compile(&physical),
        )?;
        if let Some(leaf) = checked(
            "filtered Part retention",
            &selected,
            advice::filtered_leaf(&selected, relation),
        )? {
            pipeline
                .physical
                .insert(0, quote!(retain_streams(#leaf)).to_string());
        }
    }
    checked(
        "Physical recipe",
        &physical,
        advice::hand_physical(&physical, &recipe, &mut pipeline),
    )?;
    let physical = checked(
        "Physical advice",
        &physical,
        advice::physical(&physical, &pipeline.physical),
    )?;
    let default_structures = checked(
        "structure selection",
        &physical,
        cq_structure_selection::compile(&physical),
    )?;
    for relation in &recipe.membership_relations {
        pipeline.structures.insert(
            0,
            checked(
                "provider layout",
                &physical,
                advice::provider_layout(&physical, relation),
            )?,
        );
    }
    if recipe.membership {
        pipeline.structures.extend(checked(
            "hand membership layout",
            &default_structures,
            advice::hand_membership(&default_structures),
        )?);
    }
    let selected = checked(
        "Structures advice",
        &default_structures,
        advice::structures(&default_structures, &pipeline.structures),
    )?;
    let catalog = if host {
        cq_sequence_pull::catalog()
    } else {
        cq_gpu_sequence::catalog()
    };
    let machine = if host {
        cq_sequence_pull::machine()
    } else {
        cq_gpu_sequence::machine()
    };
    let mut structures = checked(
        "catalog selection",
        &selected,
        cq_sequence_plan::select_from_catalog(&selected, &catalog),
    )?;
    let facts = checked(
        "Structures check",
        &structures,
        cq_lang_structures::contract::check(&structures),
    )?;
    // Nullable publication payloads have no device scalar ABI. A join carrying
    // one from either side needs a host-readable index, until a later projection
    // removes that column. Use checked layouts, never Rust type spellings.
    if !host && publications.is_some_and(|inputs|
        inputs.values().any(|input| input.nullable.iter().any(|value| *value)))
    {
        let nullable = nullable_inputs(&normal, publications);
        let host_structures = checked(
            "nullable host provider selection", &selected,
            cq_sequence_plan::select_from_catalog(&selected, &cq_sequence_pull::catalog()),
        )?;
        for index in 0..structures.structures.entries.len() {
            let entry = &structures.structures.entries[index];
            let name = entry.name.to_string();
            let nullable_reader = facts.physical.keyed_reads.get(&name).into_iter().flatten()
                .map(|(reader, _, _)| reader)
                .chain(facts.physical.merge_reads.get(&name).into_iter().flatten()
                    .map(|(reader, _)| reader))
                .any(|reader| facts.layouts.get(reader).is_some_and(|columns|
                    columns.iter().any(|column| nullable.contains(column)))
                    && physical.program.definitions.iter().any(|definition|
                        definition.name == reader.as_str() && matches!(
                            definition.operator,
                            cq_lang_physical::Operator::Probe(..) | cq_lang_physical::Operator::Merge(..)
                        )));
            if matches!(entry.structure, cq_lang_structures::Structure::Built { .. })
                && (nullable_reader || advice::relation_of(&structures.physical, &entry.name)
                    .and_then(|relation| publications.and_then(|inputs| inputs.get(&relation.to_string())))
                    .is_some_and(|input| input.nullable.iter().any(|value| *value)))
            {
                let cq_lang_structures::Structure::Built { keyed, .. } =
                    &host_structures.structures.entries[index].structure else {
                        unreachable!("catalog selection preserves provider entries");
                    };
                let ty = &keyed.ty;
                structures = checked("nullable host provider", &structures,
                    cq_structure_selection::retype(&structures, quote!(entry(#index) => #ty)))?;
            }
        }
    }
    // The loader, not the planner or translator, owns the input representation.
    // Record its uniform DeviceColumns adapter with the checked retype rewrite.
    for index in 0..structures.structures.entries.len() {
        let entry = &structures.structures.entries[index];
        if !host && matches!(entry.structure, cq_lang_structures::Structure::Host { .. }) {
            let host = &facts.physical.hosts[&entry.name.to_string()];
            let relation = physical
                .program
                .declarations
                .iter()
                .find(|r| r.name == host.as_str())
                .unwrap();
            if publications
                .and_then(|inputs| inputs.get(&relation.name.to_string()))
                .is_some_and(|input| !input.resident())
            {
                continue;
            }
            if kernel
                .derived
                .iter()
                .any(|(name, _)| *name == relation.name)
            {
                continue;
            }
            let types = relation.columns.iter().map(|c| &c.ty);
            structures = checked(
                "loader host adapter",
                &structures,
                cq_structure_selection::retype(
                    &structures,
                    if kernel
                        .published
                        .iter()
                        .any(|input| input.name == relation.name)
                    {
                        quote!(entry(#index) => ::cq_gpu_runtime::DeviceColumns<'static, (#(#types,)*)>)
                    } else {
                        quote!(entry(#index) => ::cq_gpu_runtime::DeviceColumns<'static, (#(#types,)*), ::cq_gpu_runtime::For>)
                    },
                ),
            )?;
        }
    }
    let mut sequence = checked(
        "Sequence lowering",
        &structures,
        cq_sequence_lowering::compile(&structures),
    )?;
    // Loader/provider facts, never host type spellings, justify device Rust.
    let origins = kernel.variable_origins().map_err(|error| Rejected {
        phase: "loader representation", error: error.to_string(), term: cpu_source.to_string(),
    })?;
    for (variable, origin) in origins {
        let metadata = manifest.column(&origin.0, &origin.1).map_err(|error| Rejected {
            phase: "loader representation", error: error.to_string(), term: cpu_source.to_string(),
        })?;
        if domains.contains_key(&origin)
            || (matches!(metadata.encoding.as_str(), "scaled_integer" | "days_since_epoch")
                && stored_types.contains_key(&origin))
        {
            sequence.representation.encoded.insert(variable);
        } else {
            sequence.representation.identity.insert(variable);
        }
    }
    if let Some(publications) = publications {
        for item in &normal.query.body {
            let cq_lang_cq::BodyItem::Positive { atom } = item else { continue };
            if publications.get(&atom.relation.to_string()).is_some_and(Publication::resident) {
                for argument in &atom.args {
                    if let syn::Expr::Path(path) = argument
                        && let Some(name) = path.path.get_ident()
                    {
                        sequence.representation.identity.insert(name.to_string());
                    }
                }
            }
        }
    }
    if !host {
        sequence = checked(
            "target representation binding",
            &sequence,
            cq_gpu_sequence::record_representation(&sequence),
        )?;
    }
    checked(
        "Sequence lowering check",
        &sequence,
        cq_sequence_lowering::check(&structures, &sequence),
    )?;
    if !pipeline.complete {
        let evidence = cq_lang_sequence::Evidence {
            facts: kernel
                .relations
                .iter()
                .flat_map(|relation| {
                    let table = relation.table.to_string();
                    let mut facts = vec![cq_lang_sequence::EvidenceFact::Count {
                        subject: relation.name.clone(),
                        rows: manifest.relations[&table].rows,
                    }];
                    // TPC-H's declared primary keys are assertions about the
                    // dbgen input, not cardinality estimates. A projected loader
                    // only carries a key when every key column is present.
                    let key: &[&str] = match table.as_str() {
                        "lineitem" => &["l_orderkey", "l_linenumber"],
                        "orders" => &["o_orderkey"],
                        "part" => &["p_partkey"],
                        "partsupp" => &["ps_partkey", "ps_suppkey"],
                        "supplier" => &["s_suppkey"],
                        "customer" => &["c_custkey"],
                        "nation" => &["n_nationkey"],
                        "region" => &["r_regionkey"],
                        _ => &[],
                    };
                    if !key.is_empty()
                        && let Some(columns) = key
                            .iter()
                            .map(|key| {
                                relation
                                    .columns
                                    .iter()
                                    .find(|(name, _)| name == key)
                                    .map(|(name, _)| name.clone())
                            })
                            .collect::<Option<Vec<_>>>()
                    {
                        facts.push(cq_lang_sequence::EvidenceFact::Key {
                            subject: relation.name.clone(),
                            columns,
                        });
                    }
                    facts
                })
                .collect(),
        };
        pipeline.sequence.insert(
            0,
            cq_sequence_plan::Advice::Evidence(evidence)
                .to_token_stream()
                .to_string(),
        );
    }
    if !kernel.evidence.facts.is_empty() {
        pipeline.sequence.insert(
            0,
            cq_sequence_plan::Advice::Evidence(kernel.evidence.clone())
                .to_token_stream()
                .to_string(),
        );
    }
    let mut authored = checked(
        "Sequence advice",
        &sequence,
        advice::apply_sequence(&sequence, &pipeline.sequence),
    )?;
    let planned = checked(
        "decompose(machine)",
        &authored.program,
        if pipeline.complete {
            Ok(cq_sequence_plan::Planned {
                program: authored.program.clone(),
                trace: Vec::new(),
            })
        } else if host {
            cq_sequence_plan::decompose(&authored.program, &machine, &catalog)
        } else if let Some(schedule) = &recipe.schedule {
            advice::hand_schedule(&authored.program, schedule)
        } else {
            cq_sequence_plan::decompose(
                &authored.program,
                &cq_gpu_sequence::machine(),
                &cq_gpu_sequence::catalog(),
            )
        },
    )?;
    authored.trace.extend(planned.trace.iter().cloned());
    let trace = advice::trace_tokens(&authored.trace);
    pipeline.sequence = authored
        .trace
        .iter()
        .map(|step| step.to_token_stream().to_string())
        .collect();
    pipeline.complete = true;
    pipeline.recipe = advice::Recipe::default();
    let artifact = json!({
        "pipeline": pipeline,
        "recipe_input": recipe,
        "publication_facts": publication,
        "sequence_input": sequence.to_token_stream().to_string(),
        "sequence_trace": trace,
        "sequence_planned": planned.program.to_token_stream().to_string(),
        "cost": checked("PL-Cost", &planned.program,
            cq_sequence_plan::cost(&planned.program, &machine))?.to_string(),
    });
    let analysis = checked(
        "machine_grade",
        &planned.program,
        cq_lang_sequence::check_grading(&planned.program, &machine),
    )?;
    let gpu = if host {
        checked(
            "host translate",
            &planned.program,
            cq_sequence_pull::translate(&planned.program, &analysis),
        )?
    } else {
        checked(
            "translate",
            &planned.program,
            cq_gpu_sequence::translate(&planned.program, &analysis),
        )?
    };

    // Independently select/lower Sequential from the identical Physical. No GPU
    // schedule, provider, partition, or canonicalization advice is copied over.
    // Lookup-first can reexpress Physical; referencing the pre-plan graph would
    // restore the discarded Cartesian prefixes instead of testing this plan.
    let cpu_structures = checked(
        "Sequential structure selection",
        &planned.program.structures.physical,
        cq_structure_selection::compile(&planned.program.structures.physical),
    )?;
    let cpu_sequence = checked(
        "Sequential lowering",
        &cpu_structures,
        cq_sequence_lowering::compile(&cpu_structures),
    )?;
    let cpu_plan = checked(
        "Sequential plan",
        &cpu_sequence,
        cq_sequence_plan::decompose(
            &cpu_sequence,
            &cq_sequence_pull::machine(),
            &cq_sequence_pull::catalog(),
        ),
    )?;
    let cpu_analysis = checked(
        "Sequential grade",
        &cpu_plan.program,
        cq_lang_sequence::check_grading(&cpu_plan.program, &cq_sequence_pull::machine()),
    )?;
    let cpu = checked(
        "Sequential translate",
        &cpu_plan.program,
        cq_sequence_pull::translate(&cpu_plan.program, &cpu_analysis),
    )?;

    let fields: Vec<_> = kernel
        .relations
        .iter()
        .map(|r| &r.name)
        .chain(kernel.derived.iter().map(|(name, _)| name))
        .collect();
    let tables: Vec<_> = kernel
        .relations
        .iter()
        .map(|r| r.table.to_string())
        .collect();
    let column_names: Vec<_> = kernel
        .relations
        .iter()
        .map(|r| {
            let columns = r.columns.iter().map(|(name, _)| name.to_string());
            quote!(&[#(#columns),*])
        })
        .collect();
    let row_types: Vec<_> = cq
        .inputs()
        .map(|r| {
            let types = r.columns.iter().map(|c| &c.ty);
            quote!((#(#types,)*))
        })
        .collect();
    let mut input_types = Vec::new();
    let mut bindings = Vec::new();
    let mut sequential_bindings = Vec::new();
    for (index, relation) in cq.inputs().take(kernel.relations.len()).enumerate() {
        let field = fields[index];
        let table = &tables[index];
        let names = &column_names[index];
        let row = &row_types[index];
        if host {
            input_types.push(quote!(Vec<#row>));
            let columns: Vec<_> = relation
                .columns
                .iter()
                .enumerate()
                .map(|(index, _)| format_ident!("column_{index}"))
                .collect();
            // DECODE ?: typed host-input loading may reconstruct Date/Decimal/String; actual opaque-Rust demand is required per column.
            let reads = relation.columns.iter().zip(&columns).enumerate().map(|(column_index, (column, variable))| {
                let name = kernel.relations[index].columns[column_index].0.to_string();
                let ty = &column.ty;
                quote!(let mut #variable = <#ty as crate::manifest::HostValue>::read(manifest, #table, #name)?.into_iter();)
            });
            bindings.push(quote!({
                #(#reads)*
                (0..manifest.relations[#table].rows)
                    .map(|_| (#(#columns.next().expect("validated manifest row count"),)*))
                    .collect()
            }));
            sequential_bindings.push(quote!(let #field = &data.#field;));
        } else {
            input_types
                .push(quote!(cq_gpu_runtime::DeviceColumns<'static, #row, cq_gpu_runtime::For>));
            bindings.push(if storage_schema.is_some() {
                quote!(catalog.device_columns::<#row>(#table)?)
            } else {
                quote!(loader.bind::<#row>(#table, #names)?)
            });
            // DECODE ?: independent Sequential oracle downloads logical inputs; this diagnostic path is not final GPU answer publication.
            sequential_bindings.push(quote!(let #field = data.#field.to_host()?;));
        }
    }
    for (index, (name, _)) in kernel.derived.iter().enumerate() {
        let row = &row_types[kernel.relations.len() + index];
        let (initializer, dependencies) = kernel.derived_input(name).map_err(|error| Rejected {
            phase: "derived input loader",
            error: error.to_string(),
            term: cpu_source.to_string(),
        })?;
        let initializer = source::dictionary_initializer(initializer, &kernel.exports).map_err(|error| Rejected {
            phase: "dictionary input loader",
            error: error.to_string(),
            term: cpu_source.to_string(),
        })?.unwrap_or_else(|| initializer.to_token_stream());
        let host_inputs = dependencies.iter().map(|relation| {
            let input = &relation.name;
            if host {
                quote!(let #input = &#input;)
            } else {
                // DECODE ?: authored derived-input Rust consumes this download, but whole-row reconstruction needs column-level read evidence.
                quote!(let #input = #input.to_host()?;)
            }
        });
        input_types.push(quote!(Vec<#row>));
        bindings.push(quote!({ #(#host_inputs)* #initializer }));
        sequential_bindings.push(quote!(let #name = &data.#name;));
    }
    let answer_columns = kernel.answer_columns().map_err(|error| Rejected {
        phase: "loader output domains",
        error: error.to_string(),
        term: cpu_source.to_string(),
    })?;
    let mut answer_types = Vec::new();
    let mut decode_bindings = Vec::new();
    let mut borrowed_outputs = Vec::new();
    let mut owned_outputs = Vec::new();
    for (index, ((_, ty), column)) in kernel.answer.iter().zip(&answer_columns).enumerate() {
        let output = syn::Index::from(index);
        answer_types.push(if let Some(output) = cq.output() {
            &output.columns[index].ty
        } else {
            column
                .as_ref()
                .and_then(|column| stored_types.get(column))
                .unwrap_or(ty)
        });
        if let Some(domain) = column.as_ref().and_then(|column| domains.get(column)) {
            let decoder = format_ident!("domain_{index}");
            decode_bindings.push(quote!(let #decoder = data.catalog.domain(#domain)?;));
            // DECODE P: final answer formatting inverts dense source-key domains on the host.
            borrowed_outputs.push(quote!(#decoder.value(*row.#output).to_string()));
            owned_outputs.push(quote!(#decoder.value(row.#output).to_string()));
        } else if column.as_ref().is_some_and(|origin| stored_types.contains_key(origin)
            && manifest.column(&origin.0, &origin.1).is_ok_and(|metadata| metadata.encoding == "dictionary"))
        {
            let (table, column) = column
                .as_ref()
                .expect("encoded string output has an input domain");
            let decoder = format_ident!("dictionary_{index}");
            decode_bindings.push(
                quote!(let #decoder = manifest.dictionary(manifest.column(#table, #column)?)?;),
            );
            // DECODE P: final answer formatting resolves source string dictionaries on the host.
            borrowed_outputs.push(quote!(#decoder[*row.#output as usize].clone()));
            owned_outputs.push(quote!(#decoder[row.#output as usize].clone()));
        } else if publication.nullable[index] {
            borrowed_outputs.push(quote!(format!("{:?}", row.#output)));
            owned_outputs.push(quote!(format!("{:?}", row.#output)));
        } else {
            borrowed_outputs.push(quote!(row.#output.to_string()));
            owned_outputs.push(quote!(row.#output.to_string()));
        }
    }
    let (catalog_field, catalog_init, load_setup) = if storage_schema.is_some() {
        (
            quote!(catalog: gpu_compress::Catalog,),
            quote!(catalog,),
            quote! {
                let source = gpu_compress::Source::load_manifest(&manifest.root)?;
                let catalog = gpu_compress::Catalog::compress(&pool, &source, &gpu_compress_host::tpch::schema())?;
            },
        )
    } else {
        (
            quote!(),
            quote!(),
            quote!(let mut loader = crate::loader::Loader::new(manifest, pool.clone());),
        )
    };
    if let Some(publications) = publications {
        let names: Vec<_> = kernel.published.iter().map(|r| &r.name).collect();
        let rows: Vec<_> = kernel
            .published
            .iter()
            .map(|r| {
                let types = r.columns.iter().map(|(_, ty)| ty);
                quote!((#(#types,)*))
            })
            .collect();
        let inputs: Vec<_> = names
            .iter()
            .zip(&rows)
            .map(|(name, row)| {
                if publications[&name.to_string()].resident() {
                    quote!(cq_gpu_runtime::DeviceColumns<'static, #row>)
                } else {
                    quote!(Vec<#row>)
                }
            })
            .collect();
        let cpu_names: Vec<_> = (0..answer_types.len()).map(syn::Index::from).collect();
        let own_row = {
            let fields = answer_types.iter().zip(&cpu_names).map(|(ty, index)| {
                quote!(<#ty as ::core::clone::Clone>::clone(
                    ::core::borrow::Borrow::<#ty>::borrow(&row.#index)
                ))
            });
            quote!((#(#fields,)*))
        };
        // DECODE V: P6 staged host execution downloads entire logical input tuples before subsequent relational operators; retain codes outside rust_reads.
        let host_inputs: Vec<_> = names
            .iter()
            .filter(|name| publications[&name.to_string()].resident())
            .map(|name| quote!(let #name = #name.to_host()?;))
            .collect();
        let resident = publication.resident();
        let output_type = if resident {
            quote!(cq_gpu_runtime::DeviceColumns<'static, Answer>)
        } else {
            quote!(Vec<Answer>)
        };
        let execute = if host {
            quote! {
                let started = ::std::time::Instant::now();
                #(#sequential_bindings)*
                #(#host_inputs)*
                let result = (#gpu).map(|row| #own_row).collect();
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                cq_gpu_runtime::publish_timings(cq_gpu_runtime::Timings {
                    build_ms: elapsed, total_ms: elapsed, wall_ms: elapsed, ..Default::default()
                });
                Ok(result)
            }
        } else {
            let publish = if !resident {
                quote!({
                    let started = ::std::time::Instant::now();
                    // DECODE ?: nonresident stage output may feed later stages or final publication; terminal-use evidence determines R/P/V.
                    let result = output.to_host()?;
                    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                    let mut timing =
                        cq_gpu_runtime::take_last_timings().ok_or("stage published no timings")?;
                    timing.build_ms += elapsed;
                    timing.total_ms += elapsed;
                    timing.wall_ms += elapsed;
                    cq_gpu_runtime::publish_timings(timing);
                    result
                })
            } else if let Some(group) = &normal.query.group {
                let columns: Vec<_> = group
                    .keys
                    .iter()
                    .chain(group.aggregates.iter().map(|a| &a.name))
                    .collect();
                let slots: Vec<_> = normal
                    .query
                    .head
                    .args
                    .iter()
                    .map(|argument| {
                        let syn::Expr::Path(path) = argument else {
                            unreachable!("checked normalized group head");
                        };
                        columns
                            .iter()
                            .position(|name| path.path.is_ident(*name))
                            .expect("checked grouped output")
                    })
                    .collect();
                let bounds: Vec<_> = slots.iter().map(|&slot| {
                    if slot < group.keys.len() {
                        let key = columns[slot];
                        for item in &cq.query.body {
                            let cq_lang_cq::BodyItem::Positive { atom } = item else { continue; };
                            let Some(field) = atom.args.iter().position(|arg| matches!(arg,
                                syn::Expr::Path(path) if path.path.is_ident(key)
                            )) else { continue; };
                            let name = &atom.relation;
                            let published = publications.get(&name.to_string());
                            let value = if published.is_some() { quote!(#name) } else { quote!(&data.#name) };
                            let resident = published.map(Publication::resident).unwrap_or(true);
                            if resident {
                                return quote!((#value).storage().bounds[#field]);
                            }
                            let field = syn::Index::from(field);
                            return quote!((#value).iter().fold(cq_gpu_runtime::Bounds::Empty, |bounds, row| {
                                let value = gpu_datalog_runtime::encode_scalar(&row.#field);
                                match bounds {
                                    cq_gpu_runtime::Bounds::Empty => cq_gpu_runtime::Bounds::Inclusive { min: value, max: value },
                                    cq_gpu_runtime::Bounds::Inclusive { min, max } => cq_gpu_runtime::Bounds::Inclusive {
                                        min: min.min(value), max: max.max(value),
                                    },
                                }
                            }));
                        }
                    }
                    quote!(cq_gpu_runtime::Bounds::Inclusive { min: i64::MIN, max: i64::MAX })
                }).collect();
                quote!({
                    let started = ::std::time::Instant::now();
                    let start = data.pool.stream().record_event(Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
                    let result = unsafe { output.into_columns::<Answer>(&[#(#slots),*], &[#(#bounds),*])? };
                    let end = data.pool.stream().record_event(Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
                    data.pool.stream().synchronize()?;
                    let elapsed = start.elapsed_ms(&end)? as f64;
                    let mut timing = cq_gpu_runtime::take_last_timings().ok_or("group stage published no timings")?;
                    timing.dedup_ms += elapsed;
                    timing.total_ms += elapsed;
                    timing.wall_ms += started.elapsed().as_secs_f64() * 1000.0;
                    cq_gpu_runtime::publish_timings(timing);
                    result
                })
            } else {
                quote!(output)
            };
            quote! {
                #(let #fields = &data.#fields;)*
                let output = #gpu;
                Ok(#publish)
            }
        };
        let mut artifact = artifact;
        artifact["publication"] = json!(if resident { "device" } else { "host" });
        artifact["publication_facts"] = json!(publication);
        artifact["answer_types"] = json!(
            answer_types
                .iter()
                .map(|ty| ty.to_token_stream().to_string())
                .collect::<Vec<_>>()
        );
        return Ok((
            quote! {
                use cq_load::{Date, Load};
                type Decimal = cq_load::Decimal<2>;
                pub(super) type Answer = (#(#answer_types,)*);
                pub(super) type Output = #output_type;
                pub(super) struct Inputs { #(#fields: #input_types,)* #catalog_field pool: ::std::sync::Arc<cq_gpu_runtime::Pool> }
                pub(super) fn load(manifest: &crate::manifest::Manifest, pool: ::std::sync::Arc<cq_gpu_runtime::Pool>) -> crate::manifest::Result<Inputs> {
                    #load_setup
                    #(let #fields: #input_types = #bindings;)*
                    Ok(Inputs { #(#fields,)* #catalog_init pool })
                }
                pub(super) fn execute(data: &Inputs, #(#names: &#inputs),*) -> crate::manifest::Result<Output> {
                    #setup_bindings
                    #execute
                }
                pub(super) fn sequential(data: &Inputs, #(#names: &Vec<#rows>),*) -> crate::manifest::Result<Vec<Answer>> {
                    #setup_bindings
                    #(#sequential_bindings)*
                    Ok((#cpu).map(|row| #own_row).collect())
                }
                pub(super) fn decode(data: &Inputs, manifest: &crate::manifest::Manifest, rows: Vec<Answer>) -> crate::manifest::Result<Vec<Vec<String>>> {
                    #(#decode_bindings)*
                    Ok(rows.into_iter().map(|row| vec![#(#owned_outputs),*]).collect())
                }
            },
            artifact,
        ));
    }
    let execute = if host {
        let slots: Vec<_> = (0..answer_types.len()).map(syn::Index::from).collect();
        let owned = answer_types.iter().zip(slots).map(|(ty, slot)| quote!(
            <#ty as ::core::clone::Clone>::clone(::core::borrow::Borrow::<#ty>::borrow(&row.#slot))
        ));
        quote! {
            let started = ::std::time::Instant::now();
            #(#sequential_bindings)*
            let rows = (#gpu).map(|row| (#(#owned,)*)).collect();
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            cq_gpu_runtime::publish_timings(cq_gpu_runtime::Timings {
                build_ms: elapsed, total_ms: elapsed, wall_ms: elapsed, ..Default::default()
            });
            Ok(rows)
        }
    } else {
        quote! {
            #(let #fields = &data.#fields;)*
            let output = #gpu;
            // DECODE P: standalone query execution downloads its final answer for host formatting and comparison.
            Ok(output.to_host()?)
        }
    };
    Ok((
        quote! {
            type Answer = (#(#answer_types,)*);
            use cq_load::{Date, Load};
            type Decimal = cq_load::Decimal<2>;
            struct Inputs { #(#fields: #input_types,)* #catalog_field }
            fn load(manifest: &crate::manifest::Manifest) -> crate::manifest::Result<Inputs> {
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                #load_setup
                #(let #fields: #input_types = #bindings;)*
                Ok(Inputs { #(#fields,)* #catalog_init })
            }
            fn gpu(data: &Inputs) -> crate::manifest::Result<Vec<Answer>> {
                #setup_bindings
                #execute
            }
            fn sequential(data: &Inputs, manifest: &crate::manifest::Manifest) -> crate::manifest::Result<Vec<Vec<String>>> {
                #setup_bindings
                #(#decode_bindings)*
                #(#sequential_bindings)*
                Ok((#cpu).map(|row| vec![#(#borrowed_outputs),*]).collect())
            }
            fn download(data: &Inputs, manifest: &crate::manifest::Manifest) -> crate::manifest::Result<Vec<Vec<String>>> {
                #(#decode_bindings)*
                Ok(gpu(data)?.into_iter().map(|row| vec![#(#owned_outputs),*]).collect())
            }
            pub(super) fn run(manifest: &crate::manifest::Manifest, query: &str, conformance_only: bool) -> crate::manifest::Result<Value> {
                let data = load(manifest)?;
                let reference = if manifest.sf == 1 { Some(sequential(&data, manifest)?) } else { None };
                crate::benchmark::run(manifest, query, || download(&data, manifest), reference, conformance_only)
            }
        },
        artifact,
    ))
}

fn string_guard_regression(out: &Path) -> Result<TokenStream> {
    let directory = out.join("string-guard-fixture");
    fs::create_dir_all(&directory)?;
    for (name, values) in [("keys.bin", [1_i32, 2, 3]), ("names.bin", [1_i32, 0, 1])] {
        fs::write(
            directory.join(name),
            values
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
    }
    fs::write(directory.join("names.txt"), "red\ngreen part\n")?;
    let key = |name: &str| {
        json!({
            "name": name, "type": "i32", "logical_type": "Integer",
            "encoding": "plain", "file": "keys.bin"
        })
    };
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec(&json!({
            "schema_version": 2, "sf": 1, "byte_order": "little",
            "relations": {
                "keys": {"rows": 3, "columns": [key("key")]},
                "part": {"rows": 3, "columns": [key("p_partkey"), {
                    "name": "p_name", "type": "i32", "logical_type": "String",
                    "encoding": "dictionary", "file": "names.bin",
                    "dictionary": "names.txt", "dictionary_index_base": 0,
                    "dictionary_entries": 2
                }]}
            }
        }))?,
    )?;
    let manifest = Manifest::load(&directory)?;
    let kernel: source::Kernel = syn::parse_str(
        "$; setup {
            let GreenNames = cq_load::dictionary_where(
                &dictionaries, \"p_name\",
                |value: &str| value.contains(\"green\"));
        } relations {
            relation Keys => keys(key: i32);
            relation Part => part(p_partkey: i32, p_name: String);
        }
        derived { relation GreenNames(code: String); }
        answer(key: i32, name: String) :-
            Keys(key), Part(key, name), GreenNames(name), where name == \"green part\";",
    )?;
    let (code, _) = compile(&kernel, &manifest, &advice::Pipeline::baseline())
        .map_err(|error| format!("String guard regression {}: {}", error.phase, error.error))?;
    let directory = directory.to_string_lossy();
    Ok(quote! {
        #[cfg(test)]
        #[allow(non_snake_case, dead_code)]
        mod string_guard_regression {
            use serde_json::Value;
            #code
            #[test]
            fn synthetic_string_literal_guard_preserves_manifest_codes() {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory)).unwrap();
                let data = load(&manifest).unwrap();
                // DECODE P: diagnostic fixture publishes typed input rows to the host test oracle while preserving string codes.
                assert_eq!(data.Part.to_host().unwrap(), vec![(1, 1_u32), (2, 0), (3, 1)]);
                assert_eq!(data.GreenNames, vec![(1_u32,)]);
                let mut actual = download(&data, &manifest).unwrap();
                actual.sort();
                assert_eq!(actual, vec![
                    vec!["1".to_owned(), "green part".to_owned()],
                    vec!["3".to_owned(), "green part".to_owned()],
                ]);
                let mut reference = sequential(&data, &manifest).unwrap();
                reference.sort();
                assert_eq!(actual, reference);
            }
        }
    })
}

fn main() -> Result<()> {
    println!("cargo:rerun-if-env-changed=TPCH_GPU_DATA_ROOT");
    println!("cargo:rerun-if-env-changed=TPCH_GPU_ADVICE");
    println!("cargo:rerun-if-env-changed=CQ_GPU_TPCH_AUTOMATIC");
    println!("cargo:rerun-if-env-changed=CQ_GPU_TPCH_QUERIES");
    // An unset/empty filter retains the full suite. Comma-separated names
    // isolate compilation itself, unlike the runner's execution-only filter.
    let queries: std::collections::BTreeSet<String> = env::var("CQ_GPU_TPCH_QUERIES")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    for name in &queries {
        if !source::SOURCES.iter().any(|(query, _)| query == name) {
            return Err(format!("unknown build query: {name}").into());
        }
    }
    let automatic = env::var("CQ_GPU_TPCH_AUTOMATIC").is_ok_and(|value| value == "1");
    let overrides: std::collections::BTreeMap<String, advice::Pipeline> =
        if let Some(path) = env::var_os("TPCH_GPU_ADVICE") {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            std::collections::BTreeMap::new()
        };
    for name in overrides.keys() {
        if !source::SOURCES.iter().any(|(query, _)| query == name) {
            return Err(format!("unknown advised query: {name}").into());
        }
    }
    let root =
        PathBuf::from(env::var("TPCH_GPU_DATA_ROOT").unwrap_or_else(|_| "/data/tpch-gpu".into()));
    let mut generated = TokenStream::new();
    let mut dispatch = Vec::new();
    let mut reports = Vec::new();
    for sf in [1, 10] {
        let directory = root.join(format!("sf{sf}"));
        println!(
            "cargo:rerun-if-changed={}",
            directory.join("manifest.json").display()
        );
        let manifest = Manifest::load(&directory)?;
        if manifest.sf != sf {
            return Err("manifest scale differs from directory".into());
        }
        for relation in manifest.relations.values() {
            for column in &relation.columns {
                if let Some(dictionary) = &column.dictionary {
                    println!(
                        "cargo:rerun-if-changed={}",
                        directory.join(dictionary).display()
                    );
                }
            }
        }
        for &(name, source) in source::SOURCES {
            if !queries.is_empty() && !queries.contains(name) {
                continue;
            }
            println!(
                "cargo:rerun-if-changed=../../../features/workloads/src/queries/retail/tpch/retail_{name}.rs"
            );
            let staged = stages::Program::import(source)?;
            let kernel = if staged.is_none() {
                Some(source::Kernel::import(source)?)
            } else {
                None
            };
            for mode in ["automatic", "advised"] {
                if mode == "automatic" && !automatic {
                    continue;
                }
                let default_h05 =
                    mode == "advised" && name == "h05" && !overrides.contains_key(name);
                let hand = if mode == "advised" && !overrides.contains_key(name) {
                    advice::Pipeline::hand(name)
                } else {
                    None
                };
                let mut pipeline = if mode == "automatic" {
                    advice::Pipeline::automatic()
                } else if let Some(pipeline) = overrides.get(name) {
                    pipeline.clone()
                } else if default_h05 {
                    advice::Pipeline::h05()
                } else if let Some(hand) = hand {
                    hand
                } else if kernel.as_ref().is_some_and(source::Kernel::is_standalone) {
                    advice::Pipeline::grouped(&kernel.as_ref().unwrap().cpu_query())?
                } else {
                    // CPU hints remain available explicitly, but an unmeasured CPU
                    // choice must not replace the existing accepted GPU baseline.
                    advice::Pipeline::baseline()
                };
                // Storage is a loader decision shared by both competing schedules.
                if name == "h05" && pipeline.storage.is_empty() {
                    pipeline.storage = "tpch_domains".into();
                }
                let mut report = json!({"query": name, "sf": sf, "mode": mode, "cpu_query": kernel.as_ref().map(|k| k.cpu_query().to_string()), "cpu_plan_advice": kernel.as_ref().map(|k| k.plan_advice.to_string()), "pipeline": pipeline});
                let generation_started = std::time::Instant::now();
                let compiled = if let Some(staged) = &staged {
                    build_stages::compile(
                        staged,
                        name,
                        &manifest,
                        mode,
                        if mode == "advised" {
                            overrides.get(name)
                        } else {
                            None
                        },
                    )
                } else {
                    compile(kernel.as_ref().unwrap(), &manifest, &pipeline)
                };
                match compiled {
                    Ok((code, artifact)) => {
                        let module = format_ident!("{name}_sf{sf}_{mode}");
                        generated.extend(quote! { #[allow(non_snake_case)] mod #module { use serde_json::Value; #code } });
                        dispatch.push(quote! { (#name, #sf, #mode) => #module::run(manifest, query, _conformance_only).map(Some), });
                        report
                            .as_object_mut()
                            .unwrap()
                            .extend(artifact.as_object().unwrap().clone());
                        report["status"] = json!("accepted");
                    }
                    Err(rejected) => {
                        let status = if rejected.phase.contains("translate") {
                            "translate rejected"
                        } else {
                            "planner rejected"
                        };
                        println!(
                            "cargo:warning={name} SF{sf} {mode}: {status} ({}): {}",
                            rejected.phase, rejected.error
                        );
                        report["status"] = json!(status);
                        report["phase"] = json!(rejected.phase);
                        report["error"] = json!(rejected.error);
                        report["term"] = json!(rejected.term);
                    }
                }
                let generation_seconds = generation_started.elapsed().as_secs_f64();
                report["generation_seconds"] = json!(generation_seconds);
                println!("cargo:warning={name} SF{sf} {mode}: generation {generation_seconds:.3}s");
                reports.push(report);
            }
        }
    }
    let executor = if dispatch.is_empty() {
        quote! {
            pub fn execute(_query: &str, _manifest: &crate::manifest::Manifest, _mode: &str, _conformance_only: bool) -> crate::manifest::Result<Option<serde_json::Value>> {
                Ok(None)
            }
        }
    } else {
        quote! {
            pub fn execute(query: &str, manifest: &crate::manifest::Manifest, mode: &str, _conformance_only: bool) -> crate::manifest::Result<Option<serde_json::Value>> {
                match (query, manifest.sf, mode) { #(#dispatch)* _ => Ok(None) }
            }
        }
    };
    generated.extend(executor);
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
    generated.extend(string_guard_regression(&out)?);
    // Keep source-map lookups bounded by ordinary Rust line lengths, rather
    // than repeatedly scanning a multi-megabyte TokenStream display line.
    let generated: syn::File = syn::parse2(generated)?;
    fs::write(out.join("queries.rs"), prettyplease::unparse(&generated))?;
    fs::write(out.join("plans.json"), serde_json::to_vec_pretty(&reports)?)?;
    // Keep the same canonical path available to the runner, rather than silently
    // measuring a different fixture after constants were encoded at build time.
    println!(
        "cargo:rustc-env=TPCH_GPU_COMPILED_ROOT={}",
        fs::canonicalize(Path::new(&root))?.display()
    );
    Ok(())
}
