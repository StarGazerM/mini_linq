//! GPU realization selected once, after provider bindings and publication
//! storage, and recorded in `Bindings.realization`. Printers consume these
//! records; `check` validates supplied records without calling `select`.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::gpu::bindings::ProviderAbi;
use cq_lang_sequence::gpu::realization::{
    BaseBounds, BoundSource, Captures, DescriptorBounds, Granularity, GroupCompletion, GroupInput,
    HostEvaluation, HostOutput, HostUnion, Interval, Phase, ProviderBuild, Realization,
    SourceAccess, SourceOrigin, SourceRealization, Step, TerminalGroup, Transfer, TransferSource,
    UnionArm,
};
use cq_lang_sequence::gpu::{Grid, Publication};
use cq_lang_sequence::{Analysis, Capability, Program, Term};
use cq_lang_structures::physical::{Definition, Operator, symbol_name};
use cq_lang_structures::syntax as cq_syntax;
use quote::format_ident;

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), format!("GPU-Realization: {message}"))
}

/// Auxiliary element kernels (union decode/pack, host-output pack) launch one
/// thread per element in blocks of the target's collective width.
fn elements() -> Grid {
    Grid::Elements { block_threads: crate::BLOCK_THREADS }
}

/// Formal rule GPU-Realization: select source access, publication steps,
/// terminal grouping, transfers, auxiliary geometry, descriptor bounds and
/// captures once, before printing.
pub(crate) fn select(program: &Program, analysis: &Analysis) -> syn::Result<Realization> {
    let selected: syn::Result<Realization> = (|| {
        let facts = Facts::new(program, analysis)?;
        let captures = captures(&facts)?;
        let sources = sources(&facts, &captures)?;
        let (steps, output) = Steps::new(&facts, &sources).run()?;
        Ok(Realization { rejection: None, sources, steps, output, captures })
    })();
    let realization = match selected {
        Ok(realization) => realization,
        Err(error) => Realization {
            rejection: Some(error.into_iter().map(|error| error.to_string()).collect::<Vec<_>>().join("\n")),
            ..Realization::default()
        },
    };
    check(program, analysis, &realization)?;
    Ok(realization)
}

/// Checked facts shared by selection and checking. No realization policy.
struct Facts<'a> {
    program: &'a Program,
    analysis: &'a Analysis,
    definitions: BTreeMap<String, Definition>,
    sequence: BTreeMap<String, &'a Term>,
    /// Physical row types with codec-encoded columns as u32 carriers.
    carriers: BTreeMap<String, BTreeMap<String, syn::Type>>,
    resident: BTreeSet<String>,
    runs: BTreeSet<String>,
    device_results: BTreeSet<String>,
    codecs: crate::runs::Codecs,
}

impl<'a> Facts<'a> {
    fn new(program: &'a Program, analysis: &'a Analysis) -> syn::Result<Self> {
        let definitions: BTreeMap<_, _> = program
            .structures
            .physical
            .program
            .definitions
            .iter()
            .map(|d| (symbol_name(&d.name), d.clone()))
            .collect();
        let sequence = program
            .sequence
            .definitions
            .iter()
            .map(|d| (symbol_name(&d.name), &d.term))
            .collect();
        let codecs = crate::runs::Codecs::check(&program.structures)?;
        let carriers = analysis
            .structures
            .physical
            .rows
            .iter()
            .map(|(name, row)| {
                let types = row
                    .types
                    .iter()
                    .map(|(column, ty)| {
                        let ty = if codecs.get(ty).is_some() { syn::parse_quote!(u32) } else { ty.clone() };
                        (column.clone(), ty)
                    })
                    .collect();
                (name.clone(), types)
            })
            .collect();
        let mut resident = BTreeSet::new();
        let mut runs = BTreeSet::new();
        for input in program.structures.physical.inputs() {
            let name = symbol_name(&input.name);
            for entry in &program.structures.structures.entries {
                if analysis.structures.physical.hosts.get(&symbol_name(&entry.name)) != Some(&name) {
                    continue;
                }
                if let cq_lang_structures::Structure::Host { reader, keyed, .. } = &entry.structure {
                    if !matches!(reader, cq_lang_structures::ReaderRef::Scan) {
                        continue;
                    }
                    if crate::runs::layout_types(&keyed.ty)?.is_some() {
                        runs.insert(name.clone());
                        resident.insert(name.clone());
                    } else if crate::native_abi(&keyed.ty) == Some(crate::NativeAbi::Columns) {
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
        Ok(Self { program, analysis, definitions, sequence, carriers, resident, runs, device_results, codecs })
    }

    fn host_level(&self, name: &str) -> bool {
        self.analysis
            .levels
            .get(name)
            .is_some_and(|level| level.last().is_some_and(|level| level == "host"))
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
            let Some(next) = definition.operator.input() else { break };
            current = symbol_name(next);
        }
        Ok(chain)
    }

    fn final_name(&self) -> syn::Result<String> {
        self.analysis
            .output
            .as_ref()
            .map(|(name, _)| name.clone())
            .or_else(|| self.program.sequence.definitions.last().map(|d| symbol_name(&d.name)))
            .ok_or_else(|| error("GPU Sequence lacks a checked output"))
    }
}

/// Formal rule GPU-Capture: extract closed operands and device Rust
/// callables once per program, in Physical definition order.
fn captures(facts: &Facts) -> syn::Result<Captures> {
    let analysis = facts.analysis;
    // Contract 7 (DESIGN anchor contract7-gpu-rows): only identity reads admit
    // device Rust; encoded predicates stay relational `where` captures.
    let mut captures = crate::capture::Captures::default();
    let mut records = Captures::default();
    for (name, definition) in &facts.definitions {
        captures.used.clear();
        if !analysis.rust_requires_host(&definition.operator)
            && (matches!(&definition.operator, Operator::Guard(guard) if guard.capture.is_some())
                || matches!(&definition.operator, Operator::Bind(bind) if bind.capture.is_some()))
        {
            return Err(syn::Error::new_spanned(
                &definition.name,
                "T-Gpu: a branch capture reference has no device realization",
            ));
        }
        let expression = match &definition.operator {
            Operator::Guard(guard) if !analysis.rust_requires_host(&definition.operator) => {
                let row = &analysis.structures.physical.rows[&symbol_name(&guard.input)];
                captures.rust_expression(&guard.condition, &row.types, &row.bindings, &syn::parse_quote!(bool), &facts.codecs)
            }
            Operator::Bind(bind) if !analysis.rust_requires_host(&definition.operator) => {
                let row = &analysis.structures.physical.rows[&symbol_name(&bind.input)];
                let names = cq_syntax::rust::bindings(&bind.pattern);
                let output = &analysis.structures.physical.rows[name].types[&symbol_name(&names[0])];
                captures.rust_expression(&bind.expression, &row.types, &row.bindings, output, &facts.codecs)
            }
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                let types = &analysis.structures.physical.rows[&symbol_name(&select.input)].types;
                captures.relational(&select.condition, types, &facts.codecs)?
            }
            _ => continue,
        };
        records.uses.insert(name.clone(), captures.used.clone());
        records.device.insert(name.clone(), expression);
    }
    for (name, definition) in &facts.definitions {
        if let Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) = &definition.operator {
            let types = &analysis.structures.physical.rows[&symbol_name(&select.input)].types;
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
        if let Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) = &mut definition.operator
            && let Some(condition) = captures.host.get(name)
        {
            select.condition = condition.clone();
        }
    }
    bound
}

fn interval(interval: &crate::rows::Interval) -> syn::Result<Interval> {
    let start = &interval.start;
    let end = &interval.end;
    Ok(Interval {
        column: interval.column.clone(),
        start: syn::parse2(quote::quote!(#start))?,
        end: syn::parse2(quote::quote!(#end))?,
    })
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
        .filter_map(|result| facts.analysis.structures.physical.hosts.get(result))
        .filter(|host| facts.resident.contains(*host))
        .collect();
    hosts
        .into_iter()
        .map(|host| Transfer { source: TransferSource::Resident(host.clone()), granularity: Granularity::CompleteRows })
        .collect()
}

/// Formal rule GPU-SourceAccess: device sources record which Physical results
/// acquire a device read, from which origin, with which access and phase.
fn sources(facts: &Facts, captures: &Captures) -> syn::Result<BTreeMap<String, SourceRealization>> {
    let analysis = facts.analysis;
    // Only checked device consumers require resident source adapters.
    // A host store can retain arbitrary Rust values (for example nullable
    // aggregates); it is not itself a host-to-device publication.
    let required = crate::rows::device_source_results(analysis);
    let host_bound = host_bound(&facts.definitions, captures);
    let demands = crate::rows::lookup_demands(facts.program);
    let mut sources = BTreeMap::new();
    for (name, definition) in &facts.definitions {
        let rename = matches!(definition.operator, Operator::Rename(_));
        if !rename && (facts.device_results.contains(name) || !required.contains(name)) {
            continue;
        }
        let dependencies = crate::rows::host_dependencies(name, &facts.definitions);
        if !rename && dependencies.iter().any(|result| facts.device_results.contains(result)) {
            continue;
        }
        let needs_host = dependencies.iter().any(|result| {
            analysis.structures.physical.hosts.get(result).is_some_and(|host| !facts.resident.contains(host))
        });
        let (origin, columns, selection) = if needs_host {
            if !required.contains(name) {
                continue;
            }
            let evaluation = HostEvaluation {
                bound: Vec::new(),
                transfers: resident_transfers(facts, &dependencies, &BTreeSet::new()),
            };
            let columns = analysis.structures.layouts[name]
                .iter()
                .map(|column| (column.clone(), column.clone()))
                .collect();
            (SourceOrigin::Uploaded { evaluation }, columns, None)
        } else {
            let Some(origin) = crate::rows::origin(name, &host_bound)? else { continue };
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
                if let Some(layouts) = analysis.structures.built.get(name) {
                    if layouts.len() != 1 || crate::runs::layout_types(&layouts[0].1)?.is_some() {
                        providers.push(name.clone());
                    }
                } else {
                    for ((result, reader, keys), ty) in &analysis.structures.fallbacks {
                        if result != name || matches!(reader, cq_lang_structures::ReaderRef::Scan) {
                            continue;
                        }
                        let mut sorted = keys.clone();
                        sorted.sort();
                        if !demands.contains(&(result.clone(), sorted.clone())) {
                            continue;
                        }
                        if crate::runs::layout_types(ty)?.is_none() {
                            return Err(error("GPU-Runs: native lookup requires a declared run layout"));
                        }
                        let provider = format!("{name}__{}", sorted.join("__"));
                        if !providers.contains(&provider) {
                            providers.push(provider);
                        }
                    }
                }
                sources.insert(name.clone(), SourceRealization {
                    origin: SourceOrigin::Runs { host: origin.host.clone(), providers },
                    columns: origin.columns.clone(),
                    access: SourceAccess::Scan,
                    phase: Phase::Setup,
                });
                continue;
            }
            let selection = origin.selection.as_ref().map(interval).transpose()?;
            (SourceOrigin::Resident { host: origin.host.clone() }, origin.columns.clone(), selection)
        };
        let built = !facts.device_results.contains(name)
            && analysis.gpu_bindings.providers.get(name).is_some_and(|layouts| {
                layouts.iter().any(|layout| layout.abi != ProviderAbi::Host)
            })
            && analysis.structures.built.contains_key(name);
        let access = if built {
            SourceAccess::Provider { selection }
        } else if let Some(selection) = selection {
            SourceAccess::Positions(selection)
        } else {
            SourceAccess::Scan
        };
        let phase = if matches!(origin, SourceOrigin::Uploaded { .. }) { Phase::Build } else { Phase::Setup };
        sources.insert(name.clone(), SourceRealization { origin, columns, access, phase });
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
        for (name, source) in sources {
            let SourceOrigin::Runs { host, .. } = &source.origin else { continue };
            for original in &facts.analysis.structures.host_layouts[host] {
                let Some((column, _)) = source.columns.iter().find(|(_, o)| *o == original) else { continue };
                let ty = facts.carriers[name][column].clone();
                bounds.measured.insert(column.clone());
                bounds.columns.insert(column.clone(), (BaseBounds::Scalar(ty), Vec::new()));
            }
        }
        let mut origins: BTreeMap<String, BTreeSet<(String, usize, bool)>> = BTreeMap::new();
        for (name, source) in sources {
            let (host, layout, upload) = match &source.origin {
                SourceOrigin::Resident { host } => (host, &facts.analysis.structures.host_layouts[host], false),
                SourceOrigin::Uploaded { .. } => (name, &facts.analysis.structures.layouts[name], true),
                SourceOrigin::Runs { .. } => continue,
            };
            for (column, original) in &source.columns {
                let index = layout.iter().position(|c| c == original).expect("source column in layout");
                origins.entry(column.clone()).or_default().insert((host.clone(), index, upload));
            }
        }
        for (column, origins) in origins {
            bounds.measured.remove(&column);
            let union = origins
                .into_iter()
                .map(|(host, index, upload)| if upload {
                    BoundSource::Upload { publication: host, index }
                } else {
                    BoundSource::Resident { host, index }
                })
                .collect();
            bounds.columns.insert(column, (BaseBounds::Union(union), Vec::new()));
        }
        for (name, row) in &facts.analysis.structures.physical.rows {
            for column in &row.computed {
                bounds
                    .columns
                    .entry(column.clone())
                    .or_insert_with(|| (BaseBounds::Scalar(facts.carriers[name][column].clone()), Vec::new()));
            }
        }
        Self { facts, sources, stored: Vec::new(), uploaded: BTreeSet::new(), downloaded: BTreeSet::new(), bounds }
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
                let term = facts.sequence.get(&name).ok_or_else(|| error("Sequence reference lacks definition"))?;
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
                let Term::Lookup { inner, .. } = inner.as_ref() else { unreachable!() };
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
            Term::Lookup { provider, keys, inner } => {
                let mut shapes = self.shapes(inner)?;
                let provider = symbol_name(provider);
                let mut keys: Vec<_> = keys.iter().map(symbol_name).collect();
                keys.sort();
                let membership = facts.analysis.capabilities.get(&(provider.clone(), keys.clone()))
                    == Some(&Capability::Membership);
                for shape in &mut shapes {
                    if !membership {
                        let result = facts.definitions.values().find_map(|definition| match &definition.operator {
                            Operator::Probe(_, keyed) | Operator::Merge(_, keyed)
                                if symbol_name(&keyed.right) == provider && symbol_name(&keyed.left) == shape.result =>
                            {
                                let mut read: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
                                read.sort();
                                (read == keys).then(|| symbol_name(&definition.name))
                            }
                            _ => None,
                        });
                        shape.result = result.ok_or_else(|| error("Sequence lookup lacks checked Physical row provenance"))?;
                    }
                    shape.operations = true;
                }
                shapes
            }
            Term::Group(..) | Term::PartitionK { .. } => {
                return Err(error("group or key partition is not a dispatch chain"));
            }
            Term::Flatten(inner) | Term::Canon(inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::Fold(_, inner) => {
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
        let dependencies = crate::rows::host_dependencies_from(result, &self.facts.definitions, &bindings);
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
                return Err(error("host consumer requires a materialized row publication"));
            }
            if self.downloaded.insert(stored.name.clone()) {
                transfers.push(Transfer {
                    source: TransferSource::Publication(stored.name.clone()),
                    granularity: Granularity::CompleteRows,
                });
            }
        }
        let bound_results = last.keys().cloned().collect();
        transfers.extend(resident_transfers(self.facts, &dependencies, &bound_results));
        Ok(HostEvaluation { bound, transfers })
    }

    /// An upload publishes its layout columns; their bounds merge pairwise.
    fn upload(&mut self, result: &str) {
        self.uploaded.insert(result.to_owned());
        for (index, column) in self.facts.analysis.structures.layouts[result].iter().enumerate() {
            let source = BoundSource::Upload { publication: result.to_owned(), index };
            self.bounds
                .columns
                .entry(column.clone())
                .or_insert_with(|| (BaseBounds::None, Vec::new()))
                .1
                .push(source);
        }
    }

    fn provider(&self, result: &str) -> Option<ProviderBuild> {
        self.facts.analysis.structures.built.get(result)?;
        let layouts = self.facts.analysis.gpu_bindings.providers.get(result)?;
        let mut columns = BTreeMap::new();
        for layout in layouts {
            for column in layout.keys.iter().chain(&layout.payload) {
                let Some((base, merges)) = self.bounds.columns.get(column) else { continue };
                columns.insert(column.clone(), DescriptorBounds {
                    base: base.clone(),
                    merges: merges.clone(),
                    measure: self.bounds.measured.contains(column) && layout.keys.contains(column),
                });
            }
        }
        Some(ProviderBuild { columns })
    }

    fn push(&mut self, name: &str, result: String, range: bool) {
        let packed = self
            .facts
            .analysis
            .gpu_publications
            .get(name)
            .is_some_and(|record| record.storage.codec().is_some());
        self.stored.push(Stored { name: name.to_owned(), result, range, packed });
    }

    fn group(&mut self, name: &syn::Ident, input: &Term) -> syn::Result<TerminalGroup> {
        let facts = self.facts;
        let shapes = self.shapes(input)?;
        let [shape] = shapes.as_slice() else {
            return Err(error("GPU group requires one canonical valuation publication"));
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
                host |= facts.analysis.rust_requires_host(&definition.operator);
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
                GroupInput::Canonical { publication: stored.name.clone() }
            } else {
                GroupInput::Stored { publication: stored.name.clone() }
            }
        } else {
            GroupInput::Source { source: shape.source.clone() }
        };
        let completion = if group.keys.is_empty() {
            GroupCompletion::GridPartials { partials: Grid::KernelOccupancy }
        } else {
            GroupCompletion::Segmented { arguments: Grid::KernelOccupancy }
        };
        Ok(TerminalGroup { group: result, input, completion })
    }

    fn run(mut self) -> syn::Result<(Vec<(String, Step)>, Option<HostOutput>)> {
        let facts = self.facts;
        let analysis = facts.analysis;
        let final_name = facts.final_name()?;
        let final_group = facts.sequence.get(&final_name).and_then(|term| grouped_boundary(term, &facts.sequence));
        let device_sources = crate::rows::device_source_results(analysis);
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
                    grouped_boundary(&definition.term, &facts.sequence).is_some_and(|(other, _)| other == group)
                }) {
                Step::GroupAlias
            } else if let (true, Some((name, input))) = (terminal, grouped_boundary(&definition.term, &facts.sequence)) {
                output = true;
                Step::Group(self.group(name, input)?)
            } else if let Some(dispatch) = analysis.dispatches.iter().find(|dispatch| {
                dispatch.inner_stores.iter().rev().find(|name| {
                    facts.sequence.get(*name).is_some_and(|term| matches!(term, Term::Store { .. }))
                }) == Some(&publication)
            }) {
                let Term::Store { inner, .. } = definition.term.clone() else {
                    return Err(error("GPU dispatch publication must be an explicit outer store"));
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
                    dispatch.gpu.as_ref().map(|execution| &execution.publication),
                    Some(Publication::DeferredRange { .. })
                );
                output |= terminal;
                let provider = if terminal { None } else { self.provider(&result) };
                self.push(&publication, result, range);
                Step::Dispatch { provider }
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
                        let index = shape.stored.ok_or_else(|| error("host union operand has not published"))?;
                        let stored = &self.stored[index];
                        if stored.range {
                            return Err(error("host union operand requires a materialized row publication"));
                        }
                        records.push(if stored.packed {
                            UnionArm::Decode { publication: stored.name.clone(), grid: elements() }
                        } else {
                            UnionArm::Copy { publication: stored.name.clone() }
                        });
                    }
                }
                let codec = analysis
                    .gpu_publications
                    .get(&publication)
                    .is_some_and(|record| record.storage.codec().is_some());
                output |= terminal && codec;
                let provider = if terminal { None } else { self.provider(&result) };
                self.push(&publication, result, false);
                Step::HostUnion(HostUnion { arms: records, pack: codec.then(elements), provider })
            } else if matches!(definition.term, Term::Store { .. })
                && host_level
                && let Some(result) = analysis.results.get(&publication)
                && device_sources.contains(result)
                && !self.has_source(result)
            {
                let evaluation = self.evaluation(result)?;
                self.upload(result);
                Step::Upload { evaluation, provider: None }
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
                    break analysis.results.get(&name).map(|source| HostOutput { source: source.clone(), pack: elements() });
                }
                let Some(Term::Store { inner, .. }) = facts.sequence.get(&name).copied() else { break None };
                let Term::Ref(parent) = inner.as_ref() else { break None };
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
/// stated invariants. The checker never calls `select`; it accepts any
/// sound record, and re-derives only uniquely determined syntax (intervals).
pub(crate) fn check(program: &Program, analysis: &Analysis, records: &Realization) -> syn::Result<()> {
    if records.rejection.is_some() {
        return Ok(());
    }
    let facts = Facts::new(program, analysis)?;
    let columns: BTreeSet<_> = analysis
        .structures
        .physical
        .rows
        .values()
        .flat_map(|row| row.types.keys().cloned())
        .collect();
    crate::capture::check(&records.captures, &columns, &facts.codecs)?;
    let host_bound = host_bound(&facts.definitions, &records.captures);
    let grid = |grid: &Grid| match grid {
        Grid::KernelOccupancy => true,
        Grid::Elements { block_threads } => *block_threads > 0,
    };
    for (name, source) in &records.sources {
        if !facts.definitions.contains_key(name) {
            return Err(error("source realizes an unknown Physical result"));
        }
        let dependencies = crate::rows::host_dependencies(name, &facts.definitions);
        let nonresident = dependencies.iter().any(|result| {
            analysis.structures.physical.hosts.get(result).is_some_and(|host| !facts.resident.contains(host))
        });
        let layout = match &source.origin {
            SourceOrigin::Resident { host } | SourceOrigin::Runs { host, .. } => {
                if !facts.resident.contains(host) || nonresident {
                    return Err(error("resident source reads a nonresident input"));
                }
                if let SourceOrigin::Runs { providers, .. } = &source.origin {
                    if !facts.runs.contains(host) || source.access != SourceAccess::Scan {
                        return Err(error("packed runs have no interval or provider access"));
                    }
                    if providers.iter().any(|provider| !analysis.gpu_bindings.native.contains_key(provider)) {
                        return Err(error("run source names an unrecorded native lookup"));
                    }
                }
                &analysis.structures.host_layouts[host]
            }
            SourceOrigin::Uploaded { evaluation } => {
                if !nonresident || source.phase != Phase::Build || !evaluation.bound.is_empty() {
                    return Err(error("upload requires a nonresident host dependency at build"));
                }
                check_transfers(&facts, evaluation, &BTreeSet::new())?;
                &analysis.structures.layouts[name]
            }
        };
        if (source.phase == Phase::Build) != matches!(source.origin, SourceOrigin::Uploaded { .. }) {
            return Err(error("only uploaded sources are constructed during the build"));
        }
        if source.columns.values().any(|original| !layout.contains(original)) {
            return Err(error("source column is absent from its origin layout"));
        }
        let selection = match &source.access {
            SourceAccess::Scan => None,
            SourceAccess::Positions(interval) => Some(interval),
            SourceAccess::Provider { selection } => {
                let built = analysis.gpu_bindings.providers.get(name).is_some_and(|layouts| {
                    layouts.iter().any(|layout| layout.abi == ProviderAbi::Range)
                });
                if !built {
                    return Err(error("provider access without a recorded Range layout"));
                }
                selection.as_ref()
            }
        };
        if let SourceOrigin::Resident { .. } = &source.origin {
            let origin = crate::rows::origin(name, &host_bound)?
                .ok_or_else(|| error("resident source lacks a rename origin"))?;
            let expected = origin.selection.as_ref().map(interval).transpose()?;
            if origin.predicate || expected.as_ref() != selection || origin.columns != source.columns {
                return Err(error("source interval differs from its recorded contiguous selections"));
            }
        } else if selection.is_some() {
            return Err(error("only resident sources fold interval selections"));
        }
    }
    let definitions: Vec<_> = program.sequence.definitions.iter().map(|d| symbol_name(&d.name)).collect();
    if records.steps.iter().map(|(name, _)| name).ne(definitions.iter()) {
        return Err(error("steps differ from the Sequence definitions"));
    }
    let final_name = facts.final_name()?;
    let final_group = facts.sequence.get(&final_name).and_then(|term| grouped_boundary(term, &facts.sequence));
    let mut published: Vec<&String> = Vec::new();
    for (name, step) in &records.steps {
        let term = facts.sequence[name];
        let dispatch = analysis.dispatches.iter().any(|dispatch| {
            dispatch.inner_stores.iter().rev().find(|store| {
                facts.sequence.get(*store).is_some_and(|term| matches!(term, Term::Store { .. }))
            }) == Some(name)
        });
        match step {
            Step::GroupAlias => {
                let same = final_group.is_some_and(|(group, _)| {
                    grouped_boundary(term, &facts.sequence).is_some_and(|(other, _)| other == group)
                });
                if !same || *name == final_name {
                    return Err(error("group alias does not name the terminal group"));
                }
            }
            Step::Group(group) => {
                let Some((ident, _)) = grouped_boundary(term, &facts.sequence).filter(|_| *name == final_name) else {
                    return Err(error("terminal group step is not the terminal grouped publication"));
                };
                let Some(Operator::Group(physical)) = facts.definitions.get(&group.group).map(|d| &d.operator) else {
                    return Err(error("terminal group lacks its Physical group"));
                };
                if symbol_name(ident) != group.group {
                    return Err(error("terminal group names another Physical group"));
                }
                match (&group.completion, physical.keys.is_empty()) {
                    (GroupCompletion::GridPartials { partials }, true) if grid(partials) => {}
                    (GroupCompletion::Segmented { arguments }, false) if grid(arguments) => {}
                    _ => return Err(error("keyless groups combine grid partials; keyed groups use the accumulator")),
                }
                match &group.input {
                    GroupInput::Canonical { publication } => {
                        let canonical = analysis
                            .gpu_publications
                            .get(publication)
                            .is_some_and(|record| record.storage.canonical_provider().is_some());
                        if !published.contains(&publication) || !canonical {
                            return Err(error("canonical group input is not a completed canonical publication"));
                        }
                    }
                    GroupInput::Stored { publication } => {
                        if !published.contains(&publication) {
                            return Err(error("group input publication has not completed"));
                        }
                    }
                    GroupInput::Source { source } => {
                        if !records.sources.contains_key(source) && !uploaded(records, analysis, source) {
                            return Err(error("group source has no device realization"));
                        }
                    }
                    GroupInput::Host { input, upload } => {
                        if *input != symbol_name(&physical.input) {
                            return Err(error("host group input is not the Physical group input"));
                        }
                        if let Some(evaluation) = upload {
                            check_transfers(&facts, evaluation, &published.iter().map(|n| (*n).clone()).collect())?;
                        } else if !records.sources.contains_key(input) && !uploaded(records, analysis, input) {
                            return Err(error("host group input is neither uploaded nor a device source"));
                        }
                    }
                }
            }
            Step::Dispatch { provider } => {
                if !dispatch {
                    return Err(error("dispatch step without a checked dispatch"));
                }
                check_provider(&facts, provider.as_ref())?;
                published.push(name);
            }
            Step::HostUnion(union) => {
                if dispatch || host_union(term, &facts.sequence).is_none() || !facts.host_store_term(name) {
                    return Err(error("host union step without a host union store"));
                }
                for arm in &union.arms {
                    let (publication, packed) = match arm {
                        UnionArm::Copy { publication } => (publication, false),
                        UnionArm::Decode { publication, grid: g } => (publication, grid(g)),
                    };
                    let codec = analysis
                        .gpu_publications
                        .get(publication)
                        .is_some_and(|record| record.storage.codec().is_some());
                    if !published.contains(&publication) || codec != packed {
                        return Err(error("union arm realization contradicts its publication storage"));
                    }
                }
                let codec = analysis.gpu_publications.get(name).is_some_and(|record| record.storage.codec().is_some());
                if codec != union.pack.as_ref().is_some_and(grid) {
                    return Err(error("union packing contradicts its recorded codec"));
                }
                check_provider(&facts, union.provider.as_ref())?;
                published.push(name);
            }
            Step::Upload { evaluation, provider } => {
                if dispatch || !facts.host_store_term(name) || !analysis.results.contains_key(name) {
                    return Err(error("upload step without a host store"));
                }
                check_transfers(&facts, evaluation, &published.iter().map(|n| (*n).clone()).collect())?;
                check_provider(&facts, provider.as_ref())?;
            }
            Step::Host => {
                if dispatch {
                    return Err(error("checked dispatch has no realization step"));
                }
            }
        }
    }
    if let Some(output) = &records.output {
        if !grid(&output.pack) || !facts.definitions.contains_key(&output.source) {
            return Err(error("host output names no Physical source"));
        }
    }
    Ok(())
}

/// Whether an earlier step uploads the Physical result `name`.
fn uploaded(records: &Realization, analysis: &Analysis, name: &str) -> bool {
    records.steps.iter().any(|(publication, step)| match step {
        Step::Upload { .. } => analysis.results.get(publication).is_some_and(|result| result == name),
        Step::Group(TerminalGroup { input: GroupInput::Host { input, upload: Some(_) }, .. }) => input == name,
        _ => false,
    })
}

fn check_transfers(facts: &Facts, evaluation: &HostEvaluation, published: &BTreeSet<String>) -> syn::Result<()> {
    for name in &evaluation.bound {
        if !published.contains(name) {
            return Err(error("host evaluation binds an incomplete publication"));
        }
    }
    for transfer in &evaluation.transfers {
        let Granularity::CompleteRows = transfer.granularity;
        let known = match &transfer.source {
            TransferSource::Publication(name) => evaluation.bound.contains(name),
            TransferSource::Resident(host) => facts.resident.contains(host),
        };
        if !known {
            return Err(error("transfer names neither a bound publication nor a resident input"));
        }
    }
    Ok(())
}

/// Descriptor bounds enclose the column: resident origins rename an input
/// column into it, uploads publish the column, Scalar is its carrier.
fn check_provider(facts: &Facts, provider: Option<&ProviderBuild>) -> syn::Result<()> {
    let Some(provider) = provider else { return Ok(()) };
    for (column, bounds) in &provider.columns {
        let sources = match &bounds.base {
            BaseBounds::Union(sources) => sources.iter().collect(),
            BaseBounds::Scalar(ty) => {
                let carrier = facts.carriers.values().find_map(|types| types.get(column));
                if carrier != Some(ty) {
                    return Err(error("descriptor Scalar bounds use another carrier"));
                }
                Vec::new()
            }
            BaseBounds::None => Vec::new(),
        };
        for source in sources.into_iter().chain(&bounds.merges) {
            let sound = match source {
                BoundSource::Resident { host, index } => {
                    facts.resident.contains(host)
                        && facts.program.structures.physical.program.definitions.iter().any(|definition| {
                            let Operator::Rename(rename) = &definition.operator else { return false };
                            symbol_name(&rename.relation) == *host
                                && rename.mapping.iter().any(|mapping| {
                                    symbol_name(&mapping.target) == *column
                                        && facts.analysis.structures.host_layouts[host].get(*index)
                                            == Some(&symbol_name(&mapping.source))
                                })
                        })
                }
                BoundSource::Upload { publication, index } => facts
                    .analysis
                    .structures
                    .layouts
                    .get(publication)
                    .is_some_and(|layout| layout.get(*index) == Some(column)),
            };
            if !sound {
                return Err(error("descriptor bounds name a domain that does not publish the column"));
            }
        }
    }
    Ok(())
}
