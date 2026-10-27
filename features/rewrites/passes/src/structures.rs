use cq_lang_structures::{self as ir, Structure, physical::Operator, symbol_name};
use proc_macro2::TokenStream;
use syn::{Ident, Type};

mod retain_lookups;
pub use retain_lookups::retain_lookups;
mod expand_lookups;
pub use expand_lookups::expand_lookups;

/// Record the existing full-key existential-storage certificate. The
/// Structures checker proves that no reader observes a non-key payload and
/// that the projected build satisfies the filtered-storage contract.
pub fn membership(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let name: Ident = syn::parse2(arguments)?;
    ir::contract::check(source)?;
    let mut target = source.clone();
    let mut matched = 0;
    for entry in &mut target.structures.entries {
        if entry.name != name {
            continue;
        }
        let Structure::Built { keyed, .. } = &mut entry.structure else {
            continue;
        };
        if keyed.levels.len() != 1 {
            return Err(syn::Error::new_spanned(
                &name,
                "membership advice requires one full-key level",
            ));
        }
        if keyed.filtered.is_none() {
            let columns = &keyed.levels[0].columns;
            keyed.filtered = Some(syn::parse_quote!(payload [#columns] constants {}));
        }
        keyed.membership = Some(Default::default());
        keyed.supports = None;
        matched += 1;
    }
    if matched != 1 {
        return Err(syn::Error::new_spanned(
            name,
            "membership advice requires one built provider",
        ));
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

/// Add every admissible covered scalar filter to the named scan occurrence
/// that is not already recorded. The default type is exact; a later
/// `retype(scan join_filter => T)` chooses its representation. This is
/// explicit positional-host advice, including when selection was not given
/// positional hosts. The same Structures contract checks the result.
pub fn join_filter(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let scan: Ident = syn::parse2(arguments)?;
    let facts = ir::contract::check(source)?;
    // Analysis coalesces runtime filters; advice must inspect every recorded
    // proof witness to remain idempotent after that coalescing.
    let recorded: Vec<_> = source
        .structures
        .entries
        .iter()
        .filter_map(|entry| match &entry.structure {
            Structure::JoinFilter { filter, .. } if entry.name == scan => Some(filter),
            _ => None,
        })
        .collect();
    let candidates: Vec<_> = source
        .physical
        .program
        .definitions
        .iter()
        .flat_map(|d| {
            ir::join_filter_candidates(source, &facts, &symbol_name(&d.name))
                .into_iter()
                .map(move |c| (&d.name, c))
        })
        .filter(|(_, c)| c.scan == symbol_name(&scan))
        .collect();
    if candidates.is_empty() {
        return Err(syn::Error::new_spanned(
            scan,
            "scan has no admissible covering exact probe or semijoin",
        ));
    }
    let origin = &facts.lineage[&symbol_name(&scan)];
    let mut target = source.clone();
    for (covering, candidate) in candidates {
        if recorded.iter().any(|f| {
            symbol_name(&f.covering) == symbol_name(covering)
                && f.key
                    .columns
                    .iter()
                    .map(symbol_name)
                    .eq([candidate.key.clone()])
        }) {
            continue;
        }
        let index = syn::Index::from(
            facts.host_layouts[&origin.base]
                .iter()
                .position(|c| c == &origin.columns[&candidate.key])
                .expect("checked lineage"),
        );
        let key = &facts.physical.rows[&candidate.build].bindings[&candidate.key];
        let build = source
            .physical
            .program
            .definitions
            .iter()
            .find(|d| symbol_name(&d.name) == candidate.build)
            .map(|d| &d.name)
            .expect("checked build");
        target.structures.entries.push(syn::parse_quote! {
            #scan join_filter = ::cq_storage::AdaptiveKeySet<i64, ::cq_storage::I64Codec, 67108864>
                build #build [#key] covering #covering marker ::cq_load::ColumnAt<#index>;
        });
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

/// Formal rule EXT-SortedRuns:
/// Retype one-level HashIndex payload builds with one integral key.
/// Host fallbacks and smallest-intersection candidates retain their providers.
pub fn sorted_runs(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    select_runs(source, arguments, RunKind::Sorted)
}

/// Formal rule EXT-RadixRuns:
/// Retype eligible singleton integral payload builds to RadixSpline runs.
/// This selects an exact provider, not an ordered-input processing algorithm.
pub fn radix_runs(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    select_runs(source, arguments, RunKind::Radix)
}

#[derive(Clone, Copy)]
enum RunKind {
    Sorted,
    Radix,
}

fn select_runs(
    source: &ir::Program,
    arguments: TokenStream,
    kind: RunKind,
) -> syn::Result<ir::Program> {
    if !arguments.is_empty() {
        let message = match kind {
            RunKind::Sorted => "sorted_runs takes no arguments",
            RunKind::Radix => "radix_runs takes no arguments",
        };
        return Err(syn::Error::new_spanned(arguments, message));
    }
    let analysis = ir::contract::check(source)?;
    let candidates = ir::physical::candidate_prefixes(&source.physical, &analysis.physical);
    let mut target = source.clone();
    for entry in &mut target.structures.entries {
        let Structure::Built { keyed, .. } = &mut entry.structure else {
            continue;
        };
        let name = symbol_name(&entry.name);
        if keyed.levels.len() != 1
            || keyed.levels[0].columns.is_empty()
            || keyed.membership.is_some()
            || !matches!(&keyed.ty, Type::Path(p) if p.qself.is_none()
                && p.path.segments.iter().map(|s| s.ident.to_string()).eq(["cq_storage", "HashIndex"]))
        {
            continue;
        }
        let columns = keyed.level_names().remove(0);
        if candidates.iter().any(|c| {
            c.sides
                .iter()
                .any(|(side, key)| *side == name && *key == columns)
        }) {
            continue;
        }
        let stored: Vec<_> = columns
            .iter()
            .filter(|column| {
                !keyed.filtered.as_ref().is_some_and(|filtered| {
                    filtered
                        .constants
                        .iter()
                        .any(|binding| symbol_name(&binding.column) == **column)
                })
            })
            .collect();
        if matches!(kind, RunKind::Radix) {
            // Constants removed from the stored key do not make a composite
            // read eligible: the public advice promises one logical key.
            if columns.len() == 1 && stored.len() == 1 {
                let mut ty = ir::ungroup_type(&analysis.physical.rows[&name].types[stored[0]]);
                while let Type::Reference(reference) = ty {
                    ty = ir::ungroup_type(&reference.elem);
                }
                if matches!(ty, Type::Path(path)
                    if ["i32", "i64", "u32", "u64"].iter().any(|name| path.path.is_ident(name)))
                {
                    keyed.ty = syn::parse_quote!(::cq_storage::RadixRun<_, _>);
                    keyed.supports = Some(syn::parse_quote!(supports [retain, expand, keys]));
                }
            }
            continue;
        }
        let computed = |result: &str| {
            analysis.physical.rows[result]
                .computed
                .iter()
                .any(|c| columns.contains(c))
        };
        let owned = computed(&name)
            || analysis
                .physical
                .key_reads
                .get(&name)
                .into_iter()
                .flatten()
                .any(|(reader, _)| {
                    source
                        .physical
                        .program
                        .definitions
                        .iter()
                        .any(|definition| {
                            if symbol_name(&definition.name) != *reader {
                                return false;
                            }
                            match &definition.operator {
                                Operator::Probe(_, read)
                                | Operator::Semi(_, read)
                                | Operator::Anti(_, read) => computed(&symbol_name(&read.left)),
                                _ => false,
                            }
                        })
                });
        if stored.len() == 1
            && cq_structure_selection::arguments::sorted_key(&analysis.physical.rows[&name].types[stored[0]])
        {
            keyed.ty = if owned {
                syn::parse_quote!(::cq_storage::SortedRun<_, _, ::cq_storage::NoDense>)
            } else {
                syn::parse_quote!(::cq_storage::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>)
            };
            keyed.supports = Some(syn::parse_quote!(supports [retain, expand, keys]));
        }
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

/// Record an exact hashed-bucket run for one-level composite scalar keys.
/// Bucket collisions are resolved by comparing every recorded key column.
pub fn hashed_runs(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    if !arguments.is_empty() {
        return Err(syn::Error::new_spanned(
            arguments,
            "hashed_runs takes no arguments",
        ));
    }
    let analysis = ir::contract::check(source)?;
    let candidates = ir::physical::candidate_prefixes(&source.physical, &analysis.physical);
    let mut target = source.clone();
    for entry in &mut target.structures.entries {
        let Structure::Built { keyed, .. } = &mut entry.structure else {
            continue;
        };
        if keyed.levels.len() != 1
            || keyed.levels[0].columns.len() < 2
            || keyed.membership.is_some()
            || !matches!(&keyed.ty, Type::Path(p) if p.qself.is_none()
                && p.path.segments.iter().map(|s| s.ident.to_string()).eq(["cq_storage", "HashIndex"]))
        {
            continue;
        }
        let name = symbol_name(&entry.name);
        let columns = keyed.level_names().remove(0);
        if candidates.iter().any(|c| {
            c.sides
                .iter()
                .any(|(side, key)| *side == name && *key == columns)
        }) {
            continue;
        }
        if columns.iter().all(|column| {
            let ty = ir::ungroup_type(&analysis.physical.rows[&name].types[column]);
            matches!(ty,
                Type::Path(p) if ["i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize", "bool", "String"]
                    .iter().any(|name| p.path.is_ident(name)))
                || matches!(ty, Type::Reference(r) if matches!(&*r.elem, Type::Path(p) if p.path.is_ident("str")))
        }) && keyed.filtered.is_none() {
            keyed.ty = syn::parse_quote!(::cq_gpu_runtime::HashedRun<_, _>);
            keyed.supports = None;
        }
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}
