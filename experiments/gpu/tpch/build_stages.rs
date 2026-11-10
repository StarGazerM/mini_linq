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
    for (index, stage) in program.stages.iter().enumerate() {
        let mut types = BTreeMap::new();
        let mut publications = BTreeMap::new();
        let mut dependencies = Vec::new();
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
            } else {
                let source = program.input(producer_name).map_err(&reject)?;
                if !matches!(source.producer, stages::Producer::Table(_)) {
                    let columns = program.input_columns(source).map_err(&reject)?;
                    let width = columns.len();
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
                            placement: advice::Placement::Host,
                            nullable: vec![false; width],
                            domains: program.input_origins(source).map_err(&reject)?,
                            grouped: None,
                        },
                    );
                    dependencies.push(stages::Program::prepared_name(source));
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
        loads.push(quote!(let #data = #module::load(manifest, pool.clone())?;));
        let final_download = stage.final_download;
        device.push(quote! {
            cq_gpu_runtime::take_last_timings();
            let #result = #module::execute(&#data, #(&#dependencies),*)?;
            let timing = cq_gpu_runtime::take_last_timings().ok_or("stage published no timings")?;
            measurement.add_stage(timing, #final_download);
        });
        sequential.push(quote!(let #result = #module::sequential(&#data, #(&#dependencies),*)?;));
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
    let artifact = json!({"stages": artifacts, "terminal_stages": terminal_stages,
        "stage_source": program.to_token_stream().to_string(), "terminal_direct": terminal_direct,
        "timing": "sum of stage phase timings; host-graded stages use elapsed host execution"});
    Ok((
        quote! {
            use cq_load::{Date, Load, StringCode};
            type Decimal = cq_load::Decimal<2>;
            #(#modules)*
            // Keep each query in its module's codegen unit, not the dispatcher.
            #[inline(never)]
            pub(super) fn run(manifest: &crate::manifest::Manifest, query: &str, conformance_only: bool) -> crate::manifest::Result<serde_json::Value> {
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                #preprocessing
                #(#loads)*
                let reference_started = ::std::time::Instant::now();
                let decoders = #first_module::decoders(&#first_data, manifest)?;
                let reference = if manifest.sf == 1 {
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
