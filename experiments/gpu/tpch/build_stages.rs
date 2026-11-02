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
            let (_, producer_name) = stage.inputs.iter().find(|(name, _)| *name == input.name)
                .ok_or_else(|| reject("checked stage input binding missing".into()))?;
            if let Some(producer) = program.publication_index(producer_name).map_err(&reject)? {
                let columns = input.columns.iter().zip(&output_types[producer])
                    .map(|(column, ty)| (column.name.clone(), ty.clone())).collect();
                types.insert(name.clone(), columns);
                publications.insert(name, publication_facts[producer].clone());
                dependencies.push(format_ident!("result_{producer}"));
            } else {
                let source = program.input(producer_name).map_err(&reject)?;
                if !matches!(source.producer, stages::Producer::Table(_)) {
                    let columns = program.input_columns(source).map_err(&reject)?;
                    let width = columns.len();
                    types.insert(name.clone(), input.columns.iter().zip(columns)
                        .map(|(column, (_, ty))| (column.name.clone(), ty)).collect());
                    publications.insert(name, crate::Publication {
                        placement: advice::Placement::Host,
                        nullable: vec![false; width],
                        domains: program.input_origins(source).map_err(&reject)?,
                        grouped: None,
                    });
                    dependencies.push(stages::Program::prepared_name(source));
                }
            }
        }
        let mut kernel = program.kernel(index, &types).map_err(&reject)?;
        kernel.published_origins = publications.iter()
            .map(|(name, publication)| (name.clone(), publication.domains.clone()))
            .collect();
        kernel.evidence.facts.extend(advice::Pipeline::staged_keys(query, index));
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
        let (code, mut artifact) = compile_stage(
            &kernel,
            manifest,
            &pipeline,
            &publications,
        )?;
        artifact["stage"] = json!(index);
        artifact["cpu_query"] = json!(kernel.cpu_query().to_string());
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
        publication_facts.push(serde_json::from_value(artifact["publication_facts"].clone())
            .map_err(|error| reject(Box::new(error)))?);
        artifacts.push(artifact);
        let module = format_ident!("stage_{index}");
        let data = format_ident!("data_{index}");
        let result = format_ident!("result_{index}");
        modules.push(quote!(mod #module { #code }));
        loads.push(quote!(let #data = #module::load(manifest, pool.clone())?;));
        device.push(quote! {
            cq_gpu_runtime::take_last_timings();
            let #result = #module::execute(&#data, #(&#dependencies),*)?;
            let timing = cq_gpu_runtime::take_last_timings().ok_or("stage published no timings")?;
            total.upload_ms += timing.upload_ms;
            total.build_ms += timing.build_ms;
            total.kernel_ms += timing.kernel_ms;
            total.dedup_ms += timing.dedup_ms;
            total.total_ms += timing.total_ms;
            total.wall_ms += timing.wall_ms;
        });
        sequential.push(quote!(let #result = #module::sequential(&#data, #(&#dependencies),*)?;));
    }
    let mut device_terminal = Vec::new();
    let mut cpu_terminal = Vec::new();
    for (position, terminal) in program.terminal.iter().enumerate() {
        let index = program.stage_index(&terminal.stage).map_err(&reject)?;
        let module = format_ident!("stage_{index}");
        let data = format_ident!("data_{index}");
        let result = format_ident!("result_{index}");
        // DECODE P: only terminal stage results download logical tuples for final user-answer composition.
        let download = if publication_facts[index].resident() {
            quote!(#result.to_host()?)
        } else {
            quote!(#result)
        };
        check_terminal_domains(program, terminal, &publication_facts[index]).map_err(&reject)?;
        let composition = terminal_projection(terminal);
        let append = if position == 0 {
            if terminal.direct {
                quote!(let mut answer = #composition;)
            } else {
                quote!(let mut answer: Vec<Vec<String>> = (#composition).collect();)
            }
        } else {
            quote!(answer.extend(#composition);)
        };
        // DECODE P: terminal GPU answer resolves source dictionaries and formats values after download.
        device_terminal.push(quote! {
            let rows = #module::decode(&#data, manifest, #download)?;
            #append
        });
        // DECODE P: terminal CPU reference answer resolves the same source dictionaries for final comparison.
        cpu_terminal.push(quote! {
            let rows = #module::decode(&#data, manifest, #result)?;
            #append
        });
    }
    let preprocessing = program.preprocessing_tokens().map_err(&reject)?;
    let terminal_stages: Vec<_> = program.terminal.iter()
        .map(|terminal| program.stage_index(&terminal.stage)).collect::<crate::manifest::Result<_>>()
        .map_err(&reject)?;
    let terminal_direct: Vec<_> = program.terminal.iter().map(|terminal| terminal.direct).collect();
    let artifact = json!({"stages": artifacts, "terminal_stages": terminal_stages,
        "stage_source": program.to_token_stream().to_string(), "terminal_direct": terminal_direct,
        "timing": "sum of stage phase timings; host-graded stages use elapsed host execution"});
    Ok((
        quote! {
            use cq_load::{Date, Load, StringCode};
            type Decimal = cq_load::Decimal<2>;
            #(#modules)*
            pub(super) fn run(manifest: &crate::manifest::Manifest, query: &str, conformance_only: bool) -> crate::manifest::Result<serde_json::Value> {
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                #preprocessing
                #(#loads)*
                let reference = if manifest.sf == 1 {
                    #(#sequential)*
                    #(#cpu_terminal)*
                    Some(answer)
                } else { None };
                crate::benchmark::run(manifest, query, || {
                    let mut total = cq_gpu_runtime::Timings::default();
                    #(#device)*
                    #(#device_terminal)*
                    cq_gpu_runtime::publish_timings(total);
                    Ok(answer)
                }, reference, conformance_only)
            }
        },
        artifact,
    ))
}

fn check_terminal_domains(
    program: &stages::Program, terminal: &stages::Terminal, publication: &crate::Publication,
) -> crate::manifest::Result<()> {
    for (slot, projection) in terminal.projection.iter().enumerate() {
        let stages::Projection::Column(source) = projection else { continue; };
        let declared = terminal.dictionaries.iter().find(|(output, _)| *output == slot)
            .map(|(_, name)| program.domain(name).map(|domain| (domain.table.to_string(), domain.column.to_string())))
            .transpose()?;
        if publication.domains.get(*source).ok_or("terminal publication width mismatch")? != &declared {
            return Err(format!("terminal {} column {slot} dictionary publication differs from its recorded domain", terminal.stage).into());
        }
    }
    Ok(())
}

fn terminal_projection(terminal: &stages::Terminal) -> TokenStream {
    if terminal.direct { return quote!(rows); }
    let mut captures = Vec::new();
    let outputs = terminal.projection.iter().enumerate().map(|(index, projection)| match projection {
        stages::Projection::Column(slot) => quote!(::std::mem::take(&mut row[#slot])),
        stages::Projection::Value(expression) => {
            let name = format_ident!("__cq_terminal_value_{index}");
            captures.push(quote!(let #name = ({ #expression }).to_string();));
            quote!(#name.clone())
        }
    }).collect::<Vec<_>>();
    quote!({ #(#captures)* rows.into_iter().map(move |mut row| vec![#(#outputs),*]) })
}
