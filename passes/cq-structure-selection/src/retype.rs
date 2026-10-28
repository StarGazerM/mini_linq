use cq_lang_structures::{self as ir, Structure, physical::Operator, symbol_name};
use proc_macro2::TokenStream;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token, Type};

#[derive(Clone, Copy)]
enum Kind {
    Payload,
    Membership,
    Ordered,
    Trie,
    Dedup,
    Rows,
    Selection,
    KeySet,
    JoinFilter,
    Group,
}

impl Kind {
    fn named(name: &Ident) -> Option<Self> {
        match name.to_string().as_str() {
            "payload" => Some(Self::Payload),
            "membership" => Some(Self::Membership),
            "ordered" => Some(Self::Ordered),
            "trie" => Some(Self::Trie),
            "dedup" => Some(Self::Dedup),
            "rows" => Some(Self::Rows),
            "selection" => Some(Self::Selection),
            "keyset" => Some(Self::KeySet),
            "join_filter" => Some(Self::JoinFilter),
            "group" => Some(Self::Group),
            _ => None,
        }
    }

    fn reads(self, operator: &Operator) -> bool {
        matches!(
            (self, operator),
            (Self::Payload, Operator::Probe(..))
                | (Self::Membership, Operator::Semi(..))
                | (Self::Ordered, Operator::Merge(..))
        )
    }
}

enum Selector {
    Entry(usize),
    Result(Ident),
    JoinFilter(Ident),
    Kind(Kind),
}

struct Retype {
    selector: Selector,
    at: Ident,
    ty: Type,
}

impl Parse for Retype {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let at: Ident = input.parse()?;
        let selector =
            if at == "entry" && input.peek(syn::token::Paren) {
                let contents;
                syn::parenthesized!(contents in input);
                let index: syn::LitInt = contents.parse()?;
                if !contents.is_empty() {
                    return Err(contents.error("expected one entry index"));
                }
                Selector::Entry(index.base10_parse()?)
            } else if (at == "result" || at == "kind") && input.peek(syn::token::Paren) {
                let contents;
                syn::parenthesized!(contents in input);
                let name: Ident = contents.parse()?;
                if !contents.is_empty() {
                    return Err(contents.error("expected one selector name"));
                }
                if at == "result" {
                    Selector::Result(name)
                } else {
                    Selector::Kind(Kind::named(&name).ok_or_else(|| {
                        syn::Error::new_spanned(name, "unknown representation kind")
                    })?)
                }
            } else {
                Kind::named(&at).map_or_else(|| Selector::Result(at.clone()), Selector::Kind)
            };
        let selector = if input.peek(Ident) {
            let kind: Ident = input.parse()?;
            match selector {
                Selector::Result(result) if kind == "join_filter" => Selector::JoinFilter(result),
                _ => {
                    return Err(syn::Error::new_spanned(
                        kind,
                        "expected result join_filter => Type",
                    ));
                }
            }
        } else {
            selector
        };
        input.parse::<Token![=>]>()?;
        let ty = input.parse()?;
        Ok(Self { selector, at, ty })
    }
}

/// Retype keyed representations on a named result, or entries serving the
/// selected read kind. Scan/point/range adapters are not keyed representations.
/// Levels, reader names and Physical are preserved, even for custom types.
/// Dedup includes both canon and computed-head state; rows selects buffers.
/// `entry(N)` selects exactly one provider-bearing entry by its zero-based
/// Structures order, including host adapters, without touching sibling entries.
pub fn retype(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let analysis = ir::contract::check(source)?;
    let request: Retype = syn::parse2(arguments)?;
    ir::concrete_type(&request.ty)?;
    let mut target = source.clone();
    let mut changed = false;
    for (index, entry) in target.structures.entries.iter_mut().enumerate() {
        let name = symbol_name(&entry.name);
        let matches =
            match request.selector {
                Selector::Entry(selected) => {
                    index == selected
                        && !matches!(
                            entry.structure,
                            Structure::Read { .. }
                                | Structure::Repr { .. }
                                | Structure::Stage { .. }
                                | Structure::Share { .. }
                        )
                }
                Selector::Result(ref result) => {
                    name == symbol_name(result) && keyed_read(&entry.structure, source)
                }
                Selector::JoinFilter(ref result) => {
                    name == symbol_name(result)
                        && matches!(entry.structure, Structure::JoinFilter { .. })
                }
                Selector::Kind(kind) => match &entry.structure {
                    Structure::Dedup { .. } | Structure::HeadDedup { .. } => {
                        matches!(kind, Kind::Dedup)
                    }
                    Structure::Rows { .. } => matches!(kind, Kind::Rows),
                    Structure::Group { .. } => matches!(kind, Kind::Group),
                    Structure::Selection { .. } => matches!(kind, Kind::Selection),
                    Structure::KeySet { .. } => matches!(kind, Kind::KeySet | Kind::Membership),
                    Structure::Read { .. }
                    | Structure::Repr { .. }
                    | Structure::Stage { .. }
                    | Structure::Share { .. } => false,
                    Structure::JoinFilter { .. } => matches!(kind, Kind::JoinFilter),
                    Structure::Host { reader, .. } => {
                        matches!(kind, Kind::Membership)
                            && matches!(reader, cq_lang_structures::ReaderRef::Membership)
                            || source.physical.program.definitions.iter().any(|d| {
                                reader.result() == Some(&d.name) && kind.reads(&d.operator)
                            })
                    }
                    Structure::Built { keyed, .. } => {
                        matches!(kind, Kind::Trie) && keyed.levels.len() > 1
                            || analysis
                                .physical
                                .keyed_reads
                                .get(&name)
                                .into_iter()
                                .flatten()
                                .any(|(reader, columns, _)| {
                                    if matches!(kind, Kind::Membership)
                                        && analysis
                                            .keysets
                                            .contains_key(&(name.clone(), columns.clone()))
                                    {
                                        return false;
                                    }
                                    ir::prefix_depth(&keyed.level_names(), columns).is_some()
                                        && source.physical.program.definitions.iter().any(|d| {
                                            symbol_name(&d.name) == *reader
                                                && kind.reads(&d.operator)
                                        })
                                })
                    }
                },
            };
        if matches {
            match &mut entry.structure {
                Structure::Built { keyed, .. } | Structure::Host { keyed, .. } => {
                    keyed.ty = request.ty.clone();
                }
                Structure::Dedup { ty, .. }
                | Structure::HeadDedup { ty, .. }
                | Structure::Rows { ty, .. }
                | Structure::Group { ty, .. }
                | Structure::Selection { ty, .. }
                | Structure::KeySet { ty, .. } => *ty = request.ty.clone(),
                Structure::JoinFilter { filter, .. } => filter.ty = request.ty.clone(),
                Structure::Read { .. }
                | Structure::Repr { .. }
                | Structure::Stage { .. }
                | Structure::Share { .. } => {
                    unreachable!("read, repr, stage and share entries carry no provider type")
                }
            }
            changed = true;
        }
    }
    if !changed {
        return Err(syn::Error::new_spanned(
            request.at,
            "selector matches no recorded representation",
        ));
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

fn keyed_read(structure: &Structure, source: &ir::Program) -> bool {
    match structure {
        Structure::Built { .. } | Structure::KeySet { .. } => true,
        Structure::Host { reader, .. } => {
            matches!(reader, cq_lang_structures::ReaderRef::Membership)
                || matches!(reader, cq_lang_structures::ReaderRef::Candidates)
                || source.physical.program.definitions.iter().any(|d| {
                    reader.result() == Some(&d.name)
                        && matches!(
                            d.operator,
                            Operator::Probe(..) | Operator::Semi(..) | Operator::Merge(..)
                        )
                })
        }
        _ => false,
    }
}
