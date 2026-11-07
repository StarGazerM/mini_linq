//! GPU realization selected once, after provider bindings and publication
//! storage, and recorded in `Bindings.realization`. Printers consume these
//! records; `check` validates supplied records without calling `select`.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::gpu::bindings::ProviderAbi;
use cq_lang_sequence::gpu::realization::{
    Auxiliary, BaseBounds, BlockGeometry, BoundSource, CandidateCount, Captures, ChoiceCarrier,
    DescriptorBounds, Granularity, GroupCompletion, GroupInput, HostEvaluation, HostOutput,
    HostUnion, Interval, Phase, ProviderBuild, Realization, Side, SmallestRealization,
    SmallestSide, SourceAccess, SourceOrigin, SourceRealization, Step, TerminalGroup, Ties,
    Transfer, TransferSource, UnionArm, Launch,
};
use cq_lang_sequence::gpu::{Grid, Publication};
use cq_lang_sequence::{Analysis, Program, Term};
use cq_lang_structures::Capability;
use cq_lang_structures::physical::{Definition, Operator, symbol_name};
use cq_lang_structures::syntax as cq_syntax;
use quote::format_ident;

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(
        proc_macro2::Span::call_site(),
        format!("GPU-Realization: {message}"),
    )
}

/// Auxiliary element kernels (union decode/pack, host-output pack) launch one
/// thread per element in blocks of the target's collective width.
fn elements() -> Grid {
    Grid::Elements {
        block_threads: crate::BLOCK_THREADS,
    }
}

/// Grid-partial blocks (and their combine block) hold one i64 value and one
/// u32 presence word per lane in dynamic shared memory.
const PARTIAL_LANE_BYTES: u32 = 12;

fn partials_geometry() -> BlockGeometry {
    BlockGeometry {
        block_threads: crate::BLOCK_THREADS,
        shared_bytes: crate::BLOCK_THREADS * PARTIAL_LANE_BYTES,
    }
}

/// Shared bytes of one keyed-group reduction block (one block per group).
const GROUP_REDUCE_SHARED_BYTES: u32 = 4096;

/// The geometry each runtime auxiliary family is compiled for: element
/// kernels in blocks of the collective width, the keyed reduction with its
/// per-group shared buffer.
fn auxiliary() -> Auxiliary {
    let elements = BlockGeometry {
        block_threads: crate::BLOCK_THREADS,
        shared_bytes: 0,
    };
    Auxiliary {
        selection: elements,
        offsets: elements,
        range: elements,
        canon: elements,
        group: elements,
        group_reduce: BlockGeometry {
            block_threads: crate::BLOCK_THREADS,
            shared_bytes: GROUP_REDUCE_SHARED_BYTES,
        },
        bounds: elements,
    }
}

/// Whether the runtime's auxiliary kernels can launch with the recorded
/// geometry: their templates are compiled for blocks of the collective width;
/// element kernels use no dynamic shared memory and the keyed reduction its
/// per-group buffer.
fn supported_auxiliary(record: &Auxiliary) -> bool {
    let elements = |geometry: &BlockGeometry| {
        geometry.block_threads == crate::BLOCK_THREADS && geometry.shared_bytes == 0
    };
    [
        &record.selection,
        &record.offsets,
        &record.range,
        &record.canon,
        &record.group,
        &record.bounds,
    ]
    .into_iter()
    .all(elements)
        && record.group_reduce.block_threads == crate::BLOCK_THREADS
        && record.group_reduce.shared_bytes == GROUP_REDUCE_SHARED_BYTES
}

/// Formal rule GPU-Realization: select source access, publication steps,
/// terminal grouping, transfers, auxiliary geometry, descriptor bounds and
/// captures once, before printing.
pub(crate) fn select(
    physical: &cq_lang_structures::physical::Program,
    structures: &cq_lang_structures::Program,
    program: &Program,
    structure_analysis: &cq_lang_structures::Analysis,
    analysis: &Analysis,
) -> syn::Result<Realization> {
    let selected: syn::Result<Realization> = (|| {
        let facts = Facts::new(physical, structures, program, structure_analysis, analysis)?;
        let captures = captures(&facts)?;
        let reusable = analysis.gpu_publications.values().any(|record| record.arena.is_some());
        let launches = analysis.dispatches.iter().map(|dispatch| {
            if reusable {
                match dispatch.gpu.as_ref().expect("graded GPU dispatch").partition {
                    cq_lang_sequence::gpu::Partition::One => Launch::ProgramOne,
                    _ => Launch::ProgramOccupancy,
                }
            } else {
                Launch::InvocationOccupancy
            }
        }).collect();
        let mut sources = sources(&facts, &captures)?;
        let (steps, output) = Steps::new(&facts, &sources).run()?;
        // One provider owner: a publication step that builds a result's
        // providers leaves its source the scan or interval read.
        for (name, source) in &mut sources {
            if let SourceAccess::Provider { selection } = &source.access
                && step_builds(&steps, analysis, name)
            {
                source.access = match selection {
                    Some(interval) => SourceAccess::Positions(interval.clone()),
                    None => SourceAccess::Scan,
                };
            }
        }
        let smallest = smallest(&facts)?;
        Ok(Realization {
            rejection: None,
            sources,
            whole_lookups: crate::rows::device_lookup_demands(program, analysis)
                .into_iter()
                .filter_map(|(name, keys)| keys.is_empty().then_some(name))
                .collect(),
            steps,
            output,
            captures,
            smallest,
            auxiliary: Some(auxiliary()),
            launches,
        })
    })();
    let realization = match selected {
        Ok(realization) => realization,
        Err(error) => Realization {
            rejection: Some(
                error
                    .into_iter()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            ..Realization::default()
        },
    };
    check(physical, structures, program, structure_analysis, analysis, &realization)?;
    Ok(realization)
}

/// The radix layout of `side` read as `[prefix][column]`, with its recorded
/// Candidates read and declared `candidates` reads.
fn candidate_layout<'a>(
    structure_analysis: &cq_lang_structures::Analysis,
    analysis: &'a Analysis,
    side: &str,
    prefix: &[String],
    column: &str,
) -> Option<&'a cq_lang_sequence::gpu::bindings::ProviderLayout> {
    let layouts = analysis.gpu_bindings.providers.get(side)?;
    let built = structure_analysis.built.get(side)?;
    layouts
        .iter()
        .zip(built)
        .find_map(|(layout, (levels, provider))| {
            let mut leading = levels.first()?.clone();
            leading.sort();
            (layout.abi == ProviderAbi::Radix
                && levels.len() == 2
                && leading == prefix
                && levels[1] == [column.to_owned()]
                && structure_analysis
                    .facts(provider)
                    .reads(&cq_lang_structures::provider::Capability::Candidates)
                && layout.radix.as_ref().is_some_and(|radix| {
                    radix.reads.contains(&(
                        prefix.to_vec(),
                        cq_lang_sequence::gpu::bindings::RadixRead::Candidates {
                            column: column.to_owned(),
                        },
                    ))
                }))
            .then_some(layout)
        })
}

/// Formal rule GPU-Smallest: a PH-Smallest probe is realized only when every
/// side is a two-level radix layout with exact candidate reads; otherwise the
/// target rejects it (never a First probe).
fn smallest(facts: &Facts) -> syn::Result<BTreeMap<String, SmallestRealization>> {
    use cq_lang_structures::physical::{Choice, Operator};
    let physical = facts.physical;
    let candidates = cq_lang_structures::physical::candidate_prefixes(
        physical,
        &facts.structure_analysis.physical,
    );
    let mut realized = BTreeMap::new();
    for definition in &physical.program.definitions {
        if !matches!(&definition.operator,
            Operator::Probe(_, keyed) if matches!(keyed.choice, Some(Choice::Smallest(_))))
        {
            continue;
        }
        let reader = symbol_name(&definition.name);
        let rejection = || {
            error(
                "T-Gpu: probe smallest is realized only over two-level radix layouts `[prefix][v]` with candidates reads on every side",
            )
        };
        let candidate = candidates
            .iter()
            .find(|candidate| symbol_name(candidate.reader) == reader)
            .ok_or_else(rejection)?;
        let mut sides = Vec::new();
        for (side, prefix) in &candidate.sides {
            candidate_layout(facts.structure_analysis, facts.analysis, side, prefix, &candidate.column)
                .ok_or_else(rejection)?;
            sides.push(SmallestSide {
                provider: side.clone(),
                prefix: prefix.clone(),
            });
        }
        realized.insert(
            reader,
            SmallestRealization {
                column: candidate.column.clone(),
                carrier: ChoiceCarrier::for_sides(sides.len()),
                sides,
                count: CandidateCount::ExactUnion,
                ties: Ties::FirstInOrder,
            },
        );
    }
    Ok(realized)
}

/// GPU-Smallest checker: one record per PH-Smallest probe, over the same
/// candidate column, every Physical side once in order, each a two-level
/// radix layout with a recorded Candidates read. Selection is not invoked.
fn check_smallest(
    facts: &Facts,
    records: &BTreeMap<String, SmallestRealization>,
) -> syn::Result<()> {
    use cq_lang_structures::physical::{Choice, Operator};
    let physical = facts.physical;
    let candidates = cq_lang_structures::physical::candidate_prefixes(
        physical,
        &facts.structure_analysis.physical,
    );
    let probes: BTreeSet<_> = physical
        .program
        .definitions
        .iter()
        .filter(|definition| {
            matches!(&definition.operator,
            Operator::Probe(_, keyed) if matches!(keyed.choice, Some(Choice::Smallest(_))))
        })
        .map(|definition| symbol_name(&definition.name))
        .collect();
    if records.keys().cloned().collect::<BTreeSet<_>>() != probes {
        return Err(error(
            "GPU-Smallest: every smallest probe needs exactly one realization",
        ));
    }
    for (reader, record) in records {
        let candidate = candidates
            .iter()
            .find(|candidate| symbol_name(candidate.reader) == *reader)
            .ok_or_else(|| error("GPU-Smallest: realization of a probe without an intersection"))?;
        let SmallestRealization {
            column,
            sides,
            count: CandidateCount::ExactUnion,
            ties: Ties::FirstInOrder,
            carrier,
        } = record;
        if *carrier != ChoiceCarrier::for_sides(sides.len()) {
            return Err(error(
                "GPU-Smallest: choice carrier does not name every side",
            ));
        }
        if *column != candidate.column
            || sides.len() != candidate.sides.len()
            || sides
                .iter()
                .zip(&candidate.sides)
                .any(|(side, (build, prefix))| {
                    side.provider != *build
                        || side.prefix != *prefix
                        || candidate_layout(facts.structure_analysis, facts.analysis, build, prefix, column).is_none()
                })
        {
            return Err(error(
                "GPU-Smallest: sides must be every Physical side once, each a candidates radix layout",
            ));
        }
    }
    Ok(())
}

/// Checked facts shared by selection and checking. No realization policy.
struct Facts<'a> {
    physical: &'a cq_lang_structures::physical::Program,
    structure_analysis: &'a cq_lang_structures::Analysis,
    program: &'a Program,
    analysis: &'a Analysis,
    definitions: BTreeMap<String, Definition>,
    sequence: BTreeMap<String, &'a Term>,
    /// Recorded device carrier of every column.
    carriers: &'a BTreeMap<String, syn::Type>,
    resident: BTreeSet<String>,
    runs: BTreeSet<String>,
    device_results: BTreeSet<String>,
    /// Physical results of nonterminal dispatch and host-union publications:
    /// their publication step builds the result's providers, so no source does.
    published: BTreeSet<String>,
    codecs: crate::runs::Codecs,
}

impl<'a> Facts<'a> {
    fn new(
        physical: &'a cq_lang_structures::physical::Program,
        structures: &cq_lang_structures::Program,
        program: &'a Program,
        structure_analysis: &'a cq_lang_structures::Analysis,
        analysis: &'a Analysis,
    ) -> syn::Result<Self> {
        let definitions: BTreeMap<_, _> = physical
            .program
            .definitions
            .iter()
            .map(|d| (symbol_name(&d.name), d.clone()))
            .collect();
        let sequence: BTreeMap<String, &Term> = program
            .sequence
            .definitions
            .iter()
            .map(|d| (symbol_name(&d.name), &d.term))
            .collect();
        let codecs =
            crate::runs::Codecs::check(physical, structures, &structure_analysis.physical.rows)?;
        let carriers = &analysis.gpu_bindings.carriers;
        let mut resident = BTreeSet::new();
        let mut runs = BTreeSet::new();
        for input in physical.inputs() {
            let name = symbol_name(&input.name);
            for entry in &structures.structures.entries {
                if structure_analysis
                    .physical
                    .hosts
                    .get(&symbol_name(&entry.name))
                    != Some(&name)
                {
                    continue;
                }
                if let cq_lang_structures::Structure::Host { reader, keyed, .. } = &entry.structure
                {
                    if !matches!(reader, cq_lang_structures::ReaderRef::Scan) {
                        continue;
                    }
                    if structure_analysis.facts(&keyed.provider).abi
                        == Some(cq_lang_structures::provider::NativeAbi::RadixRelation)
                    {
                        runs.insert(name.clone());
                        resident.insert(name.clone());
                    } else if structure_analysis.facts(&keyed.provider).abi
                        == Some(cq_lang_structures::provider::NativeAbi::Columns)
                    {
                        resident.insert(name.clone());
                    }
                }
            }
        }
        let device_results = analysis
            .dispatches
            .iter()
            .flat_map(|dispatch| {
                let source = analysis.results.get(&dispatch.outer_store);
                dispatch
                    .inner_stores
                    .iter()
                    .filter_map(|name| analysis.results.get(name))
                    .filter(move |result| Some(*result) != source)
                    .cloned()
            })
            .collect();
        let terminal = analysis
            .output
            .as_ref()
            .map(|(name, _)| name.clone())
            .or_else(|| {
                program
                    .sequence
                    .definitions
                    .last()
                    .map(|d| symbol_name(&d.name))
            });
        let host_level = |name: &str| {
            analysis
                .levels
                .get(name)
                .is_some_and(|level| level.last().is_some_and(|level| level == "host"))
        };
        let dispatched = analysis.dispatches.iter().filter_map(|dispatch| {
            dispatch.inner_stores.iter().rev().find(|name| {
                sequence
                    .get(*name)
                    .is_some_and(|term: &&Term| matches!(term, Term::Store { .. }))
            })
        });
        let unions = program
            .sequence
            .definitions
            .iter()
            .filter_map(|definition| {
                let name = symbol_name(&definition.name);
                (host_level(&name)
                    && matches!(definition.term, Term::Store { .. })
                    && host_union(&definition.term, &sequence).is_some())
                .then_some(name)
            });
        let published = dispatched
            .cloned()
            .chain(unions)
            .filter(|publication| Some(publication) != terminal.as_ref())
            .filter_map(|publication| analysis.results.get(&publication).cloned())
            .collect();
        Ok(Self {
            physical,
            structure_analysis,
            program,
            analysis,
            definitions,
            sequence,
            carriers,
            resident,
            runs,
            device_results,
            published,
            codecs,
        })
    }

    fn host_level(&self, name: &str) -> bool {
        self.analysis
            .levels
            .get(name)
            .is_some_and(|level| level.last().is_some_and(|level| level == "host"))
    }

    /// Position of the Sequence definition `name`.
    fn position(&self, name: &str) -> Option<usize> {
        self.program
            .sequence
            .definitions
            .iter()
            .position(|definition| symbol_name(&definition.name) == name)
    }

    /// Position of the first Sequence definition that looks up a provider of
    /// the Physical result `result` (directly or through its store).
    fn first_reader(&self, result: &str) -> Option<usize> {
        self.program
            .sequence
            .definitions
            .iter()
            .position(|definition| {
                crate::rows::lookup_providers(&definition.term)
                    .iter()
                    .any(|provider| {
                        self.analysis.results.get(provider).unwrap_or(provider) == result
                    })
            })
    }

    fn is_host_store(&self, name: &str) -> bool {
        self.analysis.stores.contains_key(name) && self.host_level(name)
    }

    /// A host-graded explicit Store definition.
    fn host_store_term(&self, name: &str) -> bool {
        self.host_level(name) && matches!(self.sequence.get(name), Some(Term::Store { .. }))
    }

    /// The Physical definitions a Sequence map evaluates, newest first.
    fn map_definitions(&self, name: &str, input: &str) -> syn::Result<Vec<&Definition>> {
        let mut chain = Vec::new();
        let mut current = name.to_owned();
        while current != input {
            let definition = self
                .definitions
                .get(&current)
                .ok_or_else(|| error("unknown Physical row function"))?;
            chain.push(definition);
            let Some(next) = definition.operator.input() else {
                break;
            };
            current = symbol_name(next);
        }
        Ok(chain)
    }

    fn final_name(&self) -> syn::Result<String> {
        self.analysis
            .output
            .as_ref()
            .map(|(name, _)| name.clone())
            .or_else(|| {
                self.program
                    .sequence
                    .definitions
                    .last()
                    .map(|d| symbol_name(&d.name))
            })
            .ok_or_else(|| error("GPU Sequence lacks a checked output"))
    }
}

/// Formal rule GPU-Capture: extract closed operands and device Rust
/// callables once per program, in Physical definition order.
fn captures(facts: &Facts) -> syn::Result<Captures> {
    let analysis = facts.analysis;
    let structure_analysis = facts.structure_analysis;
    // Contract 7 (DESIGN anchor contract7-gpu-rows): only identity reads admit
    // device Rust; encoded predicates stay relational `where` captures.
    let query = &facts.physical.program.captures;
    let mut captures = crate::capture::Captures::new(query);
    if query
        .items
        .iter()
        .any(|item| matches!(item, cq_syntax::Capture::Guard(_)))
        && facts
            .sequence
            .get(&facts.final_name()?)
            .and_then(|term| grouped_boundary(term, &facts.sequence))
            .is_some()
    {
        return Err(error(
            "T-Gpu: an answer gate over a grouped publication is unsupported",
        ));
    }
    let mut records = Captures {
        query: query.clone(),
        ..Captures::default()
    };
    for (name, definition) in &facts.definitions {
        captures.used.clear();
        if !analysis.rust_requires_host(&structure_analysis.physical, &definition.operator)
            && (matches!(&definition.operator, Operator::Guard(guard) if guard.capture.is_some())
                || matches!(&definition.operator, Operator::Bind(bind) if bind.capture.is_some()))
        {
            return Err(syn::Error::new_spanned(
                &definition.name,
                "T-Gpu: a branch capture reference has no device realization",
            ));
        }
        let expression = match &definition.operator {
            Operator::Guard(guard) if !analysis.rust_requires_host(&structure_analysis.physical, &definition.operator) => {
                let row = &structure_analysis.physical.rows[&symbol_name(&guard.input)];
                captures.rust_expression(
                    &guard.condition,
                    &row.types,
                    &row.bindings,
                    &syn::parse_quote!(bool),
                    None,
                    name,
                    &facts.codecs,
                )
            }
            Operator::Bind(bind) if !analysis.rust_requires_host(&structure_analysis.physical, &definition.operator) => {
                let row = &structure_analysis.physical.rows[&symbol_name(&bind.input)];
                let names = cq_syntax::rust::bindings(&bind.pattern);
                let output =
                    &structure_analysis.physical.rows[name].types[&symbol_name(&names[0])];
                captures.rust_expression(
                    &bind.expression,
                    &row.types,
                    &row.bindings,
                    output,
                    structure_analysis.physical.rows[name]
                        .logical_type_ids
                        .get(&symbol_name(&names[0]))
                        .copied(),
                    name,
                    &facts.codecs,
                )
            }
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                let types = &structure_analysis.physical.rows[&symbol_name(&select.input)].types;
                captures.relational(&select.condition, types, &facts.codecs)?
            }
            _ => continue,
        };
        records.uses.insert(name.clone(), captures.used.clone());
        records.device.insert(name.clone(), expression);
    }
    for (name, definition) in &facts.definitions {
        if let Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) =
            &definition.operator
        {
            let types = &structure_analysis.physical.rows[&symbol_name(&select.input)].types;
            let mut condition = select.condition.clone();
            captures.host_relational(&mut condition, types, &facts.codecs);
            records.host.insert(name.clone(), condition);
        }
    }
    records.captures = captures.records;
    Ok(records)
}

/// Physical definitions with selection constants bound to host captures.
pub(crate) fn host_bound(
    definitions: &BTreeMap<String, Definition>,
    captures: &Captures,
) -> BTreeMap<String, Definition> {
    let mut bound = definitions.clone();
    for (name, definition) in &mut bound {
        if let Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) =
            &mut definition.operator
            && let Some(condition) = captures.host.get(name)
        {
            select.condition = condition.clone();
        }
    }
    bound
}

/// Host evaluation transfers: resident inputs read by host Rust that are not
/// bound to completed publications download complete rows.
fn resident_transfers(
    facts: &Facts,
    dependencies: &BTreeSet<String>,
    bound: &BTreeSet<String>,
) -> Vec<Transfer> {
    let hosts: BTreeSet<_> = dependencies
        .iter()
        .filter(|result| !bound.contains(*result))
        .filter_map(|result| facts.structure_analysis.physical.hosts.get(result))
        .filter(|host| facts.resident.contains(*host))
        .collect();
    hosts
        .into_iter()
        .map(|host| Transfer {
            source: TransferSource::Resident(host.clone()),
            granularity: Granularity::CompleteRows,
        })
        .collect()
}

/// Formal rule GPU-SourceAccess: device sources record which Physical results
/// acquire a device read, from which origin, with which access and phase.
fn sources(facts: &Facts, captures: &Captures) -> syn::Result<BTreeMap<String, SourceRealization>> {
    let analysis = facts.analysis;
    let structure_analysis = facts.structure_analysis;
    // Only checked device consumers require resident source adapters.
    // A host store can retain arbitrary Rust values (for example nullable
    // aggregates); it is not itself a host-to-device publication.
    let required = crate::rows::device_source_results(facts.program, facts.structure_analysis, analysis);
    let host_bound = host_bound(&facts.definitions, captures);
    let demands = crate::rows::lookup_demands(facts.program);
    let mut sources = BTreeMap::new();
    for (name, definition) in &facts.definitions {
        let rename = matches!(definition.operator, Operator::Rename(_));
        if !rename && (facts.device_results.contains(name) || !required.contains(name)) {
            continue;
        }
        let dependencies = crate::rows::host_dependencies(name, &facts.definitions);
        if !rename
            && dependencies
                .iter()
                .any(|result| facts.device_results.contains(result))
        {
            continue;
        }
        let needs_host = dependencies.iter().any(|result| {
            structure_analysis
                .physical
                .hosts
                .get(result)
                .is_some_and(|host| !facts.resident.contains(host))
        });
        let (origin, columns, selection) = if needs_host {
            if !required.contains(name) {
                continue;
            }
            let evaluation = HostEvaluation {
                bound: Vec::new(),
                transfers: resident_transfers(facts, &dependencies, &BTreeSet::new()),
            };
            let columns = structure_analysis.layouts[name]
                .iter()
                .map(|column| (column.clone(), column.clone()))
                .collect();
            (SourceOrigin::Uploaded { evaluation }, columns, None)
        } else {
            let Some(origin) = crate::rows::origin(name, &host_bound)? else {
                continue;
            };
            // A predicate-filtered result is computed from its run as row work.
            if origin.predicate {
                continue;
            }
            if facts.runs.contains(&origin.host) {
                // Packed runs have no interval-view ABI; the selection stays row work.
                if origin.selection.is_some() {
                    continue;
                }
                let mut providers = Vec::new();
                if let Some(layouts) = structure_analysis.built.get(name) {
                    if layouts.len() != 1
                        || structure_analysis.facts(&layouts[0].1).abi
                            == Some(cq_lang_structures::provider::NativeAbi::RadixRelation)
                    {
                        providers.push(name.clone());
                    }
                } else {
                    for ((result, reader, keys), ty) in &structure_analysis.fallbacks {
                        if result != name || matches!(reader, cq_lang_structures::ReaderRef::Scan) {
                            continue;
                        }
                        let mut sorted = keys.clone();
                        sorted.sort();
                        if !demands.contains(&(result.clone(), sorted.clone())) {
                            continue;
                        }
                        if structure_analysis.facts(ty).abi
                            != Some(cq_lang_structures::provider::NativeAbi::RadixRelation)
                        {
                            return Err(error(
                                "GPU-RadixRelation: native lookup requires a declared run layout",
                            ));
                        }
                        let provider = format!("{name}__{}", sorted.join("__"));
                        if !providers.contains(&provider) {
                            providers.push(provider);
                        }
                    }
                }
                sources.insert(
                    name.clone(),
                    SourceRealization {
                        origin: SourceOrigin::RadixRelation {
                            host: origin.host.clone(),
                            providers,
                        },
                        columns: origin.columns.clone(),
                        access: SourceAccess::Scan,
                        phase: Phase::Setup,
                    },
                );
                continue;
            }
            let selection = origin.selection.clone();
            (
                SourceOrigin::Resident {
                    host: origin.host.clone(),
                },
                origin.columns.clone(),
                selection,
            )
        };
        let built = !facts.device_results.contains(name)
            && !facts.published.contains(name)
            && analysis
                .gpu_bindings
                .providers
                .get(name)
                .is_some_and(|layouts| {
                    layouts.iter().any(|layout| layout.abi != ProviderAbi::Host)
                })
            && structure_analysis.built.contains_key(name);
        let access = if built {
            SourceAccess::Provider { selection }
        } else if let Some(selection) = selection {
            SourceAccess::Positions(selection)
        } else {
            SourceAccess::Scan
        };
        let phase = if matches!(origin, SourceOrigin::Uploaded { .. }) {
            Phase::Build
        } else {
            Phase::Setup
        };
        sources.insert(
            name.clone(),
            SourceRealization {
                origin,
                columns,
                access,
                phase,
            },
        );
    }
    Ok(sources)
}

pub(crate) fn grouped_boundary<'a>(
    term: &'a Term,
    definitions: &BTreeMap<String, &'a Term>,
) -> Option<(&'a syn::Ident, &'a Term)> {
    match term {
        Term::Group(name, input) => Some((name, input)),
        Term::Store { inner, .. } | Term::Canon(inner) => grouped_boundary(inner, definitions),
        Term::Ref(name) => grouped_boundary(definitions.get(&symbol_name(name))?, definitions),
        _ => None,
    }
}

fn host_union<'a>(term: &'a Term, definitions: &BTreeMap<String, &'a Term>) -> Option<&'a [Term]> {
    match term {
        Term::Union(arms) => Some(arms),
        Term::Store { inner, .. } | Term::Canon(inner) => host_union(inner, definitions),
        Term::Ref(name) => host_union(definitions.get(&symbol_name(name))?, definitions),
        _ => None,
    }
}

struct Stored {
    name: String,
    result: String,
    range: bool,
    packed: bool,
}

/// One chain's realization-relevant shape: its source or completed
/// publication, its row maps, and whether any operation follows the source.
struct Shape {
    source: String,
    stored: Option<usize>,
    result: String,
    maps: Vec<(String, String)>,
    operations: bool,
}

/// Bounds state of device columns as publications complete in order.
#[derive(Clone, Default)]
struct BoundsState {
    columns: BTreeMap<String, (BaseBounds, Vec<BoundSource>)>,
    measured: BTreeSet<String>,
}

struct Steps<'f, 'a> {
    facts: &'f Facts<'a>,
    sources: &'f BTreeMap<String, SourceRealization>,
    stored: Vec<Stored>,
    uploaded: BTreeSet<String>,
    downloaded: BTreeSet<String>,
    bounds: BoundsState,
}

impl<'f, 'a> Steps<'f, 'a> {
    fn new(facts: &'f Facts<'a>, sources: &'f BTreeMap<String, SourceRealization>) -> Self {
        let mut bounds = BoundsState::default();
        // Run reads carry Scalar ranges; a provider key over them is measured.
        for source in sources.values() {
            let SourceOrigin::RadixRelation { host, .. } = &source.origin else {
                continue;
            };
            for original in &facts.structure_analysis.host_layouts[host] {
                let Some((column, _)) = source.columns.iter().find(|(_, o)| *o == original) else {
                    continue;
                };
                let ty = facts.carriers[column].clone();
                bounds.measured.insert(column.clone());
                bounds
                    .columns
                    .insert(column.clone(), (BaseBounds::Scalar(ty), Vec::new()));
            }
        }
        let mut origins: BTreeMap<String, BTreeSet<(String, usize, bool)>> = BTreeMap::new();
        for (name, source) in sources {
            let (host, layout, upload) = match &source.origin {
                SourceOrigin::Resident { host } => {
                    (host, &facts.structure_analysis.host_layouts[host], false)
                }
                SourceOrigin::Uploaded { .. } => {
                    (name, &facts.structure_analysis.layouts[name], true)
                }
                SourceOrigin::RadixRelation { .. } => continue,
            };
            for (column, original) in &source.columns {
                let index = layout
                    .iter()
                    .position(|c| c == original)
                    .expect("source column in layout");
                origins
                    .entry(column.clone())
                    .or_default()
                    .insert((host.clone(), index, upload));
            }
        }
        for (column, origins) in origins {
            bounds.measured.remove(&column);
            let union = origins
                .into_iter()
                .map(|(host, index, upload)| {
                    if upload {
                        BoundSource::Upload {
                            publication: host,
                            index,
                        }
                    } else {
                        BoundSource::Resident { host, index }
                    }
                })
                .collect();
            bounds
                .columns
                .insert(column, (BaseBounds::Union(union), Vec::new()));
        }
        for row in facts.structure_analysis.physical.rows.values() {
            for column in &row.computed {
                bounds.columns.entry(column.clone()).or_insert_with(|| {
                    (
                        BaseBounds::Scalar(facts.carriers[column].clone()),
                        Vec::new(),
                    )
                });
            }
        }
        Self {
            facts,
            sources,
            stored: Vec::new(),
            uploaded: BTreeSet::new(),
            downloaded: BTreeSet::new(),
            bounds,
        }
    }

    fn has_source(&self, name: &str) -> bool {
        self.sources.contains_key(name) || self.uploaded.contains(name)
    }

    fn shapes(&self, term: &Term) -> syn::Result<Vec<Shape>> {
        let facts = self.facts;
        Ok(match term {
            Term::Rows(name) => vec![Shape {
                source: symbol_name(name),
                stored: None,
                result: symbol_name(name),
                maps: Vec::new(),
                operations: false,
            }],
            Term::Ref(name) => {
                let name = symbol_name(name);
                if let Some(index) = self.stored.iter().position(|stored| stored.name == name) {
                    return Ok(vec![Shape {
                        source: name,
                        stored: Some(index),
                        result: self.stored[index].result.clone(),
                        maps: Vec::new(),
                        operations: false,
                    }]);
                }
                if facts.is_host_store(&name) {
                    let result = facts
                        .analysis
                        .results
                        .get(&name)
                        .ok_or_else(|| error("host store lacks checked row provenance"))?
                        .clone();
                    return Ok(vec![Shape {
                        source: result.clone(),
                        stored: None,
                        result,
                        maps: Vec::new(),
                        operations: false,
                    }]);
                }
                let term = facts
                    .sequence
                    .get(&name)
                    .ok_or_else(|| error("Sequence reference lacks definition"))?;
                let mut shapes = self.shapes(term)?;
                if let Some(result) = facts.analysis.results.get(&name) {
                    for shape in &mut shapes {
                        shape.result.clone_from(result);
                    }
                }
                shapes
            }
            Term::Union(arms) => {
                let mut shapes = Vec::new();
                for arm in arms {
                    shapes.extend(self.shapes(arm)?);
                }
                shapes
            }
            Term::Choose(arms) => {
                let mut shapes = Vec::new();
                for (_, arm) in arms {
                    shapes.extend(self.shapes(arm)?);
                }
                shapes
            }
            Term::Compact(name, inner) if matches!(inner.as_ref(), Term::Lookup { .. }) => {
                let Term::Lookup { inner, .. } = inner.as_ref() else {
                    unreachable!()
                };
                let mut shapes = self.shapes(inner)?;
                for shape in &mut shapes {
                    shape.result = symbol_name(name);
                    shape.operations = true;
                }
                shapes
            }
            Term::Map(name, inner) | Term::Compact(name, inner) => {
                let mut shapes = self.shapes(inner)?;
                for shape in &mut shapes {
                    shape.maps.push((symbol_name(name), shape.result.clone()));
                    shape.result = symbol_name(name);
                    shape.operations = true;
                }
                shapes
            }
            Term::Lookup {
                provider,
                keys,
                inner,
            } => {
                let mut shapes = self.shapes(inner)?;
                let provider = symbol_name(provider);
                let mut keys: Vec<_> = keys.iter().map(symbol_name).collect();
                keys.sort();
                let membership = facts
                    .analysis
                    .capabilities
                    .get(&(provider.clone(), keys.clone()))
                    == Some(&Capability::Membership);
                for shape in &mut shapes {
                    if !membership {
                        let result = facts.definitions.values().find_map(|definition| {
                            match &definition.operator {
                                Operator::Probe(_, keyed) | Operator::Merge(_, keyed)
                                    if symbol_name(&keyed.right) == provider
                                        && symbol_name(&keyed.left) == shape.result =>
                                {
                                    let mut read: Vec<_> =
                                        keyed.columns.iter().map(symbol_name).collect();
                                    read.sort();
                                    (read == keys).then(|| symbol_name(&definition.name))
                                }
                                _ => None,
                            }
                        });
                        shape.result = result.ok_or_else(|| {
                            error("Sequence lookup lacks checked Physical row provenance")
                        })?;
                    }
                    shape.operations = true;
                }
                shapes
            }
            Term::Group(..) | Term::PartitionK { .. } => {
                return Err(error("group or key partition is not a dispatch chain"));
            }
            Term::Flatten(inner)
            | Term::Canon(inner)
            | Term::Store { inner, .. }
            | Term::Tile { inner, .. }
            | Term::Fold(_, inner) => {
                let mut shapes = self.shapes(inner)?;
                for shape in &mut shapes {
                    shape.operations = true;
                }
                shapes
            }
        })
    }

    /// Host evaluation of `result` after the completed publications so far:
    /// the last publication of each result binds it; newly read publications
    /// and resident inputs download complete rows once.
    fn evaluation(&mut self, result: &str) -> syn::Result<HostEvaluation> {
        let mut last = BTreeMap::new();
        for (index, stored) in self.stored.iter().enumerate() {
            last.insert(stored.result.clone(), index);
        }
        let bindings: BTreeMap<_, _> = last
            .keys()
            .map(|result| (result.clone(), format_ident!("__cq_bound")))
            .collect();
        let dependencies =
            crate::rows::host_dependencies_from(result, &self.facts.definitions, &bindings);
        let mut transfers = Vec::new();
        let mut bound = Vec::new();
        for (index, stored) in self.stored.iter().enumerate() {
            if last.get(&stored.result) != Some(&index) {
                continue;
            }
            bound.push(stored.name.clone());
            if !dependencies.contains(&stored.result) {
                continue;
            }
            if stored.range {
                return Err(error(
                    "host consumer requires a materialized row publication",
                ));
            }
            if self.downloaded.insert(stored.name.clone()) {
                transfers.push(Transfer {
                    source: TransferSource::Publication(stored.name.clone()),
                    granularity: Granularity::CompleteRows,
                });
            }
        }
        let bound_results = last.keys().cloned().collect();
        transfers.extend(resident_transfers(
            self.facts,
            &dependencies,
            &bound_results,
        ));
        Ok(HostEvaluation { bound, transfers })
    }

    /// An upload publishes its layout columns; their bounds merge pairwise.
    fn upload(&mut self, result: &str) {
        self.uploaded.insert(result.to_owned());
        for (index, column) in self.facts.structure_analysis.layouts[result]
            .iter()
            .enumerate()
        {
            let source = BoundSource::Upload {
                publication: result.to_owned(),
                index,
            };
            self.bounds
                .columns
                .entry(column.clone())
                .or_insert_with(|| (BaseBounds::None, Vec::new()))
                .1
                .push(source);
        }
    }

    /// GPU-Realization: a provider has one owner. A nonterminal publication
    /// builds its result's providers when it precedes every reader; a reader
    /// before it is served by the source-built provider, and the publication
    /// builds none.
    fn owned_provider(
        &self,
        publication: &str,
        result: &str,
        terminal: bool,
    ) -> syn::Result<Option<ProviderBuild>> {
        if terminal {
            return Ok(None);
        }
        let provider = self.provider(result);
        let read_before = self
            .facts
            .first_reader(result)
            .is_some_and(|reader| Some(reader) < self.facts.position(publication));
        if !read_before || provider.is_none() {
            return Ok(provider);
        }
        if source_builds(self.sources, result) {
            Ok(None)
        } else {
            Err(error("a provider is read before any owner builds it"))
        }
    }

    fn provider(&self, result: &str) -> Option<ProviderBuild> {
        self.facts.structure_analysis.built.get(result)?;
        let layouts = self.facts.analysis.gpu_bindings.providers.get(result)?;
        let selected: Vec<_> = layouts
            .iter()
            .enumerate()
            .filter_map(|(index, layout)| (layout.abi != ProviderAbi::Host).then_some(index))
            .collect();
        if selected.is_empty() {
            return None;
        }
        let mut columns = BTreeMap::new();
        for &index in &selected {
            let layout = &layouts[index];
            for column in layout.keys.iter().chain(&layout.payload) {
                let Some((base, merges)) = self.bounds.columns.get(column) else {
                    continue;
                };
                columns.insert(
                    column.clone(),
                    DescriptorBounds {
                        base: base.clone(),
                        merges: merges.clone(),
                        measure: self.bounds.measured.contains(column)
                            && layout.keys.contains(column),
                    },
                );
            }
        }
        Some(ProviderBuild {
            layouts: selected,
            columns,
        })
    }

    fn push(&mut self, name: &str, result: String, range: bool) {
        let packed = self
            .facts
            .analysis
            .gpu_publications
            .get(name)
            .is_some_and(|record| record.storage.codec().is_some());
        self.stored.push(Stored {
            name: name.to_owned(),
            result,
            range,
            packed,
        });
    }

    fn group(&mut self, name: &syn::Ident, input: &Term) -> syn::Result<TerminalGroup> {
        let facts = self.facts;
        let shapes = self.shapes(input)?;
        let [shape] = shapes.as_slice() else {
            return Err(error(
                "GPU group requires one canonical valuation publication",
            ));
        };
        let result = symbol_name(name);
        let Operator::Group(group) = &facts
            .definitions
            .get(&result)
            .ok_or_else(|| error("GPU group lacks checked Physical provenance"))?
            .operator
        else {
            return Err(error("GPU group provenance is not a Physical group"));
        };
        let mut host = shape.stored.is_none() && !self.has_source(&shape.source);
        for (map, input) in &shape.maps {
            for definition in facts.map_definitions(map, input)? {
                host |= facts.analysis.rust_requires_host(&facts.structure_analysis.physical, &definition.operator);
            }
        }
        let input = if host {
            let input = symbol_name(&group.input);
            let upload = if self.has_source(&input) {
                None
            } else {
                let evaluation = self.evaluation(&input)?;
                self.upload(&input);
                Some(evaluation)
            };
            GroupInput::Host { input, upload }
        } else if let Some(index) = shape.stored {
            let stored = &self.stored[index];
            if !shape.operations && stored.packed {
                GroupInput::Canonical {
                    publication: stored.name.clone(),
                }
            } else {
                GroupInput::Stored {
                    publication: stored.name.clone(),
                }
            }
        } else {
            GroupInput::Source {
                source: shape.source.clone(),
            }
        };
        let completion = if group.keys.is_empty() {
            GroupCompletion::GridPartials {
                partials: Grid::KernelOccupancy,
                geometry: partials_geometry(),
                combine: partials_geometry(),
            }
        } else {
            GroupCompletion::Segmented {
                arguments: Grid::KernelOccupancy,
                geometry: BlockGeometry {
                    block_threads: crate::BLOCK_THREADS,
                    shared_bytes: 0,
                },
            }
        };
        Ok(TerminalGroup {
            group: result,
            input,
            completion,
        })
    }

    fn run(mut self) -> syn::Result<(Vec<(String, Step)>, Option<HostOutput>)> {
        let facts = self.facts;
        let analysis = facts.analysis;
        let final_name = facts.final_name()?;
        let final_group = facts
            .sequence
            .get(&final_name)
            .and_then(|term| grouped_boundary(term, &facts.sequence));
        let device_sources = crate::rows::device_source_results(facts.program, facts.structure_analysis, analysis);
        let mut steps = Vec::new();
        let mut output = false;
        for definition in &facts.program.sequence.definitions {
            let publication = symbol_name(&definition.name);
            let terminal = publication == final_name;
            let host_level = analysis
                .levels
                .get(&publication)
                .is_some_and(|level| level.last().is_some_and(|level| level == "host"));
            let step = if !terminal
                && final_group.is_some_and(|(group, _)| {
                    grouped_boundary(&definition.term, &facts.sequence)
                        .is_some_and(|(other, _)| other == group)
                }) {
                Step::GroupAlias
            } else if let (true, Some((name, input))) = (
                terminal,
                grouped_boundary(&definition.term, &facts.sequence),
            ) {
                output = true;
                Step::Group(self.group(name, input)?)
            } else if let Some(dispatch) = analysis.dispatches.iter().find(|dispatch| {
                dispatch.inner_stores.iter().rev().find(|name| {
                    facts
                        .sequence
                        .get(*name)
                        .is_some_and(|term| matches!(term, Term::Store { .. }))
                }) == Some(&publication)
            }) {
                let Term::Store { inner, .. } = definition.term.clone() else {
                    return Err(error(
                        "GPU dispatch publication must be an explicit outer store",
                    ));
                };
                let result = match analysis.results.get(&publication) {
                    Some(result) => result.clone(),
                    None => self
                        .shapes(&inner)?
                        .first()
                        .ok_or_else(|| error("GPU dispatch has no source"))?
                        .result
                        .clone(),
                };
                let range = matches!(
                    dispatch
                        .gpu
                        .as_ref()
                        .map(|execution| &execution.publication),
                    Some(Publication::DeferredRange { .. })
                );
                output |= terminal;
                let provider = self.owned_provider(&publication, &result, terminal)?;
                self.push(&publication, result.clone(), range);
                Step::Dispatch { result, provider }
            } else if host_level
                && matches!(definition.term, Term::Store { .. })
                && let Some(arms) = host_union(&definition.term, &facts.sequence)
            {
                let result = analysis.results[&publication].clone();
                let mut records = Vec::new();
                for arm in arms {
                    for shape in self.shapes(arm)? {
                        if shape.operations || !shape.maps.is_empty() {
                            return Err(error("host union operands must be published stores"));
                        }
                        let index = shape
                            .stored
                            .ok_or_else(|| error("host union operand has not published"))?;
                        let stored = &self.stored[index];
                        if stored.range {
                            return Err(error(
                                "host union operand requires a materialized row publication",
                            ));
                        }
                        records.push(if stored.packed {
                            UnionArm::Decode {
                                publication: stored.name.clone(),
                                grid: elements(),
                            }
                        } else {
                            UnionArm::Copy {
                                publication: stored.name.clone(),
                            }
                        });
                    }
                }
                let codec = analysis
                    .gpu_publications
                    .get(&publication)
                    .is_some_and(|record| record.storage.codec().is_some());
                output |= terminal && codec;
                let provider = self.owned_provider(&publication, &result, terminal)?;
                self.push(&publication, result, false);
                Step::HostUnion(HostUnion {
                    arms: records,
                    pack: codec.then(elements),
                    provider,
                })
            } else if matches!(definition.term, Term::Store { .. })
                && host_level
                && let Some(result) = analysis.results.get(&publication)
                && device_sources.contains(result)
                && !self.has_source(result)
            {
                let evaluation = self.evaluation(result)?;
                self.upload(result);
                Step::Upload {
                    evaluation,
                    provider: None,
                }
            } else {
                Step::Host
            };
            steps.push((publication, step));
        }
        let output = if output {
            None
        } else {
            let mut name = final_name.clone();
            loop {
                if facts.is_host_store(&name) {
                    break analysis.results.get(&name).map(|source| HostOutput {
                        source: source.clone(),
                        pack: elements(),
                    });
                }
                let Some(Term::Store { inner, .. }) = facts.sequence.get(&name).copied() else {
                    break None;
                };
                let Term::Ref(parent) = inner.as_ref() else {
                    break None;
                };
                name = symbol_name(parent);
                if !facts.sequence.contains_key(&name) {
                    break None;
                }
            }
        };
        Ok((steps, output))
    }
}

/// Validate supplied realization records against checked facts and their
/// stated invariants. The checker never calls `select` or its helpers; it
/// accepts any sound record.
pub(crate) fn check(
    physical: &cq_lang_structures::physical::Program,
    structures: &cq_lang_structures::Program,
    program: &Program,
    structure_analysis: &cq_lang_structures::Analysis,
    analysis: &Analysis,
    records: &Realization,
) -> syn::Result<()> {
    let query = &physical.program.captures;
    if query
        .items
        .iter()
        .any(|item| matches!(item, cq_syntax::Capture::Guard(_)))
    {
        let facts = Facts::new(physical, structures, program, structure_analysis, analysis)?;
        if facts
            .sequence
            .get(&facts.final_name()?)
            .and_then(|term| grouped_boundary(term, &facts.sequence))
            .is_some()
        {
            return Err(error(
                "T-Gpu: an answer gate over a grouped publication is unsupported",
            ));
        }
    }
    if records.rejection.is_some() {
        return Ok(());
    }
    if records.captures.query != *query {
        return Err(error(
            "GPU-Capture: query captures differ from checked source order",
        ));
    }
    let demands = crate::rows::device_lookup_demands(program, analysis);
    if demands
        .iter()
        .any(|(name, keys)| keys.is_empty() && !records.whole_lookups.contains(name))
        || records
            .whole_lookups
            .iter()
            .any(|name| !demands.contains(&(name.clone(), Vec::new())))
    {
        return Err(error(
            "whole-relation lookup records must cover exactly zero-key reads",
        ));
    }
    if !records.auxiliary.as_ref().is_some_and(supported_auxiliary) {
        return Err(error(
            "auxiliary kernel geometry is missing or unsupported by the runtime families",
        ));
    }
    let facts = Facts::new(physical, structures, program, structure_analysis, analysis)?;
    crate::capture::check(
        &records.captures,
        &facts.definitions,
        structure_analysis,
        analysis,
        &facts.codecs,
    )?;
    // Formal rule GPU-ProgramLaunch: every dispatch records its preparation;
    // a one-block preparation requires one root part. Program caches bind the
    // module/context and opaque compiled CUDA entry, never a Rust type spelling.
    if records.launches.len() != analysis.dispatches.len() {
        return Err(error("GPU-ProgramLaunch: every dispatch needs a launch preparation"));
    }
    for (launch, dispatch) in records.launches.iter().zip(&analysis.dispatches) {
        if *launch == Launch::ProgramOne
            && !matches!(dispatch.gpu.as_ref().expect("checked GPU dispatch").partition, cq_lang_sequence::gpu::Partition::One)
        {
            return Err(error("GPU-ProgramLaunch: one-block preparation requires the one-part partition"));
        }
    }
    check_smallest(&facts, &records.smallest)?;
    let host_bound = host_bound(&facts.definitions, &records.captures);
    // Element templates (union decode/pack, host-output pack) are compiled
    // for blocks of the collective width.
    let elements = |grid: &Grid| {
        *grid
            == Grid::Elements {
                block_threads: crate::BLOCK_THREADS,
            }
    };
    for (name, source) in &records.sources {
        if !facts.definitions.contains_key(name) {
            return Err(error("source realizes an unknown Physical result"));
        }
        let dependencies = crate::rows::host_dependencies(name, &facts.definitions);
        let nonresident = dependencies.iter().any(|result| {
            structure_analysis
                .physical
                .hosts
                .get(result)
                .is_some_and(|host| !facts.resident.contains(host))
        });
        let layout = match &source.origin {
            SourceOrigin::Resident { host } | SourceOrigin::RadixRelation { host, .. } => {
                if !facts.resident.contains(host) || nonresident {
                    return Err(error("resident source reads a nonresident input"));
                }
                if let SourceOrigin::RadixRelation { providers, .. } = &source.origin {
                    if !facts.runs.contains(host) || source.access != SourceAccess::Scan {
                        return Err(error("packed runs have no interval or provider access"));
                    }
                    if providers
                        .iter()
                        .any(|provider| !analysis.gpu_bindings.native.contains_key(provider))
                    {
                        return Err(error("run source names an unrecorded native lookup"));
                    }
                }
                &structure_analysis.host_layouts[host]
            }
            SourceOrigin::Uploaded { evaluation } => {
                if !nonresident || source.phase != Phase::Build || !evaluation.bound.is_empty() {
                    return Err(error(
                        "upload requires a nonresident host dependency at build",
                    ));
                }
                CheckState::default().evaluation(&facts, evaluation, name)?;
                &structure_analysis.layouts[name]
            }
        };
        if (source.phase == Phase::Build) != matches!(source.origin, SourceOrigin::Uploaded { .. })
        {
            return Err(error(
                "only uploaded sources are constructed during the build",
            ));
        }
        if source
            .columns
            .values()
            .any(|original| !layout.contains(original))
        {
            return Err(error("source column is absent from its origin layout"));
        }
        let selection = match &source.access {
            SourceAccess::Scan => None,
            SourceAccess::Positions(interval) => Some(interval),
            SourceAccess::Provider { selection } => {
                let built = analysis
                    .gpu_bindings
                    .providers
                    .get(name)
                    .is_some_and(|layouts| {
                        layouts.iter().any(|layout| {
                            matches!(layout.abi, ProviderAbi::Range | ProviderAbi::Radix)
                        })
                    });
                if !built {
                    return Err(error(
                        "provider access without a recorded Range or Radix layout",
                    ));
                }
                // One owner per provider: a publication step building the
                // same result's providers excludes a source-built provider.
                if step_builds(&records.steps, analysis, name) {
                    return Err(error(
                        "provider access duplicates a publication's provider build",
                    ));
                }
                selection.as_ref()
            }
        };
        if let SourceOrigin::Resident { host } = &source.origin {
            check_resident_chain(&host_bound, name, host, &source.columns, selection)?;
        } else if selection.is_some() {
            return Err(error("only resident sources fold interval selections"));
        }
    }
    let definitions: Vec<_> = program
        .sequence
        .definitions
        .iter()
        .map(|d| symbol_name(&d.name))
        .collect();
    if records
        .steps
        .iter()
        .map(|(name, _)| name)
        .ne(definitions.iter())
    {
        return Err(error("steps differ from the Sequence definitions"));
    }
    let final_name = facts.final_name()?;
    let final_group = facts
        .sequence
        .get(&final_name)
        .and_then(|term| grouped_boundary(term, &facts.sequence));
    let mut state = CheckState::default();
    for (name, step) in &records.steps {
        let term = facts.sequence[name];
        for source in crate::rows::lookup_providers(term) {
            if records.whole_lookups.contains(&source)
                && !records.sources.contains_key(&source)
                && !state.uploads.contains(&source)
                && !state
                    .published
                    .iter()
                    .any(|published| published.result == source)
            {
                return Err(error(
                    "whole-relation lookup requires a source or a preceding completed publication",
                ));
            }
        }
        let terminal = *name == final_name;
        let dispatch = analysis.dispatches.iter().find(|dispatch| {
            dispatch.inner_stores.iter().rev().find(|store| {
                facts
                    .sequence
                    .get(*store)
                    .is_some_and(|term| matches!(term, Term::Store { .. }))
            }) == Some(name)
        });
        match step {
            Step::GroupAlias => {
                let same = final_group.is_some_and(|(group, _)| {
                    grouped_boundary(term, &facts.sequence).is_some_and(|(other, _)| other == group)
                });
                if !same || terminal {
                    return Err(error("group alias does not name the terminal group"));
                }
            }
            Step::Group(group) => {
                let Some((ident, _)) = grouped_boundary(term, &facts.sequence).filter(|_| terminal)
                else {
                    return Err(error(
                        "terminal group step is not the terminal grouped publication",
                    ));
                };
                let Some(Operator::Group(physical)) =
                    facts.definitions.get(&group.group).map(|d| &d.operator)
                else {
                    return Err(error("terminal group lacks its Physical group"));
                };
                if symbol_name(ident) != group.group {
                    return Err(error("terminal group names another Physical group"));
                }
                // The group templates are compiled for occupancy grids of
                // collective-width blocks; grid partials hold one value and
                // one presence word per lane, as does each combine block.
                let collective =
                    |geometry: &BlockGeometry| geometry.block_threads == crate::BLOCK_THREADS;
                let partial = |geometry: &BlockGeometry| {
                    collective(geometry)
                        && geometry.shared_bytes == geometry.block_threads * PARTIAL_LANE_BYTES
                };
                match (&group.completion, physical.keys.is_empty()) {
                    (
                        GroupCompletion::GridPartials {
                            partials: Grid::KernelOccupancy,
                            geometry,
                            combine,
                        },
                        true,
                    ) if partial(geometry) && partial(combine) => {}
                    (
                        GroupCompletion::Segmented {
                            arguments: Grid::KernelOccupancy,
                            geometry,
                        },
                        false,
                    ) if collective(geometry) && geometry.shared_bytes == 0 => {}
                    _ => {
                        return Err(error(
                            "keyless groups combine grid partials and keyed groups use the accumulator, over their template geometry",
                        ));
                    }
                }
                match &group.input {
                    GroupInput::Canonical { publication } => {
                        let canonical = analysis
                            .gpu_publications
                            .get(publication)
                            .is_some_and(|record| record.storage.canonical_provider().is_some());
                        if state.publication(publication).is_none() || !canonical {
                            return Err(error(
                                "canonical group input is not a completed canonical publication",
                            ));
                        }
                    }
                    GroupInput::Stored { publication } => {
                        if state.publication(publication).is_none() {
                            return Err(error("group input publication has not completed"));
                        }
                    }
                    GroupInput::Source { source } => {
                        if !records.sources.contains_key(source)
                            && !uploaded(records, analysis, source)
                        {
                            return Err(error("group source has no device realization"));
                        }
                    }
                    GroupInput::Host { input, upload } => {
                        if *input != symbol_name(&physical.input) {
                            return Err(error("host group input is not the Physical group input"));
                        }
                        if let Some(evaluation) = upload {
                            state.evaluation(&facts, evaluation, input)?;
                            state.uploads.push(input.clone());
                        } else if !records.sources.contains_key(input)
                            && !uploaded(records, analysis, input)
                        {
                            return Err(error(
                                "host group input is neither uploaded nor a device source",
                            ));
                        }
                    }
                }
            }
            Step::Dispatch { result, provider } => {
                let Some(dispatch) = dispatch else {
                    return Err(error("dispatch step without a checked dispatch"));
                };
                if analysis
                    .results
                    .get(name)
                    .is_some_and(|known| known != result)
                    || !facts.definitions.contains_key(result)
                {
                    return Err(error("dispatch publishes another Physical result"));
                }
                state.provider(&facts, records, name, result, provider.as_ref(), terminal)?;
                let range = matches!(
                    dispatch
                        .gpu
                        .as_ref()
                        .map(|execution| &execution.publication),
                    Some(Publication::DeferredRange { .. })
                );
                state.publish(name, result, range);
            }
            Step::HostUnion(union) => {
                if dispatch.is_some()
                    || !facts.host_store_term(name)
                    || !analysis.results.contains_key(name)
                {
                    return Err(error("host union step without a host union store"));
                }
                let Some(arms) = host_union(term, &facts.sequence) else {
                    return Err(error("host union step without a host union store"));
                };
                // Arms are the union's operand publications, in order and
                // with multiplicity; each copies or decodes per its storage.
                let mut operands = Vec::new();
                for arm in arms {
                    union_operands(arm, &facts, &state, &mut operands)?;
                }
                if operands.len() != union.arms.len() {
                    return Err(error("union arms differ from the union's operands"));
                }
                for (arm, operand) in union.arms.iter().zip(&operands) {
                    let (publication, packed) = match arm {
                        UnionArm::Copy { publication } => (publication, false),
                        UnionArm::Decode { publication, grid } => {
                            if !elements(grid) {
                                return Err(error("union decode uses its element template"));
                            }
                            (publication, true)
                        }
                    };
                    let codec = analysis
                        .gpu_publications
                        .get(publication)
                        .is_some_and(|record| record.storage.codec().is_some());
                    let materialized = state
                        .publication(publication)
                        .is_some_and(|published| !published.range);
                    if publication != operand || !materialized || codec != packed {
                        return Err(error(
                            "union arm realization contradicts its operand publication",
                        ));
                    }
                    let staging = analysis.gpu_publications[name].union_staging.as_ref()
                        .ok_or_else(|| error("union lacks its recorded staging layout"))?;
                    match &analysis.gpu_publications[publication].storage {
                        cq_lang_sequence::gpu::publication::PublicationStorage::FlatRows(source)
                            if source == staging => {}
                        cq_lang_sequence::gpu::publication::PublicationStorage::PackedKeys(source)
                            if source.codec.fields().iter().all(|field| staging.columns.contains(&field.column))
                                && staging.columns.iter().all(|column| source.codec.fields().iter().any(|field| &field.column == column)) => {}
                        _ => return Err(error("union consumer layout differs from its published fields or stride")),
                    }
                }
                let codec = analysis
                    .gpu_publications
                    .get(name)
                    .is_some_and(|record| record.storage.codec().is_some());
                if codec != union.pack.is_some()
                    || union.pack.as_ref().is_some_and(|grid| !elements(grid))
                {
                    return Err(error("union packing contradicts its recorded codec"));
                }
                let result = &analysis.results[name];
                state.provider(
                    &facts,
                    records,
                    name,
                    result,
                    union.provider.as_ref(),
                    terminal,
                )?;
                state.publish(name, result, false);
            }
            Step::Upload {
                evaluation,
                provider,
            } => {
                if dispatch.is_some() || !facts.host_store_term(name) {
                    return Err(error("upload step without a host store"));
                }
                let Some(result) = analysis.results.get(name) else {
                    return Err(error("upload step without a host store"));
                };
                state.evaluation(&facts, evaluation, result)?;
                state.uploads.push(result.clone());
                if let Some(provider) = provider {
                    state.provider_bounds(&facts, records, result, provider)?;
                }
            }
            Step::Host => {
                if dispatch.is_some() {
                    return Err(error("checked dispatch has no realization step"));
                }
            }
        }
    }
    if let Some(output) = &records.output {
        if !elements(&output.pack) || !facts.definitions.contains_key(&output.source) {
            return Err(error("host output names no Physical source"));
        }
    }
    Ok(())
}

struct Published {
    name: String,
    result: String,
    range: bool,
}

/// Completed publications, downloads and uploads as the checked steps run.
#[derive(Default)]
struct CheckState {
    published: Vec<Published>,
    downloaded: BTreeSet<String>,
    /// Uploaded Physical results, in order.
    uploads: Vec<String>,
}

impl CheckState {
    fn publication(&self, name: &str) -> Option<&Published> {
        self.published
            .iter()
            .find(|published| published.name == name)
    }

    fn publish(&mut self, name: &str, result: &str, range: bool) {
        self.published.push(Published {
            name: name.to_owned(),
            result: result.to_owned(),
            range,
        });
    }

    /// GPU-Transfer: a host evaluation of `result` binds only the latest
    /// completed publication of each result and binds every one it depends
    /// on; it downloads exactly the bound publications it reads that no
    /// earlier evaluation downloaded (complete rows, materialized only), and
    /// exactly the resident inputs its unbound dependencies read.
    fn evaluation(
        &mut self,
        facts: &Facts,
        evaluation: &HostEvaluation,
        result: &str,
    ) -> syn::Result<()> {
        let latest = |published: &Published| {
            self.published
                .iter()
                .rev()
                .find(|other| other.result == published.result)
                .is_some_and(|other| other.name == published.name)
        };
        let mut bound = BTreeMap::new();
        for name in &evaluation.bound {
            let Some(published) = self.publication(name).filter(|p| latest(p)) else {
                return Err(error(
                    "host evaluation binds an incomplete or superseded publication",
                ));
            };
            if bound
                .insert(published.result.clone(), name.clone())
                .is_some()
            {
                return Err(error("host evaluation binds one result twice"));
            }
        }
        let bindings: BTreeMap<_, _> = bound
            .keys()
            .map(|result| (result.clone(), format_ident!("__cq_bound")))
            .collect();
        let dependencies =
            crate::rows::host_dependencies_from(result, &facts.definitions, &bindings);
        if self.published.iter().any(|published| {
            dependencies.contains(&published.result) && !bound.contains_key(&published.result)
        }) {
            return Err(error(
                "host evaluation re-evaluates a completed publication instead of binding it",
            ));
        }
        let mut residents = BTreeSet::new();
        for transfer in &evaluation.transfers {
            let Granularity::CompleteRows = transfer.granularity;
            match &transfer.source {
                TransferSource::Publication(name) => {
                    let read = self.publication(name).is_some_and(|published| {
                        !published.range
                            && bound.get(&published.result) == Some(name)
                            && dependencies.contains(&published.result)
                    });
                    if !read || !self.downloaded.insert(name.clone()) {
                        return Err(error(
                            "transfer downloads an unread, range or already downloaded publication",
                        ));
                    }
                }
                TransferSource::Resident(host) => {
                    if !facts.resident.contains(host) || !residents.insert(host.clone()) {
                        return Err(error("transfer names a nonresident or repeated input"));
                    }
                }
            }
        }
        for (result, name) in &bound {
            if dependencies.contains(result) && !self.downloaded.contains(name) {
                return Err(error(
                    "host evaluation reads a publication it never downloads",
                ));
            }
        }
        let read: BTreeSet<_> = dependencies
            .iter()
            .filter(|dependency| !bound.contains_key(*dependency))
            .filter_map(|dependency| facts.structure_analysis.physical.hosts.get(dependency))
            .filter(|host| facts.resident.contains(*host))
            .cloned()
            .collect();
        if read != residents {
            return Err(error(
                "resident transfers differ from the resident inputs the evaluation reads",
            ));
        }
        Ok(())
    }

    /// A nonterminal publication of a result with recorded GPU provider
    /// layouts builds them unless its source owns them; a terminal one builds
    /// none. The owner precedes every reader: a publication-built provider is
    /// never looked up by an earlier definition.
    fn provider(
        &self,
        facts: &Facts,
        records: &Realization,
        publication: &str,
        result: &str,
        provider: Option<&ProviderBuild>,
        terminal: bool,
    ) -> syn::Result<()> {
        let required = !terminal
            && facts.structure_analysis.built.contains_key(result)
            && facts
                .analysis
                .gpu_bindings
                .providers
                .get(result)
                .is_some_and(|layouts| {
                    layouts.iter().any(|layout| layout.abi != ProviderAbi::Host)
                })
            && !source_builds(&records.sources, result);
        match provider {
            Some(provider) if required => {
                if facts
                    .first_reader(result)
                    .is_some_and(|reader| Some(reader) < facts.position(publication))
                {
                    return Err(error(
                        "a provider is looked up before its publication builds it",
                    ));
                }
                if let cq_lang_sequence::gpu::publication::PublicationStorage::FlatRows(layout) =
                    &facts.analysis.gpu_publications[publication].storage
                {
                    if provider.columns.keys().any(|column| !layout.columns.contains(column)) {
                        return Err(error("provider consumer reads an unpublished flat column"));
                    }
                }
                self.provider_bounds(facts, records, result, provider)
            }
            None if !required => Ok(()),
            _ => Err(error(
                "provider build recorded exactly for nonterminal publications with provider layouts and no source-built owner",
            )),
        }
    }

    /// GPU-ProviderDomain: descriptor bounds enclose each column. Every named
    /// domain publishes the column (resident origins rename an input column
    /// into it, uploads publish it, Scalar is its carrier); a Union names
    /// every resident or uploaded source origin of the column; the merges are
    /// exactly the earlier uploads publishing it, in order; only key columns
    /// are measured.
    fn provider_bounds(
        &self,
        facts: &Facts,
        records: &Realization,
        result: &str,
        provider: &ProviderBuild,
    ) -> syn::Result<()> {
        let layouts = facts
            .analysis
            .gpu_bindings
            .providers
            .get(result)
            .ok_or_else(|| error("provider build of a result without provider layouts"))?;
        if layouts.iter().enumerate().any(|(index, layout)| {
            provider.layouts.contains(&index) != (layout.abi != ProviderAbi::Host)
        }) || provider.layouts.windows(2).any(|pair| pair[0] >= pair[1])
            || provider.layouts.iter().any(|&index| index >= layouts.len())
        {
            return Err(error(
                "provider build must name exactly its device layouts in order",
            ));
        }
        let keys: BTreeSet<_> = layouts.iter().flat_map(|layout| &layout.keys).collect();
        let columns: BTreeSet<_> = layouts
            .iter()
            .flat_map(|layout| layout.keys.iter().chain(&layout.payload))
            .collect();
        for (column, bounds) in &provider.columns {
            if !columns.contains(column) || (bounds.measure && !keys.contains(column)) {
                return Err(error(
                    "descriptor bounds name a non-layout column or measure a payload",
                ));
            }
            let mut origins = Vec::new();
            for (name, source) in &records.sources {
                let (publication, layout, upload) = match &source.origin {
                    SourceOrigin::Resident { host } => {
                        (host, &facts.structure_analysis.host_layouts[host], false)
                    }
                    SourceOrigin::Uploaded { .. } => {
                        (name, &facts.structure_analysis.layouts[name], true)
                    }
                    SourceOrigin::RadixRelation { .. } => continue,
                };
                let Some(original) = source.columns.get(column) else {
                    continue;
                };
                let index = layout
                    .iter()
                    .position(|c| c == original)
                    .ok_or_else(|| error("source column is absent from its origin layout"))?;
                origins.push(if upload {
                    BoundSource::Upload {
                        publication: publication.clone(),
                        index,
                    }
                } else {
                    BoundSource::Resident {
                        host: publication.clone(),
                        index,
                    }
                });
            }
            let sources: Vec<&BoundSource> = match &bounds.base {
                BaseBounds::Union(sources) => {
                    if origins.iter().any(|origin| !sources.contains(origin)) {
                        return Err(error(
                            "descriptor Union omits a source origin of the column",
                        ));
                    }
                    sources.iter().collect()
                }
                BaseBounds::Scalar(ty) => {
                    let carrier = facts.carriers.get(column);
                    if carrier != Some(ty) {
                        return Err(error("descriptor Scalar bounds use another carrier"));
                    }
                    Vec::new()
                }
                BaseBounds::None => {
                    if !origins.is_empty() || bounds.merges.is_empty() {
                        return Err(error("descriptor bounds without a domain"));
                    }
                    Vec::new()
                }
            };
            let merges: Vec<_> = self
                .uploads
                .iter()
                .filter_map(|upload| {
                    let index = facts.structure_analysis.layouts[upload]
                        .iter()
                        .position(|c| c == column)?;
                    Some(BoundSource::Upload {
                        publication: upload.clone(),
                        index,
                    })
                })
                .collect();
            if bounds.merges != merges {
                return Err(error(
                    "descriptor merges are not exactly the earlier uploads of the column",
                ));
            }
            for source in sources.into_iter().chain(&bounds.merges) {
                if !publishes(facts, source, column) {
                    return Err(error(
                        "descriptor bounds name a domain that does not publish the column",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The completed publications a host-union arm denotes, in order.
fn union_operands(
    term: &Term,
    facts: &Facts,
    state: &CheckState,
    operands: &mut Vec<String>,
) -> syn::Result<()> {
    match term {
        Term::Ref(name) => {
            let name = symbol_name(name);
            if state.publication(&name).is_some() {
                operands.push(name);
                return Ok(());
            }
            let inner = facts
                .sequence
                .get(&name)
                .ok_or_else(|| error("host union operand has not published"))?;
            union_operands(inner, facts, state, operands)
        }
        Term::Union(arms) => {
            for arm in arms {
                union_operands(arm, facts, state, operands)?;
            }
            Ok(())
        }
        _ => Err(error("host union operands must be published stores")),
    }
}

/// Whether a publication step builds the providers of the Physical result `name`.
fn step_builds(steps: &[(String, Step)], analysis: &Analysis, name: &str) -> bool {
    steps.iter().any(|(publication, step)| match step {
        Step::Dispatch {
            result,
            provider: Some(_),
        } => result == name,
        Step::HostUnion(HostUnion {
            provider: Some(_), ..
        }) => analysis
            .results
            .get(publication)
            .is_some_and(|result| result == name),
        _ => false,
    })
}

/// Whether the source of the Physical result `name` builds its provider.
fn source_builds(sources: &BTreeMap<String, SourceRealization>, name: &str) -> bool {
    sources
        .get(name)
        .is_some_and(|source| matches!(source.access, SourceAccess::Provider { .. }))
}

/// Whether an earlier step uploads the Physical result `name`.
fn uploaded(records: &Realization, analysis: &Analysis, name: &str) -> bool {
    records.steps.iter().any(|(publication, step)| match step {
        Step::Upload { .. } => analysis
            .results
            .get(publication)
            .is_some_and(|result| result == name),
        Step::Group(TerminalGroup {
            input:
                GroupInput::Host {
                    input,
                    upload: Some(_),
                },
            ..
        }) => input == name,
        _ => false,
    })
}

/// GPU-SourceAccess checker of a resident source, without replaying the
/// origin fold: the chain from `name` is selections and projections over one
/// rename of `host` whose mapping is the recorded `columns`. Every recorded
/// bound is stated by a constraint of that chain (the interval never drops a
/// kept row) and every chain constraint states recorded bounds on the
/// interval column (the folded selections no longer run as row work), so the
/// bounds are the chain's constraints as a multiset; their order is free.
fn check_resident_chain(
    definitions: &BTreeMap<String, Definition>,
    name: &str,
    host: &str,
    columns: &BTreeMap<String, String>,
    selection: Option<&Interval>,
) -> syn::Result<()> {
    use cq_syntax::EqualityValue;
    use cq_syntax::constraint::{Comparison, Constraint, Term as Operand};
    let encoded = |value: &dyn quote::ToTokens| -> syn::Result<syn::Expr> {
        syn::parse2(quote::quote!(::gpu_datalog_runtime::encode_scalar(&(#value))))
    };
    let closed = |term: &Operand| -> syn::Result<syn::Expr> {
        match term {
            Operand::Literal(value) => encoded(value),
            Operand::Closed(value) => encoded(value),
            _ => Err(error("a folded selection compares against a row value")),
        }
    };
    // (definition, origin column, side, inclusive, encoded value)
    let mut stated: Vec<(String, String, Side, bool, syn::Expr)> = Vec::new();
    let mut state = |definition: &str, column: String, op: &str, value: syn::Expr| {
        let mut push = |side, inclusive| {
            stated.push((
                definition.to_owned(),
                column.clone(),
                side,
                inclusive,
                value.clone(),
            ));
        };
        match op {
            "eq" => {
                push(Side::Lower, true);
                push(Side::Upper, true);
            }
            "ge" => push(Side::Lower, true),
            "gt" => push(Side::Lower, false),
            "lt" => push(Side::Upper, false),
            _ => push(Side::Upper, true),
        }
    };
    let mut current = name.to_owned();
    let mut selections = Vec::new();
    let rename = loop {
        let definition = definitions
            .get(&current)
            .ok_or_else(|| error("resident source chain names an unknown definition"))?;
        match &definition.operator {
            Operator::Rename(rename) => break rename,
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                selections.push((current.clone(), &select.condition));
                current = symbol_name(&select.input);
            }
            Operator::Project(project) => current = symbol_name(&project.input),
            _ => return Err(error("resident source is not a selection of a rename")),
        }
    };
    let mapping: BTreeMap<_, _> = rename
        .mapping
        .iter()
        .map(|m| (symbol_name(&m.target), symbol_name(&m.source)))
        .collect();
    if symbol_name(&rename.relation) != host || mapping != *columns {
        return Err(error("resident source columns are not its rename mapping"));
    }
    if let Some(constants) = &rename.constants {
        for equality in &constants.equalities {
            let EqualityValue::Literal(value) = &equality.right else {
                return Err(error("a folded constant is not a literal"));
            };
            state(&current, symbol_name(&equality.left), "eq", encoded(value)?);
        }
    }
    for (definition, theta) in selections {
        let origin = |column: &syn::Ident| {
            columns
                .get(&symbol_name(column))
                .cloned()
                .ok_or_else(|| error("a folded selection reads an unrenamed column"))
        };
        for equality in &theta.equalities {
            let EqualityValue::Literal(value) = &equality.right else {
                return Err(error("a folded equality is not a literal"));
            };
            state(&definition, origin(&equality.left)?, "eq", encoded(value)?);
        }
        for constraint in &theta.constraints {
            match constraint {
                Constraint::Compare {
                    left: Operand::Variable(column),
                    comparison,
                    right,
                } => {
                    let op = match comparison {
                        Comparison::Eq(_) => "eq",
                        Comparison::Ge(_) => "ge",
                        Comparison::Gt(_) => "gt",
                        Comparison::Lt(_) => "lt",
                        Comparison::Le(_) => "le",
                        _ => return Err(error("a noncontiguous selection is folded")),
                    };
                    state(&definition, origin(column)?, op, closed(right)?);
                }
                Constraint::Between {
                    term: Operand::Variable(column),
                    low,
                    high,
                } => {
                    state(&definition, origin(column)?, "ge", closed(low)?);
                    state(&definition, origin(column)?, "le", closed(high)?);
                }
                _ => return Err(error("a noncontiguous selection is folded")),
            }
        }
    }
    let key = |definition: &str, side: Side, inclusive: bool, value: &syn::Expr| {
        use quote::ToTokens;
        (
            definition.to_owned(),
            side == Side::Upper,
            inclusive,
            value.to_token_stream().to_string(),
        )
    };
    let Some(interval) = selection else {
        return if stated.is_empty() {
            Ok(())
        } else {
            Err(error("resident source drops its contiguous selections"))
        };
    };
    if interval.bounds.is_empty()
        || stated
            .iter()
            .any(|(_, column, ..)| *column != interval.column)
    {
        return Err(error("source interval is not one column's stated bounds"));
    }
    let mut expected: Vec<_> = stated
        .iter()
        .map(|(definition, _, side, inclusive, value)| key(definition, *side, *inclusive, value))
        .collect();
    let mut recorded: Vec<_> = interval
        .bounds
        .iter()
        .map(|bound| key(&bound.definition, bound.side, bound.inclusive, &bound.value))
        .collect();
    expected.sort();
    recorded.sort();
    if expected != recorded {
        return Err(error(
            "source interval bounds are not exactly its chain's contiguous constraints",
        ));
    }
    Ok(())
}

/// Whether a bounds domain publishes `column`: a resident input renamed into
/// it, or an uploaded publication's layout column.
fn publishes(facts: &Facts, source: &BoundSource, column: &str) -> bool {
    match source {
        BoundSource::Resident { host, index } => {
            facts.resident.contains(host)
                && facts.physical
                    .program
                    .definitions
                    .iter()
                    .any(|definition| {
                        let Operator::Rename(rename) = &definition.operator else {
                            return false;
                        };
                        symbol_name(&rename.relation) == *host
                            && rename.mapping.iter().any(|mapping| {
                                symbol_name(&mapping.target) == column
                                    && facts.structure_analysis.host_layouts[host].get(*index)
                                        == Some(&symbol_name(&mapping.source))
                            })
                    })
        }
        BoundSource::Upload { publication, index } => facts.structure_analysis
            .layouts
            .get(publication)
            .is_some_and(|layout| layout.get(*index).is_some_and(|c| c == column)),
    }
}

#[cfg(test)]
mod tests {
    include!("../../tests/support/stack.rs");
    use super::*;
    use cq_lang_sequence::gpu::realization::IntervalBound;

    /// The store `r` publishes the resident rename `r` below host (its inner
    /// store result equals its outer source), and `r` records a Range provider.
    fn republished() -> (Stack, Analysis) {
        republished_after(false)
    }

    /// With `late`, the lookup of `r` precedes its republication.
    fn republished_after(late: bool) -> (Stack, Analysis) {
        let early = (!late).then(|| quote::quote!(r = store rows r;));
        let late = late.then(|| quote::quote!(r = store rows r;));
        let Stack { physical, structures, program } = syn::parse_quote! {
            physical {
                relation R(a:i64,b:i64) = (&data);
                r = let rename R {a -> a,b -> b};
                driver = pipe rename R {a -> a,b -> c};
                joined = let canon probe first [a] driver,r;
                return joined as answer(a,b,c);
            }
            structures {
                providers {
                    device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns };
                    radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred };
                    range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range };
                }
                r host scan = @device_columns input { keys 0; columns [i64 => i64,i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                driver host scan = @device_columns input { keys 0; columns [i64 => i64,i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                r = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];
                joined rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                joined canon = rows;
            }
            sequence {
                #early
                input = store rows driver;
                output_rows = store canon map joined flatten lookup r [a] flatten tile 256 input;
                #late
                output count;
            }
            graded gpu { r at host.device; input at host; output_rows at device; }
        };
        let program = crate::runs::record_representation(&physical, &structures, &program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&physical, &structures, &program, &crate::machine()).unwrap();
        (Stack { physical, structures, program }, analysis)
    }

    #[test]
    fn publication_step_is_the_only_provider_owner() {
        let (Stack { physical, structures, program }, analysis) = republished();
        let structure_analysis = cq_lang_structures::contract::check(&physical, &structures).unwrap();
        let realization = select(&physical, &structures, &program, &structure_analysis, &analysis).unwrap();
        assert_eq!(realization.rejection, None);
        assert!(realization.steps.iter().any(|(publication, step)| {
            publication == "r"
                && matches!(
                    step,
                    Step::Dispatch {
                        provider: Some(_),
                        ..
                    }
                )
        }));
        assert!(!matches!(
            realization.sources["r"].access,
            SourceAccess::Provider { .. }
        ));
        let mut missing_layout = realization.clone();
        let (
            _,
            Step::Dispatch {
                provider: Some(provider),
                ..
            },
        ) = missing_layout
            .steps
            .iter_mut()
            .find(|(name, _)| name == "r")
            .unwrap()
        else {
            panic!("recorded provider owner");
        };
        provider.layouts.clear();
        assert!(check(&physical, &structures, &program, &structure_analysis, &analysis, &missing_layout).is_err());
        let mut duplicated = realization;
        duplicated.sources.get_mut("r").unwrap().access =
            SourceAccess::Provider { selection: None };
        assert!(check(&physical, &structures, &program, &structure_analysis, &analysis, &duplicated).is_err());
    }

    #[test]
    fn a_reader_before_the_publication_keeps_the_source_owner() {
        let (Stack { physical, structures, program }, analysis) = republished_after(true);
        let structure_analysis = cq_lang_structures::contract::check(&physical, &structures).unwrap();
        let realization = select(&physical, &structures, &program, &structure_analysis, &analysis).unwrap();
        assert_eq!(realization.rejection, None);
        assert!(matches!(
            realization.sources["r"].access,
            SourceAccess::Provider { .. }
        ));
        let (_, Step::Dispatch { provider, .. }) = realization
            .steps
            .iter()
            .find(|(name, _)| name == "r")
            .unwrap()
        else {
            unreachable!("r republishes below host")
        };
        assert_eq!(*provider, None);
        // Moving ownership to the later publication leaves the reader unserved.
        let built = Steps::new(
            &Facts::new(&physical, &structures, &program, &structure_analysis, &analysis).unwrap(),
            &realization.sources,
        )
        .provider("r")
        .unwrap();
        let mut late_owner = realization.clone();
        late_owner.sources.get_mut("r").unwrap().access = SourceAccess::Scan;
        for (name, step) in &mut late_owner.steps {
            if let (true, Step::Dispatch { provider, .. }) = (name == "r", step) {
                *provider = Some(built.clone());
            }
        }
        assert!(check(&physical, &structures, &program, &structure_analysis, &analysis, &late_owner).is_err());
    }

    fn graded(Stack { physical, structures, program }: Stack) -> (Stack, Analysis) {
        let program = crate::runs::record_representation(&physical, &structures, &program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&physical, &structures, &program, &crate::machine()).unwrap();
        (Stack { physical, structures, program }, analysis)
    }

    /// Two published dispatches concatenated by a host union.
    fn union() -> (Stack, Analysis) {
        graded(syn::parse_quote! {
            physical {
                relation R(a:u32) = (&data.r);
                relation S(a:u32) = (&data.s);
                r = pipe rename R {a -> a};
                s = pipe rename S {a -> a};
                answer = let canon union r,s;
                return answer as answer(a);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @device_columns input { keys 0; columns [u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,)> [];
                s host scan = @device_columns input { keys 0; columns [u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(u32,)> [];
                answer rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                answer canon = rows;
            }
            sequence {
                R = store rows r;
                S = store rows s;
                left_feed = flatten tile 256 R;
                left = store left_feed;
                right_feed = flatten tile 256 S;
                right = store right_feed;
                result = store union [left,right];
                output count;
            }
            graded gpu {
                R at host; S at host;
                left_feed at device.block; left at host;
                right_feed at device.block; right at host;
                result at host;
            }
        })
    }

    /// A probe of a provider uploaded from a host scan, or with `selected` of
    /// a contiguous key selection over a resident input.
    fn probe(selected: bool) -> (Stack, Analysis) {
        let (reader, target, index) = if selected {
            (
                quote::quote!(provider host scan = @device_columns input { keys 0; columns [i32 => i32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];),
                quote::quote!(filtered),
                quote::quote!(compact filtered rows provider),
            )
        } else {
            (
                quote::quote!(provider host scan = @host_scan as ::core::iter::Map<_, _> []; provider = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];),
                quote::quote!(provider),
                quote::quote!(rows provider),
            )
        };
        let filtered = selected.then(|| {
            quote::quote!(filtered = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];)
        });
        let selection = selected
            .then(|| quote::quote!(filtered = let select provider if (a >= 3i32, a < 9i32);));
        let host_scan = (!selected)
            .then(|| quote::quote!(host_scan = ::core::iter::Map<_, _> { reads [], build typed };));
        let retained = if selected {
            quote::quote!(pipe)
        } else {
            quote::quote!(let)
        };
        graded(syn::parse_quote! {
            physical {
                relation Driver(a:i32, value:i32) = (&data.driver);
                relation Provider(a:i32, payload:i32) = (&data.provider);
                driver = pipe rename Driver {a -> a, value -> value};
                provider = #retained rename Provider {a -> a, payload -> payload};
                #selection
                joined = pipe probe first [a] driver, #target;
                result = let canon project joined keep {a, value, payload};
                return result as answer(a, value, payload);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; #host_scan range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                driver host scan = @device_columns input { keys 0; columns [i32 => i32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns [];
                #reader
                #filtered
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
            }
            sequence {
                input = store rows driver;
                index = store #index;
                joined_rows = flatten lookup #target [a] flatten tile 256 input;
                output_rows = store canon map result joined_rows;
                output count;
            }
            graded gpu {
                input at host; index at host;
                joined_rows at host.device.block;
                output_rows at host.device;
            }
        })
    }

    /// Every mutation of a selected realization is rejected by the checker,
    /// which accepts the unmutated record.
    fn rejects(fixture: (Stack, Analysis), mutations: &[&dyn Fn(&mut Realization)]) {
        let (Stack { physical, structures, program }, analysis) = fixture;
        let structure_analysis = cq_lang_structures::contract::check(&physical, &structures).unwrap();
        let realization = select(&physical, &structures, &program, &structure_analysis, &analysis).unwrap();
        assert_eq!(realization.rejection, None);
        check(&physical, &structures, &program, &structure_analysis, &analysis, &realization).unwrap();
        for (index, mutate) in mutations.iter().enumerate() {
            let mut mutated = realization.clone();
            mutate(&mut mutated);
            assert_ne!(mutated, realization, "mutation {index} is a no-op");
            assert!(
                check(&physical, &structures, &program, &structure_analysis, &analysis, &mutated).is_err(),
                "mutation {index} was accepted"
            );
        }
    }

    fn union_arms(realization: &mut Realization) -> &mut Vec<UnionArm> {
        realization
            .steps
            .iter_mut()
            .find_map(|(_, step)| match step {
                Step::HostUnion(union) => Some(&mut union.arms),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn checker_rejects_unrecorded_query_gate_changes() {
        rejects(
            probe(false),
            &[&|realization| {
                realization
                    .captures
                    .query
                    .items
                    .push(syn::parse_quote!(if (limit != 3)));
            }],
        );
    }

    #[test]
    fn checker_rejects_union_arms_that_differ_from_the_operands() {
        rejects(
            union(),
            &[
                &|realization| {
                    union_arms(realization).pop();
                },
                &|realization| {
                    let arms = union_arms(realization);
                    arms.push(arms[0].clone());
                },
                &|realization| union_arms(realization).reverse(),
                &|realization| {
                    union_arms(realization)[0] = UnionArm::Copy {
                        publication: "R".into(),
                    };
                },
            ],
        );
    }

    #[test]
    fn checker_rejects_union_consumers_with_incompatible_flat_slots() {
        let (Stack { physical, structures, program }, analysis) = union();
        let structure_analysis = cq_lang_structures::contract::check(&physical, &structures).unwrap();
        let realization = select(&physical, &structures, &program, &structure_analysis, &analysis).unwrap();
        for missing in [false, true] {
            let mut changed = analysis.clone();
            let cq_lang_sequence::gpu::publication::PublicationStorage::FlatRows(layout) =
                &mut changed.gpu_publications.get_mut("left").unwrap().storage else {
                panic!("flat union input");
            };
            if missing {
                layout.columns.clear();
            } else {
                layout.width += 1;
            }
            assert!(check(&physical, &structures, &program, &structure_analysis, &changed, &realization).is_err());
        }
    }

    fn evaluation(realization: &mut Realization) -> &mut HostEvaluation {
        match &mut realization.sources.get_mut("provider").unwrap().origin {
            SourceOrigin::Uploaded { evaluation } => evaluation,
            _ => unreachable!("the host-scanned provider is uploaded"),
        }
    }

    #[test]
    fn checker_rejects_unread_transfers_and_misphased_uploads() {
        rejects(
            probe(false),
            &[
                &|realization| {
                    evaluation(realization).transfers.push(Transfer {
                        source: TransferSource::Resident("Driver".into()),
                        granularity: cq_lang_sequence::gpu::realization::Granularity::CompleteRows,
                    });
                },
                &|realization| evaluation(realization).bound.push("input".into()),
                &|realization| {
                    realization.sources.get_mut("provider").unwrap().phase = Phase::Setup
                },
            ],
        );
    }

    fn bounds(realization: &mut Realization) -> &mut Vec<IntervalBound> {
        match &mut realization.sources.get_mut("filtered").unwrap().access {
            SourceAccess::Provider {
                selection: Some(interval),
            } => &mut interval.bounds,
            _ => unreachable!("the key selection folds into the provider build"),
        }
    }

    #[test]
    fn checker_rejects_interval_bounds_that_differ_from_the_selection() {
        rejects(
            probe(true),
            &[
                &|realization| {
                    bounds(realization).pop();
                },
                &|realization| bounds(realization)[0].side = Side::Upper,
                &|realization| bounds(realization)[0].inclusive = false,
                &|realization| bounds(realization)[0].definition = "driver".into(),
                &|realization| bounds(realization)[0].value = syn::parse_quote!(0i64),
                &|realization| {
                    let SourceAccess::Provider { selection } =
                        &mut realization.sources.get_mut("filtered").unwrap().access
                    else {
                        unreachable!()
                    };
                    *selection = None;
                },
            ],
        );
    }

    #[test]
    fn checker_rejects_unsupported_auxiliary_geometry() {
        rejects(
            probe(false),
            &[
                &|realization| realization.auxiliary = None,
                &|realization| {
                    realization
                        .auxiliary
                        .as_mut()
                        .unwrap()
                        .selection
                        .block_threads = 0
                },
                &|realization| {
                    realization
                        .auxiliary
                        .as_mut()
                        .unwrap()
                        .group_reduce
                        .shared_bytes += 4
                },
                &|realization| {
                    realization
                        .auxiliary
                        .as_mut()
                        .unwrap()
                        .group_reduce
                        .block_threads = 128
                },
            ],
        );
    }
}
