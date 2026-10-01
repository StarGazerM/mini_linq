//! Select provider types for built let reads, independently of host capabilities.
//! Compatible membership prefixes share trie levels; ordered reads retain their own structure.
//! The denotation is `bottom + Set`: Rust chain positions are preserved. A
//! const supply is one fixed key, answered directly or by scan and filter;
//! only stream supplies can justify building reusable keyed storage.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_physical as physical;
use cq_lang_physical::policy::Policy;
use cq_lang_physical::syntax::{SupplyKind, symbol_name};
use cq_lang_physical::{ModeKind, Operator};
use cq_lang_structures as structures;
use proc_macro2::TokenStream;
use quote::quote;

/// What a read needs on a key: the strongest need picks the default type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Need {
    Contains,
    Seek,
    Ordered,
}

impl Need {
    fn default_type(self) -> syn::Type {
        match self {
            Need::Contains => syn::parse_quote!(index_provider!(semi)),
            Need::Seek => syn::parse_quote!(index_provider!(probe)),
            Need::Ordered => {
                syn::parse_quote!(index_provider!(ordered))
            }
        }
    }
}

fn sorted(columns: &[String]) -> Vec<String> {
    let mut columns = columns.to_vec();
    columns.sort();
    columns
}

/// Chain cover: when a result's demanded keys form one chain `K₁ ⊂ K₂ ⊂ …`
/// and every key but the longest is only tested for membership, one curried
/// value answers them all — levels `K₁, K₂∖K₁, …` — instead of one structure
/// per key. A probe needs the payload, so it may read only the full chain;
/// an order need keeps its own structure.
fn chain(reads: &[(&Vec<String>, &BTreeSet<Need>)]) -> Option<Vec<Vec<String>>> {
    if reads.len() < 2 {
        return None;
    }
    let mut keys: Vec<&(&Vec<String>, &BTreeSet<Need>)> = reads.iter().collect();
    keys.sort_by_key(|(c, _)| c.len());
    let mut levels = Vec::new();
    let mut previous: Vec<String> = Vec::new();
    for (i, (columns, needs)) in keys.iter().enumerate() {
        let last = i + 1 == keys.len();
        if needs.contains(&Need::Ordered) || (!last && needs.iter().any(|n| *n != Need::Contains)) {
            return None;
        }
        if (i > 0 && columns.len() == previous.len())
            || !previous.iter().all(|c| columns.contains(c))
        {
            return None;
        }
        levels.push(
            columns
                .iter()
                .filter(|c| !previous.contains(c))
                .cloned()
                .collect(),
        );
        previous = columns.to_vec();
    }
    Some(levels)
}

pub fn compile(source: &physical::Program, policy: &Policy) -> syn::Result<structures::Program> {
    let analysis = physical::contract::check(source)?;
    let program = &source.program;
    let operators: BTreeMap<String, &Operator> = program
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), &d.operator))
        .collect();
    let merge_key: BTreeMap<String, Vec<String>> = operators
        .iter()
        .filter_map(|(name, op)| match op {
            Operator::Merge(_, k) => Some((
                name.clone(),
                sorted(&k.columns.iter().map(symbol_name).collect::<Vec<_>>()),
            )),
            _ => None,
        })
        .collect();
    let mut needs: BTreeMap<(String, Vec<String>), BTreeSet<Need>> = BTreeMap::new();
    let mut require = |result: &str, columns: &[String], need: Need| {
        if analysis.modes[result] != ModeKind::Let {
            return;
        }
        needs
            .entry((result.to_string(), sorted(columns)))
            .or_default()
            .insert(need);
    };
    for (result, reads) in &analysis.keyed_reads {
        for (reader, columns, supply) in reads {
            if *supply != SupplyKind::Stream || matches!(operators[reader], Operator::Merge(..)) {
                continue;
            }
            let need = match operators[reader] {
                Operator::Semi(..) => Need::Contains,
                _ => Need::Seek,
            };
            require(result, columns, need);
        }
    }
    for (side, reads) in &analysis.merge_reads {
        for (_, columns) in reads {
            let by_stream = analysis.modes[side] == ModeKind::Pipe
                && merge_key.get(side).is_some_and(|k| *k == sorted(columns));
            if by_stream {
                continue;
            }
            require(side, columns, Need::Ordered);
        }
    }

    // The binary smallest-candidate rewrite also reads each relation at
    // prefix + new-column for membership. Keep these two-level demands
    // local: host reads, other keys and ordered reads retain their paths.
    let mut intersections = BTreeMap::new();
    if policy.smallest_intersections() {
        for pair in program.definitions.windows(2) {
            let Some((a, b, variable)) =
                physical::binary_intersection(&pair[0], &pair[1], &analysis)
            else {
                continue;
            };
            let ak = sorted(&a.columns.iter().map(symbol_name).collect::<Vec<_>>());
            let bk = sorted(
                &b.columns
                    .iter()
                    .map(symbol_name)
                    .filter(|c| c != &variable)
                    .collect::<Vec<_>>(),
            );
            let sides = [(symbol_name(&a.right), ak), (symbol_name(&b.right), bk)];
            let eligible = sides.iter().all(|(name, prefix)| {
                analysis.modes[name] == ModeKind::Let
                    && needs
                        .iter()
                        .filter(|((n, _), _)| n == name)
                        .all(|((_, c), ns)| {
                            !ns.contains(&Need::Ordered)
                                && (*c == *prefix || *c == analysis.headings[name])
                        })
                    && intersections.get(name).is_none_or(|p| p == prefix)
            });
            if eligible {
                for (name, prefix) in sides {
                    needs
                        .entry((name.clone(), prefix.clone()))
                        .or_default()
                        .insert(Need::Contains);
                    needs
                        .entry((name.clone(), analysis.headings[&name].clone()))
                        .or_default()
                        .insert(Need::Contains);
                    intersections.insert(name, prefix);
                }
            }
        }
    }

    let mut entries = TokenStream::new();
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        let ident = &definition.name;
        let reads: Vec<(&Vec<String>, &BTreeSet<Need>)> = needs
            .iter()
            .filter(|((n, _), _)| *n == name)
            .map(|((_, columns), needs)| (columns, needs))
            .collect();
        if let Some(prefix) = intersections.get(&name) {
            let full = &analysis.headings[&name];
            let tail = full
                .iter()
                .filter(|c| !prefix.contains(c))
                .map(|c| analysis.rows[&name].bindings[c].clone());
            let columns = prefix
                .iter()
                .map(|c| analysis.rows[&name].bindings[c].clone());
            let inner = needs[&(name.clone(), full.clone())]
                .iter()
                .max()
                .expect("nonempty")
                .default_type();
            entries.extend(
                quote! { #ident = index_provider!(trie #inner) [#(#columns),*][#(#tail),*]; },
            );
            continue;
        }
        {
            if let Some(chain) = chain(&reads) {
                let (_, needs) = reads
                    .iter()
                    .max_by_key(|(c, _)| c.len())
                    .expect("a chain has a longest key");
                let mut ty = needs.iter().max().expect("nonempty").default_type();
                let mut levels = TokenStream::new();
                for level in chain.iter().rev() {
                    let columns = level
                        .iter()
                        .map(|c| analysis.rows[&name].bindings[c].clone());
                    levels = quote! { [#(#columns),*] #levels };
                }
                for _ in 1..chain.len() {
                    ty = syn::parse_quote!(index_provider!(trie #ty));
                }
                entries.extend(quote! { #ident = #ty #levels; });
                continue;
            }
        }
        for (columns, needs) in reads {
            let ty = needs.iter().max().expect("nonempty").default_type();
            let columns = columns
                .iter()
                .map(|c| analysis.rows[&name].bindings[c].clone());
            entries.extend(quote! { #ident = #ty [#(#columns),*]; });
        }
    }
    let target = structures::Program {
        physical: source.clone(),
        structures: syn::parse2(quote! { structures { #entries } })?,
    };
    structures::contract::check(&target)?;
    Ok(target)
}

mod correspondence;
pub use correspondence::check;
