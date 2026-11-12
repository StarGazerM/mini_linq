#[path = "src/advice.rs"]
mod advice;
mod build_stages;
#[path = "src/key_domains.rs"]
mod key_domains;
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
    /// Resident grouped output conversion, recorded with the placement.
    #[serde(default)]
    grouped: Option<GroupedPublication>,
}

/// Formal rule DR-GroupedPublication: a resident grouped answer converts the
/// grouped machine rows into answer columns by slots checked against each head
/// argument; input bounds bind the key and match recorded input placement.
/// Aggregates and keys without an input binding take the full i64 domain.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct GroupedPublication {
    slots: Vec<usize>,
    bounds: Vec<KeyBounds>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
enum KeyBounds {
    /// Field `field` of the positive body atom over `relation`: resident
    /// column metadata, or one host scan of a host relation.
    Input {
        relation: String,
        field: usize,
        resident: bool,
    },
    Full,
}

/// Select the grouped conversion at DR-Publication: slots follow the
/// normalized head over keys then aggregates; a key's bounds come from the
/// first positive atom binding it; every input must have recorded placement.
fn grouped_publication(
    source: &cq_lang_cq::Program,
    normal: &cq_lang_cq::Program,
    inputs: &std::collections::BTreeMap<String, advice::Placement>,
) -> syn::Result<Option<GroupedPublication>> {
    let Some(group) = normal.query.group.as_ref() else {
        return Ok(None);
    };
    for item in &source.query.body {
        if let cq_lang_cq::BodyItem::Positive { atom } = item
            && !inputs.contains_key(&atom.relation.to_string())
        {
            return Err(syn::Error::new_spanned(
                &atom.relation,
                "DR-GroupedPublication: missing recorded input placement",
            ));
        }
    }
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
    let bounds = slots
        .iter()
        .map(|&slot| {
            if slot >= group.keys.len() {
                return KeyBounds::Full;
            }
            let key = columns[slot];
            source
                .query
                .body
                .iter()
                .find_map(|item| {
                    let cq_lang_cq::BodyItem::Positive { atom } = item else {
                        return None;
                    };
                    let field = atom.args.iter().position(|arg| {
                        matches!(arg,
                        syn::Expr::Path(path) if path.path.is_ident(key))
                    })?;
                    let relation = atom.relation.to_string();
                    let resident = inputs[&relation] == advice::Placement::Device;
                    Some(KeyBounds::Input {
                        relation,
                        field,
                        resident,
                    })
                })
                .unwrap_or(KeyBounds::Full)
        })
        .collect();
    Ok(Some(GroupedPublication { slots, bounds }))
}

/// Validate a recorded grouped conversion against the checked programs.
fn check_grouped(
    source: &cq_lang_cq::Program,
    normal: &cq_lang_cq::Program,
    grouped: &GroupedPublication,
    inputs: &std::collections::BTreeMap<String, advice::Placement>,
) -> syn::Result<()> {
    let fail = |message: &str| {
        Err(syn::Error::new_spanned(
            normal,
            format!("DR-GroupedPublication: {message}"),
        ))
    };
    let Some(group) = &normal.query.group else {
        return fail("ungrouped answer has a grouped conversion");
    };
    let width = group.keys.len() + group.aggregates.len();
    if grouped.slots.len() != normal.query.head.args.len()
        || grouped.bounds.len() != grouped.slots.len()
        || grouped.slots.iter().any(|slot| *slot >= width)
    {
        return fail("slots do not cover the grouped head");
    }
    for ((slot, bounds), argument) in grouped
        .slots
        .iter()
        .zip(&grouped.bounds)
        .zip(&normal.query.head.args)
    {
        let name = if let Some(key) = group.keys.get(*slot) {
            key
        } else {
            &group.aggregates[*slot - group.keys.len()].name
        };
        if !matches!(argument, syn::Expr::Path(path) if path.path.is_ident(name)) {
            return fail("slot does not denote its grouped head argument");
        }
        let KeyBounds::Input {
            relation,
            field,
            resident,
        } = bounds
        else {
            continue;
        };
        let Some(key) = group.keys.get(*slot) else {
            return fail("aggregate column takes input bounds");
        };
        let Some(placement) = inputs.get(relation) else {
            return fail("input bounds require a recorded input placement");
        };
        if *resident != (*placement == advice::Placement::Device) {
            return fail("key bounds residency differs from recorded input placement");
        }
        let bound = source.query.body.iter().any(|item| {
            matches!(item,
            cq_lang_cq::BodyItem::Positive { atom } if atom.relation == relation.as_str()
                && atom.args.get(*field).is_some_and(|arg| matches!(arg,
                    syn::Expr::Path(path) if path.path.is_ident(key))))
        });
        if !bound {
            return fail("key bounds name an atom field that does not bind the key");
        }
    }
    Ok(())
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
        let cq_lang_cq::BodyItem::Positive { atom } = item else {
            continue;
        };
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
            if matches!(
                aggregate.kind,
                cq_syntax::algebra::AggregateKind::Min | cq_syntax::algebra::AggregateKind::Max
            ) {
                nullable.insert(aggregate.name.to_string());
            }
        }
    }
    let nullable: Vec<_> = program
        .query
        .head
        .args
        .iter()
        .map(|argument| {
            matches!(argument, syn::Expr::Path(path)
            if nullable.contains(&path.path.to_token_stream().to_string()))
        })
        .collect();
    let placement = pipeline
        .publication
        .resolve(pipeline.target, nullable.iter().any(|value| *value));
    if placement == advice::Placement::Device
        && (matches!(pipeline.target, advice::Target::Host) || nullable.iter().any(|value| *value))
    {
        return Err(syn::Error::new_spanned(
            program,
            "device publication requires a device target and nonnullable columns; select host publication",
        ));
    }
    Ok(Publication {
        placement,
        nullable,
        domains: Vec::new(),
        grouped: None,
    })
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


fn compile_stage(
    kernel: &source::Kernel,
    manifest: &Manifest,
    pipeline: &advice::Pipeline,
    publications: &std::collections::BTreeMap<String, Publication>,
) -> CompileResult<(TokenStream, serde_json::Value)> {
    let cpu_source = kernel.cpu_query();
    let host = matches!(pipeline.target, advice::Target::Host);
    match pipeline.storage.as_str() {
        "" | "tpch_dictionary" => {}
        other => {
            return Err(Rejected {
                phase: "loader profile",
                error: format!("unknown storage profile {other}"),
                term: cpu_source.to_string(),
            });
        }
    };
    let mut stored_types = kernel
        .scalar_codec_types(manifest)
        .map_err(|error| Rejected {
            phase: "loader representation",
            error: error.to_string(),
            term: cpu_source.to_string(),
        })?;
    let integer_types = kernel.integer_key_types();
    let domains: std::collections::BTreeMap<_, _> = integer_types.keys()
        .map(|origin| (origin.clone(), key_domains::id(&origin.0, &origin.1).unwrap() as usize))
        .collect();
    stored_types.extend(integer_types);
    // Dictionary IDs are an input representation, independent of the schedule
    // or whether the authored query contains opaque Rust reads.
    stored_types.extend(
        kernel
            .dictionary_types(manifest)
            .map_err(|error| Rejected {
                phase: "loader representation",
                error: error.to_string(),
                term: cpu_source.to_string(),
            })?,
    );
    let encoded = kernel
        .encoded_query_with_types(manifest, &stored_types)
        .map_err(|error| Rejected {
            phase: "loader",
            error: error.to_string(),
            term: cpu_source.to_string(),
        })?;
    let mut cq: cq_lang_cq::Program = checked("CQ parse", &encoded, syn::parse2(encoded.clone()))?;
    if pipeline.recipe.automatic && !pipeline.complete {
        let path = manifest.root.join("manifest.json");
        println!("cargo:rerun-if-changed={}", path.display());
        // Stage files explicitly bind base relations to tables and columns.
        // Published intermediates are not base-table evidence subjects.
        let bindings = kernel.relations.iter().map(|relation| {
            use cq_syntax::declared_evidence::{ColumnBinding, EvidenceView, RelationBinding};
            RelationBinding {
                relation: relation.name.clone(),
                source: relation.table.to_string(),
                view: EvidenceView::Full,
                columns: relation.columns.iter().map(|(column, _)| ColumnBinding {
                    column: column.clone(), source: column.to_string(),
                }).collect(),
            }
        }).collect();
        cq.declared_evidence = checked(
            "TPC-H declared evidence",
            &cq,
            cq_workload_authored::tpch_evidence::load_with_bindings(&path, &cq.declarations, bindings),
        )?;
    }
    let normal = checked("CQ normalize", &cq, cq_lang_cq::normal::normalize(&cq))?;
    let captures = &normal.query.captures;
    let mut setup_bindings = kernel
        .setup_bindings(manifest, &quote!(#captures))
        .map_err(|error| Rejected {
            phase: "loader captures",
            error: error.to_string(),
            term: cpu_source.to_string(),
        })?;
    let domain_handles = domains.values().copied().collect::<std::collections::BTreeSet<_>>()
        .into_iter().map(|id| cq_gpu_sequence::CaptureHandle {
            name: format_ident!("__tpch_domain_{id}"),
            host_type: syn::parse_quote!(&gpu_device_columns::Domain),
            host_value: syn::parse_quote!(data.domains[#id].as_ref().ok_or("missing loaded key domain")?),
            device_type: syn::parse_quote!(gpu_device_columns::DomainView),
        }).collect();
    let mut publication = checked(
        "publication contract",
        &normal,
        publication(&normal, pipeline, Some(publications)),
    )?;
    if publication.resident() {
        // Table loaders use the selected target; staged inputs carry their
        // producer's publication placement. Record both before selecting bounds.
        let input_placements: std::collections::BTreeMap<_, _> = kernel
            .relations
            .iter()
            .map(|relation| {
                (
                    relation.name.to_string(),
                    if host {
                        advice::Placement::Host
                    } else {
                        advice::Placement::Device
                    },
                )
            })
            .chain(
                publications
                    .iter()
                    .map(|(name, input)| (name.clone(), input.placement)),
            )
            .collect();
        publication.grouped = checked(
            "grouped publication",
            &normal,
            grouped_publication(&cq, &normal, &input_placements),
        )?;
        if let Some(grouped) = &publication.grouped {
            checked(
                "grouped publication",
                &normal,
                check_grouped(&cq, &normal, grouped, &input_placements),
            )?;
        }
    }
    publication.domains = kernel
        .answer_columns()
        .map_err(|error| Rejected {
            phase: "publication domains",
            error: error.to_string(),
            term: cpu_source.to_string(),
        })?
        .into_iter()
        .map(|origin| {
            origin.filter(|(table, column)| {
                key_domains::id(table, column).is_some() || manifest
                    .column(table, column)
                    .is_ok_and(|metadata| metadata.encoding == "dictionary")
            })
        })
        .collect();
    // Automatic routes carry checked manifest evidence on CQ; authored recipes
    // retain their plan arguments and bypass evidence selection as before.
    let (normal, reduced) = checked("CQ plan advice", &normal, advice::valuation(&cq, &pipeline.plan))?;
    let (ra, steps) = checked(
        "CQ relational lowering",
        &reduced,
        cq_relational_lowering::compile(&normal, &reduced),
    )?;
    checked(
        "CQ relational lowering check",
        &ra,
        cq_relational_lowering::check(&normal, &reduced, &ra, &steps),
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
            advice::filtered_leaf(&physical, &selected, relation),
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
            advice::hand_membership(&physical, &default_structures),
        )?);
    }
    let selected = checked(
        "Structures advice",
        &default_structures,
        advice::structures(&physical, &default_structures, &pipeline.structures),
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
    // Catalog selection returns a matched (Physical, Structures) pair; keep the
    // source pair too, since nullable host selection below starts from it.
    let source_physical = physical;
    let (physical, mut structures) = checked(
        "catalog selection",
        &selected,
        cq_sequence_plan::select_from_catalog(&source_physical, &selected, &catalog),
    )?;
    let facts = checked(
        "Structures check",
        &structures,
        cq_lang_structures::contract::check(&physical, &structures),
    )?;
    // Nullable publication payloads have no device scalar ABI. A join carrying
    // one from either side needs a host-readable index, until a later projection
    // removes that column. Use checked layouts, never Rust type spellings.
    if !host
        && publications
            .values()
            .any(|input| input.nullable.iter().any(|value| *value))
    {
        let nullable = nullable_inputs(&normal, Some(publications));
        let (_, host_structures) = checked(
            "nullable host provider selection",
            &selected,
            cq_sequence_plan::select_from_catalog(&source_physical, &selected, &cq_sequence_pull::catalog()),
        )?;
        for index in 0..structures.structures.entries.len() {
            let entry = &structures.structures.entries[index];
            let name = entry.name.to_string();
            let nullable_reader = facts
                .physical
                .keyed_reads
                .get(&name)
                .into_iter()
                .flatten()
                .map(|(reader, _, _)| reader)
                .chain(
                    facts
                        .physical
                        .merge_reads
                        .get(&name)
                        .into_iter()
                        .flatten()
                        .map(|(reader, _)| reader),
                )
                .any(|reader| {
                    facts.layouts.get(reader).is_some_and(|columns| {
                        columns.iter().any(|column| nullable.contains(column))
                    }) && physical.program.definitions.iter().any(|definition| {
                        definition.name == reader.as_str()
                            && matches!(
                                definition.operator,
                                cq_lang_physical::Operator::Probe(..)
                                    | cq_lang_physical::Operator::Merge(..)
                            )
                    })
                });
            if matches!(entry.structure, cq_lang_structures::Structure::Built { .. })
                && (nullable_reader
                    || advice::relation_of(&physical, &entry.name)
                        .and_then(|relation| publications.get(&relation.to_string()))
                        .is_some_and(|input| input.nullable.iter().any(|value| *value)))
            {
                let cq_lang_structures::Structure::Built { keyed, .. } =
                    &host_structures.structures.entries[index].structure
                else {
                    unreachable!("catalog selection preserves provider entries");
                };
                let provider = &keyed.provider;
                structures = checked(
                    "nullable host provider",
                    &structures,
                    cq_structure_selection::retype(&physical, &structures, quote!(entry(#index) => #provider)),
                )?;
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
                .get(&relation.name.to_string())
                .is_some_and(|input| !input.resident())
            {
                continue;
            }
            let types = relation.columns.iter().map(|c| &c.ty);
            let input = cq_lang_structures::provider::InputSchema {
                key_count: 0,
                columns: relation
                    .columns
                    .iter()
                    .map(|column| cq_lang_structures::provider::InputColumn {
                        logical: column.ty.clone(),
                        logical_type_id: column.logical_type_id,
                        storage: column.ty.clone(),
                        codec_id: None,
                        codec: None,
                    })
                    .collect(),
            };
            let device_columns = cq_gpu_sequence::providers::declaration("device_columns");
            structures = checked(
                "loader host adapter",
                &structures,
                cq_structure_selection::retype(
                    &physical,
                    &structures,
                    if kernel
                        .published
                        .iter()
                        .any(|input| input.name == relation.name)
                    {
                        quote!(providers { #device_columns } entry(#index) => @device_columns #input as ::cq_gpu_runtime::DeviceColumns<'static, (#(#types,)*)>)
                    } else {
                        quote!(providers { #device_columns } entry(#index) => @device_columns #input as ::cq_gpu_runtime::DeviceColumns<'static, (#(#types,)*), ::cq_gpu_runtime::For>)
                    },
                ),
            )?;
        }
    }
    if !host {
        structures = checked(
            "target provider binding",
            &structures,
            cq_gpu_sequence::bind_structures(&physical, &structures),
        )?;
    }
    let mut sequence = checked(
        "Sequence lowering",
        &structures,
        cq_sequence_lowering::compile(&physical, &structures),
    )?;
    // Loader/provider facts, never host type spellings, justify device Rust.
    let origins = kernel.variable_origins().map_err(|error| Rejected {
        phase: "loader representation",
        error: error.to_string(),
        term: cpu_source.to_string(),
    })?;
    for (variable, origin) in origins {
        let metadata = manifest
            .column(&origin.0, &origin.1)
            .map_err(|error| Rejected {
                phase: "loader representation",
                error: error.to_string(),
                term: cpu_source.to_string(),
            })?;
        if domains.contains_key(&origin) {
            // The adapted declaration is a code. Its Rust reads explicitly
            // invert the recorded domain, so the input carrier is identity.
            sequence.representation.identity.insert(variable);
        } else if matches!(
            metadata.encoding.as_str(),
            "scaled_integer" | "days_since_epoch"
        ) && stored_types.contains_key(&origin)
        {
            sequence.representation.encoded.insert(variable);
        } else {
            sequence.representation.identity.insert(variable);
        }
    }
    {
        for item in &normal.query.body {
            let cq_lang_cq::BodyItem::Positive { atom } = item else {
                continue;
            };
            if publications
                .get(&atom.relation.to_string())
                .is_some_and(Publication::resident)
            {
                for argument in &atom.args {
                    if let syn::Expr::Path(path) = argument
                        && let Some(name) = path.path.get_ident()
                    {
                        if !sequence.representation.encoded.contains(&name.to_string()) {
                            sequence.representation.identity.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }
    if !host {
        sequence = checked(
            "target representation binding",
            &sequence,
            cq_gpu_sequence::record_representation(&physical, &structures, &sequence),
        )?;
    }
    checked(
        "Sequence lowering check",
        &sequence,
        cq_sequence_lowering::check(&physical, &structures, &sequence),
    )?;
    if !pipeline.complete {
        let evidence = cq_lang_sequence::Evidence {
            facts: kernel
                .relations
                .iter()
                .map(|relation| cq_lang_sequence::EvidenceFact::Count {
                    source: cq_lang_sequence::EvidenceSource::Authored {
                        provenance: syn::LitStr::new(
                            &format!(
                                "{} row counts",
                                manifest.root.join("manifest.json").display()
                            ),
                            proc_macro2::Span::call_site(),
                        ),
                    },
                    subject: relation.name.clone(),
                    rows: manifest.relations[&relation.table.to_string()].rows,
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
        advice::apply_sequence(&physical, &structures, &sequence, &pipeline.sequence),
    )?;
    let planned = checked(
        "decompose(machine)",
        &authored.program,
        if pipeline.complete {
            Ok(cq_sequence_plan::Planned {
                physical: authored.physical.clone(),
                structures: authored.structures.clone(),
                program: authored.program.clone(),
                trace: Vec::new(),
            })
        } else if host {
            cq_sequence_plan::decompose(&authored.physical, &authored.structures, &authored.program, &machine, &catalog)
        } else if let Some(schedule) = &recipe.schedule {
            advice::hand_schedule(&authored.physical, &authored.structures, &authored.program, schedule)
        } else {
            cq_sequence_plan::decompose(
                &authored.physical,
                &authored.structures,
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
    let mut artifact = json!({
        "pipeline": pipeline,
        "recipe_input": recipe,
        "publication_facts": publication,
        "physical_input": physical.to_token_stream().to_string(),
        "structures_input": structures.to_token_stream().to_string(),
        "physical_planned": planned.physical.to_token_stream().to_string(),
        "structures_planned": planned.structures.to_token_stream().to_string(),
        "sequence_input": sequence.to_token_stream().to_string(),
        "sequence_trace": trace,
        "sequence_planned": planned.program.to_token_stream().to_string(),
        "cost": checked("PL-Cost", &planned.program,
            cq_sequence_plan::cost(&planned.physical, &planned.structures, &planned.program, &machine))?.to_string(),
    });
    let analysis = checked(
        "machine_grade",
        &planned.program,
        cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine),
    )?;
    let device_captures = checked(
        "device handle bindings", &planned.program,
        cq_gpu_sequence::DeviceCaptureBindings::bind(
            &analysis.gpu_bindings.realization.captures.captures, domain_handles,
        ),
    )?;
    setup_bindings.extend(checked("host handle bindings", &planned.program, device_captures.host_setup())?);
    let host_setup_bindings = setup_bindings.clone();
    let computed_inputs = kernel.computed_publication_inputs().map_err(|error| Rejected {
        phase: "publication origin check", error: error.to_string(), term: cpu_source.to_string(),
    })?;
    artifact["computed_publication_inputs"] = json!(computed_inputs);
    let publication_origins = checked(
        "publication field origins", &planned.structures,
        cq_lang_structures::contract::check(&planned.physical, &planned.structures),
    )?;
    let computed_field = |column: &str| {
        let mut renamed = false;
        for (result, mapping) in &publication_origins.physical.renames {
            let Some(source) = mapping.get(column) else { continue; };
            renamed = true;
            let host = &publication_origins.physical.hosts[result];
            let Some(input) = planned.physical.inputs().find(|input| input.name == host.as_str()) else {
                return false;
            };
            let Some(slot) = input.columns.iter().position(|field| field.name == source.as_str()) else {
                return false;
            };
            if !computed_inputs.contains(&(host.clone(), slot)) { return false; }
        }
        renamed || publication_origins.physical.rows.values().any(|row| row.computed.contains(column))
    };
    let mut computed_fields: std::collections::BTreeMap<String, std::collections::BTreeSet<&str>> =
        std::collections::BTreeMap::new();
    for record in analysis.gpu_publications.values() {
        match &record.storage {
            cq_lang_sequence::gpu::publication::PublicationStorage::FlatRows(rows) => {
                for column in &rows.columns {
                    if computed_field(column) {
                        computed_fields.entry(column.clone()).or_default().insert("payload");
                    }
                }
            }
            cq_lang_sequence::gpu::publication::PublicationStorage::PackedKeys(keys) => {
                for field in keys.codec.fields() {
                    if computed_field(&field.column) {
                        computed_fields.entry(field.column.clone()).or_default().insert("canonical identity");
                    }
                }
            }
        }
    }
    for definition in &planned.physical.program.definitions {
        if let cq_lang_structures::physical::Operator::Group(group) = &definition.operator {
            for key in &group.keys {
                let column = cq_lang_structures::physical::symbol_name(key);
                if computed_field(&column) {
                    computed_fields.entry(column).or_default().insert("group key");
                }
            }
        }
    }
    artifact["computed_fields"] = json!(computed_fields.into_iter().map(|(column, uses)| {
        let ty = analysis.gpu_bindings.carriers[&column].to_token_stream().to_string();
        json!({"field": column, "type": ty, "uses": uses})
    }).collect::<Vec<_>>());
    // The checked GPU publication records (storage layout, union staging, set
    // build, arena), printed as recorded.
    artifact["gpu_publications"] = json!(analysis
        .gpu_publications
        .iter()
        .map(|(name, record)| (name.clone(), record.to_token_stream().to_string()))
        .collect::<std::collections::BTreeMap<_, _>>());
    let gpu = if host {
        checked(
            "host translate",
            &planned.program,
            cq_sequence_pull::translate(&planned.physical, &planned.structures, &planned.program, &analysis),
        )?
    } else {
        checked(
            "translate",
            &planned.program,
            cq_gpu_sequence::translate_with_origins(&planned.physical, &planned.structures, &planned.program, &analysis, &computed_inputs, &device_captures),
        )?
    };

    // Independently select/lower Sequential from the identical Physical. No GPU
    // schedule, provider, partition, or canonicalization advice is copied over.
    // Lookup-first can reexpress Physical; referencing the pre-plan graph would
    // restore the discarded Cartesian prefixes instead of testing this plan.
    let cpu_structures = checked(
        "Sequential structure selection",
        &planned.physical,
        cq_structure_selection::compile(&planned.physical),
    )?;
    let cpu_sequence = checked(
        "Sequential lowering",
        &cpu_structures,
        cq_sequence_lowering::compile(&planned.physical, &cpu_structures),
    )?;
    let cpu_plan = checked(
        "Sequential plan",
        &cpu_sequence,
        cq_sequence_plan::decompose(
            &planned.physical,
            &cpu_structures,
            &cpu_sequence,
            &cq_sequence_pull::machine(),
            &cq_sequence_pull::catalog(),
        ),
    )?;
    let cpu_analysis = checked(
        "Sequential grade",
        &cpu_plan.program,
        cq_lang_sequence::check_grading(&cpu_plan.physical, &cpu_plan.structures, &cpu_plan.program, &cq_sequence_pull::machine()),
    )?;
    let cpu = checked(
        "Sequential translate",
        &cpu_plan.program,
        cq_sequence_pull::translate(&cpu_plan.physical, &cpu_plan.structures, &cpu_plan.program, &cpu_analysis),
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
            // DECODE V: P6 host-stage inputs retain stored key codes; other scalar fields follow the existing typed host publication boundary.
            bindings.push(quote!(loader.bind::<#row>(#table, #names)?.to_host()?));
            sequential_bindings.push(quote!(let #field = &data.#field;));
        } else {
            input_types
                .push(quote!(cq_gpu_runtime::DeviceColumns<'static, #row, cq_gpu_runtime::For>));
            bindings.push(quote!(loader.bind::<#row>(#table, #names)?));
            // DECODE ?: independent Sequential oracle downloads logical inputs; this diagnostic path is not final GPU answer publication.
            sequential_bindings.push(quote!(let #field = data.#field.to_host()?;));
        }
    }
    let answer_columns = kernel.answer_columns().map_err(|error| Rejected {
        phase: "loader output domains",
        error: error.to_string(),
        term: cpu_source.to_string(),
    })?;
    let mut answer_types = Vec::new();
    let mut decode_bindings = Vec::new();
    let mut owned_outputs = Vec::new();
    for (index, ((_, ty), column)) in kernel.answer.iter().zip(&answer_columns).enumerate() {
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
            decode_bindings.push(quote!(let #decoder = data.domains[#domain].as_ref().ok_or("missing loaded key domain")?;));
            // DECODE P: final answer formatting restores keys through their recorded Offset/Rank inverse.
            owned_outputs.push(quote!(#decoder.value(*value).to_string()));
        } else if column.as_ref().is_some_and(|origin| {
            stored_types.contains_key(origin)
                && manifest
                    .column(&origin.0, &origin.1)
                    .is_ok_and(|metadata| metadata.encoding == "dictionary")
        }) {
            let (table, column) = column
                .as_ref()
                .expect("encoded string output has an input domain");
            let decoder = format_ident!("dictionary_{index}");
            decode_bindings.push(
                quote!(let #decoder = manifest.dictionary(manifest.column(#table, #column)?)?;),
            );
            // DECODE P: final answer formatting resolves source string dictionaries on the host.
            owned_outputs.push(quote!(#decoder[*value as usize].clone()));
        } else if publication.nullable[index] {
            owned_outputs.push(quote!(format!("{:?}", value)));
        } else {
            owned_outputs.push(quote!(value.to_string()));
        }
    }
    let catalog_field = quote!(domains: [Option<gpu_device_columns::Domain>; crate::key_domains::COUNT],);
    let catalog_init = quote!(domains: loader.domains(),);
    // Keep A-Domain provenance intact through every checker and planner.
    // Only the final Rust realization reads the prepared scalar instead.
    let mut prepared_fields = Vec::new();
    let mut prepared_types = Vec::new();
    let mut prepared_values = Vec::new();
    let mut prepare_captures = TokenStream::new();
    let mut replacements = Vec::new();
    let mut needed_names = std::collections::BTreeSet::new();
    let mut needed_captures = std::collections::BTreeSet::new();
    let mut opaque_dependency = false;
    for (index, capture) in normal.query.captures.items.iter().enumerate().rev() {
        if matches!(capture, cq_syntax::Capture::Domain(_)) || opaque_dependency
            || capture.binders().iter().any(|name| needed_names.contains(&name.to_string()))
        {
            needed_captures.insert(index);
            for expression in capture.expressions() {
                let reads = cq_syntax::rust::free_variables(expression);
                needed_names.extend(reads.names);
                opaque_dependency |= reads.opaque;
            }
        }
    }
    for (index, capture) in normal.query.captures.items.iter().enumerate() {
        if !needed_captures.contains(&index) { continue; }
        if let Some(binding) = capture.binding() {
            let value = format_ident!("__tpch_prepared_{index}");
            let pattern = &binding.pattern;
            let annotation = &binding.annotation;
            let expression = &binding.expression;
            prepare_captures.extend(quote! {
                let #value #annotation = #expression;
                let #pattern = &#value;
            });
            if let cq_syntax::Capture::Domain(domain) = capture {
                prepared_fields.push(value.clone());
                prepared_types.push(domain.ty.clone());
                prepared_values.push(quote!(#value));
                replacements.push((expression.clone(), syn::parse_quote!(data.#value)));
            }
        }
    }
    struct PreparedReads<'a>(&'a [(syn::Expr, syn::Expr)]);
    impl syn::visit_mut::VisitMut for PreparedReads<'_> {
        fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
            if let Some((_, replacement)) = self.0.iter().find(|(source, _)| source == expression) {
                *expression = replacement.clone();
            } else {
                syn::visit_mut::visit_expr_mut(self, expression);
            }
        }
    }
    let prepared_query = |tokens: TokenStream| -> std::result::Result<TokenStream, Rejected> {
        let mut expression: syn::Expr = checked("prepared capture Rust", &tokens, syn::parse2(tokens.clone()))?;
        syn::visit_mut::VisitMut::visit_expr_mut(&mut PreparedReads(&replacements), &mut expression);
        Ok(quote!(#expression))
    };
    let gpu = prepared_query(quote!(#gpu))?;
    let cpu = prepared_query(quote!(#cpu))?;
    let load_setup = if prepared_fields.is_empty() { quote!() } else {
        quote! {
            struct DomainInputs { #catalog_field }
            let data = DomainInputs { #catalog_init };
            #host_setup_bindings
            #prepare_captures
        }
    };
    {
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
                    timing.download_ms += elapsed;
                    timing.wall_ms += elapsed;
                    cq_gpu_runtime::publish_timings(timing);
                    result
                })
            } else if let Some(grouped) = &publication.grouped {
                let slots = &grouped.slots;
                let bounds: Vec<_> = grouped.bounds.iter().map(|bounds| match bounds {
                    KeyBounds::Input { relation, field, resident } => {
                        let name = format_ident!("{}", relation);
                        let value = if publications.contains_key(relation) { quote!(#name) } else { quote!(&data.#name) };
                        if *resident {
                            return quote!((#value).storage().bounds[#field]);
                        }
                        let field = syn::Index::from(*field);
                        quote!((#value).iter().fold(cq_gpu_runtime::Bounds::Empty, |bounds, row| {
                            let value = gpu_datalog_runtime::encode_scalar(&row.#field);
                            match bounds {
                                cq_gpu_runtime::Bounds::Empty => cq_gpu_runtime::Bounds::Inclusive { min: value, max: value },
                                cq_gpu_runtime::Bounds::Inclusive { min, max } => cq_gpu_runtime::Bounds::Inclusive {
                                    min: min.min(value), max: max.max(value),
                                },
                            }
                        }))
                    }
                    KeyBounds::Full => quote!(cq_gpu_runtime::Bounds::Inclusive { min: i64::MIN, max: i64::MAX }),
                }).collect();
                quote!({
                    let started = ::std::time::Instant::now();
                    let result = unsafe { output.into_columns::<Answer>(&[#(#slots),*], &[#(#bounds),*])? };
                    let mut timing = cq_gpu_runtime::take_last_timings().ok_or("group stage published no timings")?;
                    timing.wall_ms += started.elapsed().as_secs_f64() * 1000.0;
                    cq_gpu_runtime::publish_timings(timing);
                    result
                })
            } else {
                quote!(output)
            };
            let record_bounds = resident.then(|| quote! {
                cq_gpu_sequence_runtime::index_bytes::publication(module_path!(), &result.storage().bounds);
            });
            quote! {
                #(let #fields = &data.#fields;)*
                let output = #gpu;
                let result = #publish;
                #record_bounds
                Ok(result)
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
        let decode_fields: Vec<_> = (0..answer_types.len()).map(syn::Index::from).collect();
        return Ok((
            quote! {
                use cq_load::{Date, Load, StringCode};
                type Decimal = cq_load::Decimal<2>;
                pub(super) type Answer = (#(#answer_types,)*);
                pub(super) type Output = #output_type;
                pub(super) struct Inputs { #(#fields: #input_types,)* #(#prepared_fields: #prepared_types,)* #catalog_field pool: ::std::sync::Arc<cq_gpu_runtime::Pool> }
                pub(super) fn load(manifest: &crate::manifest::Manifest, pool: ::std::sync::Arc<cq_gpu_runtime::Pool>, loader: &mut crate::loader::Loader<'_>) -> crate::manifest::Result<Inputs> {
                    #load_setup
                    #(let #fields: #input_types = #bindings;)*
                    Ok(Inputs { #(#fields,)* #(#prepared_fields: #prepared_values,)* #catalog_init pool })
                }
                #[inline(never)]
                pub(super) fn execute(data: &Inputs, #(#names: &#inputs),*) -> crate::manifest::Result<Output> {
                    #setup_bindings
                    #execute
                }
                #[inline(never)]
                pub(super) fn sequential(data: &Inputs, #(#names: &Vec<#rows>),*) -> crate::manifest::Result<Vec<Answer>> {
                    #host_setup_bindings
                    #(#sequential_bindings)*
                    Ok((#cpu).map(|row| #own_row).collect())
                }
                pub(super) fn decoders<'a>(data: &'a Inputs, manifest: &crate::manifest::Manifest) -> crate::manifest::Result<(#(impl Fn(&#answer_types) -> String + 'a,)*)> {
                    #(#decode_bindings)*
                    Ok((#(move |value: &#answer_types| #owned_outputs,)*))
                }
                pub(super) fn decode(data: &Inputs, manifest: &crate::manifest::Manifest, rows: Vec<Answer>) -> crate::manifest::Result<Vec<Vec<String>>> {
                    let decoders = decoders(data, manifest)?;
                    Ok(rows.iter().map(|row| vec![#((decoders.#decode_fields)(&row.#decode_fields)),*]).collect())
                }
            },
            artifact,
        ));
    }
}

fn string_guard_regression(out: &Path) -> Result<TokenStream> {
    let directory = out.join("string-guard-fixture");
    fs::create_dir_all(&directory)?;
    for (name, values) in [
        ("keys.bin", [1_i32, 2, 3, 1]),
        ("names.bin", [0_i32, 1, 0, 0]),
        ("unique-keys.bin", [1_i32, 2, 3, 4]),
        ("sparse-keys.bin", [1_i32, 100, 1000, 1]),
    ] {
        fs::write(
            directory.join(name),
            values
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
    }
    fs::write(directory.join("names.txt"), "green part\nred\n")?;
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
                "partsupp": {"rows": 4, "columns": [key("ps_partkey")]},
                "orders": {"rows": 4, "columns": [{
                    "name": "o_orderkey", "type": "i32", "logical_type": "Integer",
                    "encoding": "plain", "file": "sparse-keys.bin"
                }]},
                "unique_rows": {"rows": 4, "columns": [
                    {
                        "name": "id", "type": "i32", "logical_type": "Integer",
                        "encoding": "plain", "file": "unique-keys.bin"
                    },
                    key("payload"), key("extra")
                ]},
                "part": {"rows": 4, "columns": [key("p_partkey"), {
                    "name": "p_name", "type": "i32", "logical_type": "String",
                    "encoding": "dictionary", "file": "names.bin",
                    "dictionary": "names.txt", "dictionary_index_base": 0,
                    "dictionary_entries": 2
                }]}
            }
        }))?,
    )?;
    let manifest = Manifest::load(&directory)?;
    let program = stages::Program::import(
        r#"
        program string_guard {
            domain Names = dictionary(part.p_name);
            input Keys(ps_partkey: i32) = table(partsupp);
            input Part(p_partkey: i32, p_name: StringCode in Names) = table(part);
            input GreenNames(code: StringCode in Names) = dictionary(Names)
                where { |value: &str| value.contains("green") };
            input DuplicatedNames(code: StringCode in Names) = host uses [GreenNames] rust {
                GreenNames.iter().chain(GreenNames.iter()).copied().collect::<Vec<_>>()
            };
            stage s0 {
                inputs { Keys = Keys; Part = Part; GreenNames = DuplicatedNames; }
                captures [];
                evidence {}
                cq {
                    struct Query;
                    relation Keys(ps_partkey: i32);
                    relation Part(p_partkey: i32, p_name: StringCode);
                    relation GreenNames(code: StringCode);
                    relation answer(key: i32, name: StringCode);
                    answer(key, name) :- Keys(key), Part(key, name),
                        GreenNames(name), where name == "green part";
                }
            }
            terminal { from s0 project [column(0), column(1)] dictionaries [1 = Names]; }
        }
    "#,
    )?;
    let mut pipeline = advice::Pipeline::baseline();
    pipeline.publication = advice::Placement::Host;
    let (code, _) = build_stages::compile(
        &program,
        "string_guard",
        &manifest,
        "advised",
        Some(&pipeline),
    )
    .map_err(|error| format!("String guard regression {}: {}", error.phase, error.error))?;
    pipeline.target = advice::Target::Host;
    let (host_code, _) = build_stages::compile(
        &program,
        "string_guard",
        &manifest,
        "advised",
        Some(&pipeline),
    )
    .map_err(|error| format!("Host set regression {}: {}", error.phase, error.error))?;
    let preprocessing = program.preprocessing_tokens()?;
    let publication_program = stages::Program::import(r#"
        program publication_origins {
            input Orders(o_orderkey: i64) = table(orders);
            stage s0 {
                inputs { Orders = Orders; }
                captures [];
                evidence {}
                cq {
                    struct Producer;
                    relation Orders(o_orderkey:i64);
                    relation answer(key:i64, narrow:i64, wide:i64);
                    answer(key, sum(narrow), sum(wide)) :- Orders(key),
                        let narrow:i64 = *key,
                        let wide:i64 = *key * 1099511627776i64;
                }
            }
            publish Produced = s0 owned;
            stage s1 {
                inputs { Produced = Produced; }
                captures [];
                evidence {}
                cq {
                    struct Consumer;
                    relation Produced(key:i64,narrow:i64,wide:i64);
                    relation answer(key:i64,narrow:i64,wide:i64);
                    answer(key,narrow,wide) :- Produced(key,narrow,wide);
                }
            }
            stage s2 {
                inputs { Produced = Produced; }
                captures [];
                evidence {}
                cq {
                    struct MissingConstant;
                    relation Produced(key:i64,narrow:i64,wide:i64);
                    relation answer(key:i64,narrow:i64,wide:i64);
                    answer(key,narrow,wide) :- Produced(key,narrow,wide), where key == 999;
                }
            }
            stage s3 {
                inputs { Produced = Produced; }
                captures [];
                evidence {}
                cq {
                    struct EdgeConstant;
                    relation Produced(key:i64,narrow:i64,wide:i64);
                    relation answer(key:i64,narrow:i64,wide:i64);
                    answer(key,narrow,wide) :- Produced(key,narrow,wide), where key == 1000;
                }
            }
            terminal {
                from s1 project [column(0),column(1),column(2)] dictionaries [];
                from s2 project [column(0),column(1),column(2)] dictionaries [];
                from s3 project [column(0),column(1),column(2)] dictionaries [];
            }
        }
    "#)?;
    let mut publication_pipeline = advice::Pipeline::baseline();
    publication_pipeline.publication = advice::Placement::Device;
    let (publication_code, _) = build_stages::compile(
        &publication_program, "publication_origins", &manifest, "advised",
        Some(&publication_pipeline),
    ).map_err(|error| format!("Publication origin regression {}: {}", error.phase, error.error))?;
    publication_pipeline.target = advice::Target::Host;
    publication_pipeline.publication = advice::Placement::Host;
    let (host_publication_code, _) = build_stages::compile(
        &publication_program, "publication_origins", &manifest, "advised",
        Some(&publication_pipeline),
    ).map_err(|error| format!("Host publication origin regression {}: {}", error.phase, error.error))?;
    let mut rank_source = String::from("program rank_constants { input Orders(o_orderkey:i64) = table(orders);");
    let mut rank_checks = TokenStream::new();
    let mut rank_terminals = String::new();
    let mut index = 0;
    for comparison in ["==", "<", "<=", ">", ">="] {
        for constant in [100i64, 99, 0, 1001] {
            rank_source.push_str(&format!(
                "stage s{index} {{ inputs {{ Orders = Orders; }} captures []; evidence {{}} cq {{
                    struct RankConstant{index}; relation Orders(key:i64); relation answer(key:i64);
                    answer(key) :- Orders(key), where key {comparison} {constant};
                }} }}"
            ));
            rank_terminals.push_str(&format!("from s{index} project [column(0)] dictionaries [];"));
            let module = format_ident!("stage_{index}");
            let mut expected = [1i64, 100, 1000].into_iter().filter(|key| match comparison {
                "==" => *key == constant,
                "<" => *key < constant,
                "<=" => *key <= constant,
                ">" => *key > constant,
                ">=" => *key >= constant,
                _ => unreachable!(),
            }).map(|key| key.to_string()).collect::<Vec<_>>();
            expected.sort();
            rank_checks.extend(quote! {
                {
                    let data = #module::load(&manifest, pool.clone(), &mut loader)?;
                    for _ in 0..2 {
                        let result = #module::execute(&data)?;
                        let mut actual = #module::decode(&data, &manifest, result)?;
                        actual.sort();
                        let expected: Vec<Vec<String>> = vec![#(vec![#expected.to_owned()]),*];
                        assert_eq!(actual, expected, "{} {}", #comparison, #constant);
                        let mut reference = #module::decode(&data, &manifest, #module::sequential(&data)?)?;
                        reference.sort();
                        assert_eq!(reference, expected);
                    }
                }
            });
            index += 1;
        }
    }
    rank_source.push_str(&format!("terminal {{ {rank_terminals} }} }}"));
    let rank_program = stages::Program::import(&rank_source)?;
    let mut rank_pipeline = advice::Pipeline::baseline();
    rank_pipeline.publication = advice::Placement::Host;
    let (rank_code, _) = build_stages::compile(
        &rank_program, "rank_constants", &manifest, "advised", Some(&rank_pipeline),
    ).map_err(|error| format!("Rank constants regression {}: {}", error.phase, error.error))?;
    rank_pipeline.target = advice::Target::Host;
    let (rank_host_code, _) = build_stages::compile(
        &rank_program, "rank_constants", &manifest, "advised", Some(&rank_pipeline),
    ).map_err(|error| format!("Host Rank constants regression {}: {}", error.phase, error.error))?;
    let directory = directory.to_string_lossy();
    Ok(quote! {
        #[cfg(test)]
        #[allow(non_snake_case, dead_code)]
        mod rank_constant_regression {
            #rank_code
            #[test]
            fn rank_comparisons_prepare_once() -> crate::manifest::Result<()> {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory))?;
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                let mut loader = crate::loader::Loader::new(&manifest, pool.clone());
                loader.prepare(&[("orders", &["o_orderkey"])])?;
                assert!(loader.domains()[0].as_ref().unwrap().is_rank());
                #rank_checks
                Ok(())
            }
        }
        #[cfg(test)]
        #[allow(non_snake_case, dead_code)]
        mod rank_host_constant_regression {
            #rank_host_code
            #[test]
            fn rank_comparisons_prepare_once() -> crate::manifest::Result<()> {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory))?;
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                let mut loader = crate::loader::Loader::new(&manifest, pool.clone());
                loader.prepare(&[("orders", &["o_orderkey"])])?;
                assert!(loader.domains()[0].as_ref().unwrap().is_rank());
                #rank_checks
                Ok(())
            }
        }
        #[cfg(test)]
        #[allow(non_snake_case, dead_code)]
        mod string_guard_regression {
            use serde_json::Value;
            #code
            #[test]
            fn synthetic_string_literal_guard_preserves_manifest_codes() -> crate::manifest::Result<()> {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory)).unwrap();
                #preprocessing
                assert_eq!(__cq_prepared_DuplicatedNames, vec![(0_u32,)]);
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0).unwrap();
                let mut loader = crate::loader::Loader::new(&manifest, pool.clone());
                loader.prepare(&[("partsupp", &["ps_partkey"]), ("part", &["p_partkey", "p_name"])])?;
                let data = stage_0::load(&manifest, pool, &mut loader).unwrap();
                let result = stage_0::execute(&data, &__cq_prepared_DuplicatedNames).unwrap();
                let mut actual = stage_0::decode(&data, &manifest, result).unwrap();
                actual.sort();
                assert_eq!(actual, vec![
                    vec!["1".to_owned(), "green part".to_owned()],
                    vec!["3".to_owned(), "green part".to_owned()],
                ]);
                let reference = stage_0::sequential(&data, &__cq_prepared_DuplicatedNames).unwrap();
                let mut reference = stage_0::decode(&data, &manifest, reference).unwrap();
                reference.sort();
                assert_eq!(actual, reference);
                Ok(())
            }
            #[test]
            fn projected_sets_share_only_identical_resident_tables() -> crate::manifest::Result<()> {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory))?;
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                let mut loader = crate::loader::Loader::new(&manifest, pool.clone());
                loader.prepare(&[("part", &["p_partkey", "p_name"])])?;
                let full = loader.bind::<(u32, i32)>("part", &["p_partkey", "p_name"])?;
                assert_eq!(full.to_host()?, vec![(0, 0), (1, 1), (2, 0)]);
                let projected = loader.bind::<(i32,)>("part", &["p_name"])?;
                assert_eq!(projected.to_host()?, vec![(0,), (1,)]);
                let alias = loader.bind::<(i32,)>("part", &["p_name"])?;
                assert!(::std::sync::Arc::ptr_eq(
                    &projected.storage().columns[0], &alias.storage().columns[0],
                ));
                assert!(::std::sync::Arc::ptr_eq(
                    &projected.storage().metadata, &alias.storage().metadata,
                ));
                assert!(!::std::sync::Arc::ptr_eq(
                    &full.storage().columns[1], &projected.storage().columns[0],
                ));
                let source = gpu_compress::Source::load_manifest(&manifest.root)?;
                let schema = gpu_compress_host::Schema {
                    domains: vec![],
                    tables: vec![("part".into(), vec![("p_name".into(), gpu_compress_host::Kind::I32)])],
                };
                let catalog = gpu_compress::Catalog::compress(&pool, &source, &schema)?;
                assert_eq!(catalog.rows("part")?, 2);
                assert_eq!(catalog.device_columns::<(i32,)>("part")?.to_host()?, vec![(0,), (1,)]);
                Ok(())
            }
            #[test]
            fn distinct_projections_reuse_base_columns_and_repeated_names() -> crate::manifest::Result<()> {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory))?;
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                let mut loader = crate::loader::Loader::new(&manifest, pool);
                let full = loader.bind::<(i32, i32)>("unique_rows", &["id", "payload"])?;
                assert_eq!(full.to_host()?, vec![(1, 1), (2, 2), (3, 3), (4, 1)]);
                let ids = loader.bind::<(i32,)>("unique_rows", &["id"])?;
                assert_eq!(ids.to_host()?, vec![(1,), (2,), (3,), (4,)]);
                assert!(::std::sync::Arc::ptr_eq(
                    &full.storage().columns[0], &ids.storage().columns[0],
                ));
                let repeated = loader.bind::<(i32, i32, i32)>("unique_rows", &["payload", "id", "id"])?;
                assert_eq!(repeated.to_host()?, vec![(1, 1, 1), (2, 2, 2), (3, 3, 3), (1, 4, 4)]);
                assert!(::std::sync::Arc::ptr_eq(
                    &full.storage().columns[1], &repeated.storage().columns[0],
                ));
                assert!(::std::sync::Arc::ptr_eq(
                    &ids.storage().columns[0], &repeated.storage().columns[1],
                ));
                assert!(::std::sync::Arc::ptr_eq(
                    &repeated.storage().columns[1], &repeated.storage().columns[2],
                ));
                let duplicates = loader.bind::<(i32, i32)>("unique_rows", &["payload", "payload"])?;
                assert_eq!(duplicates.to_host()?, vec![(1, 1), (2, 2), (3, 3)]);
                assert!(!::std::sync::Arc::ptr_eq(
                    &full.storage().columns[1], &duplicates.storage().columns[0],
                ));
                assert!(::std::sync::Arc::ptr_eq(
                    &duplicates.storage().columns[0], &duplicates.storage().columns[1],
                ));
                let extended = loader.bind::<(i32, i32)>("unique_rows", &["id", "extra"])?;
                assert_eq!(extended.to_host()?, vec![(1, 1), (2, 2), (3, 3), (4, 1)]);
                assert!(::std::sync::Arc::ptr_eq(
                    &ids.storage().columns[0], &extended.storage().columns[0],
                ));
                Ok(())
            }
            #[test]
            fn verified_keys_cover_only_matching_projections_and_bytes() -> crate::manifest::Result<()> {
                let mut manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory))?;
                let hash = manifest.read_column_checked("unique_rows", "id", true)?.1.unwrap();
                manifest.relations.get_mut("unique_rows").unwrap().verified_keys = vec![
                    serde_json::from_value(serde_json::json!({
                        "set": "verified", "rows": 4, "types": {"id": "i32"},
                        "columns": {"id": hash}
                    }))?
                ];
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                let mut loader = crate::loader::Loader::new(&manifest, pool.clone());
                assert_eq!(loader.bind::<(i32, i32)>("unique_rows", &["id", "payload"])?.to_host()?,
                    vec![(1, 1), (2, 2), (3, 3), (4, 1)]);
                assert_eq!(loader.bind::<(i32,)>("unique_rows", &["payload"])?.to_host()?,
                    vec![(1,), (2,), (3,)]);
                drop(loader);
                // A certificate copied onto another column cannot authorize duplicate rows.
                manifest.relations.get_mut("unique_rows").unwrap().verified_keys = vec![
                    serde_json::from_value(serde_json::json!({
                        "set": "verified", "rows": 4, "types": {"payload": "i32"},
                        "columns": {"payload": hash}
                    }))?
                ];
                let mut loader = crate::loader::Loader::new(&manifest, pool);
                assert_eq!(loader.bind::<(i32,)>("unique_rows", &["payload"])?.to_host()?,
                    vec![(1,), (2,), (3,)]);
                Ok(())
            }
        }
        #[cfg(test)]
        #[allow(non_snake_case, dead_code)]
        mod host_input_regression {
            #host_code
            #[test]
            fn typed_host_inputs_and_prepared_producers_are_sets() -> crate::manifest::Result<()> {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory))?;
                #preprocessing
                assert_eq!(__cq_prepared_DuplicatedNames, vec![(0_u32,)]);
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                let mut loader = crate::loader::Loader::new(&manifest, pool.clone());
                loader.prepare(&[("partsupp", &["ps_partkey"]), ("part", &["p_partkey", "p_name"])])?;
                let data = stage_0::load(&manifest, pool, &mut loader)?;
                let result = stage_0::execute(&data, &__cq_prepared_DuplicatedNames)?;
                let mut actual = stage_0::decode(&data, &manifest, result)?;
                actual.sort();
                assert_eq!(actual, vec![
                    vec!["1".to_owned(), "green part".to_owned()],
                    vec!["3".to_owned(), "green part".to_owned()],
                ]);
                Ok(())
            }
        }
        #[cfg(test)]
        #[allow(non_snake_case, dead_code)]
        mod publication_origin_regression {
            #publication_code
            #[test]
            fn staged_narrow_computed_wide_computed_and_domain_keys_are_exact() -> crate::manifest::Result<()> {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory))?;
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                let mut loader = crate::loader::Loader::new(&manifest, pool.clone());
                loader.prepare(&[("orders", &["o_orderkey"])])?;
                assert!(loader.domains()[0].as_ref().unwrap().is_rank());
                let producer = stage_0::load(&manifest, pool.clone(), &mut loader)?;
                let consumer = stage_1::load(&manifest, pool.clone(), &mut loader)?;
                let missing = stage_2::load(&manifest, pool.clone(), &mut loader)?;
                let edge = stage_3::load(&manifest, pool, &mut loader)?;
                gpu_device_columns::rank::take_host_inverse_report();
                let rows = stage_0::execute(&producer)?;
                assert_eq!(gpu_device_columns::rank::take_host_inverse_report().0, 0,
                    "device Rust must not initialize a host inverse mirror");
                assert_eq!(rows.storage().bounds, vec![
                    gpu_device_columns::Bounds::Inclusive { min: 0, max: 2 },
                    gpu_device_columns::Bounds::Inclusive { min: 1, max: 1000 },
                    gpu_device_columns::Bounds::Inclusive { min: 1i64 << 40, max: 1000i64 << 40 },
                ]);
                let answer = stage_1::execute(&consumer, &rows)?;
                let mut actual = stage_1::decode(&consumer, &manifest, answer.to_host()?)?;
                actual.sort();
                assert_eq!(actual, [1i64, 100, 1000].into_iter().map(|key| vec![
                    key.to_string(), key.to_string(), (key << 40).to_string(),
                ]).collect::<Vec<_>>());
                let reference = stage_0::sequential(&producer)?;
                let reference = stage_1::sequential(&consumer, &reference)?;
                let mut reference = stage_1::decode(&consumer, &manifest, reference)?;
                reference.sort();
                assert_eq!(actual, reference);
                assert!(stage_2::execute(&missing, &rows)?.to_host()?.is_empty());
                let edge_rows = stage_3::execute(&edge, &rows)?.to_host()?;
                assert_eq!(stage_3::decode(&edge, &manifest, edge_rows)?, vec![vec![
                    "1000".to_owned(), "1000".to_owned(), (1000i64 << 40).to_string(),
                ]]);
                Ok(())
            }
        }
        #[cfg(test)]
        #[allow(non_snake_case, dead_code)]
        mod publication_host_origin_regression {
            #host_publication_code
            #[test]
            fn staged_host_rank_rust_read_uses_owning_handle() -> crate::manifest::Result<()> {
                let manifest = crate::manifest::Manifest::load(::std::path::Path::new(#directory))?;
                let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
                let mut loader = crate::loader::Loader::new(&manifest, pool.clone());
                loader.prepare(&[("orders", &["o_orderkey"])])?;
                let bytes = loader.domains()[0].as_ref().unwrap().bytes();
                let producer = stage_0::load(&manifest, pool, &mut loader)?;
                gpu_device_columns::rank::take_host_inverse_report();
                let rows = stage_0::execute(&producer)?;
                assert_eq!(gpu_device_columns::rank::take_host_inverse_report().0, bytes);
                let mut actual = stage_0::decode(&producer, &manifest, rows)?;
                actual.sort();
                assert_eq!(actual, [1i64, 100, 1000].into_iter().map(|key| vec![
                    key.to_string(), key.to_string(), (key << 40).to_string(),
                ]).collect::<Vec<_>>());
                assert_eq!(gpu_device_columns::rank::take_host_inverse_report().0, 0);
                Ok(())
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
            println!("cargo:rerun-if-changed=queries/{name}.stages");
            let staged = stages::Program::import(source)?;
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
                } else if staged.stages.len() == 1 {
                    advice::Pipeline::grouped(&staged.stages[0].program.to_token_stream())?
                } else {
                    // CPU hints remain available explicitly, but an unmeasured CPU
                    // choice must not replace the existing accepted GPU baseline.
                    advice::Pipeline::baseline()
                };
                let mut report = json!({"query": name, "sf": sf, "mode": mode,
                    "stage_source": staged.to_token_stream().to_string(), "pipeline": pipeline});
                let generation_started = std::time::Instant::now();
                let compiled = build_stages::compile(
                    &staged,
                    name,
                    &manifest,
                    mode,
                    if staged.stages.len() == 1 {
                        Some(&pipeline)
                    } else if mode == "advised" {
                        overrides.get(name)
                    } else {
                        None
                    },
                );
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
