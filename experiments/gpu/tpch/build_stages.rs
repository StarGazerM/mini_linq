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
            if let Some(&producer) = program.aliases.get(&name) {
                if producer >= index {
                    return Err(Rejected {
                        phase: "stage dependency",
                        error: format!("{name} is not a preceding publication"),
                        term: stage.program.to_token_stream().to_string(),
                    });
                }
                let columns = input
                    .columns
                    .iter()
                    .zip(&output_types[producer])
                    .map(|(column, ty)| (column.name.clone(), ty.clone()))
                    .collect();
                types.insert(name.clone(), columns);
                publications.insert(name, publication_facts[producer].clone());
                dependencies.push(format_ident!("result_{producer}"));
            } else if let Some(preprocess) =
                program.preprocessing.iter().find(|p| p.name == input.name)
            {
                types.insert(name.clone(), preprocess.columns.clone());
                publications.insert(name, crate::Publication {
                    placement: advice::Placement::Host,
                    nullable: vec![false; preprocess.columns.len()],
                });
                dependencies.push(preprocess.name.clone());
            }
        }
        let kernel = program.kernel(index, &types).map_err(&reject)?;
        let pipeline = if let Some(pipeline) = override_pipeline {
            pipeline.clone()
        } else if mode == "automatic" {
            advice::Pipeline::automatic()
        } else {
            advice::Pipeline::staged(query, index)
        };
        let (code, mut artifact) = compile_stage(
            &kernel,
            manifest,
            &pipeline,
            Some(&publications),
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
    for &index in &program.terminal {
        let module = format_ident!("stage_{index}");
        let data = format_ident!("data_{index}");
        let result = format_ident!("result_{index}");
        let download = if publication_facts[index].resident() {
            quote!(#result.to_host()?)
        } else {
            quote!(#result)
        };
        let composition =
            terminal_map(&program.stages[index], output_types[index].len()).map_err(&reject)?;
        device_terminal.push(quote! {
            let rows = #module::decode(&#data, manifest, #download)?;
            answer.extend(#composition);
        });
        cpu_terminal.push(quote! {
            let rows = #module::decode(&#data, manifest, #result)?;
            answer.extend(#composition);
        });
    }
    let preprocessing = program.preprocessing_tokens();
    let artifact = json!({"stages": artifacts, "terminal_stages": program.terminal,
        "timing": "sum of stage phase timings; host-graded stages use elapsed host execution"});
    Ok((
        quote! {
            #(#modules)*
            pub(super) fn run(manifest: &crate::manifest::Manifest, query: &str, conformance_only: bool) -> crate::manifest::Result<serde_json::Value> {
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                #preprocessing
                #(#loads)*
                let reference = if manifest.sf == 1 {
                    #(#sequential)*
                    let mut answer = Vec::new();
                    #(#cpu_terminal)*
                    Some(answer)
                } else { None };
                crate::benchmark::run(manifest, query, || {
                    let mut total = cq_gpu_runtime::Timings::default();
                    #(#device)*
                    let mut answer = Vec::new();
                    #(#device_terminal)*
                    cq_gpu_runtime::publish_timings(total);
                    Ok(answer)
                }, reference, conformance_only)
            }
        },
        artifact,
    ))
}

fn terminal_map(stage: &stages::Stage, width: usize) -> crate::manifest::Result<TokenStream> {
    let Some(map) = &stage.map else {
        return Ok(quote!(rows));
    };
    if stages::check_identity_map(map).is_ok() {
        return Ok(quote!(rows));
    }
    let map = stages::owned_map(map);
    let body = if let syn::Expr::Block(block) = map.body.as_ref() {
        match block.block.stmts.last() {
            Some(syn::Stmt::Expr(expression, None)) => expression,
            _ => return Err("terminal ownership map must return a tuple".into()),
        }
    } else {
        map.body.as_ref()
    };
    let syn::Expr::Tuple(tuple) = body else {
        return Err("terminal ownership map must return a tuple".into());
    };
    let Some(syn::Pat::Tuple(pattern)) = map.inputs.first() else {
        return Err("terminal ownership map must destructure its row".into());
    };
    if pattern.elems.len() != width {
        return Err("terminal ownership map input width differs".into());
    }
    let mut outputs = Vec::new();
    let mut used = std::collections::BTreeSet::new();
    for expression in &tuple.elems {
        match expression {
            syn::Expr::Path(path) => {
                let slot = pattern
                    .elems
                    .iter()
                    .position(|pat| {
                        matches!(pat,
                            syn::Pat::Ident(name) if path.path.is_ident(&name.ident)
                        )
                    })
                    .ok_or("terminal map references an unbound field")?;
                if !used.insert(slot) {
                    return Err("terminal map repeats a field".into());
                }
                outputs.push(quote!(::std::mem::take(&mut row[#slot])));
            }
            syn::Expr::Lit(literal) => outputs.push(quote!((#literal).to_string())),
            _ => {
                return Err(
                    "terminal map must be an ownership projection or literal extension".into(),
                );
            }
        }
    }
    Ok(quote!(rows.into_iter().map(|mut row| vec![#(#outputs),*])))
}
