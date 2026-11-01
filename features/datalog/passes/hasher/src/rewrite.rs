use cq_datalog_support::ensure;
use cq_lang_datalog_round as rounds;
use cq_lang_datalog_storage as storage;
use storage::provider::{self, Token};
use syn::{Type, ext::IdentExt};

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

impl Choice {
    fn hasher(self) -> storage::Hasher {
        match self {
            Self::Fx => storage::Hasher::Fx,
            Self::Keyed => storage::Hasher::Keyed,
        }
    }
}

/// Whether a token's instances take the selected hasher: hashed tokens with a
/// spelling function. Verbatim (authored/device) tokens remain opaque.
fn selects(tokens: &[Token], provider: &storage::Provider) -> syn::Result<bool> {
    let declaration = provider::lookup(tokens, &provider.token)?;
    Ok(declaration.hashed && declaration.spelling != provider::Spelling::Verbatim)
}

/// Record the selected hasher state and respell the instance from its schema.
fn select(
    tokens: &[Token],
    provider: &mut storage::Provider,
    hasher: &mut storage::Hasher,
    (key, payload): (&Type, &Type),
    choice: Choice,
) -> syn::Result<()> {
    if selects(tokens, provider)? {
        *hasher = choice.hasher();
        provider.ty = provider::spell(
            provider::lookup(tokens, &provider.token)?,
            key,
            payload,
            *hasher,
        )?;
    }
    Ok(())
}

fn layout_schema(program: &storage::Program, layout: &storage::Layout) -> (Type, Type) {
    let declaration = program
        .incremental
        .strata
        .source
        .declarations
        .iter()
        .find(|declaration| declaration.name == layout.relation)
        .expect("checked layout relation");
    storage::layout_schema(declaration, &layout.columns, layout.key)
}

fn key_schema() -> (Type, Type) {
    (syn::parse_quote!(_), syn::parse_quote!(()))
}

pub(crate) fn storage(source: &storage::Program, choice: Choice) -> syn::Result<storage::Program> {
    storage::contract::check(source)?;
    let mut target = source.clone();
    for layout in target.layouts.iter_mut().chain(&mut target.delta_layouts) {
        let (key, payload) = layout_schema(source, layout);
        select(
            &source.tokens,
            &mut layout.provider,
            &mut layout.hasher,
            (&key, &payload),
            choice,
        )?;
    }
    check_storage(source, &target, choice)?;
    Ok(target)
}

pub(crate) fn round(source: &rounds::Program, choice: Choice) -> syn::Result<rounds::Program> {
    rounds::contract::check(source)?;
    let mut target = source.clone();
    let (key, payload) = key_schema();
    for stratum in &mut target.context.strata {
        for set in &mut stratum.keys {
            select(
                &source.storage.tokens,
                &mut set.provider,
                &mut set.hasher,
                (&key, &payload),
                choice,
            )?;
        }
    }
    check_round(source, &target, choice)?;
    Ok(target)
}

/// Independent relation on the original and selected instance: the token is
/// kept, a selecting token records the chosen hasher, any other instance is
/// unchanged. The target contract checks the spelling of the recorded state.
fn check_instance(
    tokens: &[Token],
    (source, source_hasher): (&storage::Provider, storage::Hasher),
    (target, target_hasher): (&storage::Provider, storage::Hasher),
    choice: Choice,
) -> syn::Result<()> {
    ensure(
        source.token == target.token,
        &target.token,
        "hasher changed a provider token",
    )?;
    if selects(tokens, source)? {
        ensure(
            target_hasher == choice.hasher(),
            &target.token,
            "hasher selected the wrong hash state",
        )
    } else {
        ensure(
            source == target && source_hasher == target_hasher,
            &target.token,
            "hasher changed an opaque provider",
        )
    }
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
        check_instance(
            &source.tokens,
            (&before.provider, before.hasher),
            (&after.provider, after.hasher),
            choice,
        )?;
        after.provider = before.provider.clone();
        after.hasher = before.hasher;
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
            check_instance(
                &source.storage.tokens,
                (&before.provider, before.hasher),
                (&after.provider, after.hasher),
                choice,
            )?;
            after.provider = before.provider.clone();
            after.hasher = before.hasher;
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
