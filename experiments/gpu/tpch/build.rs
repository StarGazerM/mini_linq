#[path = "src/advice.rs"]
mod advice;
#[path = "src/manifest.rs"]
#[allow(dead_code)]
mod manifest;
#[path = "src/source.rs"]
mod source;

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
    profile: &str,
) -> CompileResult<(TokenStream, serde_json::Value)> {
    let cpu_source = kernel.cpu_query();
    // Registration passes a query name only for an authored hand profile.
    let hand_profile = !matches!(profile, "automatic" | "advised" | "h05");
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
    let mut stored_types = std::collections::BTreeMap::new();
    let mut domains = std::collections::BTreeMap::new();
    if pipeline.storage == "tpch_dictionary" {
        // Authored constant-only string queries use the manifest's existing
        // column-local i32 dictionaries. The source adapter resolves literals;
        // this does not change predicates or teach the GPU to execute Rust.
        for relation in &kernel.relations {
            for (column, logical) in &relation.columns {
                if logical.to_token_stream().to_string() == "String" {
                    stored_types.insert(
                        (relation.table.to_string(), column.to_string()),
                        syn::parse_quote!(i32),
                    );
                }
            }
        }
    }
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
    let encoded = kernel
        .encoded_query_with_types(manifest, &stored_types)
        .map_err(|error| Rejected {
            phase: "loader",
            error: error.to_string(),
            term: cpu_source.to_string(),
        })?;
    let cq: cq_lang_cq::Program = checked("CQ parse", &encoded, syn::parse2(encoded.clone()))?;
    let normal = checked("CQ normalize", &cq, cq_lang_cq::normal::normalize(&cq))?;
    let reduced = checked(
        "CQ plan advice",
        &normal,
        advice::valuation(&normal, &pipeline.plan),
    )?;
    let ra = checked("CQ elaborate", &reduced, cq_elaborate::compile(&reduced))?;
    checked(
        "CQ elaborate check",
        &ra,
        cq_elaborate::check(&reduced, &ra),
    )?;
    let ix = checked("index demands", &ra, cq_index_policy::compile(&ra))?;
    let mut pipeline = pipeline.clone();
    let physical = checked("Physical lowering", &ix, cq_physical_lowering::compile(&ix))?;
    if profile == "h05" {
        let prefix = checked(
            "H05 retained prefix",
            &physical,
            advice::joined_prefix(&physical, "Customer"),
        )?;
        pipeline
            .physical
            .insert(0, quote!(retain_streams(#prefix)).to_string());
        pipeline.physical.push(advice::hoist_inputs(&physical));
    }
    if profile == "h09" {
        let selected = checked(
            "filtered Part layout",
            &physical,
            cq_structure_selection::compile(&physical),
        )?;
        if let Some(leaf) = checked(
            "filtered Part retention",
            &selected,
            advice::filtered_leaf(&selected, "Part"),
        )? {
            pipeline
                .physical
                .insert(0, quote!(retain_streams(#leaf)).to_string());
        }
    }
    if hand_profile {
        checked(
            "hand Physical choices",
            &physical,
            advice::hand_physical(&physical, profile, &mut pipeline),
        )?;
    }
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
    if profile == "h05" {
        pipeline.structures.insert(
            0,
            checked(
                "H05 provider layout",
                &physical,
                advice::provider_layout(&physical, "Supplier"),
            )?,
        );
    }
    if hand_profile && profile != "h04" {
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
    let mut structures = checked(
        "catalog selection",
        &selected,
        cq_sequence_plan::select_from_catalog(&selected, &cq_gpu_sequence::catalog()),
    )?;
    let facts = checked(
        "Structures check",
        &structures,
        cq_lang_structures::contract::check(&structures),
    )?;
    // The loader, not the planner or translator, owns the input representation.
    // Record its uniform DeviceColumns adapter with the checked retype rewrite.
    for index in 0..structures.structures.entries.len() {
        let entry = &structures.structures.entries[index];
        if matches!(entry.structure, cq_lang_structures::Structure::Host { .. }) {
            let host = &facts.physical.hosts[&entry.name.to_string()];
            let relation = physical
                .program
                .declarations
                .iter()
                .find(|r| r.name == host.as_str())
                .unwrap();
            if kernel
                .derived
                .iter()
                .any(|(name, _)| *name == relation.name)
            {
                continue;
            }
            if relation
                .columns
                .iter()
                .any(|column| column.ty.to_token_stream().to_string() == "String")
            {
                continue;
            }
            let types = relation.columns.iter().map(|c| &c.ty);
            structures = checked(
                "loader host adapter",
                &structures,
                cq_rewrite_passes::retype(
                    &structures,
                    quote!(entry(#index) => ::cq_gpu_runtime::DeviceColumns<'static, (#(#types,)*), ::cq_gpu_runtime::For>),
                ),
            )?;
        }
    }
    let sequence = checked(
        "Sequence lowering",
        &structures,
        cq_sequence_lowering::compile(&structures),
    )?;
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
    let mut authored = checked(
        "Sequence advice",
        &sequence,
        advice::apply_sequence(&sequence, &pipeline.sequence),
    )?;
    let planned = checked(
        "plan(machine)",
        &authored.program,
        if pipeline.complete {
            Ok(cq_sequence_plan::Planned {
                program: authored.program.clone(),
                trace: Vec::new(),
            })
        } else if profile == "h05" {
            advice::h05_schedule(&authored.program)
        } else if hand_profile {
            advice::hand_schedule(&authored.program, profile)
        } else if profile == "automatic" {
            cq_sequence_plan::plan(
                &authored.program,
                &cq_gpu_sequence::machine(),
                &cq_gpu_sequence::catalog(),
            )
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
    let artifact = json!({
        "pipeline": pipeline,
        "sequence_input": sequence.to_token_stream().to_string(),
        "sequence_trace": trace,
        "sequence_planned": planned.program.to_token_stream().to_string(),
        "cost": checked("PL-Cost", &planned.program,
            cq_sequence_plan::cost(&planned.program, &cq_gpu_sequence::machine()))?.to_string(),
    });
    let analysis = checked(
        "machine_grade",
        &planned.program,
        cq_lang_sequence::check_grading(&planned.program, &cq_gpu_sequence::machine()),
    )?;
    let gpu = checked(
        "translate",
        &planned.program,
        cq_gpu_sequence::translate(&planned.program, &analysis),
    )?;

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
        if relation
            .columns
            .iter()
            .any(|column| column.ty.to_token_stream().to_string() == "String")
        {
            input_types.push(quote!(Vec<#row>));
            let columns: Vec<_> = relation
                .columns
                .iter()
                .enumerate()
                .map(|(index, _)| format_ident!("column_{index}"))
                .collect();
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
        let host_inputs = dependencies.iter().map(|relation| {
            let input = &relation.name;
            if relation
                .columns
                .iter()
                .any(|(_, ty)| ty.to_token_stream().to_string() == "String")
            {
                quote!(let #input = &#input;)
            } else {
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
            borrowed_outputs.push(quote!(#decoder.value(*row.#output).to_string()));
            owned_outputs.push(quote!(#decoder.value(row.#output).to_string()));
        } else if kernel.is_standalone()
            && ty.to_token_stream().to_string() == "String"
            && answer_types.last().unwrap().to_token_stream().to_string() != "String"
        {
            let (table, column) = column
                .as_ref()
                .expect("encoded string output has an input domain");
            let decoder = format_ident!("dictionary_{index}");
            decode_bindings.push(
                quote!(let #decoder = manifest.dictionary(manifest.column(#table, #column)?)?;),
            );
            borrowed_outputs.push(quote!(#decoder[*row.#output as usize].clone()));
            owned_outputs.push(quote!(#decoder[row.#output as usize].clone()));
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
            quote!(let mut loader = crate::loader::Loader::new(manifest, pool);),
        )
    };
    Ok((
        quote! {
            type Answer = (#(#answer_types,)*);
            struct Inputs { #(#fields: #input_types,)* #catalog_field }
            fn load(manifest: &crate::manifest::Manifest) -> crate::manifest::Result<Inputs> {
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                #load_setup
                #(let #fields: #input_types = #bindings;)*
                Ok(Inputs { #(#fields,)* #catalog_init })
            }
            fn gpu(data: &Inputs) -> crate::manifest::Result<Vec<Answer>> {
                #(let #fields = &data.#fields;)*
                let output = #gpu;
                Ok(output.to_host()?)
            }
            fn sequential(data: &Inputs, manifest: &crate::manifest::Manifest) -> crate::manifest::Result<Vec<Vec<String>>> {
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
        "$; setup {} relations {
            relation Keys => keys(key: i32);
            relation Part => part(p_partkey: i32, p_name: String);
        }
        answer(key: i32) :- Keys(key), Part(key, name), if name == \"green part\";",
    )?;
    let (code, _) = compile(&kernel, &manifest, &advice::Pipeline::baseline(), "advised")
        .map_err(|error| format!("String guard regression {}: {}", error.phase, error.error))?;
    let directory = directory.to_string_lossy();
    Ok(quote! {
        #[cfg(test)]
        #[allow(non_snake_case, dead_code)]
        mod string_guard_regression {
            use serde_json::Value;
            #code
            #[test]
            fn synthetic_string_literal_guard_executes_on_host() {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory)).unwrap();
                let data = load(&manifest).unwrap();
                assert_eq!(data.Part, vec![
                    (1, "green part".to_owned()), (2, "red".to_owned()), (3, "green part".to_owned())
                ]);
                let mut actual = download(&data, &manifest).unwrap();
                actual.sort();
                assert_eq!(actual, vec![vec!["1".to_owned()], vec!["3".to_owned()]]);
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
            if let Some((_, reason)) = source::UNSUPPORTED.iter().find(|(query, _)| query == name) {
                return Err(format!("unsupported build query {name}: {reason}").into());
            }
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
    let unsupported: Vec<_> = source::UNSUPPORTED.iter().map(|(query, reason)| {
        json!({"query": query, "status": "unsupported staged source", "reason": reason})
    }).collect();
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
            let kernel = source::Kernel::import(source)?;
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
                let hand_profile = hand.is_some();
                let mut pipeline = if mode == "automatic" {
                    advice::Pipeline::baseline()
                } else if let Some(pipeline) = overrides.get(name) {
                    pipeline.clone()
                } else if default_h05 {
                    advice::Pipeline::h05()
                } else if let Some(hand) = hand {
                    hand
                } else if kernel.is_standalone() {
                    advice::Pipeline::grouped(&kernel.cpu_query())?
                } else {
                    // CPU hints remain available explicitly, but an unmeasured CPU
                    // choice must not replace the existing accepted GPU baseline.
                    advice::Pipeline::baseline()
                };
                // Storage is a loader decision shared by both competing schedules.
                if name == "h05" && pipeline.storage.is_empty() {
                    pipeline.storage = "tpch_domains".into();
                }
                let mut report = json!({"query": name, "sf": sf, "mode": mode, "cpu_query": kernel.cpu_query().to_string(), "cpu_plan_advice": kernel.plan_advice.to_string(), "pipeline": pipeline});
                let profile = if default_h05 {
                    "h05"
                } else if hand_profile {
                    name
                } else {
                    mode
                };
                let generation_started = std::time::Instant::now();
                match compile(&kernel, &manifest, &pipeline, profile) {
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
    fs::write(
        out.join("unsupported.json"),
        serde_json::to_vec_pretty(&unsupported)?,
    )?;
    // Keep the same canonical path available to the runner, rather than silently
    // measuring a different fixture after constants were encoded at build time.
    println!(
        "cargo:rustc-env=TPCH_GPU_COMPILED_ROOT={}",
        fs::canonicalize(Path::new(&root))?.display()
    );
    Ok(())
}
