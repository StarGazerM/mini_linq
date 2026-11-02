//! Sharding is an extension premise, not a base-language execution policy.
//! Admission ownership quantifies over runtime shard IDs: `[start, end)` is the
//! complete domain and `owner = shard_id` is injective. Sharded canonical
//! layouts choose a power-of-two shard count at least the Rayon worker count
//! and route full tuples by hash. Each full/delta layout has one exclusive owner.
//! Publication joins every layout owner before the next evaluation, including
//! owners publishing an empty delta.
use cq_datalog_support::ensure;
use cq_lang_datalog_round::{self as round, Program, storage};
use proc_macro2::TokenStream;
use storage::record;
use syn::Ident;

pub const DEFAULT_THRESHOLD: usize = 1024;
pub const DEFAULT_GRAIN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardedProgram {
    pub round: Program,
    pub certificate: ShardCertificate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardCertificate {
    pub threshold: usize,
    pub grain: usize,
    /// Extension-wide task realization, including rules without a scan binding.
    /// Scan executions repeat this choice as their per-hole witness; key-set
    /// holes are excluded because their consumer already constructs a set.
    pub self_dedup: Option<Ident>,
    pub shard_count: Ident,
    pub admission: Vec<Admission>,
    pub layouts: Vec<LayoutOwnership>,
    pub delta_layouts: Vec<LayoutOwnership>,
    pub publication: Vec<PublicationBarrier>,
}

/// A symbolic partition of every runtime shard, not a fixed compile-time count.
/// The identity owner map gives one and only one writer to each shard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardOwnership {
    pub start: usize,
    pub end: Ident,
    pub owner: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    pub relation: usize,
    pub hash: Ident,
    pub ownership: ShardOwnership,
}

/// One exclusive owner for a whole native layout; full and delta domains are
/// separate. Each domain's owner map is a total permutation of its layout IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutOwnership {
    pub index: usize,
    pub owner: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicationBarrier {
    pub stratum: usize,
    pub relations: Vec<usize>,
    pub layouts: Vec<usize>,
    pub delta_layouts: Vec<usize>,
    pub barrier: Ident,
}

record!(ShardedProgram {
    round: Program,
    certificate: ShardCertificate
});
record!(ShardCertificate {
    threshold: usize, grain: usize, self_dedup: Option<Ident>, shard_count: Ident,
    admission: Vec<Admission>, layouts: Vec<LayoutOwnership>,
    delta_layouts: Vec<LayoutOwnership>, publication: Vec<PublicationBarrier>
});
record!(ShardOwnership {
    start: usize,
    end: Ident,
    owner: Ident
});
record!(Admission {
    relation: usize,
    hash: Ident,
    ownership: ShardOwnership
});
record!(LayoutOwnership {
    index: usize,
    owner: usize
});
record!(PublicationBarrier {
    stratum: usize, relations: Vec<usize>, layouts: Vec<usize>,
    delta_layouts: Vec<usize>, barrier: Ident
});

#[derive(Clone, Copy)]
struct Arguments {
    threshold: usize,
    grain: usize,
}

impl syn::parse::Parse for Arguments {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut threshold = None;
        let mut grain = None;
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            let value: syn::LitInt = input.parse()?;
            let value = value.base10_parse::<usize>()?;
            let slot = if name == "threshold" {
                &mut threshold
            } else if name == "grain" {
                if value == 0 {
                    return Err(syn::Error::new_spanned(name, "grain must be positive"));
                }
                &mut grain
            } else {
                return Err(syn::Error::new_spanned(name, "expected threshold or grain"));
            };
            if slot.replace(value).is_some() {
                return Err(syn::Error::new_spanned(
                    name,
                    "duplicate partition argument",
                ));
            }
            if input.is_empty() {
                break;
            }
            input.parse::<syn::Token![,]>()?;
        }
        Ok(Self {
            threshold: threshold.unwrap_or(DEFAULT_THRESHOLD),
            grain: grain.unwrap_or(DEFAULT_GRAIN),
        })
    }
}

/// Select bulk providers for IDB layouts. Each IDB canonical layout is an
/// ordinary keyed layout: by default only canonical layouts become sharded;
/// `layouts(sharded)` and `layouts(runs)` (exclusive) select every keyed IDB
/// layout, canonical included. EDB and unit-key layouts remain native.
pub fn compile_storage(source: &storage::Program) -> syn::Result<storage::Program> {
    compile_storage_with(source, TokenStream::new())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LayoutChoice {
    Native,
    Sharded,
    Runs,
}

struct StorageArguments {
    layouts: LayoutChoice,
}

impl syn::parse::Parse for StorageArguments {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut layouts = None;
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            let content;
            syn::parenthesized!(content in input);
            let provider: Ident = content.parse()?;
            ensure(
                content.is_empty(),
                &provider,
                "unexpected provider arguments",
            )?;
            if name == "layouts" {
                ensure(
                    layouts.is_none(),
                    &name,
                    "layout selections are mutually exclusive",
                )?;
                layouts = Some(if provider == "sharded" {
                    LayoutChoice::Sharded
                } else if provider == "runs" {
                    LayoutChoice::Runs
                } else {
                    return Err(syn::Error::new_spanned(
                        provider,
                        "expected sharded or runs",
                    ));
                });
            } else {
                return Err(syn::Error::new_spanned(name, "expected layouts"));
            }
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }
        Ok(Self {
            layouts: layouts.unwrap_or(LayoutChoice::Native),
        })
    }
}

pub fn compile_storage_with(
    source: &storage::Program,
    arguments: TokenStream,
) -> syn::Result<storage::Program> {
    let arguments: StorageArguments = syn::parse2(arguments)?;
    storage::contract::check(source)?;
    let mut target = source.clone();
    let canonical: Vec<_> = source
        .providers
        .iter()
        .zip(&source.incremental.strata.source.declarations)
        .filter(|(_, declaration)| declaration.value.is_none())
        .map(|(provider, _)| provider.canonical)
        .collect();
    let native = arguments.layouts == LayoutChoice::Native;
    let full = target
        .layouts
        .iter_mut()
        .enumerate()
        .map(|(i, l)| (Some(i), l));
    let delta = target.delta_layouts.iter_mut().map(|l| (None, l));
    for (index, layout) in full.chain(delta) {
        if native && !index.is_some_and(|index| canonical.contains(&index)) {
            continue;
        }
        if layout.key > 0
            && source
                .incremental
                .strata
                .source
                .declarations
                .iter()
                .any(|declaration| {
                    declaration.name == layout.relation && declaration.value.is_none()
                })
        {
            if ![storage::provider::HASH_SET, storage::provider::HASH_MAP]
                .iter()
                .any(|token| layout.provider.token == token)
            {
                continue;
            }
            let (token, hasher) = if arguments.layouts == LayoutChoice::Runs {
                ensure(
                    layout.hasher == storage::Hasher::Fx,
                    &layout.provider.token,
                    "layouts(runs) requires the default storage hasher BuildHasherDefault<FxHasher>",
                )?;
                (run_layout(), storage::Hasher::None)
            } else {
                (sharded_layout(), layout.hasher)
            };
            let declaration = source
                .incremental
                .strata
                .source
                .declarations
                .iter()
                .find(|declaration| declaration.name == layout.relation)
                .expect("checked IDB layout relation");
            let (key, payload) = storage::layout_schema(declaration, &layout.columns, layout.key);
            layout.provider = storage::Provider {
                ty: storage::provider::spell(&token, &key, &payload, hasher)?,
                token: token.token.clone(),
            };
            layout.hasher = hasher;
            layout.protocol = storage::LayoutProtocol::Bulk;
            storage::provider::declare(&mut target.tokens, token);
        }
    }
    check_storage(source, &target)?;
    Ok(target)
}

/// Formal rule CK-DL-ShardStorage: reverse eligible layout substitutions; every
/// IDB canonical layout must be a sharded or sorted-run layout.
/// Demand exact equality after erasure, including all layout metadata and opaque
/// CQ queues; unit-key and EDB layouts cannot be substituted.
pub fn check_storage(source: &storage::Program, target: &storage::Program) -> syn::Result<()> {
    storage::contract::check(source)?;
    storage::contract::check(target)?;
    ensure(
        source.providers.len() == target.providers.len()
            && source.layouts.len() == target.layouts.len()
            && source.delta_layouts.len() == target.delta_layouts.len(),
        target,
        "sharded storage changed provider coverage",
    )?;
    // Partition only appends declarations of its own tokens that it references.
    ensure(
        target.tokens.starts_with(&source.tokens)
            && target.tokens[source.tokens.len()..].iter().all(|token| {
                (*token == sharded_layout() || *token == run_layout())
                    && target
                        .layouts
                        .iter()
                        .chain(&target.delta_layouts)
                        .any(|layout| layout.provider.token == token.token)
            }),
        target,
        "sharded storage changed the provider token table",
    )?;
    let mut erased = target.clone();
    erased.tokens.truncate(source.tokens.len());
    for (index, declaration) in source
        .incremental
        .strata
        .source
        .declarations
        .iter()
        .enumerate()
    {
        if declaration.value.is_none() {
            let canonical = &target.layouts[target.providers[index].canonical];
            ensure(
                canonical.protocol == storage::LayoutProtocol::Bulk,
                target,
                "IDB canonical layouts require a sharded or sorted-run provider",
            )?;
        }
    }
    for (original, rewritten) in source
        .layouts
        .iter()
        .chain(&source.delta_layouts)
        .zip(erased.layouts.iter_mut().chain(&mut erased.delta_layouts))
    {
        if (rewritten.provider != original.provider || rewritten.hasher != original.hasher)
            && original.key > 0
            && source
                .incremental
                .strata
                .source
                .declarations
                .iter()
                .any(|declaration| {
                    declaration.name == original.relation && declaration.value.is_none()
                })
        {
            // Token identity: a base key set or grouped map becomes a sharded
            // layout keeping its hasher, or a run layout from the Fx default.
            // The target contract checks each token's protocol and spelling.
            let native = [storage::provider::HASH_SET, storage::provider::HASH_MAP]
                .iter()
                .any(|token| original.provider.token == token);
            let sharded =
                rewritten.provider.token == SHARDED_LAYOUT && rewritten.hasher == original.hasher;
            let runs = rewritten.provider.token == RUN_LAYOUT
                && original.hasher == storage::Hasher::Fx
                && rewritten.hasher == storage::Hasher::None;
            ensure(
                native && (sharded || runs) && rewritten.protocol == storage::LayoutProtocol::Bulk,
                target,
                "incorrect layout provider",
            )?;
            rewritten.provider = original.provider.clone();
            rewritten.hasher = original.hasher;
            rewritten.protocol = original.protocol;
        }
    }
    ensure(
        erased == *source,
        target,
        "sharded storage changed an unrelated record",
    )
}

pub fn compile_sharded(source: &Program) -> syn::Result<ShardedProgram> {
    compile_sharded_with(source, TokenStream::new())
}

/// Partition a Round program whose IDB canonical layouts are sharded or sorted runs.
/// Threshold controls the sequential fallback; grain controls scan chunk size.
/// A zero threshold forces the parallel path; grain must remain nonzero.
/// The default `self_dedup per_task` realization removes duplicate owned
/// candidates within each chunk or unpartitioned rule task before admission.
/// A checked certificate with an absent policy and absent scan witnesses retains
/// the direct-vector realization. Both erase to the same task-realized Round.
/// Rule holes ending in the standard argument-free `sequence`, `sequential`,
/// `translate_pull` tail use the registered Structures push adapter instead.
/// Custom lowering, grades, rewrites and arguments, missing push registry entries,
/// and key holes retain their authored queues.
pub fn compile_sharded_with(
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<ShardedProgram> {
    let arguments: Arguments = syn::parse2(arguments)?;
    round::contract::check(source)?;
    ensure(
        source.execution.is_empty(),
        source,
        "partition requires an unannotated Round program",
    )?;
    check_providers(source)?;
    let realized = super::realize_tasks(source)?;
    let target = ShardedProgram {
        round: super::partition_scans(&realized),
        certificate: certificate(source, arguments),
    };
    check_sharded(source, &target)?;
    Ok(target)
}

/// Formal rule CK-DL-Partition: certify exact scan coverage and reversible single-occurrence substitution.
/// Formal rule CK-DL-ShardOwnership: certify disjoint admission shards, exclusive layout owners and a complete publication barrier.
/// Compose the checked Round terminal realization with exact scan erasure,
/// exhaustive scheduling, ownership, and the complete publication barrier.
/// After restoring only the permitted standard terminal/interface replacements,
/// source equality covers every other CQ queue token, discharged fact, lower
/// packet, and admission-owner list. No registry premises are introduced.
pub fn check_sharded(source: &Program, target: &ShardedProgram) -> syn::Result<()> {
    round::contract::check(source)?;
    round::contract::check(&target.round)?;
    let erased = round::erase(&target.round)?;
    round::check_rewrite(source, &erased)?;
    let mut restored = erased;
    for rule in source
        .context
        .strata
        .iter()
        .flat_map(|s| s.seeds.iter().chain(&s.steps))
    {
        let before = &source.holes[rule.hole];
        if let Some(start) = super::push_tail(source, before) {
            let after = &mut restored.holes[rule.hole];
            ensure(
                after.interface == round::HoleInterface::Push
                    && after.pipeline.passes.len() == start + 1
                    && after.pipeline.passes[start].path == super::push_path()
                    && after.pipeline.passes[start].arguments.is_empty(),
                &target.round.holes[rule.hole],
                "sharded task changed its registered push realization",
            )?;
            after.pipeline.passes.truncate(start);
            after
                .pipeline
                .passes
                .extend_from_slice(&before.pipeline.passes[start..]);
            after.interface = before.interface;
        }
    }
    ensure(
        source.execution.is_empty() && restored == *source,
        target,
        "sharded partition changed the erased Round program beyond its task terminals",
    )?;
    check_providers(source)?;
    let mut annotations = target.round.execution.iter();
    for rule in source
        .context
        .strata
        .iter()
        .flat_map(|s| s.seeds.iter().chain(&s.steps))
    {
        if rule.scan.is_none() {
            continue;
        }
        let annotation = annotations.next().ok_or_else(|| {
            syn::Error::new_spanned(target, "sharded partition omitted an eligible scan")
        })?;
        ensure(
            annotation.hole == rule.hole
                && annotation.occurrence == 0
                && annotation.strategy == "rayon_sharded",
            target,
            "sharded partition changed scan coverage, occurrence or strategy",
        )?;
        ensure(
            annotation.self_dedup == target.certificate.self_dedup,
            target,
            "task self-deduplication policy disagrees with its execution witness",
        )?;
    }
    ensure(
        annotations.next().is_none(),
        target,
        "sharded partition added an ineligible scan",
    )?;
    ensure(
        target
            .certificate
            .self_dedup
            .as_ref()
            .is_none_or(|policy| policy == "per_task"),
        target,
        "task self-deduplication must be per_task or absent",
    )?;
    ensure(
        target.certificate.grain > 0,
        target,
        "grain must be positive",
    )?;
    let certificate = &target.certificate;
    ensure(
        certificate.shard_count == "power_of_two_at_least_threads",
        target,
        "invalid shard count policy",
    )?;
    let mut admitted = vec![false; source.context.relations.len()];
    for admission in &certificate.admission {
        ensure(
            source
                .context
                .relations
                .get(admission.relation)
                .is_some_and(|relation| relation.declaration.value.is_none()),
            target,
            "admission relation must be an IDB",
        )?;
        ensure(
            !admitted[admission.relation],
            target,
            "duplicate admission relation",
        )?;
        admitted[admission.relation] = true;
        ensure(
            admission.hash == "full_tuple",
            target,
            "admission must hash the full tuple",
        )?;
        ensure(
            admission.ownership.start == 0
                && admission.ownership.end == "shard_count"
                && admission.ownership.owner == "shard_id",
            target,
            "admission ownership must cover all shards with owner shard_id",
        )?;
    }
    ensure(
        source
            .context
            .relations
            .iter()
            .enumerate()
            .all(|(index, relation)| relation.declaration.value.is_some() || admitted[index]),
        target,
        "missing admission relation",
    )?;
    for (owners, count) in [
        (&certificate.layouts, source.context.layouts.len()),
        (
            &certificate.delta_layouts,
            source.context.delta_layouts.len(),
        ),
    ] {
        let mut covered = vec![false; count];
        let mut occupied = vec![false; count];
        for ownership in owners {
            ensure(ownership.index < count, target, "layout index out of range")?;
            ensure(
                !covered[ownership.index],
                target,
                "duplicate layout coverage",
            )?;
            covered[ownership.index] = true;
            ensure(ownership.owner < count, target, "layout owner out of range")?;
            ensure(!occupied[ownership.owner], target, "layout owner collision")?;
            occupied[ownership.owner] = true;
        }
        ensure(
            covered.iter().all(|covered| *covered),
            target,
            "missing layout coverage",
        )?;
    }
    let mut published = vec![false; source.context.strata.len()];
    for publication in &certificate.publication {
        let stratum = source
            .context
            .strata
            .get(publication.stratum)
            .ok_or_else(|| syn::Error::new_spanned(target, "publication stratum out of range"))?;
        ensure(
            !published[publication.stratum],
            target,
            "duplicate publication barrier",
        )?;
        published[publication.stratum] = true;
        ensure(
            publication.barrier == "before_evaluate",
            target,
            "publication barrier must precede evaluation",
        )?;
        let mut relations = vec![false; source.context.relations.len()];
        for &relation in &publication.relations {
            ensure(
                stratum.members.contains(&relation),
                target,
                "publication relation is not a stratum member",
            )?;
            ensure(
                !relations[relation],
                target,
                "duplicate publication relation",
            )?;
            relations[relation] = true;
        }
        ensure(
            stratum.members.iter().all(|&relation| relations[relation]),
            target,
            "missing publication relation",
        )?;
        for (indexes, layouts) in [
            (&publication.layouts, &source.context.layouts),
            (&publication.delta_layouts, &source.context.delta_layouts),
        ] {
            let mut covered = vec![false; layouts.len()];
            for &index in indexes {
                ensure(
                    layouts
                        .get(index)
                        .is_some_and(|layout| stratum.members.contains(&layout.relation)),
                    target,
                    "publication layout is not a stratum member",
                )?;
                ensure(!covered[index], target, "duplicate publication layout")?;
                covered[index] = true;
            }
            ensure(
                layouts.iter().enumerate().all(|(index, layout)| {
                    !stratum.members.contains(&layout.relation) || covered[index]
                }),
                target,
                "missing publication layout",
            )?;
        }
    }
    ensure(
        published.iter().all(|published| *published),
        target,
        "missing publication barrier",
    )
}

fn ownership() -> ShardOwnership {
    ShardOwnership {
        start: 0,
        end: syn::parse_quote!(shard_count),
        owner: syn::parse_quote!(shard_id),
    }
}

fn certificate(program: &Program, arguments: Arguments) -> ShardCertificate {
    let indexes = |layouts: &[round::Layout]| {
        (0..layouts.len())
            .map(|index| LayoutOwnership {
                index,
                owner: index,
            })
            .collect()
    };
    ShardCertificate {
        threshold: arguments.threshold,
        grain: arguments.grain,
        self_dedup: Some(syn::parse_quote!(per_task)),
        shard_count: syn::parse_quote!(power_of_two_at_least_threads),
        admission: program
            .context
            .relations
            .iter()
            .enumerate()
            .filter(|(_, relation)| relation.declaration.value.is_none())
            .map(|(relation, _)| Admission {
                relation,
                hash: syn::parse_quote!(full_tuple),
                ownership: ownership(),
            })
            .collect(),
        layouts: indexes(&program.context.layouts),
        delta_layouts: indexes(&program.context.delta_layouts),
        publication: program
            .context
            .strata
            .iter()
            .enumerate()
            .map(|(stratum, component)| {
                let member_layouts = |layouts: &[round::Layout]| {
                    layouts
                        .iter()
                        .enumerate()
                        .filter(|(_, layout)| component.members.contains(&layout.relation))
                        .map(|(index, _)| index)
                        .collect()
                };
                PublicationBarrier {
                    stratum,
                    relations: component.members.clone(),
                    layouts: member_layouts(&program.context.layouts),
                    delta_layouts: member_layouts(&program.context.delta_layouts),
                    barrier: syn::parse_quote!(before_evaluate),
                }
            })
            .collect(),
    }
}

fn check_providers(program: &Program) -> syn::Result<()> {
    let tokens = &program.storage.tokens;
    for relation in &program.context.relations {
        if relation.declaration.value.is_none() {
            // Layout providers are checked below; admission needs a bulk canonical.
            ensure(
                program
                    .context
                    .layouts
                    .get(relation.providers.canonical)
                    .is_some_and(|layout| layout.protocol == storage::LayoutProtocol::Bulk),
                program,
                "sharded Round requires a sharded or sorted-run canonical layout for each IDB",
            )?;
            ensure(
                relation.providers.delta.token == storage::provider::ROWS_VEC
                    && relation.providers.next.token == storage::provider::ROWS_VEC,
                program,
                "sharded Round requires vector delta and next providers",
            )?;
        }
    }
    // Storage's contract checks each layout's protocol and spelling against its
    // token. Non-bulk layouts need only their declared set or grouped-map
    // operations; bulk tokens are this extension's own, on keyed IDB layouts.
    for layout in program
        .context
        .layouts
        .iter()
        .chain(&program.context.delta_layouts)
    {
        let declaration = &program.context.relations[layout.relation].declaration;
        let token = &layout.provider.token;
        let eligible = if layout.protocol == storage::LayoutProtocol::Bulk {
            (token == SHARDED_LAYOUT || token == RUN_LAYOUT)
                && declaration.value.is_none()
                && layout.key > 0
        } else {
            storage::provider::lookup(tokens, token)?.admission
                == storage::provider::Admission::None
        };
        ensure(
            eligible,
            program,
            "sharded Round requires set or grouped-map layouts or an eligible sharded_layout or run_layout",
        )?;
    }
    Ok(())
}

/// Parallel runtime tokens, declared by partition (DL-ShardedAdmission).
pub const SHARDED_LAYOUT: &str = "sharded_layout";
pub const RUN_LAYOUT: &str = "run_layout";

/// Hash-sharded bulk layout: admission routes candidates by hash and task-local
/// deduplication reuses the layout's own hasher state. `ShardedLayout` answers
/// `IndexRead`/`PointLookup`, `ExactMembership` and `Candidates` on its key;
/// its slice cursor is not `Copy`.
pub fn sharded_layout() -> storage::provider::Token {
    storage::provider::declared(
        SHARDED_LAYOUT,
        storage::provider::Spelling::Sharded,
        Some(storage::LayoutProtocol::Bulk),
        storage::provider::Admission::ShardedHash,
        true,
        storage::provider::TaskHasher::Layout,
        GROUPED_READS,
        &[],
    )
}

/// Sorted-run bulk layout: admission subtracts sorted candidate runs.
/// `RunLayout` answers the same reads; its `RunValues` cursor is not `Copy`.
pub fn run_layout() -> storage::provider::Token {
    storage::provider::declared(
        RUN_LAYOUT,
        storage::provider::Spelling::Runs,
        Some(storage::LayoutProtocol::Bulk),
        storage::provider::Admission::SortedSubtract,
        false,
        storage::provider::TaskHasher::Fx,
        GROUPED_READS,
        &[],
    )
}

const GROUPED_READS: &[storage::provider::HostRead] = &[
    storage::provider::HostRead::Exact,
    storage::provider::HostRead::Membership,
    storage::provider::HostRead::Candidates,
];

impl cq_stage::Language for ShardedProgram {
    fn check(&self) -> syn::Result<()> {
        check_sharded(&round::erase(&self.round)?, self)
    }

    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        Self::check(source)?;
        Self::check(target)?;
        ensure(
            round::erase(&source.round)? == round::erase(&target.round)?,
            target,
            "sharded rewrite changed the erased Round program",
        )
    }
}
