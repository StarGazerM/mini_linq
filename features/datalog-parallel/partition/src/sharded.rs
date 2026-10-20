//! Sharding is an extension premise, not a base-language execution policy.
//! Admission ownership quantifies over runtime shard IDs: `[start, end)` is the
//! complete domain and `owner = shard_id` is injective. Seen providers choose a
//! power-of-two shard count at least the Rayon worker count and hash full tuples.
//! Each native full/delta layout instead has one exclusive identity owner.
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
/// separate. In each domain, `owner = index` is total and injective.
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
    threshold: usize, grain: usize, shard_count: Ident,
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
            let tuple = hash_set_element(&provider.seen).ok_or_else(|| {
                syn::Error::new_spanned(
                    &provider.seen,
                    "sharded admission requires a HashSet<Tuple> provider",
                )
            })?;
            provider.seen = syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<#tuple>);
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
            let tuple = hash_set_element(original).ok_or_else(|| {
                syn::Error::new_spanned(
                    original,
                    "sharded admission requires a HashSet<Tuple> provider",
                )
            })?;
            let expected: Type =
                syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<#tuple>);
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
    let target = ShardedProgram {
        round: super::partition_scans(source),
        certificate: certificate(source, arguments),
    };
    check_sharded(source, &target)?;
    Ok(target)
}

/// Formal rule CK-DL-Partition: certify exact scan coverage and reversible single-occurrence substitution.
/// Formal rule CK-DL-ShardOwnership: certify disjoint admission shards, exclusive layout owners and a complete publication barrier.
/// Check exact CQ erasure, exhaustive scan scheduling, ownership, and the
/// complete publication barrier. Base Round independently checks each layout's
/// maintenance ordering and coverage before permitting another evaluation.
pub fn check_sharded(source: &Program, target: &ShardedProgram) -> syn::Result<()> {
    round::contract::check(source)?;
    round::contract::check(&target.round)?;
    ensure(
        source.execution.is_empty() && round::erase(&target.round)? == *source,
        target,
        "sharded partition changed the erased Round program or its CQ pipelines",
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
    }
    ensure(
        annotations.next().is_none(),
        target,
        "sharded partition added an ineligible scan",
    )?;
    ensure(
        target.certificate.grain > 0,
        target,
        "grain must be positive",
    )?;
    let expected = certificate(
        source,
        Arguments {
            threshold: target.certificate.threshold,
            grain: target.certificate.grain,
        },
    );
    ensure(
        target.certificate == expected,
        target,
        "shard certificate must cover disjoint admission and layout owners and publish every layout before evaluation",
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
            let seen: Type = syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<#tuple>);
            let rows: Type = syn::parse_quote!(::std::vec::Vec<#tuple>);
            ensure(
                relation.providers.seen == seen,
                program,
                "sharded Round requires ShardedSeen<Tuple> for each IDB",
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
        let expected: Type = syn::parse_quote!(
            ::std::collections::HashMap<
                (#(#keys,)*),
                ::std::vec::Vec<(#(#values,)*)>,
                ::cq_storage::HashState
            >
        );
        ensure(
            layout.provider == expected,
            program,
            "sharded Round requires native HashMap<K, Vec<V>, HashState> for every layout",
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

fn hash_set_element(ty: &Type) -> Option<&Type> {
    let arguments = type_arguments(ty, &["cq_storage", "HashSet"])
        .or_else(|| type_arguments(ty, &["std", "collections", "HashSet"]))?;
    if !(1..=2).contains(&arguments.len()) {
        return None;
    }
    argument_type(arguments.first()?)
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
