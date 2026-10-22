//! Sharding is an extension premise, not a base-language execution policy.
//! Admission ownership quantifies over runtime shard IDs: `[start, end)` is the
//! complete domain and `owner = shard_id` is injective. Seen providers choose a
//! power-of-two shard count at least the Rayon worker count and hash full tuples.
//! Each native full/delta layout instead has one exclusive owner.
//! Publication joins every layout owner before the next evaluation, including
//! owners publishing an empty delta.
use cq_datalog_support::ensure;
use cq_lang_datalog_round::{self as round, Program, storage};
use proc_macro2::TokenStream;
use storage::record;
use syn::{Ident, Type};

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

/// Rewrite only IDB membership. All full/delta layout types and other Storage
/// records, including the EDB canonicalizer and opaque initializers, are
/// preserved exactly.
pub fn compile_storage(source: &storage::Program) -> syn::Result<storage::Program> {
    storage::contract::check(source)?;
    let mut target = source.clone();
    for (provider, declaration) in target
        .providers
        .iter_mut()
        .zip(&source.incremental.strata.source.declarations)
    {
        if declaration.value.is_none() {
            provider.seen = sharded_seen(&provider.seen)?;
        }
    }
    check_storage(source, &target)?;
    Ok(target)
}

pub fn compile_storage_with(
    source: &storage::Program,
    arguments: TokenStream,
) -> syn::Result<storage::Program> {
    ensure(
        arguments.is_empty(),
        &arguments,
        "sharded storage takes no arguments",
    )?;
    compile_storage(source)
}

/// Formal rule CK-DL-ShardStorage: preserve every record except IDB seen substitutions.
/// Independently reverse only those membership substitutions, then demand exact
/// equality, including every native full/delta layout and opaque CQ queue.
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
    let mut erased = target.clone();
    for (index, declaration) in source
        .incremental
        .strata
        .source
        .declarations
        .iter()
        .enumerate()
    {
        if declaration.value.is_none() {
            let original = &source.providers[index].seen;
            let expected = sharded_seen(original)?;
            ensure(
                target.providers[index].seen == expected,
                target,
                "incorrect sharded seen provider",
            )?;
            erased.providers[index].seen = original.clone();
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

/// Partition a Round program with sharded seen admission and native layouts.
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
    for relation in &program.context.relations {
        if relation.declaration.value.is_none() {
            let types = relation.declaration.columns.iter().map(|column| &column.ty);
            let tuple: Type = syn::parse_quote!((#(#types,)*));
            let seen_arguments = type_arguments(
                &relation.providers.seen,
                &["cq_datalog_parallel_runtime", "ShardedSeen"],
            );
            let rows: Type = syn::parse_quote!(::std::vec::Vec<#tuple>);
            ensure(
                seen_arguments.is_some_and(|arguments| {
                    (1..=2).contains(&arguments.len())
                        && argument_type(&arguments[0]) == Some(&tuple)
                        && arguments
                            .iter()
                            .all(|argument| argument_type(argument).is_some())
                }),
                program,
                "sharded Round requires ShardedSeen<Tuple, Hasher> for each IDB",
            )?;
            ensure(
                relation.providers.delta == rows && relation.providers.next == rows,
                program,
                "sharded Round requires vector delta and next providers",
            )?;
        }
    }
    for layout in program
        .context
        .layouts
        .iter()
        .chain(&program.context.delta_layouts)
    {
        let declaration = &program.context.relations[layout.relation].declaration;
        let types: Vec<_> = layout
            .columns
            .iter()
            .map(|&i| &declaration.columns[i].ty)
            .collect();
        let (keys, values) = types.split_at(layout.key);
        let arguments = type_arguments(&layout.provider, &["std", "collections", "HashMap"]);
        let key: Type = syn::parse_quote!((#(#keys,)*));
        let value: Type = syn::parse_quote!(::std::vec::Vec<(#(#values,)*)>);
        ensure(
            arguments.is_some_and(|arguments| {
                arguments.len() == 3
                    && argument_type(&arguments[0]) == Some(&key)
                    && argument_type(&arguments[1]) == Some(&value)
                    && argument_type(&arguments[2]).is_some()
            }),
            program,
            "sharded Round requires native HashMap<K, Vec<V>, Hasher> for every layout",
        )?;
    }
    Ok(())
}

fn type_arguments<'a>(
    ty: &'a Type,
    names: &[&str],
) -> Option<&'a syn::punctuated::Punctuated<syn::GenericArgument, syn::Token![,]>> {
    let Type::Path(path) = ty else { return None };
    if path.qself.is_some()
        || path.path.leading_colon.is_none()
        || path.path.segments.len() != names.len()
    {
        return None;
    }
    for (index, (segment, name)) in path.path.segments.iter().zip(names).enumerate() {
        if segment.ident != *name {
            return None;
        }
        if index + 1 == names.len() {
            if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                return Some(&arguments.args);
            }
        } else if !matches!(segment.arguments, syn::PathArguments::None) {
            return None;
        }
    }
    None
}

fn argument_type(argument: &syn::GenericArgument) -> Option<&Type> {
    match argument {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    }
}

fn sharded_seen(ty: &Type) -> syn::Result<Type> {
    let invalid = || {
        syn::Error::new_spanned(
            ty,
            "sharded admission requires a HashSet<Tuple, Hasher> provider",
        )
    };
    let alias = type_arguments(ty, &["cq_storage", "HashSet"]);
    let arguments = alias
        .or_else(|| type_arguments(ty, &["std", "collections", "HashSet"]))
        .ok_or_else(invalid)?;
    let tuple = arguments
        .first()
        .and_then(argument_type)
        .ok_or_else(invalid)?;
    if alias.is_some() && arguments.len() == 1 {
        return Ok(syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<#tuple>));
    }
    let hasher: Type = match arguments.len() {
        1 => syn::parse_quote!(::std::collections::hash_map::RandomState),
        2 if alias.is_none() => argument_type(&arguments[1]).ok_or_else(invalid)?.clone(),
        _ => return Err(invalid()),
    };
    Ok(syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<#tuple, #hasher>))
}

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
