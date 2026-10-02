//! Select provider types for built let reads, independently of host capabilities.
//! Compatible membership prefixes share trie levels; ordered reads retain their own structure.
//! Smallest intersections select candidate levels independently for each retained
//! side; reusable hosts retain their candidate and membership fallbacks.
//! The denotation is `bottom + Set`: Rust chain positions are preserved. A
//! const supply is one fixed key, answered directly or by scan and filter;
//! only stream supplies can justify building reusable keyed storage.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_physical as physical;
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
            Need::Contains => syn::parse_quote!(::std::collections::HashSet<_>),
            Need::Seek => syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>),
            Need::Ordered => {
                syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>)
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

pub fn compile(source: &physical::Program) -> syn::Result<structures::Program> {
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
        if analysis.modes[result] == ModeKind::Pipe {
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
            // Smallest enumerates candidate keys, rather than probing the
            // prefix for row payloads. Its two-level demand is recorded below.
            if matches!(operators[reader], Operator::Probe(_, k) if matches!(k.choice, Some(physical::Choice::Smallest(_))))
            {
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
    let mut candidate_keys = BTreeSet::new();
    for pair in program.definitions.windows(2) {
        if !matches!(&pair[0].operator, Operator::Probe(_, k) if matches!(k.choice, Some(physical::Choice::Smallest(_))))
        {
            continue;
        }
        if let Some((a, b, v)) = physical::binary_intersection(&pair[0], &pair[1], &analysis) {
            candidate_keys.insert((
                symbol_name(&a.right),
                sorted(&a.columns.iter().map(symbol_name).collect::<Vec<_>>()),
            ));
            candidate_keys.insert((
                symbol_name(&b.right),
                sorted(
                    &b.columns
                        .iter()
                        .map(symbol_name)
                        .filter(|c| *c != v)
                        .collect::<Vec<_>>(),
                ),
            ));
        }
    }
    let mut entries = TokenStream::new();
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        let ident = &definition.name;
        let prefixes: Vec<_> = candidate_keys
            .iter()
            .filter(|(n, _)| *n == name && analysis.modes[n] == ModeKind::Let)
            .map(|(_, prefix)| prefix)
            .collect();
        let full = &analysis.headings[&name];
        for prefix in &prefixes {
            let tail = full
                .iter()
                .filter(|c| !prefix.contains(c))
                .map(|c| analysis.rows[&name].bindings[c].clone());
            let columns = prefix
                .iter()
                .map(|c| analysis.rows[&name].bindings[c].clone());
            // The inner value enumerates the new column and checks full-key
            // membership. A full-key payload probe may share it too; ordered
            // reads keep their separate structure.
            let inner = needs
                .get(&(name.clone(), full.clone()))
                .into_iter()
                .flatten()
                .filter(|need| **need != Need::Ordered)
                .max()
                .copied()
                .unwrap_or(Need::Contains)
                .default_type();
            entries.extend(
                quote! { #ident = ::cq_storage::Trie<'_, _, #inner> [#(#columns),*][#(#tail),*]; },
            );
        }
        let remaining: Vec<_> = needs
            .iter()
            .filter(|((n, _), _)| *n == name)
            .filter_map(|((_, columns), needs)| {
                let remaining: BTreeSet<_> = needs
                    .iter()
                    .copied()
                    // Candidate levels answer full-key reads and prefix
                    // membership, but ordinary prefix payloads and ordered
                    // cursors still need their own storage.
                    .filter(|need| {
                        prefixes.is_empty()
                            || *need == Need::Ordered
                            || (columns != full
                                && (*need != Need::Contains || !prefixes.contains(&columns)))
                    })
                    .collect();
                (!remaining.is_empty()).then_some((columns, remaining))
            })
            .collect();
        let reads: Vec<_> = remaining.iter().map(|(c, ns)| (*c, ns)).collect();
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
                    ty = syn::parse_quote!(::cq_storage::Trie<'_, _, #ty>);
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
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        let ident = &definition.name;
        if definition.canon.is_some() {
            entries.extend(quote!(#ident canon = ::std::collections::HashSet<_>;));
        }
        if analysis.modes[&name] != ModeKind::Pipe {
            let ty: syn::Type = if analysis.modes[&name] == ModeKind::Exists {
                syn::parse_quote!(::cq_storage::Exists)
            } else {
                syn::parse_quote!(::std::vec::Vec<_>)
            };
            entries.extend(quote!(#ident rows = #ty;));
        }
        if analysis.hosts.contains_key(&name) {
            entries.extend(quote!(#ident host scan = ::core::iter::Map<_, _> [];));
        }
        if analysis.hosts.contains_key(&name) && analysis.modes[&name] == ModeKind::Pipe {
            for (reader, columns, supply) in analysis.keyed_reads.get(&name).into_iter().flatten() {
                let rd = &program
                    .definitions
                    .iter()
                    .find(|d| symbol_name(&d.name) == *reader)
                    .expect("checked reader")
                    .name;
                let ty: syn::Type = match supply {
                    SupplyKind::Const | SupplyKind::Range => {
                        syn::parse_quote!(::core::iter::Map<_, _>)
                    }
                    SupplyKind::Stream if matches!(operators[reader], Operator::Merge(..)) => {
                        Need::Ordered.default_type()
                    }
                    SupplyKind::Stream => {
                        let payload = candidate_keys.contains(&(name.clone(), columns.clone()))
                            || analysis.key_reads[&name].iter().any(|(r, c)| {
                                c == columns && matches!(operators[r], Operator::Probe(..))
                            });
                        if payload {
                            Need::Seek.default_type()
                        } else if matches!(operators[reader], Operator::Semi(_, k) if matches!(k.choice, Some(physical::Choice::Lazy(_))))
                        {
                            syn::parse_quote!(::cq_storage::dispatch::KeySet<_, false>)
                        } else {
                            syn::parse_quote!(::cq_storage::dispatch::KeySet<_>)
                        }
                    }
                };
                let columns = columns.iter().map(|c| &analysis.rows[&name].bindings[c]);
                entries.extend(quote!(#ident host #rd = #ty [#(#columns),*];));
            }
        }
    }
    let mut extra = BTreeSet::new();
    for pair in program.definitions.windows(2) {
        if !matches!(&pair[0].operator, Operator::Probe(_, k) if matches!(k.choice, Some(physical::Choice::Smallest(_))))
        {
            continue;
        }
        let (a, b, v) = physical::binary_intersection(&pair[0], &pair[1], &analysis)
            .expect("checked intersection");
        for (name, prefix) in [
            (
                symbol_name(&a.right),
                sorted(&a.columns.iter().map(symbol_name).collect::<Vec<_>>()),
            ),
            (
                symbol_name(&b.right),
                sorted(
                    &b.columns
                        .iter()
                        .map(symbol_name)
                        .filter(|c| *c != v)
                        .collect::<Vec<_>>(),
                ),
            ),
        ] {
            if !analysis.hosts.contains_key(&name) || analysis.modes[&name] != ModeKind::Pipe {
                continue;
            }
            let ident = &program
                .definitions
                .iter()
                .find(|d| symbol_name(&d.name) == name)
                .expect("host result")
                .name;
            for (reader, cols) in [
                ("candidates", prefix),
                ("membership", analysis.headings[&name].clone()),
            ] {
                if !extra.insert((name.clone(), reader, cols.clone())) {
                    continue;
                }
                let columns = cols.iter().map(|c| &analysis.rows[&name].bindings[c]);
                let rd = syn::Ident::new(reader, proc_macro2::Span::call_site());
                let payload = reader == "candidates"
                    || analysis
                        .key_reads
                        .get(&name)
                        .into_iter()
                        .flatten()
                        .any(|(r, c)| *c == cols && matches!(operators[r], Operator::Probe(..)));
                let ty: syn::Type = if payload {
                    Need::Seek.default_type()
                } else {
                    syn::parse_quote!(::cq_storage::dispatch::KeySet<_>)
                };
                entries.extend(quote!(#ident host #rd = #ty [#(#columns),*];));
            }
        }
    }
    if structures::head_needs_dedup(source, &analysis) {
        let returned = &program.returned;
        entries.extend(quote!(#returned head = ::std::collections::HashSet<_>;));
    }
    let target = structures::Program {
        physical: source.clone(),
        structures: syn::parse2(quote! { structures { #entries } })?,
    };
    check(source, &target)?;
    Ok(target)
}

mod correspondence;
pub use correspondence::check;

pub use structures::check_rewrite;
