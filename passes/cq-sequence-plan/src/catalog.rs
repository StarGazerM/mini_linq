use cq_lang_sequence::structures::{CanonSource, Capability, Provider};
use cq_lang_sequence::{Analysis, Catalog, Obligation, Program};
use cq_syntax::symbol_name;

pub(crate) fn check(program: &Program, analysis: &Analysis, catalog: &Catalog) -> syn::Result<()> {
    // Formal rule PL-Catalog: a program declaration of a token the target
    // declares is that declaration; the catalog is keyed by token id.
    for declaration in &analysis.structures.providers.declarations {
        if catalog
            .declaration(&declaration.token)
            .is_some_and(|declared| declared != declaration)
        {
            return Err(syn::Error::new_spanned(
                &declaration.token,
                format!(
                    "PL-Catalog: declaration of provider token `{}` differs from the target catalog",
                    declaration.token
                ),
            ));
        }
    }
    for obligation in &analysis.obligations {
        let available = match obligation {
            Obligation::Provider {
                provider,
                capability,
                instance,
                ..
            } => provider_available(analysis, catalog, provider, capability, instance),
            Obligation::Canon { result, canonical } => {
                let levels = result_levels(program, analysis, result, true);
                canon_available(analysis, catalog, canonical.as_ref(), |registered| {
                    levels.iter().all(|level| registered.contains(level))
                })
            }
            Obligation::Codec { ordered, .. } => catalog.codecs.iter().any(|token| {
                (!ordered || token.ordered)
                    && analysis
                        .levels
                        .values()
                        .all(|level| token.levels.contains(level))
            }),
            Obligation::Failure { .. } => true,
        };
        if !available {
            let reads = if let Obligation::Provider { provider, .. } = obligation {
                analysis
                    .provider_reads
                    .iter()
                    .filter(|(_, providers)| providers.contains(provider))
                    .map(|(name, _)| format!("{name}@{}", analysis.levels[name].join(".")))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                String::new()
            };
            return Err(syn::Error::new(
                program.structures.physical.program.returned.span(),
                format!("P-Decompose: unmet catalog obligation {obligation:?}; reads [{reads}]"),
            ));
        }
    }
    Ok(())
}

/// Formal rule PL-Distinct: whether the target discharges a canon boundary
/// reading `canonical` (`None`: the returned answer boundary). `admits` tells
/// whether a registered level set covers the boundary's execution levels.
///
/// A canonical rows record, or the answer's identity index, needs a catalog
/// entry of its recorded token declaring Distinct; an unrecorded answer index
/// any Distinct entry. A Built reference is discharged by its own Provider
/// obligations; a host carrier is read only at host.
pub(crate) fn canon_available(
    analysis: &Analysis,
    catalog: &Catalog,
    canonical: Option<&CanonSource>,
    admits: impl Fn(&[Vec<String>]) -> bool,
) -> bool {
    let provider = match canonical {
        Some(CanonSource::Rows(provider)) => Some(provider),
        Some(CanonSource::Index(_)) => return true,
        Some(CanonSource::Host) => return admits(&[vec!["host".to_owned()]]),
        // Structures records at most the returned result's identity index.
        None => analysis.structures.head_dedup.values().next(),
    };
    catalog.providers.iter().any(|entry| {
        provider.is_none_or(|provider| entry.covers(provider))
            && catalog
                .facts(&entry.provider)
                .is_some_and(|facts| facts.reads(&Capability::Distinct))
            && admits(&entry.levels)
    })
}

/// Check the complete target promise, including scheduled read levels.
pub(crate) fn provider_available(
    analysis: &Analysis,
    catalog: &Catalog,
    provider: &str,
    capability: &Capability,
    instance: &Provider,
) -> bool {
    let levels = provider_levels(analysis, provider);
    provider_admitted(
        analysis,
        catalog,
        provider,
        capability,
        instance,
        levels.iter().any(|level| level.len() > 1),
        |registered| levels.iter().all(|level| registered.contains(level)),
    )
}

/// The single provider admission predicate shared by the final catalog check
/// and placement's prediction of a read level. `below_host` tells whether a
/// read executes below host; `admits` whether a registered level set covers
/// the read levels. The catalog entry of the recorded instance's tokens
/// supplies the levels; its declared facts the capabilities.
pub(crate) fn provider_admitted(
    analysis: &Analysis,
    catalog: &Catalog,
    provider: &str,
    capability: &Capability,
    instance: &Provider,
    below_host: bool,
    admits: impl Fn(&[Vec<String>]) -> bool,
) -> bool {
    // Formal rule PL-Resident: Only an explicit resident token admits an
    // unretained host handle below host; all read/level facts still apply.
    let needs_resident = analysis.structures.host_read(provider) && below_host;
    catalog.providers.iter().any(|entry| {
        entry.covers(instance)
            && catalog.facts(&entry.provider).is_some_and(|facts| {
                (!needs_resident || facts.reads(&Capability::Resident)) && facts.reads(capability)
            })
            && admits(&entry.levels)
    })
}

fn result_levels(
    program: &Program,
    analysis: &Analysis,
    result: &str,
    canonical: bool,
) -> Vec<Vec<String>> {
    let mut levels = Vec::new();
    for definition in &program.sequence.definitions {
        let name = symbol_name(&definition.name);
        // Canon obligations name the owning Sequence definition, not its Physical result.
        if name == result
            && let Some(level) = analysis.levels.get(&name)
        {
            let level = if canonical {
                analysis
                    .dispatches
                    .iter()
                    .find(|dispatch| dispatch.members.contains(&name))
                    .map_or(level, |dispatch| &dispatch.level)
            } else {
                level
            };
            if !levels.contains(level) {
                levels.push(level.clone());
            }
        }
    }
    if levels.is_empty() {
        levels.push(vec!["host".to_owned()]);
    }
    levels
}

fn provider_levels(analysis: &Analysis, provider: &str) -> Vec<Vec<String>> {
    let mut levels = Vec::new();
    for (name, providers) in &analysis.provider_reads {
        if providers.contains(provider) {
            if let Some(level) = analysis.levels.get(name) {
                let level = analysis
                    .dispatches
                    .iter()
                    .find(|dispatch| dispatch.inner_stores.contains(name))
                    .map_or(level, |dispatch| &dispatch.producer_level);
                if !levels.contains(level) {
                    levels.push(level.clone());
                }
            }
        }
    }
    levels
}
#[cfg(test)]
mod tests {
    use super::*;
    use cq_lang_sequence::structures::Declaration;

    #[test]
    fn resident_host_reads_require_explicit_complete_catalog_facts() {
        let structures = syn::parse_quote! {
            physical {
                relation L(a:i32); relation R(a:i32,b:i32);
                l = pipe rename L@0 {a -> a};
                r = pipe rename R@1 {a -> a,b -> b};
                answer = let canon probe first [a] l,r;
                return answer as answer(a,b);
            }
            structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; opaque_native = OpaqueNative<i32> { reads [exact, range, membership], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
                l host scan = @reader as Reader [];
                r host scan = @opaque_native as OpaqueNative<i32> [];
                r host result(answer) = @opaque_native as OpaqueNative<i32> [a];
                answer rows = @sort as ::cq_storage::dedup::Sort;
                answer canon = rows;
            }
        };
        let program = cq_sequence_lowering::compile(&structures).unwrap();
        let mut analysis = cq_lang_sequence::contract::check(&program).unwrap();
        let level = vec!["host".into(), "device".into(), "block".into()];
        for (reader, providers) in &analysis.provider_reads {
            if providers.contains("r") {
                analysis.levels.insert(reader.clone(), level.clone());
            }
        }
        let instance = syn::parse_quote!(@opaque_native as OpaqueNative<i32>);
        let declaration: Declaration = syn::parse_quote!(
            opaque_native = OpaqueNative<_> { reads [exact, range, membership], build typed };
        );
        let mut catalog = Catalog::default();
        catalog.register(declaration, vec![level]);
        for capability in [Capability::Exact, Capability::Range, Capability::Membership] {
            assert!(!provider_available(
                &analysis,
                &catalog,
                "r",
                &capability,
                &instance
            ));
        }
        catalog.declarations[0]
            .facts
            .reads
            .push(Capability::Resident);
        for capability in [Capability::Exact, Capability::Range, Capability::Membership] {
            assert!(provider_available(
                &analysis,
                &catalog,
                "r",
                &capability,
                &instance
            ));
        }
        let mut missing = catalog.clone();
        missing.declarations[0]
            .facts
            .reads
            .retain(|c| *c != Capability::Membership);
        assert!(!provider_available(
            &analysis,
            &missing,
            "r",
            &Capability::Membership,
            &instance
        ));
        let mut wrong_level = catalog.clone();
        wrong_level.providers[0].levels = vec![vec!["host".into()]];
        assert!(!provider_available(
            &analysis,
            &wrong_level,
            "r",
            &Capability::Range,
            &instance
        ));
        assert!(!provider_available(
            &analysis,
            &catalog,
            "r",
            &Capability::Range,
            &syn::parse_quote!(@unrelated as OpaqueNative<i32>)
        ));
    }
}
