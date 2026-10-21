use cq_datalog_support::ensure;
use cq_lang_datalog_round as rounds;
use cq_lang_datalog_storage as storage;
use syn::{GenericArgument, PathArguments, Type, ext::IdentExt};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Choice {
    Fx,
    Keyed,
}

impl syn::parse::Parse for Choice {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name = input.call(syn::Ident::parse_any)?;
        let choice = match name.to_string().as_str() {
            "fx" => Self::Fx,
            "keyed" => Self::Keyed,
            _ => {
                return Err(syn::Error::new_spanned(
                    name,
                    "expected hasher choice `fx` or `keyed`",
                ));
            }
        };
        if !input.is_empty() {
            return Err(input.error("expected exactly one hasher choice: `fx` or `keyed`"));
        }
        Ok(choice)
    }
}

#[derive(Clone, Copy)]
enum Provider<'a> {
    Map(&'a Type, &'a Type),
    Set(&'a Type),
    Sharded(&'a Type),
    Other,
}

fn path_is(path: &syn::Path, names: &[&str]) -> bool {
    path.segments.len() == names.len()
        && path
            .segments
            .iter()
            .zip(names)
            .all(|(segment, name)| segment.ident == *name)
}

/// Inspect only the outer provider; arbitrary aliases and nested Rust remain opaque.
fn provider(ty: &Type) -> syn::Result<Provider<'_>> {
    let Type::Path(path) = ty else {
        return Ok(Provider::Other);
    };
    if path.qself.is_some() {
        return Ok(Provider::Other);
    }
    let path = &path.path;
    let map = path_is(path, &["std", "collections", "HashMap"]);
    let set = path_is(path, &["std", "collections", "HashSet"]);
    let alias = path_is(path, &["cq_storage", "HashSet"]);
    let sharded = path_is(path, &["cq_datalog_parallel_runtime", "ShardedSeen"]);
    if !map && !set && !alias && !sharded {
        return Ok(Provider::Other);
    }
    let malformed = || {
        syn::Error::new_spanned(
            ty,
            "malformed hash provider: expected its row/key/value types and optional hasher",
        )
    };
    let last = path.segments.last().expect("recognized nonempty path");
    if path
        .segments
        .iter()
        .take(path.segments.len() - 1)
        .any(|s| !s.arguments.is_empty())
    {
        return Err(malformed());
    }
    let PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return Err(malformed());
    };
    let arity = if map {
        2..=3
    } else if alias {
        1..=1
    } else {
        1..=2
    };
    if !arity.contains(&arguments.args.len())
        || !arguments
            .args
            .iter()
            .all(|arg| matches!(arg, GenericArgument::Type(_)))
    {
        return Err(malformed());
    }
    let GenericArgument::Type(first) = &arguments.args[0] else {
        unreachable!()
    };
    if map {
        let GenericArgument::Type(second) = &arguments.args[1] else {
            unreachable!()
        };
        Ok(Provider::Map(first, second))
    } else if sharded {
        Ok(Provider::Sharded(first))
    } else {
        Ok(Provider::Set(first))
    }
}

fn select(ty: &mut Type, choice: Choice) -> syn::Result<()> {
    *ty = match (provider(ty)?, choice) {
        (Provider::Map(key, value), Choice::Fx) => syn::parse_quote!(
            ::std::collections::HashMap<#key, #value,
                ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>
        ),
        (Provider::Map(key, value), Choice::Keyed) => syn::parse_quote!(
            ::std::collections::HashMap<#key, #value, ::cq_storage::HashState>
        ),
        (Provider::Set(row), Choice::Fx) => syn::parse_quote!(
            ::std::collections::HashSet<#row,
                ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>
        ),
        (Provider::Set(row), Choice::Keyed) => syn::parse_quote!(::cq_storage::HashSet<#row>),
        (Provider::Sharded(row), Choice::Fx) => syn::parse_quote!(
            ::cq_datalog_parallel_runtime::ShardedSeen<#row,
                ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>
        ),
        (Provider::Sharded(row), Choice::Keyed) => syn::parse_quote!(
            ::cq_datalog_parallel_runtime::ShardedSeen<#row>
        ),
        (Provider::Other, _) => return Ok(()),
    };
    Ok(())
}

pub(crate) fn storage(source: &storage::Program, choice: Choice) -> syn::Result<storage::Program> {
    storage::contract::check(source)?;
    let mut target = source.clone();
    for layout in target.layouts.iter_mut().chain(&mut target.delta_layouts) {
        select(&mut layout.provider, choice)?;
    }
    for (fields, declaration) in target
        .providers
        .iter_mut()
        .zip(&source.incremental.strata.source.declarations)
    {
        if declaration.value.is_some() {
            select(&mut fields.canonical_set, choice)?;
        } else {
            select(&mut fields.seen, choice)?;
        }
    }
    check_storage(source, &target, choice)?;
    Ok(target)
}

pub(crate) fn round(source: &rounds::Program, choice: Choice) -> syn::Result<rounds::Program> {
    rounds::contract::check(source)?;
    let mut target = source.clone();
    for stratum in &mut target.context.strata {
        for key in &mut stratum.keys {
            select(&mut key.provider, choice)?;
        }
    }
    check_round(source, &target, choice)?;
    Ok(target)
}

/// Independent relation on the original and selected provider: never invokes selection.
fn check_provider(source: &Type, target: &Type, choice: Choice) -> syn::Result<()> {
    let kind = provider(source)?;
    let (names, data, hasher): (&[&str], (&Type, Option<&Type>), Option<Type>) =
        match (&kind, choice) {
            (Provider::Other, _) => {
                return ensure(
                    source == target,
                    target,
                    "hasher changed an opaque provider",
                );
            }
            (Provider::Map(key, value), _) => (
                &["std", "collections", "HashMap"],
                (*key, Some(*value)),
                Some(match choice {
                    Choice::Fx => {
                        syn::parse_quote!(
                            ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
                        )
                    }
                    Choice::Keyed => syn::parse_quote!(::cq_storage::HashState),
                }),
            ),
            (Provider::Set(row), Choice::Fx) => (
                &["std", "collections", "HashSet"],
                (*row, None),
                Some(syn::parse_quote!(
                    ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
                )),
            ),
            (Provider::Set(row), Choice::Keyed) => (&["cq_storage", "HashSet"], (*row, None), None),
            (Provider::Sharded(row), Choice::Fx) => (
                &["cq_datalog_parallel_runtime", "ShardedSeen"],
                (*row, None),
                Some(syn::parse_quote!(
                    ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
                )),
            ),
            (Provider::Sharded(row), Choice::Keyed) => (
                &["cq_datalog_parallel_runtime", "ShardedSeen"],
                (*row, None),
                None,
            ),
        };
    let Type::Path(path) = target else {
        return Err(syn::Error::new_spanned(
            target,
            "hasher target is not a native hash provider",
        ));
    };
    ensure(
        path.qself.is_none() && path.path.leading_colon.is_some() && path_is(&path.path, names),
        target,
        "hasher selected the wrong provider",
    )?;
    ensure(
        path.path
            .segments
            .iter()
            .take(names.len() - 1)
            .all(|s| s.arguments.is_empty()),
        target,
        "hasher changed provider path arguments",
    )?;
    let PathArguments::AngleBracketed(arguments) = &path.path.segments.last().unwrap().arguments
    else {
        return Err(syn::Error::new_spanned(
            target,
            "hasher target is missing type arguments",
        ));
    };
    let arity = 1 + usize::from(data.1.is_some()) + usize::from(hasher.is_some());
    ensure(
        arguments.args.len() == arity,
        target,
        "hasher changed provider arity",
    )?;
    let expected = std::iter::once(data.0).chain(data.1).chain(hasher.as_ref());
    for (actual, expected) in arguments.args.iter().zip(expected) {
        ensure(
            matches!(actual, GenericArgument::Type(ty) if ty == expected),
            target,
            "hasher changed key/value types or selected the wrong hash state",
        )?;
    }
    Ok(())
}

pub(crate) fn check_storage(
    source: &storage::Program,
    target: &storage::Program,
    choice: Choice,
) -> syn::Result<()> {
    storage::check_rewrite(source, target)?;
    ensure(
        source.layouts.len() == target.layouts.len()
            && source.delta_layouts.len() == target.delta_layouts.len(),
        target,
        "hasher changed layout coverage",
    )?;
    let mut unchanged = target.clone();
    for (before, after) in source.layouts.iter().chain(&source.delta_layouts).zip(
        unchanged
            .layouts
            .iter_mut()
            .chain(&mut unchanged.delta_layouts),
    ) {
        check_provider(&before.provider, &after.provider, choice)?;
        after.provider = before.provider.clone();
    }
    for ((before, after), declaration) in source
        .providers
        .iter()
        .zip(&mut unchanged.providers)
        .zip(&source.incremental.strata.source.declarations)
    {
        if declaration.value.is_some() {
            check_provider(&before.canonical_set, &after.canonical_set, choice)?;
            after.canonical_set = before.canonical_set.clone();
        } else {
            check_provider(&before.seen, &after.seen, choice)?;
            after.seen = before.seen.clone();
        }
    }
    ensure(
        source == &unchanged,
        target,
        "hasher changed non-hash storage decisions",
    )
}

pub(crate) fn check_round(
    source: &rounds::Program,
    target: &rounds::Program,
    choice: Choice,
) -> syn::Result<()> {
    rounds::check_rewrite(source, target)?;
    ensure(
        source.context.strata.len() == target.context.strata.len(),
        target,
        "hasher changed stratum coverage",
    )?;
    let mut unchanged = target.clone();
    for (before, after) in source
        .context
        .strata
        .iter()
        .zip(&mut unchanged.context.strata)
    {
        ensure(
            before.keys.len() == after.keys.len(),
            target,
            "hasher changed negation key coverage",
        )?;
        for (before, after) in before.keys.iter().zip(&mut after.keys) {
            check_provider(&before.provider, &after.provider, choice)?;
            after.provider = before.provider.clone();
        }
    }
    ensure(
        source == &unchanged,
        target,
        "hasher changed non-hash round decisions",
    )
}

#[cfg(test)]
mod tests;
