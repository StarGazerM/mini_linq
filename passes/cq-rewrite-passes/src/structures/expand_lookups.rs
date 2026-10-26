use cq_lang_structures::{self as ir, physical::Operator, symbol_name};
use proc_macro2::TokenStream;
use syn::parse::{Parse, ParseStream};

struct Advice {
    size: syn::LitInt,
    depth: usize,
}

impl Parse for Advice {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let size = input.parse()?;
        let depth = if input.is_empty() {
            2
        } else {
            input.parse::<syn::Token![,]>()?;
            let keyword: syn::Ident = input.parse()?;
            if keyword != "depth" {
                return Err(syn::Error::new_spanned(keyword, "expected depth N"));
            }
            let depth: syn::LitInt = input.parse()?;
            let value = depth.base10_parse()?;
            if value == 0 {
                return Err(syn::Error::new_spanned(
                    depth,
                    "expand depth must be positive",
                ));
            }
            value
        };
        Ok(Self { size, depth })
    }
}

/// Record `stage = batch N expand` at each unstaged probe whose recorded provider
/// exposes exact, length-known ranges: a SortedRun build, a PrefixRun prefix,
/// a resident sorted host, or the first probe of a Smallest intersection with
/// PrefixRun candidate builds on both sides. HashedRun buckets are not exact ranges.
/// Each candidate is kept only if the language's stage contract
/// (ST-Expand/ST-Stage) accepts it; Physical and provider decisions are unchanged.
/// `N, depth D` limits nested expansion frames on one stream (default D = 2).
/// Deeper probes retain lane-local traversal. This is staging advice, not a
/// representation change or a claim that arbitrary frames fit every device.
pub fn expand_lookups(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let Advice { size, depth } = syn::parse2(arguments)?;
    if size.base10_parse::<usize>()? == 0 {
        return Err(syn::Error::new_spanned(
            size,
            "expand_lookups batch size must be positive",
        ));
    }
    let facts = ir::contract::check(source)?;
    let mut target = source.clone();
    let definitions: std::collections::BTreeMap<_, _> = source
        .physical
        .program
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.name), definition))
        .collect();
    let mut expands = facts.expands.clone();
    for definition in &source.physical.program.definitions {
        let Operator::Probe(_, probe) = &definition.operator else {
            continue;
        };
        if facts.stages.contains_key(&symbol_name(&definition.name)) {
            continue;
        }
        let mut ancestor = symbol_name(&probe.left);
        let mut nesting = 0;
        loop {
            if facts.physical.modes[&ancestor] != ir::physical::ModeKind::Pipe
                || facts.retained.contains_key(&ancestor)
            {
                break;
            }
            nesting += usize::from(expands.contains(&ancestor));
            let Some(input) = definitions[&ancestor].operator.input() else {
                break;
            };
            ancestor = symbol_name(input);
        }
        if nesting >= depth {
            continue;
        }
        let name = &definition.name;
        target
            .structures
            .entries
            .push(syn::parse_quote!(#name stage = batch #size expand;));
        if ir::contract::check(&target).is_err() {
            target.structures.entries.pop();
        } else {
            expands.insert(symbol_name(name));
        }
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}
