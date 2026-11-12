use crate::manifest::Manifest;
use crate::{CompileResult, Rejected, advice, compile_stage, stages};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use serde_json::json;
use std::collections::BTreeMap;

/// Formal rule DR-Target: Compile the selected input target once; rejection never changes it.
pub fn compile(
    program: &stages::Program,
    query: &str,
    manifest: &Manifest,
    mode: &str,
    override_pipeline: Option<&advice::Pipeline>,
) -> CompileResult<(TokenStream, serde_json::Value)> {
    let reject = |error: Box<dyn std::error::Error>| Rejected {
        phase: "stage import",
        error: error.to_string(),
        term: String::new(),
    };
    let mut modules = Vec::new();
    let mut loads = Vec::new();
    let mut device = Vec::new();
    let mut sequential = Vec::new();
    let mut artifacts = Vec::new();
    let mut output_types: Vec<Vec<syn::Type>> = Vec::new();
    let mut publication_facts: Vec<crate::Publication> = Vec::new();
    let mut host_uploads = BTreeMap::new();
    let mut host_references = BTreeMap::new();
    for (index, stage) in program.stages.iter().enumerate() {
        let mut types = BTreeMap::new();
        let mut publications = BTreeMap::new();
        let mut dependencies = Vec::new();
        let mut sequential_dependencies = Vec::new();
        for input in stage.program.inputs() {
            let name = input.name.to_string();
            let (_, producer_name) = stage
                .inputs
                .iter()
                .find(|(name, _)| *name == input.name)
                .ok_or_else(|| reject("checked stage input binding missing".into()))?;
            if let Some(producer) = program.publication_index(producer_name).map_err(&reject)? {
                let columns = input
                    .columns
                    .iter()
                    .zip(&output_types[producer])
                    .map(|(column, ty)| (column.name.clone(), ty.clone()))
                    .collect();
                types.insert(name.clone(), columns);
                publications.insert(name, publication_facts[producer].clone());
                dependencies.push(format_ident!("result_{producer}"));
                sequential_dependencies.push(format_ident!("result_{producer}"));
            } else {
                let source = program.input(producer_name).map_err(&reject)?;
                if !matches!(source.producer, stages::Producer::Table(_)) {
                    let mut columns = program.input_columns(source).map_err(&reject)?;
                    let width = columns.len();
                    let mut origins = program.input_origins(source).map_err(&reject)?;
                    let mut key_fields = Vec::new();
                    let raw_types: Vec<_> = columns.iter().map(|(_, ty)| ty.clone()).collect();
                    for (slot, ((column, ty), origin)) in columns.iter_mut().zip(&mut origins).enumerate() {
                        if let Some((table, field)) = crate::key_domains::host_origin(&source.name.to_string(), &column.to_string()) {
                            let id = crate::key_domains::id(table, field).expect("static host key membership");
                            *origin = Some((table.to_owned(), field.to_owned()));
                            *ty = syn::parse_quote!(u32);
                            let slot = syn::Index::from(slot);
                            key_fields.push(quote!((::std::mem::offset_of!((#(#raw_types,)*), #slot), #id)));
                        }
                    }
                    let prepared = stages::Program::prepared_name(source);
                    let normalized = !key_fields.is_empty();
                    if normalized {
                        if key_fields.len() != width {
                            return Err(reject("host key upload requires a fully recorded integer tuple".into()));
                        }
                        let device_name = format_ident!("__cq_device_{}", source.name);
                        let reference_name = format_ident!("__cq_reference_{}", source.name);
                        let code_types: Vec<_> = columns.iter().map(|(_, ty)| ty).collect();
                        host_uploads.insert(source.name.to_string(), quote! {
                            let #device_name = loader.upload_host_keys::<(#(#raw_types,)*), (#(#code_types,)*)>(
                                &#prepared, &[#(#key_fields),*],
                            )?;
                        });
                        host_references.insert(source.name.to_string(), quote!(let #reference_name = #device_name.to_host()?;));
                        dependencies.push(device_name);
                        sequential_dependencies.push(reference_name);
                    } else {
                        dependencies.push(prepared.clone());
                        sequential_dependencies.push(prepared);
                    }
                    types.insert(
                        name.clone(),
                        input
                            .columns
                            .iter()
                            .zip(columns)
                            .map(|(column, (_, ty))| (column.name.clone(), ty))
                            .collect(),
                    );
                    publications.insert(
                        name,
                        crate::Publication {
                            placement: if normalized { advice::Placement::Device } else { advice::Placement::Host },
                            nullable: vec![false; width],
                            domains: origins,
                            grouped: None,
                        },
                    );
                }
            }
        }
        let mut kernel = program.kernel(index, &types).map_err(&reject)?;
        kernel.published_origins = publications
            .iter()
            .map(|(name, publication)| (name.clone(), publication.domains.clone()))
            .collect();
        let mut pipeline = if let Some(pipeline) = override_pipeline {
            pipeline.clone()
        } else if mode == "automatic" {
            advice::Pipeline::automatic()
        } else {
            advice::Pipeline::staged(query, index)
        };
        if override_pipeline.is_none() {
            pipeline.publication = advice::Placement::Automatic;
        }
        let (code, mut artifact) = compile_stage(&kernel, manifest, &pipeline, &publications)?;
        artifact["stage"] = json!(index);
        artifact["cpu_query"] = json!(kernel.cpu_query().to_string());
        artifact["final_download"] = json!(stage.final_download);
        artifact["dependencies"] = json!(
            kernel
                .published
                .iter()
                .map(|r| r.name.to_string())
                .collect::<Vec<_>>()
        );
        output_types.push(
            artifact["answer_types"]
                .as_array()
                .unwrap()
                .iter()
                .map(|ty| syn::parse_str(ty.as_str().unwrap()))
                .collect::<syn::Result<_>>()
                .map_err(|error| reject(Box::new(error)))?,
        );
        publication_facts.push(
            serde_json::from_value(artifact["publication_facts"].clone())
                .map_err(|error| reject(Box::new(error)))?,
        );
        artifacts.push(artifact);
        let module = format_ident!("stage_{index}");
        let data = format_ident!("data_{index}");
        let result = format_ident!("result_{index}");
        modules.push(quote!(mod #module { #code }));
        loads.push(quote!(let #data = #module::load(manifest, pool.clone(), &mut loader)?;));
        let final_download = stage.final_download;
        device.push(quote! {
            cq_gpu_runtime::take_last_timings();
            let #result = #module::execute(&#data, #(&#dependencies),*)?;
            let timing = cq_gpu_runtime::take_last_timings().ok_or("stage published no timings")?;
            measurement.add_stage(timing, #final_download);
        });
        sequential.push(quote!(let #result = #module::sequential(&#data, #(&#sequential_dependencies),*)?;));
    }
    let mut device_terminal = Vec::new();
    let mut cpu_terminal = Vec::new();
    for (position, terminal) in program.terminal.iter().enumerate() {
        let index = program.stage_index(&terminal.stage).map_err(&reject)?;
        let result = format_ident!("result_{index}");
        // DECODE P: terminal stage results download native answer tuples; source dictionary resolution is deferred until validation selects rows.
        let download = if publication_facts[index].resident() {
            quote!(measurement.final_download(|| #result.to_host())?)
        } else {
            quote!(#result)
        };
        check_terminal_domains(program, terminal, &publication_facts[index]).map_err(&reject)?;
        let composition = terminal_projection(terminal);
        let append = if position == 0 {
            if terminal.direct {
                quote!(let mut answer = #composition;)
            } else {
                quote!(let mut answer: Vec<_> = (#composition).collect();)
            }
        } else {
            quote!(answer.extend(#composition);)
        };
        device_terminal.push(quote! {
            let rows = #download;
            #append
        });
        cpu_terminal.push(quote! {
            let rows = #result;
            #append
        });
    }
    let first = &program.terminal[0];
    let first_index = program.stage_index(&first.stage).map_err(&reject)?;
    let first_module = format_ident!("stage_{first_index}");
    let first_data = format_ident!("data_{first_index}");
    let formatted_fields: Vec<_> = first.projection.iter().enumerate().map(|(position, projection)| {
        let output = syn::Index::from(position);
        match projection {
            stages::Projection::Column(slot) => {
                let source = syn::Index::from(*slot);
                quote!((decoders.#source)(&row.#output))
            }
            stages::Projection::Value(_) => quote!(row.#output.to_string()),
        }
    }).collect();
    let preprocessing = program.preprocessing_tokens().map_err(&reject)?;
    let mut bound_columns: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for input in &program.inputs {
        if let stages::Producer::Table(table) = &input.producer {
            let columns = bound_columns.entry(table.to_string()).or_default();
            for column in input.columns.as_ref().expect("table input schema") {
                columns.insert(column.name.to_string());
            }
        }
    }
    let bound_projections: Vec<_> = bound_columns.iter().map(|(table, columns)| {
        let columns: Vec<_> = columns.iter().collect();
        quote!((#table, &[#(#columns),*][..]))
    }).collect();
    let host_uploads: Vec<_> = host_uploads.into_values().collect();
    let host_references: Vec<_> = host_references.into_values().collect();
    let terminal_stages: Vec<_> = program
        .terminal
        .iter()
        .map(|terminal| program.stage_index(&terminal.stage))
        .collect::<crate::manifest::Result<_>>()
        .map_err(&reject)?;
    let terminal_direct: Vec<_> = program
        .terminal
        .iter()
        .map(|terminal| terminal.direct)
        .collect();
    let timing = if host_uploads.is_empty() {
        "sum of stage phase timings; host-graded stages use elapsed host execution"
    } else {
        "sum of stage phase timings; host-graded stages use elapsed host execution; NOTE: host-built input upload/normalization is load-side (host_input_upload_ms, separate from base domain_ms), unlike base e4e08c99 where host-built input upload occurred in execute; authored host preprocessing remains load-side; indexes remain timed"
    };
    let artifact = json!({"stages": artifacts, "terminal_stages": terminal_stages,
        "stage_source": program.to_token_stream().to_string(), "terminal_direct": terminal_direct,
        "timing": timing});
    Ok((
        quote! {
            use cq_load::{Date, Load, StringCode};
            type Decimal = cq_load::Decimal<2>;
            #(#modules)*
            // Keep each query in its module's codegen unit, not the dispatcher.
            #[inline(never)]
            pub(super) fn run(manifest: &crate::manifest::Manifest, query: &str, conformance_only: bool) -> crate::manifest::Result<serde_json::Value> {
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                let mut loader = crate::loader::Loader::new(manifest, pool.clone());
                loader.prepare(&[#(#bound_projections),*])?;
                #preprocessing
                #(#host_uploads)*
                #(#loads)*
                drop(loader);
                pool.release_unused()?;
                crate::loader::finish_load(&pool);
                let reference_started = ::std::time::Instant::now();
                let decoders = #first_module::decoders(&#first_data, manifest)?;
                let reference = if manifest.sf == 1 {
                    #(#host_references)*
                    #(#sequential)*
                    #(#cpu_terminal)*
                    // DECODE P: the complete terminal CPU reference resolves source dictionaries once before the benchmark loop.
                    Some(answer.iter().map(|row| vec![#(#formatted_fields),*]).collect())
                } else { None };
                let reference_seconds = reference_started.elapsed().as_secs_f64();
                crate::benchmark::run(manifest, query, #mode, || {
                    let mut measurement = crate::benchmark::Measurement::default();
                    #(#device)*
                    #(#device_terminal)*
                    cq_gpu_runtime::publish_timings(measurement.timings);
                    Ok((answer, measurement.boundary))
                // DECODE P: terminal GPU answer rows resolve source dictionaries only when full or sampled validation requests their values.
                }, |row| vec![#(#formatted_fields),*], reference, reference_seconds, conformance_only)
            }
        },
        artifact,
    ))
}

fn check_terminal_domains(
    program: &stages::Program,
    terminal: &stages::Terminal,
    publication: &crate::Publication,
) -> crate::manifest::Result<()> {
    for (slot, projection) in terminal.projection.iter().enumerate() {
        let stages::Projection::Column(source) = projection else {
            continue;
        };
        let declared = terminal
            .dictionaries
            .iter()
            .find(|(output, _)| *output == slot)
            .map(|(_, name)| {
                program
                    .domain(name)
                    .map(|domain| (domain.table.to_string(), domain.column.to_string()))
            })
            .transpose()?;
        if declared.is_none() && publication.domains.get(*source).and_then(Option::as_ref)
            .is_some_and(|(table, column)| crate::key_domains::id(table, column).is_some())
        {
            continue;
        }
        if publication
            .domains
            .get(*source)
            .ok_or("terminal publication width mismatch")?
            != &declared
        {
            return Err(format!(
                "terminal {} column {slot} dictionary publication differs from its recorded domain",
                terminal.stage
            )
            .into());
        }
    }
    Ok(())
}

fn terminal_projection(terminal: &stages::Terminal) -> TokenStream {
    if terminal.direct {
        return quote!(rows);
    }
    let outputs = terminal
        .projection
        .iter()
        .map(|projection| match projection {
            stages::Projection::Column(slot) => {
                let slot = syn::Index::from(*slot);
                quote!(row.#slot)
            }
            stages::Projection::Value(expression) => quote!({ #expression }),
        })
        .collect::<Vec<_>>();
    quote!(rows.into_iter().map(|row| (#(#outputs,)*)))
}
