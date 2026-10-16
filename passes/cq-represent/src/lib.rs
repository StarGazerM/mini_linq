//! Record opt-in positional carriers for direct pipe chains.
//!
//! The default leaves all decisions untouched. `positions` selects every eligible
//! scan chain; `positions(r, s)` selects named results and their scan ancestors.
//! Every prefix decision is recorded explicitly. Canonical boundaries, retained
//! rows and join outputs keep tuple carriers. Join transport (RowId index payloads
//! and build-side representations) is future work; the language already admits
//! multi-base representations independently of this printer policy.
use cq_lang_structures as structures;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
use structures::physical::{ModeKind, Operator};
use structures::syntax::symbol_name;
use syn::parse::{Parse, Parser};
fn eligible(
    source: &structures::Program,
    facts: &structures::Analysis,
) -> BTreeMap<String, Result<Option<String>, &'static str>> {
    let origins = structures::representation_bases(source, facts);
    let mut decisions = BTreeMap::new();
    for definition in &source.physical.program.definitions {
        let name = symbol_name(&definition.name);
        let decision = if facts.physical.modes[&name] != ModeKind::Pipe {
            Err("retained results use tuple carriers")
        } else if definition.canon.is_some() {
            Err("canonical boundaries use tuple carriers")
        } else {
            let parent = match &definition.operator {
                Operator::Rename(_) => Ok(None),
                Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                    Ok(Some(symbol_name(&s.input)))
                }
                Operator::Project(p) => Ok(Some(symbol_name(&p.input))),
                Operator::Semi(_, k) | Operator::Anti(_, k) => Ok(Some(symbol_name(&k.left))),
                _ => Err("operator is not a direct positional scan-chain step"),
            };
            parent.and_then(|parent| {
                if parent
                    .as_ref()
                    .is_some_and(|p| !matches!(decisions.get(p), Some(Ok(_))))
                {
                    return Err("input is outside an eligible positional scan chain");
                }
                let Some(origin) = origins.get(&name).and_then(|origins| origins.first()) else {
                    return Err("result has no independently carried scan occurrence");
                };
                let covered: BTreeSet<_> = origin.columns.keys().collect();
                if facts.physical.headings[&name].iter().any(|column| {
                    !covered.contains(column)
                        && !facts.physical.rows[&name].computed.contains(column)
                }) {
                    return Err("heading contains a column without lineage or owned storage");
                }
                Ok(parent)
            })
        };
        decisions.insert(name, decision);
    }
    decisions
}

fn representation(
    name: &str,
    origins: &BTreeMap<String, Vec<structures::Lineage>>,
    facts: &structures::Analysis,
) -> syn::Result<structures::Repr> {
    let bases = origins[name].iter().take(1).map(|origin| {
        let base = syn::Ident::new(&origin.base, proc_macro2::Span::call_site());
        let columns = origin.columns.iter().map(|(logical, host)| {
            let logical = &facts.physical.rows[name].bindings[logical];
            let host = syn::Ident::new(host, proc_macro2::Span::call_site());
            quote!(#logical -> #host)
        });
        quote!(#base lineage { #(#columns),* })
    });
    let owned = facts.physical.rows[name]
        .computed
        .iter()
        .filter(|column| facts.physical.headings[name].contains(column))
        .map(|column| &facts.physical.rows[name].bindings[column]);
    syn::parse2(quote!(positions { #(#bases),* } owned [#(#owned),*]))
}

/// Leave representation decisions unchanged unless explicitly requested.
pub fn compile(source: &structures::Program) -> syn::Result<structures::Program> {
    compile_with(source, TokenStream::new())
}

/// Select all direct scan chains, or the dependency closure of named results.
pub fn compile_with(
    source: &structures::Program,
    arguments: TokenStream,
) -> syn::Result<structures::Program> {
    let facts = structures::contract::check(source)?;
    if arguments.is_empty() {
        return Ok(source.clone());
    }
    let names = (|input: syn::parse::ParseStream<'_>| {
        let policy: syn::Ident = input.parse()?;
        if policy != "positions" {
            return Err(syn::Error::new_spanned(
                policy,
                "expected positions or positions(results)",
            ));
        }
        let names = if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            Some(content.parse_terminated(syn::Ident::parse, syn::Token![,])?)
        } else {
            None
        };
        Ok(names)
    })
    .parse2(arguments)?;
    let eligible = eligible(source, &facts);
    let mut selected = BTreeSet::new();
    if let Some(names) = names {
        for ident in names {
            let mut name = symbol_name(&ident);
            loop {
                let parent = eligible
                    .get(&name)
                    .ok_or_else(|| {
                        syn::Error::new_spanned(
                            &ident,
                            format!("{name} cannot be carried as positions: unknown result"),
                        )
                    })?
                    .as_ref()
                    .map_err(|reason| {
                        syn::Error::new_spanned(
                            &ident,
                            format!("{name} cannot be carried as positions: {reason}"),
                        )
                    })?;
                selected.insert(name.clone());
                let Some(parent) = parent else { break };
                name = parent.clone();
            }
        }
    } else {
        selected.extend(
            eligible
                .iter()
                .filter(|(_, decision)| decision.is_ok())
                .map(|(name, _)| name.clone()),
        );
    }
    let origins = structures::representation_bases(source, &facts);
    let mut target = source.clone();
    for definition in &source.physical.program.definitions {
        let name = symbol_name(&definition.name);
        if !selected.contains(&name) || facts.repr.contains_key(&name) {
            continue;
        }
        let repr = representation(&name, &origins, &facts)?;
        let ident = &definition.name;
        target
            .structures
            .entries
            .push(syn::parse2(quote!(#ident repr = #repr;))?);
    }
    check(source, &target)?;
    Ok(target)
}

/// Check additive decisions structurally, without invoking the compiler policy.
pub fn check(source: &structures::Program, target: &structures::Program) -> syn::Result<()> {
    structures::check_rewrite(source, target)?;
    let facts = structures::contract::check(source)?;
    let eligible = eligible(source, &facts);
    let origins = structures::representation_bases(source, &facts);
    let before = &source.structures.entries;
    let after = &target.structures.entries;
    if after.len() < before.len() || after[..before.len()] != before[..] {
        return Err(syn::Error::new_spanned(
            target,
            "representation pass must preserve existing entries",
        ));
    }
    let mut selected: BTreeSet<_> = facts
        .repr
        .iter()
        .filter_map(|(name, repr)| {
            matches!(repr, structures::Repr::Positions { .. }).then_some(name.clone())
        })
        .collect();
    for entry in &after[before.len()..] {
        let name = symbol_name(&entry.name);
        let structures::Structure::Repr { repr, .. } = &entry.structure else {
            return Err(syn::Error::new_spanned(
                entry,
                "representation pass may only append repr decisions",
            ));
        };
        let Some(Ok(parent)) = eligible.get(&name) else {
            return Err(syn::Error::new_spanned(
                entry,
                "result cannot be carried as positions",
            ));
        };
        if parent
            .as_ref()
            .is_some_and(|parent| !selected.contains(parent))
            || repr.to_token_stream().to_string()
                != representation(&name, &origins, &facts)?
                    .to_token_stream()
                    .to_string()
        {
            return Err(syn::Error::new_spanned(
                entry,
                "repr decision differs from derived scan-chain representation",
            ));
        }
        selected.insert(name);
    }
    Ok(())
}
