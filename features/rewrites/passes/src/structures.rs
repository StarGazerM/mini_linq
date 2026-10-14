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
            if (at == "result" || at == "kind") && input.peek(syn::token::Paren) {
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
pub fn retype(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let analysis = ir::contract::check(source)?;
    let request: Retype = syn::parse2(arguments)?;
    ir::concrete_type(&request.ty)?;
    let mut target = source.clone();
    let mut changed = false;
    for entry in &mut target.structures.entries {
        let name = symbol_name(&entry.name);
        let matches = match request.selector {
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
                Structure::Selection { .. } => matches!(kind, Kind::Selection),
                Structure::KeySet { .. } => matches!(kind, Kind::KeySet | Kind::Membership),
                Structure::Read { .. } => false,
                Structure::JoinFilter { .. } => matches!(kind, Kind::JoinFilter),
                Structure::Host { reader, .. } => {
                    matches!(kind, Kind::Membership) && reader == "membership"
                        || source.physical.program.definitions.iter().any(|d| {
                            symbol_name(&d.name) == symbol_name(reader) && kind.reads(&d.operator)
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
                                        symbol_name(&d.name) == *reader && kind.reads(&d.operator)
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
                | Structure::Selection { ty, .. }
                | Structure::KeySet { ty, .. } => *ty = request.ty.clone(),
                Structure::JoinFilter { filter, .. } => filter.ty = request.ty.clone(),
                Structure::Read { .. } => unreachable!("read is not a provider type"),
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
            reader == "membership"
                || reader == "candidates"
                || source.physical.program.definitions.iter().any(|d| {
                    symbol_name(&d.name) == symbol_name(reader)
                        && matches!(
                            d.operator,
                            Operator::Probe(..) | Operator::Semi(..) | Operator::Merge(..)
                        )
                })
        }
        _ => false,
    }
}

/// Add a covered scalar filter to the named scan occurrence. The default type
/// is exact; a later `retype(scan join_filter => T)` chooses its representation.
/// This is explicit positional-host advice, including when selection was not
/// given positional hosts. The same Structures contract checks the result.
pub fn join_filter(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let scan: Ident = syn::parse2(arguments)?;
    let facts = ir::contract::check(source)?;
    if facts.join_filters.contains_key(&symbol_name(&scan)) {
        return Ok(source.clone());
    }
    let mut candidates = source.physical.program.definitions.iter().filter_map(|d| {
        let (candidate, build, key, _) =
            ir::join_filter_candidate(source, &facts, &symbol_name(&d.name))?;
        (candidate == symbol_name(&scan)).then_some((&d.name, build, key))
    });
    let Some((covering, build, key)) = candidates.next() else {
        return Err(syn::Error::new_spanned(
            scan,
            "scan has no admissible covering exact probe",
        ));
    };
    if candidates.next().is_some() {
        return Err(syn::Error::new_spanned(
            scan,
            "scan has ambiguous covering probes",
        ));
    }
    let origin = &facts.lineage[&symbol_name(&scan)];
    let index = syn::Index::from(
        facts.host_layouts[&origin.base]
            .iter()
            .position(|c| c == &origin.columns[&key])
            .expect("checked lineage"),
    );
    let key = &facts.physical.rows[&build].bindings[&key];
    let build = source
        .physical
        .program
        .definitions
        .iter()
        .find(|d| symbol_name(&d.name) == build)
        .map(|d| &d.name)
        .expect("checked build");
    let mut target = source.clone();
    target.structures.entries.push(syn::parse_quote! {
        #scan join_filter = ::cq_storage::AdaptiveKeySet<i64, ::cq_storage::I64Codec, 67108864>
            build #build [#key] covering #covering marker ::cq_load::ColumnAt<#index>;
    });
    ir::check_rewrite(source, &target)?;
    Ok(target)
}
