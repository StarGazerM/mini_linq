//! Select provider types for built let reads, independently of host capabilities.
//! Compatible membership prefixes share trie levels; ordered reads retain their own structure.
//! Smallest intersections select candidate levels independently for each retained
//! side; reusable hosts retain their candidate and membership fallbacks.
//! The denotation is `bottom + Set`: Rust chain positions are preserved. A
//! const supply is one fixed key, answered directly or by scan and filter;
//! only stream supplies can justify building reusable keyed storage.

//! Selection preserves Physical union/anti choices and chooses dedup types, anti indices and direct chained-scan union index builds.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
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
            Need::Contains => syn::parse_quote!(::cq_storage::HashSet<_>),
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

fn payload_provider(ty: &mut syn::Type) {
    if *ty == syn::parse_quote!(::cq_storage::HashSet<_>) {
        *ty = Need::Seek.default_type();
        return;
    }
    if let syn::Type::Path(path) = ty {
        if let Some(segment) = path.path.segments.last_mut() {
            if segment.ident == "Trie" {
                if let syn::PathArguments::AngleBracketed(args) = &mut segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = args.args.last_mut() {
                        payload_provider(inner);
                    }
                }
            }
        }
    }
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
                Operator::Semi(..) | Operator::Anti(..) => Need::Contains,
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
            entries.extend(quote!(#ident canon = ::cq_storage::HashSet<_>;));
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
                        } else if matches!(operators[reader], Operator::Semi(_, k) | Operator::Anti(_, k) if matches!(k.choice, Some(physical::Choice::Lazy(_))))
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
        entries.extend(quote!(#returned head = ::cq_storage::HashSet<_>;));
    }
    // Record the storage decision, including its row projection, before any
    // realization. Readers on one key share a single filtered build.
    let mut selected: structures::Structures = syn::parse2(quote!(structures { #entries }))?;
    let required = structures::required_columns(source, &analysis);
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        let direct_union = matches!(&definition.operator,Operator::Union(union) if union.origin.as_ref().is_none_or(|origin|origin.over.is_none()));
        if analysis.modes[&name] != ModeKind::Let
            || analysis.scanned.contains(&name) && !direct_union
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
        let select = match &definition.operator {
            Operator::Point(_, select) | Operator::Range(_, select)
                if built.len() == 1
                    && matches!(&selected.entries[built[0]].structure,structures::Structure::Built {keyed,..} if keyed.levels.len()==1) =>
            {
                Some(Some(select))
            }
            Operator::Union(_) => Some(None),
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
                    .map(|select| structures::selection_constants(select))
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
                        payload_provider(&mut keyed.ty);
                    }
                    keyed.filtered = Some(projection.clone());
                    // The recorded projection contains no witness-dependent
                    // payload. Integral borrowed keys have an exact dense codec.
                    let stored = key
                        .concat()
                        .into_iter()
                        .filter(|column| !constants.contains_key(column))
                        .collect::<Vec<_>>();
                    if key.len() == 1
                        && stored.len() == 1
                        && required[&name]
                            .iter()
                            .all(|column| key[0].contains(column) || constants.contains_key(column))
                        && analysis.rows[&name].computed.is_empty()
                        && matches!(structures::ungroup_type(&analysis.rows[&name].types[&stored[0]]), syn::Type::Path(p) if p.path.is_ident("i64") || p.path.is_ident("i32"))
                        && analysis
                            .keyed_reads
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .all(|(_, columns, _)| columns == &key[0])
                        && analysis
                            .key_reads
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .all(|(reader, _)| match operators[reader] {
                                Operator::Probe(_, read)
                                | Operator::Semi(_, read)
                                | Operator::Anti(_, read) => !analysis.rows
                                    [&symbol_name(&read.left)]
                                    .computed
                                    .iter()
                                    .any(|column| key[0].contains(column)),
                                _ => false,
                            })
                    {
                        keyed.membership = Some(Default::default());
                        keyed.ty = syn::parse_quote!(::cq_storage::AdaptiveKeySet<_, ::cq_storage::BorrowedI64TupleCodec, 16777216>);
                    }
                }
            }
        }
        if built.len()>1 && !built.iter().all(|i|matches!(&selected.entries[*i].structure,structures::Structure::Built {keyed,..} if keyed.filtered.is_some())) { continue; }
        if analysis.scanned.contains(&name) {
            continue;
        }
        selected.entries.retain(|entry| {
            entry.name != definition.name
                || !matches!(entry.structure, structures::Structure::Rows { .. })
        });
    }
    let target = structures::Program {
        physical: source.clone(),
        structures: selected,
    };
    check(source, &target)?;
    Ok(target)
}

mod correspondence;
pub use correspondence::check;

pub use structures::check_rewrite;

mod positional;

/// Default selection with explicit positional host capabilities. Arguments are
/// `positional(R, S)`; omitted hosts retain their original representations. The
/// generated marker/PositionalRows/BlockRead obligations check each assertion.
pub fn compile_with(
    source: &physical::Program,
    arguments: TokenStream,
) -> syn::Result<structures::Program> {
    use syn::parse::Parser;
    if arguments.is_empty() {
        return compile(source);
    }
    let parser = |input: syn::parse::ParseStream<'_>| {
        let keyword: syn::Ident = input.parse()?;
        if keyword != "positional" {
            return Err(syn::Error::new_spanned(
                keyword,
                "expected positional(host, ...)",
            ));
        }
        let body;
        syn::parenthesized!(body in input);
        let names = structures::CommaList::<syn::Ident>::parse_terminated(&body)?;
        if !input.is_empty() {
            return Err(input.error("unexpected selection arguments"));
        }
        Ok(names.iter().map(symbol_name).collect::<BTreeSet<_>>())
    };
    let hosts = parser.parse2(arguments)?;
    let mut target = compile(source)?;
    positional::choose(&mut target, &hosts)?;
    check(source, &target)?;
    Ok(target)
}
