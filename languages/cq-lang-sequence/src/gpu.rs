//! Checked GPU launch and publication refinement. Emitters consume these records.
use crate::{Analysis, Capability, Monoid, Obligation, Program, Term};
use cq_lang_structures::physical::symbol_name;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Machine {
    pub block_threads: u32,
    /// Catalog identity predicate, never a type-name suffix test.
    pub contiguous_range: fn(&syn::Type) -> bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Partition {
    One,
    Tiles { rows: usize },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Publication {
    Rows,
    DeferredRange { provider: String, keys: Vec<String> },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OffsetScan {
    None,
    DeviceExclusive,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoldCompletion {
    Local,
    BlockPartials { monoids: Vec<Monoid> },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Synchronization {
    CountAndFailure,
    RangeOffsets,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grid {
    KernelOccupancy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Execution {
    pub partition: Partition,
    /// SM count times actual kernel resident blocks, capped by domain tile count.
    pub grid: Grid,
    pub block_threads: u32,
    pub publication: Publication,
    pub offsets: OffsetScan,
    pub fold: FoldCompletion,
    pub synchronization: Vec<Synchronization>,
}

/// Formal rule GPU-Launch: grading records partition, kernel occupancy policy,
/// fold completion and synchronization before any launch is emitted.
pub(crate) fn derive(
    program: &Program,
    analysis: &Analysis,
    machine: &Machine,
) -> syn::Result<Vec<Execution>> {
    if machine.block_threads == 0 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "GPU-Launch: zero block width",
        ));
    }
    let definitions: BTreeMap<_, _> = program
        .sequence
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), &d.term))
        .collect();
    let physical: BTreeMap<_, _> = program
        .structures
        .physical
        .program
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), &d.operator))
        .collect();
    let published: BTreeSet<_> = analysis
        .dispatches
        .iter()
        .filter_map(|d| {
            d.inner_stores
                .iter()
                .rev()
                .find(|name| matches!(definitions[*name], Term::Store { .. }))
                .cloned()
        })
        .collect();
    let mut sources = published.clone();
    sources.extend(
        program
            .sequence
            .definitions
            .iter()
            .filter_map(|definition| {
                let name = symbol_name(&definition.name);
                (matches!(definition.term, Term::Store { .. })
                    && analysis
                        .levels
                        .get(&name)
                        .is_some_and(|level| level.len() == 1))
                .then_some(name)
            }),
    );
    let mut host_results = BTreeSet::new();
    let mut host_references = BTreeSet::new();
    for definition in &program.sequence.definitions {
        let name = symbol_name(&definition.name);
        if analysis
            .levels
            .get(&name)
            .is_some_and(|level| level.len() == 1)
            && !published.contains(&name)
        {
            references(
                &definition.term,
                &definitions,
                &sources,
                &mut host_references,
            );
            if let Some(result) = analysis.results.get(&name) {
                let mut pending = vec![result.clone()];
                while let Some(result) = pending.pop() {
                    if host_results.insert(result.clone()) {
                        if let Some(operator) = physical.get(&result) {
                            pending.extend(operator.operands().into_iter().map(symbol_name));
                        }
                    }
                }
            }
        }
    }
    let final_name = program
        .sequence
        .definitions
        .last()
        .map(|d| symbol_name(&d.name));
    let mut executions = Vec::with_capacity(analysis.dispatches.len());
    for dispatch in &analysis.dispatches {
        let mut execution = Execution {
            partition: dispatch
                .parts
                .map_or(Partition::One, |rows| Partition::Tiles { rows }),
            grid: Grid::KernelOccupancy,
            block_threads: machine.block_threads,
            publication: Publication::Rows,
            offsets: OffsetScan::None,
            fold: FoldCompletion::Local,
            synchronization: vec![Synchronization::CountAndFailure],
        };
        if let Some(name) = dispatch
            .inner_stores
            .iter()
            .rev()
            .find(|name| matches!(definitions[*name], Term::Store { .. }))
        {
            let term = definitions[name];
            let inner = if let Term::Store { inner, .. } = term {
                inner.as_ref()
            } else {
                term
            };
            let chains = linear(inner, &definitions, &sources, &analysis.capabilities);
            for steps in &chains {
                let mut dimensions = Vec::new();
                for step in steps {
                    match step {
                        Step::Tile => dimensions.push(true),
                        Step::Lookup(..) => dimensions.push(false),
                        Step::Flatten => {
                            dimensions.pop();
                        }
                        Step::Fold(monoid) if dimensions.last() != Some(&true) => {
                            match &mut execution.fold {
                                FoldCompletion::Local => {
                                    execution.fold = FoldCompletion::BlockPartials {
                                        monoids: vec![monoid.clone()],
                                    }
                                }
                                FoldCompletion::BlockPartials { monoids } => {
                                    monoids.push(monoid.clone())
                                }
                            }
                            // A whole-input fold has a distributed implementation cover,
                            // independent of the single logical output part.
                            if dispatch.parts.is_none() {
                                execution.partition = Partition::Tiles {
                                    rows: machine.block_threads as usize,
                                };
                            }
                        }
                        _ => {}
                    }
                }
                if chains.len() == 1
                    && Some(name) != final_name.as_ref()
                    && !host_references.contains(name)
                    && analysis.results.get(name).is_some_and(|result| {
                        !analysis.structures.built.contains_key(result)
                            && !host_results.contains(result)
                    })
                {
                    if let Some(at) = steps.iter().rposition(|s| matches!(s, Step::Lookup(..))) {
                        if steps[at + 1..]
                            .iter()
                            .all(|s| matches!(s, Step::Flatten | Step::Store))
                            && steps[at + 1..].iter().any(|s| matches!(s, Step::Flatten))
                        {
                            let Step::Lookup(provider, keys) = &steps[at] else {
                                unreachable!()
                            };
                            if analysis.obligations.iter().any(|o| {
                                matches!(o,
                                Obligation::Provider { provider: p, keys: k, ty, .. }
                                if p == provider && k == keys && (machine.contiguous_range)(ty))
                            }) {
                                execution.publication = Publication::DeferredRange {
                                    provider: provider.clone(),
                                    keys: keys.clone(),
                                };
                                execution.offsets = OffsetScan::DeviceExclusive;
                                execution
                                    .synchronization
                                    .push(Synchronization::RangeOffsets);
                            }
                        }
                    }
                }
            }
        }
        executions.push(execution);
    }
    Ok(executions)
}

#[derive(Clone)]
enum Step {
    Tile,
    Lookup(String, Vec<String>),
    Flatten,
    Store,
    Fold(Monoid),
    Other,
}
fn linear(
    term: &Term,
    defs: &BTreeMap<String, &Term>,
    published: &BTreeSet<String>,
    capabilities: &BTreeMap<(String, Vec<String>), Capability>,
) -> Vec<Vec<Step>> {
    let (inner, step) = match term {
        Term::Rows(_) => return vec![Vec::new()],
        Term::Union(arms) => {
            return arms
                .iter()
                .flat_map(|arm| linear(arm, defs, published, capabilities))
                .collect();
        }
        Term::Choose(arms) => {
            return arms
                .iter()
                .flat_map(|(_, arm)| linear(arm, defs, published, capabilities))
                .collect();
        }
        Term::Ref(name) => {
            let name = symbol_name(name);
            return if published.contains(&name) {
                vec![Vec::new()]
            } else {
                linear(defs[&name], defs, published, capabilities)
            };
        }
        Term::Compact(_, inner) if matches!(inner.as_ref(), Term::Lookup { .. }) => {
            let Term::Lookup { inner, .. } = inner.as_ref() else {
                unreachable!()
            };
            (inner.as_ref(), Step::Other)
        }
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Canon(inner)
        | Term::Group(_, inner) => (inner.as_ref(), Step::Other),
        Term::Flatten(inner) => (inner.as_ref(), Step::Flatten),
        Term::Store { inner, .. } => (inner.as_ref(), Step::Store),
        Term::Tile { inner, .. } => (inner.as_ref(), Step::Tile),
        Term::Fold(monoid, inner) => (inner.as_ref(), Step::Fold(monoid.clone())),
        Term::Lookup {
            provider,
            keys,
            inner,
        } => {
            let provider = symbol_name(provider);
            let mut keys: Vec<_> = keys.iter().map(symbol_name).collect();
            keys.sort();
            let step = if capabilities.get(&(provider.clone(), keys.clone()))
                == Some(&Capability::Membership)
            {
                Step::Other
            } else {
                Step::Lookup(provider, keys)
            };
            (inner.as_ref(), step)
        }
        Term::PartitionK { inner, .. } => (inner.as_ref(), Step::Other),
    };
    let mut chains = linear(inner, defs, published, capabilities);
    for steps in &mut chains {
        steps.push(step.clone());
    }
    chains
}
fn references(
    term: &Term,
    defs: &BTreeMap<String, &Term>,
    sources: &BTreeSet<String>,
    output: &mut BTreeSet<String>,
) {
    match term {
        Term::Ref(name) => {
            let name = symbol_name(name);
            if output.insert(name.clone()) && !sources.contains(&name) {
                references(defs[&name], defs, sources, output);
            }
        }
        Term::Rows(_) => {}
        Term::Choose(arms) => {
            for (_, arm) in arms {
                references(arm, defs, sources, output);
            }
        }
        Term::Union(arms) => {
            for arm in arms {
                references(arm, defs, sources, output);
            }
        }
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Canon(inner)
        | Term::Group(_, inner)
        | Term::Fold(_, inner)
        | Term::Store { inner, .. }
        | Term::Tile { inner, .. }
        | Term::Lookup { inner, .. }
        | Term::PartitionK { inner, .. } => references(inner, defs, sources, output),
    }
}
