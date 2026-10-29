use cq_lang_sequence::{Analysis, Catalog, Obligation, Program};
use cq_syntax::symbol_name;
use quote::ToTokens;
use syn::{GenericArgument, PathArguments, Type};

pub(crate) fn check(program: &Program, analysis: &Analysis, catalog: &Catalog) -> syn::Result<()> {
    for obligation in &analysis.obligations {
        let available = match obligation {
            Obligation::Provider {
                provider,
                capability,
                ty,
                ..
            } => provider_available(analysis, catalog, provider, capability, ty),
            Obligation::Canon { result, ty } => {
                let levels = result_levels(program, analysis, result, true);
                catalog.canons.iter().any(|token| {
                    ty.as_ref().is_none_or(|ty| matches_type(&token.ty, ty))
                        && levels.iter().all(|level| token.levels.contains(level))
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

/// Check the complete target promise, including scheduled read levels and row
/// element types. A matching family name alone does not authorize retyping.
pub(crate) fn provider_available(
    analysis: &Analysis,
    catalog: &Catalog,
    provider: &str,
    capability: &cq_lang_sequence::Capability,
    ty: &Type,
) -> bool {
    let levels = provider_levels(analysis, provider);
    // Formal rule PL-Resident: Only an explicit resident token admits an
    // unretained host handle below host; all read/type/level facts still apply.
    let needs_resident =
        analysis.structures.host_read(provider) && levels.iter().any(|level| level.len() > 1);
    catalog.providers.iter().any(|token| {
        matches_type(&token.ty, ty)
            && (!needs_resident
                || token
                    .capabilities
                    .contains(&cq_lang_sequence::Capability::Resident))
            && token.elements.as_ref().is_none_or(|accepted| {
                analysis.structures.layouts[provider].iter().all(|column| {
                    accepted.iter().any(|ty| {
                        matches_type(
                            ty,
                            &analysis.structures.physical.rows[provider].types[column],
                        )
                    })
                })
            })
            && token.capabilities.contains(capability)
            && levels.iter().all(|level| token.levels.contains(level))
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

/// Match a concrete runtime type against a catalog family with generic holes.
pub fn matches_type(pattern: &Type, actual: &Type) -> bool {
    match (pattern, actual) {
        (Type::Infer(_), _) => true,
        (Type::Path(pattern), Type::Path(actual)) if pattern.qself == actual.qself => {
            // Provider paths are tokens, not resolved Rust names. Leading `::`
            // is immaterial; every named path segment remains significant.
            pattern.path.segments.len() == actual.path.segments.len()
                && pattern
                    .path
                    .segments
                    .iter()
                    .zip(&actual.path.segments)
                    .all(|(p, a)| {
                        p.ident == a.ident && matches_arguments(&p.arguments, &a.arguments)
                    })
        }
        (Type::Tuple(p), Type::Tuple(a)) => {
            p.elems.len() == a.elems.len()
                && p.elems
                    .iter()
                    .zip(&a.elems)
                    .all(|(p, a)| matches_type(p, a))
        }
        (Type::Reference(p), Type::Reference(a)) => {
            p.mutability == a.mutability
                && p.lifetime
                    .as_ref()
                    .is_none_or(|l| l.ident == "_" || Some(l) == a.lifetime.as_ref())
                && matches_type(&p.elem, &a.elem)
        }
        _ => pattern.to_token_stream().to_string() == actual.to_token_stream().to_string(),
    }
}

fn matches_arguments(pattern: &PathArguments, actual: &PathArguments) -> bool {
    match (pattern, actual) {
        (PathArguments::AngleBracketed(p), PathArguments::AngleBracketed(a)) => {
            p.args.len() == a.args.len()
                && p.args.iter().zip(&a.args).all(|(p, a)| match (p, a) {
                    (GenericArgument::Type(Type::Infer(_)), _) => true,
                    (GenericArgument::Type(p), GenericArgument::Type(a)) => matches_type(p, a),
                    (GenericArgument::Lifetime(p), GenericArgument::Lifetime(a)) => {
                        p.ident == "_" || p == a
                    }
                    _ => p == a,
                })
        }
        _ => pattern == actual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_lang_sequence::{Capability, ProviderToken};

    #[test]
    fn resident_host_reads_require_explicit_complete_catalog_facts() {
        let structures = syn::parse_quote! {
            physical {
                relation L(a:i32); relation R(a:i32,b:i32);
                l = pipe rename L@0 {a -> a};
                r = pipe rename R@1 {a -> a,b -> b};
                answer = pipe canon probe first [a] l,r;
                return answer as answer(a,b);
            }
            structures {
                l host scan = Reader [];
                r host scan = OpaqueNative<i32> [];
                r host result(answer) = OpaqueNative<i32> [a];
                answer canon = Canon;
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
        let ty = syn::parse_quote!(OpaqueNative<i32>);
        let mut catalog = Catalog {
            providers: vec![ProviderToken {
                ty: syn::parse_quote!(OpaqueNative<_>),
                capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
                supports: Vec::new(),
                levels: vec![level],
                elements: Some(vec![syn::parse_quote!(i32)]),
            }],
            ..Catalog::default()
        };
        for capability in [Capability::Exact, Capability::Range, Capability::Membership] {
            assert!(!provider_available(
                &analysis,
                &catalog,
                "r",
                &capability,
                &ty
            ));
        }
        catalog.providers[0].capabilities.push(Capability::Resident);
        for capability in [Capability::Exact, Capability::Range, Capability::Membership] {
            assert!(provider_available(
                &analysis,
                &catalog,
                "r",
                &capability,
                &ty
            ));
        }
        let mut missing = catalog.clone();
        missing.providers[0]
            .capabilities
            .retain(|c| *c != Capability::Membership);
        assert!(!provider_available(
            &analysis,
            &missing,
            "r",
            &Capability::Membership,
            &ty
        ));
        let mut wrong_level = catalog.clone();
        wrong_level.providers[0].levels = vec![vec!["host".into()]];
        assert!(!provider_available(
            &analysis,
            &wrong_level,
            "r",
            &Capability::Range,
            &ty
        ));
        let mut wrong_elements = catalog.clone();
        wrong_elements.providers[0].elements = Some(vec![syn::parse_quote!(String)]);
        assert!(!provider_available(
            &analysis,
            &wrong_elements,
            "r",
            &Capability::Range,
            &ty
        ));
        assert!(!provider_available(
            &analysis,
            &catalog,
            "r",
            &Capability::Range,
            &syn::parse_quote!(Unrelated<i32>)
        ));
    }
}
