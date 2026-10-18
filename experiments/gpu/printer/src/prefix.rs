//! Explicit device representation for an existing multi-level keyed build.
//!
//! Physical's complete join key and reads are unchanged. A PrefixRun records
//! its directory column as an index into the checked Structures row layout.
//! Explicit choices may reorder singleton key levels; any displaced singleton
//! prefix read receives its own recorded SortedRun. Later levels remain stable
//! residual equalities. The terminal printer consumes these provider choices.
use cq_lang_structures::{self as structures, Structure, symbol_name};
use proc_macro2::TokenStream;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token};

struct Choice {
    build: Ident,
    column: Ident,
}

impl Parse for Choice {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let build = input.parse()?;
        input.parse::<Token![=>]>()?;
        let column = input.parse()?;
        Ok(Self { build, column })
    }
}

/// Decode a recorded column, never infer one from the current key order.
/// The index addresses Structures' checked layout, not a projected GPU payload.
pub(crate) fn recorded_column<'a>(ty: &syn::Type, layout: &'a [String]) -> syn::Result<&'a str> {
    let invalid = || {
        syn::Error::new_spanned(
            ty,
            "CQ_GPU_PREFIX_COLUMN: PrefixRun requires a fourth generic argument recording an in-range integral column index",
        )
    };
    let syn::Type::Path(path) = ty else {
        return Err(invalid());
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(invalid());
    };
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(invalid());
    };
    if arguments.args.len() != 4 {
        return Err(invalid());
    }
    let Some(syn::GenericArgument::Const(syn::Expr::Lit(value))) = arguments.args.last() else {
        return Err(invalid());
    };
    let syn::Lit::Int(index) = &value.lit else {
        return Err(invalid());
    };
    layout
        .get(index.base10_parse::<usize>().map_err(|_| invalid())?)
        .map(String::as_str)
        .ok_or_else(invalid)
}

pub fn compile(
    source: &structures::Program,
    arguments: TokenStream,
) -> syn::Result<structures::Program> {
    let choice = if arguments.is_empty() {
        None
    } else {
        Some(syn::parse2::<Choice>(arguments)?)
    };
    let facts = structures::contract::check(source)?;
    let mut target = source.clone();
    let mut chosen = false;
    let mut additional = Vec::<structures::Entry>::new();
    let mut added_keys = std::collections::BTreeSet::new();
    let mut added_rows = std::collections::BTreeSet::new();
    for entry in &mut target.structures.entries {
        let Structure::Built { keyed, .. } = &mut entry.structure else {
            continue;
        };
        let name = symbol_name(&entry.name);
        let selected = choice.as_ref().is_some_and(|choice| {
            let build = symbol_name(&choice.build);
            name == build
                || facts
                    .lineage
                    .get(&name)
                    .is_some_and(|origin| origin.base == build)
        });
        if keyed.levels.len() < 2 || keyed.membership.is_some() {
            if selected {
                return Err(syn::Error::new_spanned(
                    &entry.name,
                    "prefix_runs requires a multi-level payload Trie",
                ));
            }
            continue;
        }
        let syn::Type::Path(path) = &keyed.ty else {
            if selected {
                return Err(syn::Error::new_spanned(
                    &keyed.ty,
                    "prefix_runs requires a Trie",
                ));
            }
            continue;
        };
        if !path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Trie")
        {
            if selected {
                return Err(syn::Error::new_spanned(
                    &keyed.ty,
                    "prefix_runs requires a Trie",
                ));
            }
            continue;
        }
        if keyed.columns().any(|column| {
            !matches!(&facts.physical.rows[&name].types[&symbol_name(column)], syn::Type::Path(path)
                if path.path.is_ident("i32") || path.path.is_ident("i64"))
        }) {
            if selected {
                return Err(syn::Error::new_spanned(
                    &entry.name,
                    "prefix_runs requires integral key columns",
                ));
            }
            continue;
        }
        if selected {
            let choice = choice.as_ref().expect("selected explicit choice");
            let column = symbol_name(&choice.column);
            let level = keyed
                .levels
                .iter()
                .position(|level| {
                    level.columns.len() == 1 && symbol_name(&level.columns[0]) == column
                })
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        &choice.column,
                        "prefix_runs choice must name a singleton key level",
                    )
                })?;
            let previous = keyed.level_names();
            // Keep every Physical read: a displaced singleton prefix gets an
            // independent ordinary run, while full-key reads use this directory.
            keyed.levels[..=level].rotate_right(1);
            let reordered = keyed.level_names();
            for (_, keys, supply) in facts.physical.keyed_reads.get(&name).into_iter().flatten() {
                if *supply != structures::physical::syntax::SupplyKind::Stream
                    || structures::prefix_depth(&previous, keys).is_none()
                    || structures::prefix_depth(&reordered, keys).is_some()
                    || facts.built[&name]
                        .iter()
                        .any(|(levels, _)| levels.len() == 1 && levels[0] == *keys)
                {
                    continue;
                }
                if keys.len() != 1 {
                    return Err(syn::Error::new_spanned(
                        &choice.column,
                        "prefix_runs cannot replace a displaced composite-prefix read with a single-column run",
                    ));
                }
                if added_keys.insert((name.clone(), keys.clone())) {
                    let build = &entry.name;
                    // Multiple providers share one explicitly retained row source.
                    if !facts.retained.contains_key(&name) && added_rows.insert(name.clone()) {
                        additional.push(syn::parse_quote! {
                            #build rows = ::std::vec::Vec<_>;
                        });
                    }
                    let key = &facts.physical.rows[&name].bindings[&keys[0]];
                    additional.push(syn::parse_quote! {
                        #build = ::cq_storage::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec> [#key];
                    });
                }
            }
            chosen = true;
        }
        if keyed.levels[0].columns.len() != 1 {
            continue;
        }
        let column = symbol_name(&keyed.levels[0].columns[0]);
        let index = syn::Index::from(
            facts.layouts[&name]
                .iter()
                .position(|name| name == &column)
                .expect("checked key column"),
        );
        keyed.ty = syn::parse_quote!(::cq_gpu_runtime::PrefixRun<_, _, ::cq_storage::BorrowedI64TupleCodec, #index>);
    }
    if let Some(choice) = choice.filter(|_| !chosen) {
        return Err(syn::Error::new_spanned(
            choice.build,
            "prefix_runs choice matched no multi-level build or base relation",
        ));
    }
    target.structures.entries.extend(additional);
    structures::check_rewrite(source, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::{ToTokens, quote};

    fn source() -> structures::Program {
        syn::parse_quote! {
            physical {
                relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
                l = pipe rename L@0 {a -> x,b -> y};
                r = let rename R@1 {a -> x,b -> y,c -> z};
                p = pipe canon probe first [x,y] l,r;
                return p as answer(x,y,z);
            }
            structures {
                l host scan = ::core::iter::Map<_,_> [];
                r = ::cq_storage::Trie<'_,_,::cq_storage::HashIndex<'_,_,_>> [x][y];
                r host scan = ::core::iter::Map<_,_> [];
                p canon = ::cq_gpu_runtime::DeviceSet<_>;
            }
        }
    }

    #[test]
    fn preserves_all_key_levels_and_physical_reads() {
        let source = source();
        let target = compile(&source, TokenStream::new()).unwrap();
        assert_eq!(target.physical, source.physical);
        let Structure::Built { keyed, .. } = &target.structures.entries[1].structure else {
            panic!("built entry");
        };
        assert_eq!(
            keyed.level_names(),
            vec![vec!["x".to_owned()], vec!["y".to_owned()]]
        );
        let facts = structures::contract::check(&target).unwrap();
        assert_eq!(
            recorded_column(&keyed.ty, &facts.layouts["r"]).unwrap(),
            "x"
        );
    }

    #[test]
    fn explicit_relation_choice_records_and_uses_the_selected_directory_column() {
        let source = source();
        let target = compile(&source, quote!(R => y)).unwrap();
        assert_eq!(target.physical, source.physical);
        assert_eq!(
            target.physical.to_token_stream().to_string(),
            source.physical.to_token_stream().to_string(),
        );
        let Structure::Built { keyed, .. } = &target.structures.entries[1].structure else {
            panic!("built entry");
        };
        assert_eq!(
            keyed.level_names(),
            vec![vec!["y".to_owned()], vec!["x".to_owned()]],
        );
        let facts = structures::contract::check(&target).unwrap();
        assert_eq!(
            recorded_column(&keyed.ty, &facts.layouts["r"]).unwrap(),
            "y"
        );
        let reads = crate::compiler::validate(&target, &facts).unwrap();
        assert_eq!(reads[0].keys, ["x", "y"]);
        assert_eq!(reads[0].source, "rank-point+residual");
        let generated = crate::compiler::compile(&target, crate::options::Options::default())
            .unwrap()
            .to_token_stream()
            .to_string();
        let residual = quote!(
            ::cq_gpu_runtime::run_column(__cq_gpu_run_view_r, 0usize, __cq_gpu_row_p)
                == ::cq_gpu_runtime::column(__driver, 0usize, __row)
        )
        .to_string();
        assert!(generated.contains(&residual));
    }

    #[test]
    fn explicit_result_choice_is_supported() {
        let target = compile(&source(), quote!(r => y)).unwrap();
        let facts = structures::contract::check(&target).unwrap();
        assert_eq!(
            recorded_column(&facts.built["r"][0].1, &facts.layouts["r"]).unwrap(),
            "y"
        );
    }

    #[test]
    fn rejects_unknown_build_and_nonkey_choice() {
        for (arguments, expected) in [
            (quote!(Missing => x), "matched no multi-level build"),
            (quote!(R => z), "singleton key level"),
            (quote!(R => missing), "singleton key level"),
        ] {
            assert!(
                compile(&source(), arguments)
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        }
    }

    #[test]
    fn explicit_choice_cannot_discard_an_existing_prefix_read() {
        let source: structures::Program = syn::parse_quote! {
            physical {
                relation L(a:i64); relation R(a:i64,b:i64);
                l = pipe rename L@0 {a -> x};
                r = let rename R@1 {a -> x,b -> y};
                p = pipe canon semi eager [x] l,r;
                return p as answer(x);
            }
            structures {
                l host scan = ::core::iter::Map<_,_> [];
                r = ::cq_storage::Trie<'_,_,::cq_storage::HashIndex<'_,_,_>> [x][y];
                r host scan = ::core::iter::Map<_,_> [];
                p canon = ::cq_gpu_runtime::DeviceSet<_>;
            }
        };
        structures::contract::check(&source).unwrap();
        assert!(compile(&source, quote!(R => y)).is_err());
    }

    #[test]
    fn printer_rejects_recorded_column_outside_the_directory_level() {
        let mut target = compile(&source(), quote!(R => y)).unwrap();
        let Structure::Built { keyed, .. } = &mut target.structures.entries[1].structure else {
            panic!("built entry");
        };
        keyed.ty = syn::parse_quote!(
            ::cq_gpu_runtime::PrefixRun<_, _, ::cq_storage::BorrowedI64TupleCodec, 0>
        );
        let facts = structures::contract::check(&target).unwrap();
        assert!(
            crate::compiler::validate(&target, &facts)
                .unwrap_err()
                .to_string()
                .contains("CQ_GPU_PREFIX_LEVEL")
        );
    }

    #[test]
    fn rejects_missing_and_out_of_range_recorded_columns() {
        let layout = vec!["x".to_owned(), "y".to_owned()];
        for ty in [
            syn::parse_quote!(::cq_gpu_runtime::PrefixRun<_, _, ::cq_storage::BorrowedI64TupleCodec>),
            syn::parse_quote!(::cq_gpu_runtime::PrefixRun<_, _, ::cq_storage::BorrowedI64TupleCodec, 2>),
        ] {
            assert!(
                recorded_column(&ty, &layout)
                    .unwrap_err()
                    .to_string()
                    .contains("CQ_GPU_PREFIX_COLUMN")
            );
        }
    }

    #[test]
    fn explicit_choice_records_a_separate_run_for_displaced_prefix_reads() {
        let source: structures::Program = syn::parse_quote! {
            physical {
                relation L(a:i64); relation R(a:i64,b:i64); relation M(a:i64,b:i64);
                l = pipe rename L@0 {a -> x};
                r = let rename R@1 {a -> x,b -> y};
                m = let rename M@2 {a -> x,b -> y};
                s = pipe semi eager [x] l,r;
                t = pipe probe first [x] s,m;
                p = pipe canon probe first [x,y] t,r;
                return p as answer(x,y);
            }
            structures {
                l host scan = ::core::iter::Map<_,_> [];
                r = ::cq_storage::Trie<'_,_,::cq_storage::HashIndex<'_,_,_>> [x][y];
                r host scan = ::core::iter::Map<_,_> [];
                m = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
                m host scan = ::core::iter::Map<_,_> [];
                p canon = ::cq_gpu_runtime::DeviceSet<_>;
            }
        };
        let target = compile(&source, quote!(R => y)).unwrap();
        assert_eq!(target.physical, source.physical);
        assert_eq!(
            target.physical.to_token_stream().to_string(),
            source.physical.to_token_stream().to_string(),
        );
        let facts = structures::contract::check(&target).unwrap();
        assert_eq!(facts.built["r"].len(), 2);
        assert_eq!(facts.retained["r"], syn::parse_quote!(::std::vec::Vec<_>));
        let (levels, ty) = &facts.built["r"][0];
        assert_eq!(levels, &vec![vec!["y".to_owned()], vec!["x".to_owned()]]);
        assert_eq!(recorded_column(ty, &facts.layouts["r"]).unwrap(), "y");
        let (levels, ty) = &facts.built["r"][1];
        assert_eq!(levels, &vec![vec!["x".to_owned()]]);
        assert_eq!(
            ty,
            &syn::parse_quote!(
                ::cq_storage::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>
            )
        );
        let reads = crate::compiler::validate(&target, &facts).unwrap();
        let semi = reads.iter().find(|read| read.reader == "s").unwrap();
        assert_eq!(semi.keys, ["x"]);
        assert_eq!(semi.source, "rank-point");
        let probe = reads.iter().find(|read| read.reader == "p").unwrap();
        assert_eq!(probe.keys, ["x", "y"]);
        assert_eq!(probe.source, "rank-point+residual");
        let generated = crate::compiler::compile(&target, crate::options::Options::default())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert_eq!(generated.matches("prepare_run").count(), 3);
        let residual = quote!(
            ::cq_gpu_runtime::run_column(__cq_gpu_run_view_r, 0usize, __cq_gpu_row_p)
                == ::cq_gpu_runtime::column(__driver, 0usize, __row)
        )
        .to_string();
        assert!(generated.contains(&residual));
        assert!(generated.contains("__cq_gpu_run_view_r_1 . rank"));
    }
}
