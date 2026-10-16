//! Record bounded total lookahead without changing Physical execution.
//! Defaults preserve the built-probe batching policy; `batch(N)` also stages
//! maximal total pipes. Every proposed boundary is checked by Structures.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_structures as structures;
use proc_macro2::TokenStream;
use quote::quote;
use structures::{
    physical::{ModeKind, Operator},
    symbol_name,
};
use syn::parse::{Parse, ParseStream};

struct Policy(Option<usize>);
impl Parse for Policy {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self(None));
        }
        input.parse::<structures::kw::batch>()?;
        let contents;
        syn::parenthesized!(contents in input);
        let size: syn::LitInt = contents.parse()?;
        let size = size.base10_parse()?;
        if size == 0 || !contents.is_empty() || !input.is_empty() {
            return Err(input.error("expected batch(N) with a positive integer"));
        }
        Ok(Self(Some(size)))
    }
}

fn entry(name: &syn::Ident, size: usize) -> structures::Entry {
    let size = syn::LitInt::new(&size.to_string(), name.span());
    syn::parse_quote!(#name stage = batch #size;)
}

fn decisions(
    source: &structures::Program,
    batch: Option<usize>,
) -> syn::Result<BTreeMap<String, usize>> {
    let facts = structures::contract::check(source)?;
    let definitions: BTreeMap<_, _> = source
        .physical
        .program
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let valid = |name: &str, size| {
        if facts.stages.contains_key(name) {
            return false;
        }
        let mut candidate = source.clone();
        candidate
            .structures
            .entries
            .push(entry(&definitions[name].name, size));
        structures::contract::check(&candidate).is_ok()
    };
    let scan_spine = |mut name: String| {
        loop {
            match &definitions[&name].operator {
                Operator::Rename(_) => return true,
                Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                    name = symbol_name(&s.input)
                }
                Operator::Project(p) => name = symbol_name(&p.input),
                _ => return false,
            }
        }
    };
    let mut result = BTreeMap::new();
    for definition in definitions.values() {
        let Operator::Probe(_, keyed) = &definition.operator else {
            continue;
        };
        let left = symbol_name(&keyed.left);
        let right = symbol_name(&keyed.right);
        let mut columns: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
        columns.sort();
        let built = !facts.host_read(&right)
            && facts.built.get(&right).is_some_and(|built| {
                built
                    .iter()
                    .any(|(levels, _)| levels.len() == 1 && levels[0] == columns)
            });
        let constant_key = facts.filtered.get(&right).is_some_and(|filtered| {
            filtered
                .constants
                .iter()
                .any(|binding| columns.contains(&symbol_name(&binding.column)))
        });
        if built
            && facts.physical.rows[&left].computed.is_empty()
            && !facts.membership.contains(&(right, columns))
            && !constant_key
            && scan_spine(left.clone())
            && valid(&left, 128)
        {
            result.insert(left, 128);
        }
    }
    if let Some(size) = batch {
        let eligible: BTreeSet<_> = definitions
            .keys()
            .filter(|name| facts.physical.modes[*name] == ModeKind::Pipe && valid(name, size))
            .cloned()
            .collect();
        let continued: BTreeSet<_> = eligible
            .iter()
            .filter_map(|name| {
                let input = definitions[name].operator.input()?;
                let input = symbol_name(input);
                (!facts.retained.contains_key(&input) && !facts.stages.contains_key(&input))
                    .then_some(input)
            })
            .collect();
        for name in eligible.difference(&continued) {
            result.entry(name.clone()).or_insert(size);
        }
    }
    Ok(result)
}

/// Preserve current probe batching, with scalar evaluation elsewhere.
pub fn compile(source: &structures::Program) -> syn::Result<structures::Program> {
    compile_with(source, TokenStream::new())
}

/// `batch(N)` additionally records maximal total segments from for producers.
pub fn compile_with(
    source: &structures::Program,
    arguments: TokenStream,
) -> syn::Result<structures::Program> {
    let policy: Policy = syn::parse2(arguments)?;
    let decisions = decisions(source, policy.0)?;
    let mut target = source.clone();
    for definition in &source.physical.program.definitions {
        if let Some(&size) = decisions.get(&symbol_name(&definition.name)) {
            target
                .structures
                .entries
                .push(entry(&definition.name, size));
        }
    }
    structures::contract::check(&target)?;
    Ok(target)
}

/// Check the recorded policy structurally, never by rerunning compilation.
/// A non-default batch size is carried by the additional boundary entries.
pub fn check(source: &structures::Program, target: &structures::Program) -> syn::Result<()> {
    structures::check_rewrite(source, target)?;
    let source_facts = structures::contract::check(source)?;
    let target_facts = structures::contract::check(target)?;
    let unchanged: Vec<_> = target
        .structures
        .entries
        .iter()
        .filter(|entry| {
            !matches!(entry.structure, structures::Structure::Stage { .. })
                || source_facts.stages.contains_key(&symbol_name(&entry.name))
        })
        .collect();
    if unchanged != source.structures.entries.iter().collect::<Vec<_>>() {
        return Err(syn::Error::new_spanned(
            quote!(#target),
            "staging must preserve existing decisions",
        ));
    }
    let added: BTreeMap<_, _> = target_facts
        .stages
        .into_iter()
        .filter(|(name, _)| !source_facts.stages.contains_key(name))
        .collect();
    if added == decisions(source, None)? {
        return Ok(());
    }
    for size in added.values().copied().collect::<BTreeSet<_>>() {
        if added == decisions(source, Some(size))? {
            return Ok(());
        }
    }
    Err(syn::Error::new_spanned(
        quote!(#target),
        "stage entries do not match the derived batching policy",
    ))
}
