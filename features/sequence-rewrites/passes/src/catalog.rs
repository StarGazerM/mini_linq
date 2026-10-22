use cq_lang_sequence::{Capability, Catalog, structures};
use cq_syntax::symbol_name;
use quote::quote;
use structures::{Structure, physical::Operator};

/// Replace built provider types with the first catalog token satisfying all of
/// their read obligations. Entry order and catalog order are significant.
///
/// Structures has no machine grading: its publication level is `host`. Later
/// Sequence planning checks the selected types at every actual execution level.
/// Host adapters and non-provider decisions are preserved. Unlike `plan`, this
/// explicit selection operation replaces recorded types, including authored ones.
/// Each replacement uses the same checked retype operation as rewrite macros.
// Formal rule PL-Select: Select the first host-capable token satisfying every entry obligation through checked retype.
pub fn select_from_catalog(
    source: &structures::Program,
    catalog: &Catalog,
) -> syn::Result<structures::Program> {
    let analysis = structures::contract::check(source)?;
    let candidates = structures::physical::candidate_prefixes(&source.physical, &analysis.physical);
    let mut target = source.clone();
    for (index, entry) in source.structures.entries.iter().enumerate() {
        let name = symbol_name(&entry.name);
        let mut required = Vec::new();
        let kind = match &entry.structure {
            Structure::Built { keyed, .. } => {
                let levels = keyed.level_names();
                for (reader, columns, _) in analysis
                    .physical
                    .keyed_reads
                    .get(&name)
                    .into_iter()
                    .flatten()
                {
                    if structures::prefix_depth(&levels, columns).is_none() {
                        continue;
                    }
                    // A separate positional key set discharges membership reads.
                    let operator = source
                        .physical
                        .program
                        .definitions
                        .iter()
                        .find(|definition| symbol_name(&definition.name) == *reader)
                        .map(|definition| &definition.operator);
                    let capability = match operator {
                        Some(Operator::Semi(..) | Operator::Anti(..)) => {
                            if analysis
                                .keysets
                                .contains_key(&(name.clone(), columns.clone()))
                            {
                                continue;
                            }
                            Capability::Membership
                        }
                        Some(Operator::Merge(..)) => Capability::Ordered,
                        _ => Capability::Exact,
                    };
                    if !required.contains(&capability) {
                        required.push(capability);
                    }
                }
                for (_, columns) in analysis
                    .physical
                    .merge_reads
                    .get(&name)
                    .into_iter()
                    .flatten()
                {
                    if structures::prefix_depth(&levels, columns).is_some()
                        && !required.contains(&Capability::Ordered)
                    {
                        required.push(Capability::Ordered);
                    }
                }
                for (result, prefix) in candidates.iter().flat_map(|candidate| &candidate.sides) {
                    if *result == name && structures::prefix_depth(&levels, prefix).is_some() {
                        for capability in [Capability::Range, Capability::Membership] {
                            if !required.contains(&capability) {
                                required.push(capability);
                            }
                        }
                    }
                }
                if keyed.membership.is_some() && !required.contains(&Capability::Membership) {
                    required.push(Capability::Membership);
                }
                "index"
            }
            Structure::Rows { .. } => {
                required.push(Capability::Range);
                "rows"
            }
            Structure::Selection { .. }
            | Structure::KeySet { .. }
            | Structure::JoinFilter { .. } => {
                required.push(Capability::Membership);
                "set"
            }
            Structure::Dedup { .. } | Structure::HeadDedup { .. } => "canon",
            Structure::Host { .. }
            | Structure::Read { .. }
            | Structure::Repr { .. }
            | Structure::Stage { .. }
            | Structure::Share { .. } => continue,
        };
        let ty = if kind == "canon" {
            catalog.canons.iter().find(|token| at_host(&token.levels)).map(|token| &token.ty)
        } else {
            catalog.providers.iter().find(|token| {
                at_host(&token.levels)
                    && required.iter().all(|capability| token.capabilities.contains(capability))
            }).map(|token| &token.ty)
        }.ok_or_else(|| syn::Error::new(
            entry.name.span(),
            format!("PL-Select: structure `{name}` entry {index} ({kind}) has unmet catalog obligation {required:?} at host"),
        ))?;
        let index = syn::LitInt::new(&index.to_string(), entry.name.span());
        target =
            cq_rewrite_passes::retype(&target, quote!(entry(#index) => #ty)).map_err(|error| {
                syn::Error::new(
                    error.span(),
                    format!("PL-Select: structure `{name}` ({kind}): {error}"),
                )
            })?;
    }
    Ok(target)
}

fn at_host(levels: &[Vec<String>]) -> bool {
    levels
        .iter()
        .any(|level| level.len() == 1 && level[0] == "host")
}
