//! Checked GPU launch and publication refinement. Emitters consume these records.
pub mod bindings;
pub mod publication;
pub mod realization;

use crate::{Analysis, Capability, Monoid, Obligation, OutputProtocol, Program, Term};
use cq_lang_structures::physical::symbol_name;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Machine {
    pub block_threads: u32,
    /// Initial row capacity of a `reserve` output without a declared capacity.
    pub reserve_rows: usize,
    /// GR-RetainPrefix: evidence-bounded rows a stored prefix must reach to
    /// replace a stream root smaller than one root tile as the distributed root.
    pub retain_prefix_rows: usize,
    /// GR-RootTile: root tiles that fill the device (two resident blocks per
    /// SM); a smaller estimated root is cut into about this many tiles.
    pub root_tiles: usize,
    /// Target refinement records concrete provider/storage bindings upstream.
    pub bindings: fn(&crate::structures::physical::Program, &crate::structures::Program, &Program, &crate::structures::Analysis, &Analysis) -> syn::Result<bindings::Bindings>,
    /// Target realization after publication storage is recorded.
    pub realize: fn(&crate::structures::physical::Program, &crate::structures::Program, &Program, &crate::structures::Analysis, &Analysis) -> syn::Result<realization::Realization>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Partition {
    /// Every source has an evidence bound at most `max_rows` and no materialized child.
    One { max_rows: u32 },
    Tiles {
        rows: usize,
    },
    /// Large domains use ordinary root tiles. A nonempty one-tile chain can
    /// distribute the positions of its recorded child level across blocks.
    Fanout { rows: usize },
    /// GPU-RootSpread: root tiles of `clamp(ceil(domain / resident), 1, max_rows)`
    /// rows, `resident` the recorded launch preparation's block capacity. The
    /// resulting tile count caps the grid except when root sharing is recorded.
    /// `max_rows` is the declared tile, else one block pass of source rows.
    Spread {
        max_rows: usize,
    },
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
    /// GPU-HeadCursor: count and failure are observed by the enclosing
    /// publication point, never by this dispatch.
    HeadCursor,
    RangeOffsets,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grid {
    /// SM count times actual kernel resident blocks, capped by the part count.
    KernelOccupancy,
    /// One thread per element: `ceil(elements / block_threads)` blocks.
    Elements { block_threads: u32 },
}
/// Cold reservation for an ordinary output dispatch. A seed is never a
/// certificate that the output fits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReservationSeed {
    Zero,
    /// The Store's declared capacity.
    Declared,
    /// The physical result's conditional BE-PrefixBound.
    Evidence(u32),
    /// The machine's default reservation for an undeclared `reserve`.
    Rows(usize),
}

/// Owner of the capacity hint. Only a validated prefix is published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InitialCapacity {
    NoStore,
    /// Context-local compiled dispatch state, initialized from the seed.
    DispatchCount(ReservationSeed),
    /// The arena's last validated count (GPU-ReservePrevious).
    PreviousCount,
}
/// Output compaction: each lane gathers its live stripe items and the block
/// reserves one contiguous output range per stripe batch. A fold result
/// publishes through a per-row reservation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reservation {
    StripeBatch,
}
/// Implementation scope of a fold at its stripe level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoldScope {
    /// Block partials over the whole dispatch domain, then the combine step.
    Whole,
    /// Block-local fold enclosing the first `n` local tiles of its level.
    Tiles(usize),
}
/// One strip-mined level: the dispatch source, then each expanding lookup in
/// pre-order. Implicit tile defaults are resolved here, never in a printer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Strip {
    /// Rows per expansion batch; None at the source, which uses `partition`.
    pub rows: Option<usize>,
    /// Rows per lane per stripe batch.
    pub items: usize,
    /// Nested local partitions after the level's first tile.
    pub nested: Vec<usize>,
    pub fold: Option<FoldScope>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainExecution {
    pub strips: Vec<Strip>,
    /// Last child strip at which a one-tile root must split; an earlier scan
    /// with enough children for the grid claims the split first.
    pub split_child: Option<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Execution {
    pub partition: Partition,
    pub block_threads: u32,
    pub publication: Publication,
    pub offsets: OffsetScan,
    pub fold: FoldCompletion,
    pub synchronization: Vec<Synchronization>,
    pub capacity: InitialCapacity,
    pub reservation: Reservation,
    /// One record per publication chain, in Sequence constructor order.
    pub chains: Vec<ChainExecution>,
    pub share: Share,
    pub meta: Meta,
}

/// GPU-ResidentMeta: where a launch attempt's count, failure and claim-cursor
/// words live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Meta {
    /// A fresh zeroed allocation per attempt.
    Fresh,
    /// The arena's resident words, zero on entry. The last block to finish
    /// publishes count and failure to the arena's result words and re-zeroes
    /// every word the launch used, so no attempt allocates or clears metadata.
    Resident,
}

/// GPU-ExpansionShare: how blocks divide the descendants of a root tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Share {
    /// The block that claimed the root tile expands all of its descendants.
    None,
    /// At every expansion, a block holding more than `min_chunks` child
    /// batches publishes its expansion state to one of `slots` pool slots;
    /// idle blocks copy the state and claim batches until all are claimed.
    Expansions { slots: usize, min_chunks: usize },
    /// GPU-FrontierResume: after its first batch, a producer may save an
    /// expansion with at least `min_chunks` remaining. The second launch
    /// partitions those saved batches among its blocks.
    Frontier { slots: usize, min_chunks: usize },
}

/// Pool slots for published expansions under `Spread`.
pub const SHARE_SLOTS: usize = 256;
/// An expansion with more child batches left than this is published. A copy
/// of the expansion state and the claims cost about as much as a few batches,
/// so only expansions with many batches left pay for them.
pub const SHARE_MIN_CHUNKS: usize = 32;

// The arena frontier layout uses 256 lengths followed by an aligned u64
// prefix in the resident pool. This is not the general stealing pool.
const FRONTIER_SLOTS: usize = 256;

/// Formal rule GPU-Launch: grading records partition, fold completion and
/// synchronization before emission; target realization owns launch preparation.
pub(crate) fn derive(
    physical: &crate::structures::physical::Program,
    structure_analysis: &crate::structures::Analysis,
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
    let (definitions, published, sources) = chain_sources(program, analysis);
    let physical: BTreeMap<_, _> = physical.program.definitions
        .iter()
        .map(|d| (symbol_name(&d.name), &d.operator))
        .collect();
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
            partition: Partition::Tiles {
                rows: dispatch.parts.unwrap_or(machine.block_threads as usize),
            },
            block_threads: machine.block_threads,
            publication: Publication::Rows,
            offsets: OffsetScan::None,
            fold: FoldCompletion::Local,
            synchronization: vec![Synchronization::CountAndFailure],
            capacity: InitialCapacity::NoStore,
            reservation: Reservation::StripeBatch,
            chains: Vec::new(),
            share: Share::None,
            meta: Meta::Fresh,
        };
        if let Some(name) = dispatch
            .inner_stores
            .iter()
            .rev()
            .find(|name| matches!(definitions[*name], Term::Store { .. }))
        {
            let term = definitions[name];
            let (inner, declared) = if let Term::Store { inner, capacity } = term {
                (inner.as_ref(), capacity.is_some())
            } else {
                (term, false)
            };
            // Formal rule GPU-DispatchReservation: seed the ordinary dispatch's
            // context-local count slot; neither a seed nor history is a bound.
            execution.capacity = match dispatch.concatenation {
                OutputProtocol::ReservePrevious => InitialCapacity::PreviousCount,
                _ if declared => InitialCapacity::DispatchCount(ReservationSeed::Declared),
                _ => {
                    let seed = match prefix_rows(analysis, name) {
                        Some(rows) => ReservationSeed::Evidence(rows),
                        None if dispatch.concatenation == OutputProtocol::Reserve => {
                            ReservationSeed::Rows(machine.reserve_rows)
                        }
                        None => ReservationSeed::Zero,
                    };
                    InitialCapacity::DispatchCount(seed)
                }
            };
            let chains = linear(inner, &definitions, &sources, &analysis.capabilities);
            for steps in &chains {
                let mut dimensions = Vec::new();
                for step in steps {
                    match step {
                        Step::Tile { .. } => dimensions.push(true),
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
                        }
                        _ => {}
                    }
                }
                if chains.len() == 1
                    && Some(name) != final_name.as_ref()
                    && !host_references.contains(name)
                    && analysis.results.get(name).is_some_and(|result| {
                        !structure_analysis.built.contains_key(result)
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
                                Obligation::Provider { provider: p, keys: k, instance, .. }
                                if p == provider && k == keys
                                    && structure_analysis.providers.get(&instance.token)
                                        .is_some_and(|declaration| declaration.facts.contiguous_range))
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
            let deferred = match &execution.publication {
                Publication::DeferredRange { provider, keys } => Some((provider, keys)),
                Publication::Rows => None,
            };
            execution.chains = chains
                .iter()
                .map(|steps| {
                    let mut strips = Vec::new();
                    for (operations, source) in levels(steps, deferred) {
                        strips.push(strip(operations, source, machine.block_threads as usize));
                    }
                    ChainExecution { strips, split_child: None }
                })
                .collect();
            let width = dispatch.parts.unwrap_or(machine.block_threads as usize);
            if (program.sequence.roots != crate::Roots::Spread || !fold_free(&execution))
                && underfilled_expansion(&execution, &chains, analysis, width)
            {
                execution.partition = Partition::Fanout { rows: width };
                for (steps, chain) in chains.iter().zip(&mut execution.chains) {
                    if chain.strips.len() > 1 && root_estimate(steps, analysis).is_none_or(|n| n <= width as u128) {
                        let children = root_loop_children(steps, chain);
                        chain.split_child = (children != 0).then_some(children);
                    }
                }
            } else if dispatch.parts.is_none()
                && let Some(max_rows) = root_bound(&execution, &chains, analysis)
                && max_rows <= execution.block_threads
            {
                execution.partition = Partition::One { max_rows };
            }
        }
        if let Some(max_rows) = spread_rows(program, &execution, dispatch.parts) {
            execution.partition = Partition::Spread { max_rows };
            execution.share = shared_expansions(&execution);
        }
        execution.meta = resident_meta(&execution);
        executions.push(execution);
    }
    Ok(executions)
}

/// Formal rule GPU-RootSpread: under `roots spread`, a fold-free dispatch with
/// a publication records `Spread`, bounded by its declared distributed tile or
/// else one block pass of its widest source strip. Folds keep their declared
/// partition, because block-local fold scopes depend on the root tile.
fn spread_rows(program: &Program, execution: &Execution, parts: Option<usize>) -> Option<usize> {
    (program.sequence.roots == crate::Roots::Spread
        && !execution.chains.is_empty()
        && fold_free(execution))
        .then(|| {
            parts.unwrap_or_else(|| {
                let items = execution
                    .chains
                    .iter()
                    .filter_map(|chain| chain.strips.first())
                    .map(|strip| strip.items)
                    .max()
                    .unwrap_or(1);
                execution.block_threads as usize * items
            })
        })
}

fn root_result<'a>(steps: &'a [Step], analysis: &'a Analysis) -> Option<&'a String> {
    match steps.first()? {
        Step::Rows(name) => Some(name),
        Step::Stored(name) => analysis.results.get(name),
        _ => None,
    }
}

fn root_rows(steps: &[Step], analysis: &Analysis) -> Option<u128> {
    analysis.prefix_bounds.get(root_result(steps, analysis)?)?.rows
}

/// BE-PrefixEstimate of the root: geometry advice, never a capacity.
fn root_estimate(steps: &[Step], analysis: &Analysis) -> Option<u128> {
    analysis.prefix_bounds.get(root_result(steps, analysis)?)?.estimate
}

// A whole fold's suffix executes in its combine kernel, not in the root loop.
fn root_loop_children(steps: &[Step], chain: &ChainExecution) -> usize {
    steps.iter().take_while(|s| !matches!(s, Step::Fold(_)))
        .filter(|s| matches!(s, Step::Lookup(..)))
        .take(chain.strips.len().saturating_sub(1)).count()
}

// Formal rule GPU-OnePart: the evidence for every source must fit a block,
// and no strip may materialize a child. Deferred ranges publish parents.
fn root_bound(execution: &Execution, chains: &[Vec<Step>], analysis: &Analysis) -> Option<u32> {
    if chains.is_empty() { return None; }
    chains.iter().zip(&execution.chains).try_fold(0u32, |max, (steps, chain)| {
        if chain.strips.len() != 1 { return None; }
        Some(max.max(u32::try_from(root_rows(steps, analysis)?).ok()?))
    })
}

// Formal rule GPU-DistributeExpansion: tile-local folds must preserve their
// exact cover. Whole-domain folds instead combine the blocks' disjoint work.
fn underfilled_expansion(
    execution: &Execution, chains: &[Vec<Step>], analysis: &Analysis, width: usize,
) -> bool {
    execution.chains.iter().flat_map(|c| &c.strips)
        .all(|s| !matches!(s.fold, Some(FoldScope::Tiles(_))))
        && chains.iter().zip(&execution.chains).any(|(steps, chain)| {
            root_loop_children(steps, chain) != 0
                && root_estimate(steps, analysis).is_none_or(|rows| rows <= width as u128)
        })
}

fn fold_free(execution: &Execution) -> bool {
    execution.fold == FoldCompletion::Local
        && execution.chains.iter().flat_map(|chain| &chain.strips)
            .all(|strip| strip.fold.is_none())
}

fn expanding_spread(execution: &Execution) -> bool {
    matches!(
        (&execution.partition, execution.chains.as_slice()),
        (Partition::Spread { .. }, [chain]) if chain.strips.len() >= 2
    )
}

/// Formal rule GPU-ExpansionShare: a spread single-chain dispatch that expands
/// at least once may hand any expansion's child batches to other blocks. A
/// block whose expansion holds more than `min_chunks` batches publishes the
/// expansion's block state (parents, range begins, offsets) to one of `slots`
/// pool slots; every batch is claimed exactly once, by the owner or by an idle
/// block that copied the state, so the published multiset is unchanged.
fn shared_expansions(execution: &Execution) -> Share {
    if expanding_spread(execution) {
        Share::Expansions { slots: SHARE_SLOTS, min_chunks: SHARE_MIN_CHUNKS }
    } else {
        Share::None
    }
}

fn prefix_rows(analysis: &Analysis, store: &str) -> Option<u32> {
    let result = analysis.results.get(store)?;
    u32::try_from(analysis.prefix_bounds.get(result)?.rows?).ok()
}

/// Formal rule GPU-ResidentMeta: an arena terminal (`reserve_previous`, the
/// recorded arena protocol) without a block-partial combine keeps its metadata
/// in the arena's resident words. The combine kernel reads the dispatch's
/// words after the main kernel, so a block-partial fold keeps fresh words.
fn resident_meta(execution: &Execution) -> Meta {
    if execution.capacity == InitialCapacity::PreviousCount
        && execution.fold == FoldCompletion::Local
    {
        Meta::Resident
    } else {
        Meta::Fresh
    }
}

/// Sequence definitions, dispatch publications, and the publication
/// boundaries where a dispatch chain stops (completed or host stores).
fn chain_sources<'a>(
    program: &'a Program,
    analysis: &Analysis,
) -> (
    BTreeMap<String, &'a Term>,
    BTreeSet<String>,
    BTreeSet<String>,
) {
    let definitions: BTreeMap<_, _> = program
        .sequence
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), &d.term))
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
    (definitions, published, sources)
}

/// Strip-mined levels of one chain in pre-order: the source, then the
/// remainder after each expanding lookup. A fold placed at a level splits its
/// walk at the fold; a deferred-range lookup closes the chain without a level.
fn levels<'a>(
    steps: &'a [Step],
    deferred: Option<(&String, &Vec<String>)>,
) -> Vec<(&'a [Step], bool)> {
    fn level<'a>(
        operations: &'a [Step],
        source: bool,
        deferred: Option<(&String, &Vec<String>)>,
        output: &mut Vec<(&'a [Step], bool)>,
    ) {
        output.push((operations, source));
        let at = fold_scope(operations, source).and_then(|_| {
            operations
                .iter()
                .position(|step| matches!(step, Step::Fold(_)))
        });
        match at {
            Some(at) => {
                walk(&operations[..at], deferred, output);
                walk(&operations[at + 1..], deferred, output);
            }
            None => walk(operations, deferred, output),
        }
    }
    fn walk<'a>(
        operations: &'a [Step],
        deferred: Option<(&String, &Vec<String>)>,
        output: &mut Vec<(&'a [Step], bool)>,
    ) {
        for (at, step) in operations.iter().enumerate() {
            if let Step::Lookup(provider, keys) = step {
                let rest = &operations[at + 1..];
                if deferred == Some((provider, keys))
                    && rest
                        .iter()
                        .all(|step| matches!(step, Step::Flatten | Step::Store))
                {
                    return;
                }
                level(rest, false, deferred, output);
                return;
            }
        }
    }
    let mut output = Vec::new();
    level(steps, true, deferred, &mut output);
    output
}

/// Tiles of a level before its first expanding lookup or fold.
fn local_tiles(operations: &[Step]) -> Vec<(usize, Option<usize>)> {
    operations
        .iter()
        .take_while(|step| !matches!(step, Step::Lookup(..) | Step::Fold(_)))
        .filter_map(|step| match step {
            Step::Tile { n, items } => Some((*n, *items)),
            _ => None,
        })
        .collect()
}

/// Innermost open partition: a tile index, or None for an open lookup/none.
fn active_tile(operations: &[Step]) -> Option<usize> {
    let mut partitions = Vec::new();
    for (index, step) in operations.iter().enumerate() {
        match step {
            Step::Tile { .. } => partitions.push(Some(index)),
            Step::Lookup(..) => partitions.push(None),
            Step::Flatten => {
                partitions.pop();
            }
            _ => {}
        }
    }
    partitions.last().copied().flatten()
}

/// The level's fold scope: the fold's innermost open partition is a tile of
/// this level, or the whole domain at the source with no open tile.
fn fold_scope(operations: &[Step], source: bool) -> Option<FoldScope> {
    let at = operations
        .iter()
        .position(|step| matches!(step, Step::Fold(_)))?;
    let target = active_tile(&operations[..at]);
    let next = operations
        .iter()
        .position(|step| matches!(step, Step::Lookup(..)))
        .unwrap_or(at)
        .min(at);
    let current = active_tile(&operations[..next]);
    if !(source && target.is_none()) && (target != current || target.is_none()) {
        return None;
    }
    Some(match target {
        None => FoldScope::Whole,
        Some(target) => FoldScope::Tiles(
            operations[..=target]
                .iter()
                .filter(|step| matches!(step, Step::Tile { .. }))
                .count(),
        ),
    })
}

/// Resolve a level's implicit strip defaults: omitted items cover a tile with
/// one block pass, an expansion without a tile batches one block of children,
/// and an untiled level processes one item per lane.
fn strip(operations: &[Step], source: bool, block: usize) -> Strip {
    let tiles: Vec<_> = local_tiles(operations)
        .into_iter()
        .map(|(n, items)| (n, items.unwrap_or(n.div_ceil(block))))
        .collect();
    Strip {
        rows: (!source).then(|| tiles.first().map_or(block, |tile| tile.0)),
        items: tiles.last().map_or(1, |tile| tile.1),
        nested: tiles.iter().skip(1).map(|tile| tile.0).collect(),
        fold: fold_scope(operations, source),
    }
}

fn resident_raw_terminal<'a>(
    program: &Program,
    analysis: &'a Analysis,
    dispatch: &crate::Dispatch,
) -> Option<&'a publication::PublicationRecord> {
    let (terminal, OutputProtocol::ReservePrevious) = analysis.output.as_ref()? else {
        return None;
    };
    if dispatch.inner_stores.last() != Some(terminal)
        || program.sequence.output != Some(OutputProtocol::ReservePrevious)
        || dispatch.concatenation != OutputProtocol::ReservePrevious
    {
        return None;
    }
    let execution = dispatch.gpu.as_ref()?;
    if execution.meta != Meta::Resident
        || execution.capacity != InitialCapacity::PreviousCount
        || execution.publication != Publication::Rows
        || execution.offsets != OffsetScan::None
        || execution.fold != FoldCompletion::Local
    {
        return None;
    }
    analysis.gpu_publications.get(terminal).filter(|record| record.arena.is_some())
}

/// Formal rule GPU-HeadCursor: the enclosing binding can own completion of
/// a raw terminal append only when no subsequent Sequence operation needs
/// its length, offsets, or partial folds. Intermediate stores stay private.
fn admits_head_cursor(program: &Program, analysis: &Analysis, dispatch: &crate::Dispatch) -> bool {
    use publication::{Granularity, PackedKeys, PublicationStorage};
    resident_raw_terminal(program, analysis, dispatch).is_some_and(|record| {
        matches!(&record.storage,
            PublicationStorage::PackedKeys(PackedKeys { granularity: Granularity::Rows, .. }))
    })
}

/// Refine the graded dispatch only at a binding-requested head terminal.
/// A refusal leaves the ordinary arena protocol untouched.
pub fn request_head_cursor(program: &Program, analysis: &mut Analysis) -> bool {
    let Some(index) = analysis.dispatches.iter().position(|dispatch| {
        admits_head_cursor(program, analysis, dispatch)
            && dispatch.gpu.as_ref().is_some_and(|execution| matches!(
                execution.synchronization.as_slice(),
                [Synchronization::CountAndFailure] | [Synchronization::HeadCursor]
            ))
    }) else {
        return false;
    };
    analysis.dispatches[index].gpu.as_mut().unwrap().synchronization[0] = Synchronization::HeadCursor;
    true
}

/// Formal rule GPU-FrontierResume: a resident raw terminal with a single
/// expanding, fold-free spread chain can publish its unfinished intervals
/// in one launch and resume them before the same arena attempt completes.
fn admits_frontier(program: &Program, analysis: &Analysis, dispatch: &crate::Dispatch) -> bool {
    resident_raw_terminal(program, analysis, dispatch).is_some()
        && program.sequence.roots == crate::Roots::Spread
        && dispatch.gpu.as_ref().is_some_and(|execution| {
            expanding_spread(execution) && fold_free(execution)
        })
}

/// Refine the graded terminal's expansion share at an explicit arena request.
/// Unlike grouped append, an unsupported frontier is a translation error.
pub fn request_frontier(program: &Program, analysis: &mut Analysis) -> syn::Result<()> {
    let index = analysis.dispatches.iter().position(|dispatch| {
        dispatch.gpu.as_ref().is_some_and(|execution| {
            matches!(execution.share, Share::Expansions { slots: FRONTIER_SLOTS, .. })
        }) && admits_frontier(program, analysis, dispatch)
    });
    let Some(index) = index else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "GPU-FrontierResume requires a resident raw arena terminal with a single expanding, fold-free spread chain and a 256-slot expansion pool",
        ));
    };
    analysis.dispatches[index].gpu.as_mut().unwrap().share =
        Share::Frontier { slots: FRONTIER_SLOTS, min_chunks: 2 };
    Ok(())
}

/// GPU-Launch checker: validate supplied launch records against the
/// checked dispatches without selecting defaults. Any positive implicit strip
/// is sound; explicit tiles, fold scopes and the output protocol are facts.
/// Offset scans and synchronization follow the recorded publication kind.
pub fn check(program: &Program, analysis: &Analysis) -> syn::Result<()> {
    let error = |message: &str| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("GPU-Launch: {message}"),
        )
    };
    let (definitions, _, sources) = chain_sources(program, analysis);
    for dispatch in &analysis.dispatches {
        let Some(execution) = &dispatch.gpu else {
            return Err(error("dispatch lacks its launch record"));
        };
        if execution.block_threads == 0 {
            return Err(error("zero block width"));
        }
        if matches!(execution.partition, Partition::Tiles { rows: 0 }) {
            return Err(error("zero partition rows"));
        }
        // GPU-OffsetScan: deferred lengths need an exclusive scan and its
        // total/failure readback after the producer's count/failure readback.
        // Rows have neither range offsets nor a range readback.
        let (offsets, synchronization): (_, &[Synchronization]) = match &execution.publication {
            Publication::Rows if execution.synchronization == [Synchronization::HeadCursor] => {
                if !admits_head_cursor(program, analysis, dispatch) {
                    return Err(error("GPU-HeadCursor requires a resident raw packed-key terminal"));
                }
                (OffsetScan::None, &[Synchronization::HeadCursor])
            }
            Publication::Rows => (OffsetScan::None, &[Synchronization::CountAndFailure]),
            Publication::DeferredRange { .. } => (
                OffsetScan::DeviceExclusive,
                &[
                    Synchronization::CountAndFailure,
                    Synchronization::RangeOffsets,
                ],
            ),
        };
        if execution.offsets != offsets {
            return Err(error("offset scan contradicts the recorded publication"));
        }
        if execution.synchronization.as_slice() != synchronization {
            return Err(error(
                "synchronization contradicts the recorded publication",
            ));
        }
        // Explicit roots policy fixes spread geometry, independent of estimates.
        let spreads = program.sequence.roots == crate::Roots::Spread
            && !execution.chains.is_empty()
            && fold_free(execution);
        let partition = match &execution.partition {
            Partition::Spread { max_rows } =>
                spreads && *max_rows > 0 && dispatch.parts.is_none_or(|parts| parts == *max_rows),
            Partition::Fanout { rows } =>
                !spreads && *rows > 0 && dispatch.parts.is_none_or(|parts| parts == *rows),
            Partition::Tiles { rows } => !spreads && dispatch.parts.is_none_or(|parts| parts == *rows),
            Partition::One { max_rows } =>
                !spreads && dispatch.parts.is_none() && *max_rows <= execution.block_threads,
        };
        if !partition {
            return Err(error(
                "root partition contradicts the recorded roots policy",
            ));
        }
        // GPU-ExpansionShare: sharing is optional; when recorded, the dispatch
        // is a spread single chain that expands at least once, with between
        // one and 65,536 pool slots and a u32 batch threshold.
        if let Share::Expansions { slots, min_chunks } = execution.share {
            if !expanding_spread(execution)
                || !(1..=65_536).contains(&slots)
                || u32::try_from(min_chunks).is_err()
            {
                return Err(error(
                    "expansion sharing requires a spread single-chain dispatch that expands",
                ));
            }
        }
        if let Share::Frontier { slots, min_chunks } = execution.share {
            if !admits_frontier(program, analysis, dispatch)
                || slots != FRONTIER_SLOTS
                || min_chunks == 0
                || u32::try_from(min_chunks).is_err()
            {
                return Err(error(
                    "GPU-FrontierResume requires a resident expanding spread arena terminal, a 256-slot pool and a positive u32 batch threshold",
                ));
            }
        }
        // GPU-ResidentMeta: fresh words are always sound; resident words need
        // the arena terminal protocol without a block-partial combine.
        if execution.meta == Meta::Resident
            && !(execution.capacity == InitialCapacity::PreviousCount
                && execution.fold == FoldCompletion::Local)
        {
            return Err(error(
                "resident metadata contradicts the arena terminal protocol",
            ));
        }
        let Some(name) = dispatch
            .inner_stores
            .iter()
            .rev()
            .find(|name| matches!(definitions[*name], Term::Store { .. }))
        else {
            if execution.capacity != InitialCapacity::NoStore {
                return Err(error("capacity without a publication store"));
            }
            if !execution.chains.is_empty() || !matches!(execution.partition, Partition::Tiles { .. }) {
                return Err(error("chain records without a publication store"));
            }
            continue;
        };
        let Term::Store { inner, capacity } = definitions[name] else {
            unreachable!()
        };
        use InitialCapacity::DispatchCount;
        let evidence = prefix_rows(analysis, name);
        let capacity = match (&dispatch.concatenation, &execution.capacity, capacity) {
            (OutputProtocol::ReservePrevious, InitialCapacity::PreviousCount, _) => true,
            (OutputProtocol::Bounded | OutputProtocol::Count | OutputProtocol::Reserve,
                DispatchCount(ReservationSeed::Declared), Some(_)) => true,
            (OutputProtocol::Count | OutputProtocol::Reserve,
                DispatchCount(ReservationSeed::Evidence(rows)), None) => Some(*rows) == evidence,
            (OutputProtocol::Count, DispatchCount(ReservationSeed::Zero), None) => evidence.is_none(),
            (OutputProtocol::Reserve, DispatchCount(ReservationSeed::Rows(rows)), None) => {
                evidence.is_none() && *rows > 0
            }
            _ => false,
        };
        if !capacity {
            return Err(error("initial capacity contradicts the output protocol"));
        }
        let deferred = match &execution.publication {
            Publication::DeferredRange { provider, keys } => Some((provider, keys)),
            Publication::Rows => None,
        };
        let chains = linear(inner, &definitions, &sources, &analysis.capabilities);
        if chains.len() != execution.chains.len() {
            return Err(error("chain records differ from the publication's chains"));
        }
        if let Partition::One { max_rows } = execution.partition
            && root_bound(execution, &chains, analysis).is_none_or(|bound| bound > max_rows)
        {
            return Err(error("GPU-OnePart: one part lacks bounded non-expanding root evidence"));
        }
        // Formal rule GPU-FanoutLegal: check the supplied geometry, not the
        // estimates or the last-child preference used to select it.
        let fanout = matches!(execution.partition, Partition::Fanout { .. });
        if fanout {
            if execution.chains.iter().flat_map(|c| &c.strips)
                .any(|s| matches!(s.fold, Some(FoldScope::Tiles(_))))
            {
                return Err(error("GPU-FanoutLegal: fanout cannot split a tile-local fold"));
            }
            if !chains.iter().zip(&execution.chains).any(|(s, c)| root_loop_children(s, c) > 0) {
                return Err(error("GPU-FanoutLegal: fanout requires a materialized root-loop child"));
            }
        }
        for (steps, record) in chains.iter().zip(&execution.chains) {
            if record.split_child.is_some_and(|i| !fanout || i == 0 || i > root_loop_children(steps, record)) {
                return Err(error("GPU-ChildSlice: split must name a materialized root-loop child of a fanout partition"));
            }
            let levels = levels(steps, deferred);
            if levels.len() != record.strips.len() {
                return Err(error("strip records differ from the chain's levels"));
            }
            for ((operations, source), strip) in levels.into_iter().zip(&record.strips) {
                let tiles = local_tiles(operations);
                let explicit_items = tiles.last().and_then(|tile| tile.1);
                let sound = strip.items > 0
                    && explicit_items.is_none_or(|items| items == strip.items)
                    && strip
                        .nested
                        .iter()
                        .eq(tiles.iter().skip(1).map(|tile| &tile.0))
                    && match (source, strip.rows) {
                        (true, None) => true,
                        (false, Some(rows)) => {
                            rows > 0 && tiles.first().is_none_or(|tile| tile.0 == rows)
                        }
                        _ => false,
                    }
                    && strip.fold == fold_scope(operations, source)
                    && (strip.fold != Some(FoldScope::Whole)
                        || matches!(execution.fold, FoldCompletion::BlockPartials { .. }));
                if !sound {
                    return Err(error("strip record contradicts its tiles or fold scope"));
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
enum Step {
    Rows(String),
    Stored(String),
    Tile { n: usize, items: Option<usize> },
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
        Term::Rows(name) => return vec![vec![Step::Rows(symbol_name(name))]],
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
                vec![vec![Step::Stored(name)]]
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
        Term::Tile { inner, n, items } => (
            inner.as_ref(),
            Step::Tile {
                n: *n,
                items: *items,
            },
        ),
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
