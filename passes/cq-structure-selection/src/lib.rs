//! Select provider types for built let reads. Host reads are native only where
//! the input declares the capability on exactly the read's key
//! (SEL-NativeHost); every other host read records its fallback provider.
//! Compatible membership prefixes share trie levels; ordered reads retain their own structure.
//! Smallest intersections select candidate levels independently for each retained
//! side; reusable hosts retain their candidate and membership fallbacks.
//! Membership-only projections with a one-column stored key record dense
//! membership (rustc requires its `OrderKey`); wider keys record hash
//! membership. Ordinary key providers retain enumeration for scan filters.
//! Rust key types are opaque.
//! The denotation is `bottom + Set`: Rust chain positions are preserved. A
//! const supply is one fixed key, answered directly or by scan and filter;
//! only stream supplies can justify building reusable keyed storage.
//! Explicit checked decisions are available through [`retype`] and
//! [`build_host_reads`], independently of optional rewrite macro frontends.
//! These preserve the existing Structures and Physical rewrite contracts.

//! Selection preserves Physical union/anti choices and chooses dedup types, anti indices and direct chained-scan union index builds.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! branch arms retain their explicit relational predicates. Rust expressions
//! remain opaque and total; only `where` supplies relational constraints.
//!
pub mod arguments;
mod host_reads;
mod retype;

pub use host_reads::build_host_reads;
pub use retype::retype;
use structures::provider::base as providers;

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_physical as physical;
use cq_lang_physical::syntax::{SupplyKind, symbol_name};
use cq_lang_physical::{ModeKind, Operator};
use cq_lang_structures as structures;
use proc_macro2::TokenStream;
use quote::quote;

/// What a read needs on a key: the strongest need picks the default token.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Need {
    Contains,
    Seek,
    Ordered,
}

impl Need {
    fn provider(self) -> structures::Provider {
        match self {
            // Semi/anti/full-key probes must answer membership for each outer
            // row before that outer stream ends. This is a keyed read index,
            // not whole-stream self-canonicalization (which records Sort).
            Need::Contains => providers::declaration("hash_set").instance(),
            Need::Seek => providers::declaration("hash_index").instance(),
            Need::Ordered => providers::declaration("btree_map_grouped").instance(),
        }
    }
}

/// Nest `inner` under `depth - 1` trie levels.
fn trie(mut provider: structures::Provider, depth: usize) -> structures::Provider {
    for _ in 1..depth {
        provider = providers::declaration("trie")
            .compose(provider)
            .expect("trie declares its nested argument");
    }
    provider
}

/// The base instance of `token` spelled `ty`.
fn instance(token: &str, ty: syn::Type) -> structures::Provider {
    providers::instance(token, ty)
}

/// Formal rule STR-AdaptiveMembership (selection): a membership-only store
/// over a one-column stored key records dense ordinal membership, whose
/// `OrderKey` bound rustc checks; wider keys record exact hash membership.
fn membership_provider(stored_arity: usize) -> structures::Provider {
    if stored_arity == 1 {
        providers::declaration("dense_membership").instance()
    } else {
        providers::declaration("adaptive_membership").instance()
    }
}

/// Formal rule SEL-NativeHost: the first of `wanted` that the host input of
/// rename `name` declares on exactly `columns` (result attributes), IF-HostDecl,
/// with the key columns in host declaration order (the host's key layout).
/// Only the declaration decides; the host's Rust type is never inspected.
fn native_read(
    source: &physical::Program,
    analysis: &physical::Analysis,
    name: &str,
    columns: &[String],
    wanted: &[physical::syntax::HostRead],
) -> Option<(physical::syntax::HostRead, Vec<String>)> {
    let input = analysis.hosts.get(name)?;
    let declaration = source
        .inputs()
        .find(|declaration| symbol_name(&declaration.name) == *input)?;
    let renames = &analysis.renames[name];
    let mut key = columns
        .iter()
        .map(|column| renames.get(column).cloned())
        .collect::<Option<Vec<_>>>()?;
    key.sort();
    let host = declaration.host(&key)?;
    let read = wanted.iter().copied().find(|read| host.reads(*read))?;
    let ordered = declaration
        .columns
        .iter()
        .map(|column| symbol_name(&column.name))
        .filter(|column| key.contains(column))
        .map(|column| {
            columns
                .iter()
                .find(|attribute| renames[*attribute] == column)
                .expect("renamed key column")
                .clone()
        })
        .collect();
    Some((read, ordered))
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
    let mut full_key_probes = BTreeSet::new();
    let candidates = physical::candidate_prefixes(source, &analysis);
    let candidate_sources: BTreeSet<_> = candidates
        .iter()
        .flat_map(|candidate| candidate.sides.iter().map(|(name, _)| name.clone()))
        .collect();
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
            if columns.is_empty()
                || *supply != SupplyKind::Stream
                || matches!(operators[reader], Operator::Merge(..))
            {
                continue;
            }
            // Smallest enumerates candidate keys, rather than probing the
            // prefix for row payloads. Its two-level demand is recorded below.
            if matches!(operators[reader], Operator::Probe(_, k) if matches!(k.choice, Some(physical::Choice::Smallest(_))))
            {
                continue;
            }
            let full_key_probe = matches!(operators[reader], Operator::Probe(..))
                && !candidate_sources.contains(result)
                && columns == &analysis.headings[result]
                && reads.iter().all(|(reader, key, supply)| {
                    key == columns && *supply == SupplyKind::Stream
                        // A shared smallest-intersection adapter also enumerates
                        // candidates; retain its existing seek-provider policy.
                        && !matches!(operators[reader], Operator::Probe(_, k)
                            if matches!(k.choice, Some(physical::Choice::Smallest(_))))
                })
                && analysis.merge_reads.get(result).is_none_or(Vec::is_empty);
            let need = if full_key_probe {
                full_key_probes.insert((result.clone(), columns.clone()));
                Need::Contains
            } else {
                match operators[reader] {
                    Operator::Semi(..) | Operator::Anti(..) => Need::Contains,
                    _ => Need::Seek,
                }
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

    // The k-way smallest-candidate rewrite also reads each relation at
    // prefix + new-column for membership. Keep these two-level demands
    // local: host reads, other keys and ordered reads retain their paths.
    let candidate_keys: BTreeSet<_> = candidates
        .iter()
        .flat_map(|c| c.sides.iter().cloned())
        .collect();
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
            let need = needs
                .get(&(name.clone(), full.clone()))
                .into_iter()
                .flatten()
                .filter(|need| **need != Need::Ordered)
                .max()
                .copied()
                .unwrap_or(Need::Contains);
            let inner = trie(need.provider(), 2);
            entries.extend(quote! { #ident = #inner [#(#columns),*][#(#tail),*]; });
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
                let ty = needs.iter().max().expect("nonempty").provider();
                let mut levels = TokenStream::new();
                for level in chain.iter().rev() {
                    let columns = level
                        .iter()
                        .map(|c| analysis.rows[&name].bindings[c].clone());
                    levels = quote! { [#(#columns),*] #levels };
                }
                let ty = trie(ty, chain.len());
                entries.extend(quote! { #ident = #ty #levels; });
                continue;
            }
        }
        for (columns, needs) in reads {
            // Formal rule STR-FullKey: a complete set-row key needs only an
            // existence witness; preserve that decision for every consumer.
            let membership = needs.iter().max() == Some(&Need::Contains)
                && full_key_probes.contains(&(name.clone(), columns.to_vec()));
            let ty = if membership {
                membership_provider(columns.len())
            } else {
                needs.iter().max().expect("nonempty").provider()
            };
            let marker = membership.then(|| quote!(membership));
            let columns = columns
                .iter()
                .map(|c| analysis.rows[&name].bindings[c].clone());
            entries.extend(quote! { #ident = #ty [#(#columns),*] #marker; });
        }
    }
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        let ident = &definition.name;
        if let Operator::Group(group) = &definition.operator
            && !group.keys.is_empty()
        {
            entries.extend(group_accumulator(ident, group));
        }
        // Formal rule SEL-Canon: a canon result's storage is its canonical
        // index. A stored result keeps its rows record, typed by the host
        // sorted-array provider whose build yields distinct rows; a pipe host
        // rename's canonical index is its host carrier.
        if definition.canon.is_some() {
            if analysis.modes[&name] == ModeKind::Pipe {
                entries.extend(quote!(#ident canon = host;));
            } else {
                entries.extend(quote!(#ident canon = rows;));
            }
        }
        if analysis.modes[&name] != ModeKind::Pipe {
            let ty = if analysis.modes[&name] == ModeKind::Exists {
                providers::declaration("exists").instance()
            } else if definition.canon.is_some() {
                providers::declaration("sort").instance()
            } else {
                providers::declaration("vec").instance()
            };
            entries.extend(quote!(#ident rows = #ty;));
        }
        if analysis.hosts.contains_key(&name) {
            let scan = providers::declaration("host_scan").instance();
            entries.extend(quote!(#ident host scan = #scan [];));
            // Leading constants of the occurrence: one native point read
            // when the input declares `exact` on exactly that prefix.
            if let Operator::Rename(rename) = &definition.operator
                && let Some(constants) = &rename.constants
                && definition.canon.is_none()
            {
                let literals: BTreeSet<String> = constants
                    .equalities
                    .iter()
                    .map(|equality| symbol_name(&equality.left))
                    .collect();
                let input = &analysis.hosts[&name];
                if let Some(declaration) = source
                    .inputs()
                    .find(|declaration| symbol_name(&declaration.name) == *input)
                {
                    let prefix: Vec<&syn::Ident> = declaration
                        .columns
                        .iter()
                        .map(|column| &column.name)
                        .take_while(|column| literals.contains(&symbol_name(column)))
                        .collect();
                    let key = sorted(&prefix.iter().map(|c| symbol_name(c)).collect::<Vec<_>>());
                    if !key.is_empty()
                        && declaration
                            .host(&key)
                            .is_some_and(|host| host.reads(physical::syntax::HostRead::Exact))
                    {
                        entries.extend(quote!(#ident native scan = exact [#(#prefix),*];));
                    }
                }
            }
        }
        if analysis.hosts.contains_key(&name) && analysis.modes[&name] == ModeKind::Pipe {
            for (reader, columns, supply) in analysis.keyed_reads.get(&name).into_iter().flatten() {
                if columns.is_empty() {
                    continue;
                }
                let rd = &program
                    .definitions
                    .iter()
                    .find(|d| symbol_name(&d.name) == *reader)
                    .expect("checked reader")
                    .name;
                let membership = full_key_probes.contains(&(name.clone(), columns.clone()));
                use physical::syntax::HostRead;
                let native = match (supply, operators[reader]) {
                    (SupplyKind::Const, Operator::Point(_, select)) => select
                        .condition
                        .constant_keys()
                        .filter(|keys| keys.keys().cloned().collect::<Vec<_>>() == *columns)
                        .filter(|_| {
                            let input = &analysis.hosts[&name];
                            let key = sorted(
                                &columns
                                    .iter()
                                    .map(|column| analysis.renames[&name][column].clone())
                                    .collect::<Vec<_>>(),
                            );
                            source
                                .inputs()
                                .find(|declaration| symbol_name(&declaration.name) == *input)
                                .is_some_and(|declaration| declaration.prefix(&key))
                        })
                        .and_then(|_| {
                            native_read(source, &analysis, &name, columns, &[HostRead::Exact])
                        }),
                    (SupplyKind::Range, Operator::Range(_, select)) => select
                        .condition
                        .range_bounds()
                        .filter(|(column, _)| [column.clone()] == *columns.as_slice())
                        .and_then(|_| {
                            native_read(
                                source,
                                &analysis,
                                &name,
                                columns,
                                &[HostRead::Range, HostRead::Ordered],
                            )
                        }),
                    // Sequence realizes a merge side as keyed lookups. Its
                    // native lookup protocol cannot consume an ordered cursor,
                    // so retain the ordered fallback instead.
                    (SupplyKind::Stream, Operator::Merge(..)) => None,
                    (SupplyKind::Stream, Operator::Probe(..)) if membership => {
                        native_read(source, &analysis, &name, columns, &[HostRead::Membership])
                    }
                    (SupplyKind::Stream, Operator::Probe(..)) => {
                        native_read(source, &analysis, &name, columns, &[HostRead::Exact])
                    }
                    (SupplyKind::Stream, Operator::Semi(..) | Operator::Anti(..)) => {
                        native_read(source, &analysis, &name, columns, &[HostRead::Membership])
                    }
                    _ => None,
                };
                if let Some((read, key)) = native {
                    let read = syn::Ident::new(read.name(), proc_macro2::Span::call_site());
                    let columns = key.iter().map(|c| &analysis.rows[&name].bindings[c]);
                    entries.extend(quote!(#ident native result(#rd) = #read [#(#columns),*];));
                    continue;
                }
                let ty = match supply {
                    SupplyKind::Const | SupplyKind::Range => {
                        providers::declaration("host_scan").instance()
                    }
                    SupplyKind::Stream if membership => membership_provider(columns.len()),
                    SupplyKind::Stream if matches!(operators[reader], Operator::Merge(..)) => {
                        Need::Ordered.provider()
                    }
                    SupplyKind::Stream => {
                        let payload = candidate_keys.contains(&(name.clone(), columns.clone()))
                            || analysis.key_reads[&name].iter().any(|(r, c)| {
                                c == columns && matches!(operators[r], Operator::Probe(..))
                            });
                        if payload {
                            Need::Seek.provider()
                        } else {
                            // STR-ReaderChoice: the reader's recorded eager
                            // or lazy choice prints the declared argument.
                            providers::declaration("keyset").instance()
                        }
                    }
                };
                let marker = membership.then(|| quote!(membership));
                let columns = columns.iter().map(|c| &analysis.rows[&name].bindings[c]);
                entries.extend(quote!(#ident host result(#rd) = #ty [#(#columns),*] #marker;));
            }
        }
    }
    let mut extra = BTreeSet::new();
    for candidate in &candidates {
        for (name, prefix) in candidate.sides.clone() {
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
                (structures::ReaderRef::Candidates, prefix),
                (
                    structures::ReaderRef::Membership,
                    analysis.headings[&name].clone(),
                ),
            ] {
                if !extra.insert((name.clone(), reader.key(), cols.clone())) {
                    continue;
                }
                let wanted = if matches!(reader, structures::ReaderRef::Candidates) {
                    physical::syntax::HostRead::Candidates
                } else {
                    physical::syntax::HostRead::Membership
                };
                if let Some((read, key)) = native_read(source, &analysis, &name, &cols, &[wanted]) {
                    let read = syn::Ident::new(read.name(), proc_macro2::Span::call_site());
                    let columns = key.iter().map(|c| &analysis.rows[&name].bindings[c]);
                    entries.extend(quote!(#ident native #reader = #read [#(#columns),*];));
                    continue;
                }
                let columns = cols.iter().map(|c| &analysis.rows[&name].bindings[c]);
                let payload = matches!(reader, structures::ReaderRef::Candidates)
                    || analysis
                        .key_reads
                        .get(&name)
                        .into_iter()
                        .flatten()
                        .any(|(r, c)| *c == cols && matches!(operators[r], Operator::Probe(..)));
                let ty = if payload {
                    Need::Seek.provider()
                } else {
                    providers::declaration("keyset").instance()
                };
                entries.extend(quote!(#ident host #reader = #ty [#(#columns),*];));
            }
        }
    }
    if program.head_dedup() {
        let returned = &program.returned;
        let head = providers::declaration("sort").instance();
        entries.extend(quote!(#returned head = #head;));
    }
    // Record the storage decision, including its row projection, before any
    // realization. Readers on one key share a single filtered build.
    let mut selected: structures::Structures =
        syn::parse2(quote!(structures { providers {} #entries }))?;
    selected.declare(&providers::declarations())?;
    let required = structures::required_columns(source, &analysis);
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        let direct_union = matches!(&definition.operator,Operator::Union(union) if union.origin.as_ref().is_none_or(|origin|origin.over.is_none()));
        if analysis.modes[&name] != ModeKind::Let
            || analysis.scanned.contains(&name) && !direct_union
            || analysis
                .keyed_reads
                .get(&name)
                .is_some_and(|reads| reads.iter().any(|(_, columns, _)| columns.is_empty()))
        {
            continue;
        }
        let built: Vec<_> = selected
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                (entry.name == definition.name
                    && matches!(entry.structure, structures::Structure::Built { .. }))
                .then_some(i)
            })
            .collect();
        let multi_union = matches!(&definition.operator,Operator::Union(union) if union.origin.as_ref().is_none_or(|origin|origin.over.is_none()));
        if built.is_empty() || built.len() != 1 && !multi_union {
            continue;
        }
        // SRDatalog `canonical_index`: prefer a complete index the result's
        // reads already use. A one-level complete-row index read only for
        // existence is built from the candidates directly (duplicates do not
        // change existence), so it is the canonical index and no rows record
        // is stored. Otherwise the rows record (identity index) stays canonical.
        if definition.canon.is_some() {
            let structures::Structure::Built { keyed, .. } = &selected.entries[built[0]].structure
            else {
                unreachable!()
            };
            let mut heading = analysis.headings[&name].clone();
            heading.sort();
            let existence = analysis.keyed_reads.get(&name).into_iter().flatten().all(
                |(reader, columns, _)| {
                    *columns == heading
                        && (keyed.membership.is_some()
                            || matches!(operators[reader], Operator::Semi(..) | Operator::Anti(..)))
                },
            );
            if built.len() == 1
                && keyed.levels.len() == 1
                && keyed.filtered.is_none()
                && keyed.level_names() == [heading]
                && existence
                && !analysis.scanned.contains(&name)
                && analysis.merge_reads.get(&name).is_none_or(Vec::is_empty)
            {
                let level = keyed.levels[0].clone();
                for entry in &mut selected.entries {
                    if entry.name == definition.name
                        && let structures::Structure::Dedup { index, .. } = &mut entry.structure
                    {
                        *index = structures::CanonIndex::Built(level.clone());
                    }
                }
                selected.entries.retain(|entry| {
                    entry.name != definition.name
                        || !matches!(entry.structure, structures::Structure::Rows { .. })
                });
                continue;
            }
        }
        let select = match &definition.operator {
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select)
                if built.len() == 1
                    && matches!(&selected.entries[built[0]].structure,structures::Structure::Built {keyed,..} if keyed.levels.len()==1) =>
            {
                Some(Some(select))
            }
            Operator::Union(_) => Some(None),
            Operator::Rename(rename)
                if rename.constants.is_none()
                    && built.len() == 1
                    && required[&name].len() < analysis.headings[&name].len()
                    && matches!(&selected.entries[built[0]].structure, structures::Structure::Built { keyed, .. } if keyed.levels.len() == 1) =>
            {
                Some(None)
            }
            _ => None,
        };
        if let Some(select) = select {
            let keys = built
                .iter()
                .map(|i| match &selected.entries[*i].structure {
                    structures::Structure::Built { keyed, .. } => keyed.level_names(),
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>();
            let supported = analysis.keyed_reads.get(&name).into_iter().flatten().all(|(reader,columns,supply)| {
                *supply==SupplyKind::Stream && keys.iter().any(|levels|structures::prefix_depth(levels,columns).is_some()) && (matches!(operators[reader],Operator::Probe(_,key) if matches!(key.choice,Some(physical::Choice::First(_)))) || matches!(operators[reader],Operator::Semi(..)|Operator::Anti(..)))
            });
            if supported {
                let constants = select
                    .map(|select| structures::reconstructible_constants(select))
                    .unwrap_or_default();
                let payload = required[&name]
                    .iter()
                    .filter(|c| !constants.contains_key(*c))
                    .map(|c| &analysis.rows[&name].bindings[c])
                    .collect::<Vec<_>>();
                let bindings = constants
                    .iter()
                    .map(|(column, value)| {
                        let column = &analysis.rows[&name].bindings[column];
                        quote!(#column = #value)
                    })
                    .collect::<Vec<_>>();
                let projection: structures::Filtered =
                    syn::parse2(quote!(payload [#(#payload),*] constants {#(#bindings),*}))?;
                for (i, key) in built.iter().zip(&keys) {
                    let structures::Structure::Built { keyed, .. } =
                        &mut selected.entries[*i].structure
                    else {
                        unreachable!()
                    };
                    if multi_union
                        && required[&name]
                            .iter()
                            .any(|column| !key.iter().flatten().any(|key| key == column))
                    {
                        // This generated provider must reconstruct rows, not just
                        // answer membership. Rebuild its recorded trie levels
                        // from the payload token's declaration.
                        keyed.provider = trie(Need::Seek.provider(), key.len());
                    }
                    keyed.filtered = Some(projection.clone());
                    // Preserve the singleton stored-key policy without inspecting
                    // Rust types. Reconstructed constants do not occupy key storage.
                    if key.len() == 1
                        && key[0]
                            .iter()
                            .filter(|column| !constants.contains_key(*column))
                            .count()
                            == 1
                        && required[&name]
                            .iter()
                            .all(|column| key[0].contains(column) || constants.contains_key(column))
                        && analysis
                            .keyed_reads
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .all(|(_, columns, _)| columns == &key[0])
                    {
                        keyed.membership = Some(Default::default());
                        keyed.provider = membership_provider(1);
                    }
                }
            }
        }
        if built.len()>1 && !built.iter().all(|i|matches!(&selected.entries[*i].structure,structures::Structure::Built {keyed,..} if keyed.filtered.is_some())) { continue; }
        // A canonical rows record stays: an unscanned single consumer streams
        // its distinct-row build into that build (STR-RowsByValue).
        if analysis.scanned.contains(&name) || definition.canon.is_some() {
            continue;
        }
        selected.entries.retain(|entry| {
            entry.name != definition.name
                || !matches!(entry.structure, structures::Structure::Rows { .. })
        });
    }
    // The membership provider follows the stored key after filtering:
    // reconstructed constants occupy no key storage.
    for entry in &mut selected.entries {
        let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
            continue;
        };
        if keyed.membership.is_none()
            || !["adaptive_membership", "dense_membership"]
                .iter()
                .any(|token| keyed.provider.token == token)
        {
            continue;
        }
        let constants: BTreeSet<String> = keyed
            .filtered
            .iter()
            .flat_map(|filtered| &filtered.constants)
            .map(|binding| symbol_name(&binding.column))
            .collect();
        let stored = keyed
            .columns()
            .filter(|column| !constants.contains(&symbol_name(column)))
            .count();
        keyed.provider = membership_provider(stored);
    }
    selected.declare(&providers::declarations())?;
    // Built providers are actually constructed. Do not impose unused fallback
    // bounds on native host reads (for example Ord-only keys need not be Hash).
    // A one-level keyed payload build records the key enumeration its token declares.
    for entry in &mut selected.entries {
        let keyed = match &mut entry.structure {
            structures::Structure::Built { keyed, .. } => keyed,
            _ => continue,
        };
        if keyed.membership.is_none()
            && keyed.levels.len() == 1
            && !keyed.levels[0].columns.is_empty()
            && selected
                .providers
                .facts(&keyed.provider)
                .is_some_and(|facts| facts.supports(structures::ProviderCapability::Keys))
        {
            keyed.supports = Some(syn::parse_quote!(supports[keys]));
        }
    }
    let target = structures::Program {
        physical: source.clone(),
        structures: selected,
        encoded: Default::default(),
        consumer: None,
    };
    check(source, &target)?;
    Ok(target)
}

/// Formal rule SEL-Group: Record the keyed aggregate table and accumulator tuple types.
fn group_accumulator(name: &syn::Ident, group: &physical::syntax::algebra::Group) -> TokenStream {
    let types = group.aggregates.iter().map(|aggregate| &aggregate.ty);
    let provider = instance(
        "group_map",
        syn::parse_quote!(::std::collections::HashMap<_, (#(#types,)*), ::cq_storage::HashState>),
    );
    quote!(#name group = #provider;)
}

mod correspondence;
pub use correspondence::check;

mod positional;
mod blocks;

/// Select with comma-separated context facts and/or `positional(R, S)`.
/// Record facts before choosing representations and checking correspondence.
pub fn compile_with(
    source: &physical::Program,
    arguments: TokenStream,
) -> syn::Result<structures::Program> {
    compile_with_consumer(source, arguments, None)
}

/// Bind a transported outer-consumer requirement before selecting any reads.
/// This requirement is not optimization advice and cannot discharge set demand.
pub fn compile_with_consumer(
    source: &physical::Program,
    arguments: TokenStream,
    consumer: Option<&structures::OutputConsumer>,
) -> syn::Result<structures::Program> {
    let arguments: Arguments = syn::parse2(arguments)?;
    let mut recorded = source.clone();
    arguments.facts.apply(&mut recorded)?;
    let mut target = compile(&recorded)?;
    target.consumer = consumer.cloned();
    if let Some(hosts) = arguments.hosts {
        positional::choose(&mut target, &hosts)?;
    }
    if let Some(blocks) = arguments.blocks {
        blocks::choose(&mut target, &blocks)?;
    }
    check(&recorded, &target)?;
    Ok(target)
}

/// Context premises authorized by this pass's explicit argument packet.
pub fn context(arguments: TokenStream) -> syn::Result<physical::Context> {
    Ok(syn::parse2::<Arguments>(arguments)?.facts.context())
}

#[derive(Default)]
struct Arguments {
    facts: physical::FactPacket,
    hosts: Option<BTreeSet<String>>,
    blocks: Option<blocks::Advice>,
}

impl syn::parse::Parse for Arguments {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut arguments = Self::default();
        while !input.is_empty() {
            if input.peek(physical::kw::consumer) || input.peek(physical::kw::host_set) {
                arguments.facts.parse_fact(input)?;
            } else {
                let keyword: syn::Ident = input.parse()?;
                if keyword == "blocks" {
                    if arguments.blocks.is_some() {
                        return Err(syn::Error::new_spanned(keyword, "duplicate blocks arguments"));
                    }
                    let body;
                    syn::parenthesized!(body in input);
                    arguments.blocks = Some(body.parse()?);
                    if !input.is_empty() { input.parse::<syn::Token![,]>()?; }
                    continue;
                }
                if keyword != "positional" {
                    return Err(syn::Error::new_spanned(
                        keyword,
                        "expected blocks(...), positional(host, ...), consumer = canonical, or host_set R discharged",
                    ));
                }
                if arguments.hosts.is_some() {
                    return Err(syn::Error::new_spanned(
                        keyword,
                        "duplicate positional arguments",
                    ));
                }
                let body;
                syn::parenthesized!(body in input);
                let names = structures::CommaList::<syn::Ident>::parse_terminated(&body)?;
                arguments.hosts = Some(names.iter().map(symbol_name).collect());
            }
            if !input.is_empty() {
                let _: syn::Token![,] = input.parse()?;
            }
        }
        Ok(arguments)
    }
}
