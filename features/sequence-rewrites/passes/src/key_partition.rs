//! Checked key-disjoint decomposition; routing choices remain explicit advice.
use cq_lang_sequence::{Definition, Program, Term};
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, Token, bracketed};

/// Replayable routing advice. Empty keys select contiguous tiles only for a
/// globally unique unkeyed Group; other groups route complete valuation keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPartition {
    pub name: Ident,
    pub input: Ident,
    pub keys: Vec<Ident>,
    pub parts: usize,
}

impl Parse for KeyPartition {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![,]>()?;
        let producer = input.parse()?;
        input.parse::<Token![,]>()?;
        let keys;
        bracketed!(keys in input);
        let keys = keys.parse_terminated(Ident::parse, Token![,])?.into_iter().collect();
        input.parse::<Token![,]>()?;
        let parts = input.parse::<LitInt>()?.base10_parse()?;
        Ok(Self { name, input: producer, keys, parts })
    }
}

impl ToTokens for KeyPartition {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { name, input, keys, parts } = self;
        let parts = LitInt::new(&parts.to_string(), name.span());
        tokens.extend(quote!(#name, #input, [#(#keys),*], #parts));
    }
}

pub fn key_partition(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    key_partition_in(&mut Default::default(), source, &syn::parse2(arguments)?)
}

/// Introduce a named publication before routing. Grading separately proves that
/// publication is at an ancestor of the fixed-width separated pool.
pub fn key_partition_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    advice: &KeyPartition,
) -> syn::Result<Program> {
    checker.check_shared(source)?;
    let index = super::definition(source, &advice.name)?;
    if advice.parts == 0 {
        return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPart: positive partition geometry required"));
    }
    if source.sequence.definitions.iter().any(|d| d.name == advice.input) {
        return Err(syn::Error::new_spanned(&advice.input, "key partition publication name already exists"));
    }
    fn split(term: &Term, advice: &KeyPartition, source: &Program) -> syn::Result<(Term, Term)> {
        let routed = || Term::PartitionK {
            keys: advice.keys.clone(),
            parts: advice.parts,
            inner: Box::new(Term::Ref(advice.input.clone())),
        };
        match term {
            Term::Canon(inner) if !advice.keys.is_empty() => Ok((
                *inner.clone(),
                Term::Flatten(Box::new(Term::Canon(Box::new(routed())))),
            )),
            Term::Group(group, inner) => {
                let global = source.structures.physical.program.definitions.iter()
                    .find(|definition| definition.name == *group)
                    .is_some_and(|definition| matches!(&definition.operator,
                        cq_lang_sequence::structures::physical::Operator::Group(group) if group.keys.is_empty()));
                if global && advice.keys.is_empty() {
                    return Ok((*inner.clone(), Term::Group(group.clone(), Box::new(Term::Tile {
                        n: advice.parts, items: None,
                        inner: Box::new(Term::Ref(advice.input.clone())),
                    }))));
                }
                if advice.keys.is_empty() {
                    return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartGroup: keyed groups require routing keys"));
                }
                let (input, routed) = match inner.as_ref() {
                    Term::Canon(input) => (*input.clone(), Term::Canon(Box::new(routed()))),
                    input => (input.clone(), if global { Term::Canon(Box::new(routed())) } else { routed() }),
                };
                let grouped = Term::Group(group.clone(), Box::new(routed));
                Ok((input, if global { grouped } else { Term::Flatten(Box::new(grouped)) }))
            }
            Term::Store { capacity, inner } => {
                let (input, output) = if matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)) {
                    split(inner, advice, source)?
                } else {
                    if advice.keys.is_empty() {
                        return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: provider routing keys must be nonempty"));
                    }
                    (*inner.clone(), routed())
                };
                Ok((input, Term::Store { capacity: *capacity, inner: Box::new(output) }))
            }
            _ => Err(syn::Error::new_spanned(&advice.name, "key_partition requires Canon with routing keys, Group, or provider Store")),
        }
    }
    let (input, output) = split(&source.sequence.definitions[index].term, advice, source)?;
    let output = if matches!(output, Term::Store { .. }) {
        output
    } else {
        // Pool publication evaluates the collective at its task child.
        Term::Store { capacity: None, inner: Box::new(output) }
    };
    // A bare Store is only a provider candidate when the physical result is
    // actually consumed by a lookup. The construction checker checks every key.
    let mut provider = None;
    if matches!(&source.sequence.definitions[index].term, Term::Store { inner, .. }
        if !matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)))
    {
        let analysis = checker.check_shared(source)?;
        let result = analysis.results.get(&symbol_name(&advice.name));
        fn reads(term: &Term, result: &str, keys: &[Ident], count: &mut usize) -> bool {
            match term {
                Term::Lookup { provider, keys: lookup, inner } => {
                    if symbol_name(provider) == result {
                        *count += 1;
                        if lookup != keys { return false; }
                    }
                    reads(inner, result, keys, count)
                }
                Term::Map(_, t) | Term::Compact(_, t) | Term::Flatten(t) | Term::Fold(_, t)
                | Term::Group(_, t) | Term::Canon(t) | Term::Store { inner: t, .. }
                | Term::Tile { inner: t, .. } | Term::PartitionK { inner: t, .. } => reads(t, result, keys, count),
                Term::Union(arms) => arms.iter().all(|t| reads(t, result, keys, count)),
                Term::Choose(arms) => arms.iter().all(|(_, t)| reads(t, result, keys, count)),
                Term::Rows(_) | Term::Ref(_) => true,
            }
        }
        let mut count = 0;
        if !result.is_some_and(|result| source.sequence.definitions.iter().all(|d| reads(&d.term, result, &advice.keys, &mut count))) || count == 0 {
            return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: Store must build a lookup provider"));
        }
        provider = result.cloned();
    }
    let mut target = source.clone();
    if let Some(provider) = provider {
        let mut found = false;
        for (entry_index, entry) in source.structures.structures.entries.iter().enumerate() {
            if symbol_name(&entry.name) != provider { continue; }
            let cq_lang_sequence::structures::Structure::Built { keyed, .. } = &entry.structure else { continue };
            let keys: Vec<_> = advice.keys.iter().map(symbol_name).collect();
            if keyed.level_names().first() != Some(&keys) {
                return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: routing keys must equal the complete outer provider key"));
            }
            let ty = &keyed.ty;
            let entry_index = LitInt::new(&entry_index.to_string(), advice.name.span());
            target.structures = cq_rewrite_passes::retype(
                &target.structures, quote!(entry(#entry_index) => ::cq_storage::sharded::Sharded<#ty>),
            )?;
            found = true;
        }
        if !found {
            return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: no built provider layout"));
        }
    }
    target.sequence.definitions[index].term = output;
    target.sequence.definitions.insert(index, Definition {
        name: advice.input.clone(),
        term: Term::Store { capacity: None, inner: Box::new(input) },
    });
    super::checked_in(checker, source, target)
}
