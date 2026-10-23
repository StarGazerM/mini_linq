use cq_lang_sequence::{Analysis, Catalog, Obligation, Program, Term};
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
            } => {
                let levels = provider_levels(program, analysis, provider);
                catalog.providers.iter().any(|token| {
                    matches_type(&token.ty, ty)
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
            return Err(syn::Error::new(
                program.structures.physical.program.returned.span(),
                format!("P-Decompose: unmet catalog obligation {obligation:?}"),
            ));
        }
    }
    Ok(())
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
        if analysis.results.get(&name).is_some_and(|r| r == result)
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

fn provider_levels(program: &Program, analysis: &Analysis, provider: &str) -> Vec<Vec<String>> {
    let mut levels = Vec::new();
    for definition in &program.sequence.definitions {
        if reads_provider(&definition.term, provider) {
            let name = symbol_name(&definition.name);
            if let Some(level) = analysis.levels.get(&name) {
                let level = analysis
                    .dispatches
                    .iter()
                    .find(|dispatch| dispatch.inner_stores.contains(&name))
                    .map_or(level, |dispatch| &dispatch.producer_level);
                if !levels.contains(level) {
                    levels.push(level.clone());
                }
            }
        }
    }
    if levels.is_empty() {
        result_levels(program, analysis, provider, false)
    } else {
        levels
    }
}

fn reads_provider(term: &Term, provider: &str) -> bool {
    match term {
        Term::Lookup {
            provider: name,
            inner,
            ..
        } => symbol_name(name) == provider || reads_provider(inner, provider),
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. }
        | Term::Tile { inner, .. } => reads_provider(inner, provider),
        Term::Union(arms) => arms.iter().any(|term| reads_provider(term, provider)),
        Term::Choose(arms) => arms.iter().any(|(_, term)| reads_provider(term, provider)),
        _ => false,
    }
}

fn matches_type(pattern: &Type, actual: &Type) -> bool {
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
