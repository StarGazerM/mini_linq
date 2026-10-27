//! Sorted index layouts: which sorted indexes serve a built result's keyed
//! reads, and in what column order.
//!
//! Formal rule EXT-SortedIndexes: for a built `let` result whose stream reads
//! are probes or semijoins on integral keys, record a minimum set of
//! lexicographic layouts such that every read key is exactly a level prefix
//! of one layout (a minimum chain cover of the read key sets). Each chain's
//! successive differences become singleton levels, ordered by when their
//! variable is first bound in the reading stream (ties: the result's column
//! order), as SRDatalog orders an access pattern by the join's variables.
//! An explicit `Rel => [a][b]...` choice replaces the default layout of that
//! base relation or result; it must still serve every read. Physical is
//! unchanged; only built providers' levels and default types change, so
//! `sorted_runs` / `prefix_runs` then record the sorted representations.
//! Which layout is canonical for dedup or maintenance is not decided here.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_structures::{self as ir, Structure, physical::Operator, symbol_name};
use cq_structure_selection::arguments::sorted_key;
use proc_macro2::TokenStream;
use syn::{Ident, Token, Type};

/// Minimum chain cover of `demands` (Dilworth via bipartite matching): a
/// fewest set of chains `K1 ⊂ K2 ⊂ …`, each returned as its successive
/// differences (levels, each sorted). Every demand is the union of a level
/// prefix of exactly one chain. A demand for which `extendable` is false
/// ends its chain (e.g. a payload probe, which must read all built levels).
pub fn chain_cover<T: Ord + Clone>(
    demands: &BTreeSet<Vec<T>>,
    extendable: impl Fn(&Vec<T>) -> bool,
) -> Vec<Vec<Vec<T>>> {
    let sets: Vec<&Vec<T>> = demands.iter().collect();
    let open: Vec<bool> = sets.iter().map(|set| extendable(set)).collect();
    fn augment<T: Ord>(
        left: usize,
        sets: &[&Vec<T>],
        seen: &mut [bool],
        matched: &mut [Option<usize>],
    ) -> bool {
        for right in 0..sets.len() {
            if !seen[right]
                && sets[left].len() < sets[right].len()
                && sets[left].iter().all(|key| sets[right].contains(key))
            {
                seen[right] = true;
                if matched[right].is_none() || augment(matched[right].unwrap(), sets, seen, matched)
                {
                    matched[right] = Some(left);
                    return true;
                }
            }
        }
        false
    }
    let mut matched = vec![None; sets.len()];
    for (left, _) in open.iter().enumerate().filter(|(_, open)| **open) {
        augment(left, &sets, &mut vec![false; sets.len()], &mut matched);
    }
    let mut successor = vec![None; sets.len()];
    for (right, left) in matched.iter().enumerate() {
        if let Some(left) = left {
            successor[*left] = Some(right);
        }
    }
    let mut chains = Vec::new();
    for start in 0..sets.len() {
        if matched[start].is_some() {
            continue;
        }
        let mut levels = vec![sets[start].clone()];
        let mut current = start;
        while let Some(next) = successor[current] {
            levels.push(
                sets[next]
                    .iter()
                    .filter(|key| !sets[current].contains(key))
                    .cloned()
                    .collect(),
            );
            current = next;
        }
        chains.push(levels);
    }
    chains
}

/// Independent check of a cover: levels nonempty, sorted and disjoint within
/// a chain, and the level prefixes of all chains are exactly `demands`.
pub fn check_chain_cover<T: Ord + Clone>(
    demands: &BTreeSet<Vec<T>>,
    chains: &[Vec<Vec<T>>],
) -> Result<(), &'static str> {
    let mut prefixes = BTreeSet::new();
    for chain in chains {
        let mut prefix = BTreeSet::new();
        for level in chain {
            if level.is_empty()
                || !level.windows(2).all(|pair| pair[0] < pair[1])
                || level.iter().any(|key| !prefix.insert(key.clone()))
                || !prefixes.insert(prefix.iter().cloned().collect::<Vec<_>>())
            {
                return Err(
                    "chain levels must be nonempty, sorted, disjoint and cover each demand exactly once",
                );
            }
        }
    }
    if &prefixes != demands {
        return Err("chain prefixes differ from the demanded keys");
    }
    Ok(())
}

/// `Rel => [a][b]…`: an explicit layout for a base relation or result.
struct Choice {
    build: Ident,
    levels: Vec<ir::Level>,
}

impl syn::parse::Parse for Choice {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let build = input.parse()?;
        input.parse::<Token![=>]>()?;
        let mut levels = vec![input.parse()?];
        while input.peek(syn::token::Bracket) {
            levels.push(input.parse()?);
        }
        Ok(Self { build, levels })
    }
}

fn default_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && {
        let path: Vec<_> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
        path == ["cq_storage", "HashIndex"] || path == ["cq_storage", "HashSet"] || path == ["cq_storage", "Trie"]
    })
}

/// Index of the definition at which `column` of `result` is first bound,
/// following each operator's stream input (its first operand).
fn birth(
    program: &ir::physical::Program,
    headings: &BTreeMap<String, Vec<String>>,
    result: &str,
    column: &str,
) -> usize {
    let definitions = &program.program.definitions;
    let index = definitions
        .iter()
        .position(|d| symbol_name(&d.name) == result)
        .expect("checked result");
    let operands = definitions[index].operator.operands();
    match operands.first() {
        Some(input)
            if headings
                .get(&symbol_name(input))
                .is_some_and(|h| h.iter().any(|c| c == column)) =>
        {
            birth(program, headings, &symbol_name(input), column)
        }
        _ => index,
    }
}

pub fn sorted_indexes(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let choices: Vec<Choice> = if arguments.is_empty() {
        Vec::new()
    } else {
        syn::parse::Parser::parse2(ir::CommaList::<Choice>::parse_terminated, arguments)?
            .into_iter()
            .collect()
    };
    let facts = ir::contract::check(source)?;
    let physical = &facts.physical;
    let operators: BTreeMap<String, &Operator> = source
        .physical
        .program
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), &d.operator))
        .collect();
    let candidates = ir::physical::candidate_prefixes(&source.physical, physical);
    let mut used = vec![false; choices.len()];
    let mut target = source.clone();
    let names: BTreeSet<String> = target
        .structures
        .entries
        .iter()
        .filter(|e| matches!(&e.structure, Structure::Built { .. }))
        .map(|e| symbol_name(&e.name))
        .collect();
    for name in names {
        let built: Vec<usize> = (0..target.structures.entries.len())
            .filter(|&i| {
                symbol_name(&target.structures.entries[i].name) == name
                    && matches!(
                        &target.structures.entries[i].structure,
                        Structure::Built { .. }
                    )
            })
            .collect();
        let eligible = built
            .iter()
            .all(|&i| match &target.structures.entries[i].structure {
                Structure::Built { keyed, .. } => {
                    keyed.membership.is_none() && default_type(&keyed.ty)
                }
                _ => false,
            });
        let reads: Vec<_> = physical
            .keyed_reads
            .get(&name)
            .into_iter()
            .flatten()
            .collect();
        let keyed_only = reads.iter().all(|(reader, _, supply)| {
            let operator = operators[reader.as_str()];
            *supply == ir::physical::syntax::SupplyKind::Stream
                && (matches!(operator, Operator::Probe(_, k) if matches!(k.choice, Some(ir::physical::Choice::First(_))))
                    || matches!(operator, Operator::Semi(..) | Operator::Anti(..)))
        });
        let candidate = candidates
            .iter()
            .any(|c| c.sides.iter().any(|(side, _)| *side == name));
        let integral_keys = reads.iter().all(|(_, columns, _)| {
            columns
                .iter()
                .all(|c| sorted_key(&physical.rows[&name].types[c]))
        });
        let chosen = choices.iter().position(|choice| {
            let build = symbol_name(&choice.build);
            build == name
                || facts
                    .lineage
                    .get(&name)
                    .is_some_and(|origin| origin.base == build)
        });
        if !eligible || reads.is_empty() || !keyed_only || candidate || !integral_keys {
            if let Some(choice) = chosen {
                return Err(syn::Error::new_spanned(
                    &choices[choice].build,
                    "sorted_indexes choice names no eligible built result",
                ));
            }
            continue;
        }
        let demands: BTreeSet<Vec<String>> = reads
            .iter()
            .map(|(_, columns, _)| {
                let mut columns = columns.clone();
                columns.sort();
                columns
            })
            .collect();
        let layouts: Vec<Vec<String>> = if let Some(choice) = chosen {
            used[choice] = true;
            let layout: Vec<String> = choices[choice]
                .levels
                .iter()
                .flat_map(|l| l.columns.iter().map(symbol_name))
                .collect();
            let singletons: Vec<Vec<String>> = layout.iter().map(|c| vec![c.clone()]).collect();
            if choices[choice].levels.iter().any(|l| l.columns.len() != 1)
                || layout.iter().any(|c| !physical.headings[&name].contains(c))
                || demands
                    .iter()
                    .any(|d| ir::prefix_depth(&singletons, d).is_none())
            {
                return Err(syn::Error::new_spanned(
                    &choices[choice].build,
                    "sorted_indexes choice must be singleton levels of the result's columns serving every read as a prefix",
                ));
            }
            vec![layout]
        } else {
            // A key read by a payload probe must be a whole layout.
            let probed: BTreeSet<Vec<String>> = reads
                .iter()
                .filter(|(reader, _, _)| matches!(operators[reader.as_str()], Operator::Probe(..)))
                .map(|(_, columns, _)| {
                    let mut columns = columns.clone();
                    columns.sort();
                    columns
                })
                .collect();
            let chains = chain_cover(&demands, |key| !probed.contains(key));
            check_chain_cover(&demands, &chains)
                .map_err(|detail| syn::Error::new(proc_macro2::Span::call_site(), detail))?;
            // Within a level, order columns by when the reading stream binds
            // them; the level is the difference between two read keys.
            let heading = &physical.headings[&name];
            let left_of = |key: &Vec<String>| -> Option<String> {
                reads
                    .iter()
                    .find(|(_, columns, _)| {
                        let mut columns = columns.clone();
                        columns.sort();
                        &columns == key
                    })
                    .and_then(|(reader, _, _)| {
                        operators[reader.as_str()]
                            .operands()
                            .first()
                            .map(|l| symbol_name(l))
                    })
            };
            chains
                .iter()
                .map(|chain| {
                    let mut prefix: Vec<String> = Vec::new();
                    let mut layout = Vec::new();
                    for level in chain {
                        prefix.extend(level.iter().cloned());
                        prefix.sort();
                        let left = left_of(&prefix);
                        let mut level = level.clone();
                        level.sort_by_key(|column| {
                            let bound = left.as_deref().map_or(usize::MAX, |l| {
                                birth(&source.physical, &physical.headings, l, column)
                            });
                            (bound, heading.iter().position(|c| c == column))
                        });
                        layout.extend(level);
                    }
                    layout
                })
                .collect()
        };
        let mut layouts = layouts;
        layouts.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
        let template = target.structures.entries[built[0]].clone();
        let mut replacements = Vec::new();
        for layout in &layouts {
            // A layout read in full by a probe carries the payload; one read
            // only for membership is a set.
            let full = {
                let mut full = layout.clone();
                full.sort();
                full
            };
            let payload = reads.iter().any(|(reader, columns, _)| {
                let mut columns = columns.clone();
                columns.sort();
                columns == full && matches!(operators[reader.as_str()], Operator::Probe(..))
            });
            let mut entry = template.clone();
            let Structure::Built { keyed, .. } = &mut entry.structure else {
                unreachable!()
            };
            let row = &physical.rows[&name];
            keyed.levels = layout
                .iter()
                .map(|c| {
                    let column = &row.bindings[c];
                    syn::parse_quote!([#column])
                })
                .collect();
            let mut ty: Type = if payload {
                syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>)
            } else {
                syn::parse_quote!(::cq_storage::HashSet<_>)
            };
            for _ in 1..layout.len() {
                ty = syn::parse_quote!(::cq_storage::Trie<'_, _, #ty>);
            }
            keyed.ty = ty;
            keyed.supports = if payload && layout.len() == 1 {
                Some(syn::parse_quote!(supports [keys]))
            } else {
                None
            };
            replacements.push(entry);
        }
        let first = built[0];
        for &i in built.iter().rev() {
            target.structures.entries.remove(i);
        }
        for (offset, entry) in replacements.into_iter().enumerate() {
            target.structures.entries.insert(first + offset, entry);
        }
    }
    if let Some(unused) = used.iter().position(|used| !used) {
        return Err(syn::Error::new_spanned(
            &choices[unused].build,
            "sorted_indexes choice matched no built result",
        ));
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}
