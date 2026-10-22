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
        error: error.to_string(),
        term: term.to_token_stream().to_string(),
    })
}

fn compile(kernel: &source::Kernel, manifest: &Manifest) -> CompileResult<TokenStream> {
    let cpu_source = kernel.cpu_query();
    let encoded = kernel.encoded_query(manifest).map_err(|error| Rejected {
        phase: "loader",
        error: error.to_string(),
        term: cpu_source.to_string(),
    })?;
    let cq: cq_lang_cq::Program = checked("CQ parse", &encoded, syn::parse2(encoded.clone()))?;
    let normal = checked("CQ normalize", &cq, cq_lang_cq::normal::normalize(&cq))?;
    let order = cq_order::Options {
        heuristic: cq_order::Heuristic::Written,
        split: cq_order::Split::Coarse,
    };
    let vo = checked("CQ order", &normal, cq_order::compile(&normal, &order))?;
    checked("CQ order check", &vo, cq_order::check(&normal, &vo, &order))?;
    let reductions = cq_reduce::Options {
        semijoins: cq_reduce::Reductions::Masks,
    };
    let reduced = checked("CQ reduce", &vo, cq_reduce::compile(&vo, &reductions))?;
    checked(
        "CQ reduce check",
        &reduced,
        cq_reduce::check(&vo, &reduced, &reductions),
    )?;
    let ra = checked("CQ elaborate", &reduced, cq_elaborate::compile(&reduced))?;
    checked(
        "CQ elaborate check",
        &ra,
        cq_elaborate::check(&reduced, &ra),
    )?;
    let ix = checked("index demands", &ra, cq_index_policy::compile(&ra))?;
    let physical = checked("Physical lowering", &ix, cq_physical_lowering::compile(&ix))?;
    let default_structures = checked(
        "structure selection",
        &physical,
        cq_structure_selection::compile(&physical),
    )?;
    let mut structures = checked(
        "catalog selection",
        &default_structures,
        cq_sequence_plan::select_from_catalog(&default_structures, &cq_gpu_sequence::catalog()),
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
    let planned = checked(
        "plan(machine)",
        &sequence,
        cq_sequence_plan::plan(
            &sequence,
            &cq_gpu_sequence::machine(),
            &cq_gpu_sequence::catalog(),
        ),
    )?;
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
    let cpu_sequence = checked(
        "Sequential lowering",
        &default_structures,
        cq_sequence_lowering::compile(&default_structures),
    )?;
    let cpu_plan = checked(
        "Sequential plan",
        &cpu_sequence,
        cq_sequence_plan::plan(
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

    let fields: Vec<_> = kernel.relations.iter().map(|r| &r.name).collect();
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
        .declarations
        .iter()
        .map(|r| {
            let types = r.columns.iter().map(|c| &c.ty);
            quote!((#(#types,)*))
        })
        .collect();
    let answer_types = kernel.answer.iter().map(|(_, ty)| ty);
    let outputs: Vec<_> = (0..kernel.answer.len()).map(syn::Index::from).collect();
    let borrowed_outputs = outputs.iter().map(|index| quote!(*row.#index as i64));
    let owned_outputs = outputs.iter().map(|index| quote!(row.#index as i64));
    Ok(quote! {
        type Answer = (#(#answer_types,)*);
        struct Inputs { #(#fields: cq_gpu_runtime::DeviceColumns<'static, #row_types, cq_gpu_runtime::For>,)* }
        fn load(manifest: &crate::manifest::Manifest) -> crate::manifest::Result<Inputs> {
            let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
            let mut loader = crate::loader::Loader::new(manifest, pool);
            Ok(Inputs { #(#fields: loader.bind::<#row_types>(#tables, #column_names)?,)* })
        }
        fn gpu(data: &Inputs) -> cq_gpu_runtime::DeviceColumns<'_, Answer> {
            #(let #fields = &data.#fields;)*
            #gpu
        }
        fn sequential(data: &Inputs) -> crate::manifest::Result<Vec<Vec<i64>>> {
            #(let #fields = data.#fields.to_host()?;)*
            Ok((#cpu).map(|row| vec![#(#borrowed_outputs),*]).collect())
        }
        fn download(data: &Inputs) -> crate::manifest::Result<Vec<Vec<i64>>> {
            Ok(gpu(data).to_host()?.into_iter().map(|row| vec![#(#owned_outputs),*]).collect())
        }
        pub(super) fn run(manifest: &crate::manifest::Manifest, query: &str, conformance_only: bool) -> crate::manifest::Result<Value> {
            let data = load(manifest)?;
            let reference = if manifest.sf == 1 { Some(sequential(&data)?) } else { None };
            crate::benchmark::run(manifest, query, || download(&data), reference, conformance_only)
        }
    })
}

fn main() -> Result<()> {
    println!("cargo:rerun-if-env-changed=TPCH_GPU_DATA_ROOT");
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
            println!(
                "cargo:rerun-if-changed=../../../features/workloads/src/queries/retail/tpch/retail_{name}.rs"
            );
            let kernel = source::Kernel::import(source)?;
            let mut report =
                json!({"query": name, "sf": sf, "cpu_query": kernel.cpu_query().to_string()});
            match compile(&kernel, &manifest) {
                Ok(code) => {
                    let module = format_ident!("{name}_sf{sf}");
                    generated.extend(quote! { #[allow(non_snake_case)] mod #module { use serde_json::Value; #code } });
                    dispatch.push(quote! { (#name, #sf) => #module::run(manifest, query, _conformance_only).map(Some), });
                    report["status"] = json!("accepted");
                }
                Err(rejected) => {
                    let status = if rejected.phase.contains("translate") {
                        "translate rejected"
                    } else {
                        "planner rejected"
                    };
                    println!(
                        "cargo:warning={name} SF{sf}: {status} ({}): {}",
                        rejected.phase, rejected.error
                    );
                    report["status"] = json!(status);
                    report["phase"] = json!(rejected.phase);
                    report["error"] = json!(rejected.error);
                    report["term"] = json!(rejected.term);
                }
            }
            reports.push(report);
        }
    }
    let executor = if dispatch.is_empty() {
        quote! {
            pub fn execute(_query: &str, _manifest: &crate::manifest::Manifest, _conformance_only: bool) -> crate::manifest::Result<Option<serde_json::Value>> {
                Ok(None)
            }
        }
    } else {
        quote! {
            pub fn execute(query: &str, manifest: &crate::manifest::Manifest, _conformance_only: bool) -> crate::manifest::Result<Option<serde_json::Value>> {
                match (query, manifest.sf) { #(#dispatch)* _ => Ok(None) }
            }
        }
    };
    generated.extend(executor);
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
    fs::write(out.join("queries.rs"), generated.to_string())?;
    fs::write(out.join("plans.json"), serde_json::to_vec_pretty(&reports)?)?;
    // Keep the same canonical path available to the runner, rather than silently
    // measuring a different fixture after constants were encoded at build time.
    println!(
        "cargo:rustc-env=TPCH_GPU_COMPILED_ROOT={}",
        fs::canonicalize(Path::new(&root))?.display()
    );
    Ok(())
}
