//! Physical → Structures: bind every keyed read to a value that answers it.
//!
//! This pass decides, for each host relation, the facts the policy states
//! about its value (key, order, canonicity) and, for each read Physical
//! makes through a key that no host answers, the Rust type built over the
//! result. Rules, from mandatory to advisory:
//!
//! 1. Admissibility. Every `probe`, `semi`, `range` and `merge` read of `r`
//!    on columns `C` must be answered: by the host's key when `r` renames a
//!    host keyed on the columns `C` translates to (ordered, where the read
//!    needs order), by the stream itself when `r` is a pipe sorted on `C` by
//!    its own merge, else by a structure built over `r` keyed on `C`. What
//!    cannot be built — a read of a pipe or an alias no host answers — is
//!    left to the Structures contract to report: no default hides it.
//! 2. Guarantee. Defaults preserve the plan's cost premises: O(1) expected
//!    `seek`/`contains`, so probes get `cq_storage::HashIndex` and semijoins
//!    a `HashSet` of keys. An ordered structure (`BTreeMap`, O(log n)) is
//!    chosen only where a read needs key order — a merge side or a range.
//!    One structure per (result, columns): the strongest need picks the type.
//! 3. Advice. A relation declared `#[index(Type)]` fixes the type of every
//!    structure built over it (through renames): any Rust type implementing
//!    `cq_storage::Build` and the traits the reads use, which rustc checks.
//!    Query-specific choices for derived results are authored directly in
//!    the Structures module.
//!
//! The canonical return is Physical's premise (`canon` flag or key
//! inference); nothing is built to make it hold.

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::physical::{self, ModeKind, Operator};
use cq_ir::policy::Policy;
use cq_ir::{structures, symbol_name};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

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
            Need::Seek => syn::parse_quote!(::cq_storage::HashIndex<_, _>),
            Need::Ordered => {
                syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>)
            }
        }
    }
}

/// A host key as the policy states it: sorted column names and order.
struct HostKey {
    columns: Vec<String>,
    ordered: bool,
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
        if columns.len() == previous.len() || !previous.iter().all(|c| columns.contains(c)) {
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

pub fn compile(source: &physical::Module, policy: &Policy) -> syn::Result<structures::Module> {
    let analysis = physical::contract::check(source)?;
    let ra_module = &source.index_requirements.relational_plan;
    let layouts: BTreeMap<String, Vec<String>> = ra_module
        .program
        .inputs()
        .map(|input| {
            (
                symbol_name(&input.name),
                input.columns.iter().map(|c| symbol_name(&c.name)).collect(),
            )
        })
        .collect();

    // Hosts, exactly as the policy states them.
    let mut host_keys: BTreeMap<String, Option<HostKey>> = BTreeMap::new();
    let mut hosts = TokenStream::new();
    for host in &policy.hosts {
        let name = symbol_name(&host.name);
        let Some(layout) = layouts.get(&name) else {
            return Err(syn::Error::new_spanned(
                &host.name,
                "policy names an undeclared host relation",
            ));
        };
        let key = match &host.key {
            Some(key) => {
                for column in &key.columns {
                    if !layout.contains(&symbol_name(column)) {
                        return Err(syn::Error::new_spanned(
                            column,
                            format!("`{name}` has no column `{column}`"),
                        ));
                    }
                }
                let mut columns: Vec<String> = key.columns.iter().map(symbol_name).collect();
                columns.sort();
                columns.dedup();
                Some(HostKey {
                    columns,
                    ordered: key.ordered.is_some(),
                })
            }
            None => None,
        };
        if host_keys.insert(name, key).is_some() {
            return Err(syn::Error::new_spanned(
                &host.name,
                "host relation configured twice",
            ));
        }
        let (ident, key, canon) = (&host.name, &host.key, &host.canon);
        hosts.extend(quote! { #ident #key #canon; });
    }
    if let Some(missing) = layouts.keys().find(|name| !host_keys.contains_key(*name)) {
        return Err(syn::Error::new_spanned(
            policy,
            format!("policy must list host relation `{missing}`"),
        ));
    }

    // The host key answering a read of `result` on `columns` through its
    // rename, as `structures::Analysis::host_key_for` will see it.
    let host_key_for = |result: &str, columns: &[String]| -> Option<&HostKey> {
        let host = analysis.hosts.get(result)?;
        let key = host_keys[host].as_ref()?;
        let rename = &analysis.renames[result];
        let translated = sorted(
            &columns
                .iter()
                .map(|c| rename[c].clone())
                .collect::<Vec<_>>(),
        );
        (translated == key.columns).then_some(key)
    };

    // Advice declared on the host relation applies to its renames.
    let mut advice: BTreeMap<String, syn::Type> = BTreeMap::new();
    for input in ra_module.program.inputs() {
        if let Some(attr) = input.index_advice()? {
            advice.insert(symbol_name(&input.name), attr.parse_args()?);
        }
    }

    // What every read requires on (result, key), unless a host or the stream
    // itself answers it. Only a `let` can carry a structure; anything else
    // left unanswered is the contract's diagnostic, not a default.
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
    for (result, reads) in &analysis.key_reads {
        for (reader, columns) in reads {
            if host_key_for(result, columns).is_some() {
                continue;
            }
            let need = match operators[reader] {
                Operator::Semi(..) => Need::Contains,
                _ => Need::Seek,
            };
            require(result, columns, need);
        }
    }
    for (result, reads) in &analysis.range_reads {
        for (_, column) in reads {
            let columns = std::slice::from_ref(column);
            if host_key_for(result, columns).is_some_and(|k| k.ordered) {
                continue;
            }
            require(result, columns, Need::Ordered);
        }
    }
    for (side, reads) in &analysis.merge_reads {
        for (_, columns) in reads {
            let by_host = host_key_for(side, columns).is_some_and(|k| k.ordered);
            let by_stream = analysis.modes[side] == ModeKind::Pipe
                && merge_key.get(side).is_some_and(|k| *k == sorted(columns));
            if by_host || by_stream {
                continue;
            }
            require(side, columns, Need::Ordered);
        }
    }

    // The binary smallest-candidate rewrite also reads each relation at
    // prefix + new-column for membership. Keep these two-level demands
    // local: advice, hosts, other keys and ordered reads retain their paths.
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
                !analysis
                    .hosts
                    .get(name)
                    .is_some_and(|h| advice.contains_key(h))
                    && !analysis.key_reads[name]
                        .iter()
                        .any(|(_, c)| host_key_for(name, c).is_some())
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
        let advised = analysis.hosts.get(&name).and_then(|host| advice.get(host));
        if let Some(prefix) = intersections.get(&name) {
            let full = &analysis.headings[&name];
            let tail = full
                .iter()
                .filter(|c| !prefix.contains(c))
                .map(|c| format_ident!("{c}"));
            let columns = prefix.iter().map(|c| format_ident!("{c}"));
            let inner = needs[&(name.clone(), full.clone())]
                .iter()
                .max()
                .expect("nonempty")
                .default_type();
            entries.extend(
                quote! { #ident = ::cq_storage::Trie<'_, _, #inner> [#(#columns),*][#(#tail),*]; },
            );
            continue;
        }
        if advised.is_none() {
            if let Some(chain) = chain(&reads) {
                let (_, needs) = reads
                    .iter()
                    .max_by_key(|(c, _)| c.len())
                    .expect("a chain has a longest key");
                let mut ty = needs.iter().max().expect("nonempty").default_type();
                let mut levels = TokenStream::new();
                for level in chain.iter().rev() {
                    let columns = level.iter().map(|c| format_ident!("{c}"));
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
            let ty = match advised {
                Some(ty) => ty.clone(),
                None => needs.iter().max().expect("nonempty").default_type(),
            };
            let columns = columns.iter().map(|c| format_ident!("{c}"));
            entries.extend(quote! { #ident = #ty [#(#columns),*]; });
        }
    }
    let target: structures::Module =
        syn::parse2(quote! { #source structures { #hosts #entries } })?;
    structures::contract::check(&target)?;
    Ok(target)
}

/// Physical → Structures correspondence: Physical preserved, the contract
/// holds, and the module is exactly the selection under `policy`.
pub fn check(source: &physical::Module, target: &structures::Module, policy: &Policy) -> bool {
    target.physical == *source
        && structures::contract::well_formed(target)
        && compile(source, policy).is_ok_and(|expected| expected == *target)
}
