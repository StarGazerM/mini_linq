//! Explicit device representation for an existing multi-level keyed build.
//!
//! Physical's complete join key and Structures' key levels are unchanged. A
//! PrefixRun clusters the first integral key and realizes later levels by
//! stable residual equality over that range; this is a representation choice,
//! not a planner decision in the terminal printer.
use cq_lang_structures::{self as structures, Structure, symbol_name};
use proc_macro2::TokenStream;

pub fn compile(
    source: &structures::Program,
    arguments: TokenStream,
) -> syn::Result<structures::Program> {
    if !arguments.is_empty() {
        return Err(syn::Error::new_spanned(
            arguments,
            "cq_gpu::prefix_runs accepts no arguments",
        ));
    }
    let facts = structures::contract::check(source)?;
    let mut target = source.clone();
    for entry in &mut target.structures.entries {
        let Structure::Built { keyed, .. } = &mut entry.structure else {
            continue;
        };
        if keyed.levels.len() < 2
            || keyed.levels[0].columns.len() != 1
            || keyed.membership.is_some()
        {
            continue;
        }
        let syn::Type::Path(path) = &keyed.ty else {
            continue;
        };
        if !path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Trie")
        {
            continue;
        }
        let name = symbol_name(&entry.name);
        if keyed.columns().any(|column| {
            !matches!(&facts.physical.rows[&name].types[&symbol_name(column)], syn::Type::Path(path)
                if path.path.is_ident("i32") || path.path.is_ident("i64"))
        }) {
            continue;
        }
        keyed.ty = syn::parse_quote!(::cq_gpu_runtime::PrefixRun<_, _, ::cq_storage::BorrowedI64TupleCodec>);
    }
    structures::check_rewrite(source, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_all_key_levels_and_physical_reads() {
        let source: structures::Program = syn::parse_quote! {
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
                p canon = ::cq_storage::HashSet<_>;
            }
        };
        let target = compile(&source, TokenStream::new()).unwrap();
        assert_eq!(target.physical, source.physical);
        let Structure::Built { keyed, .. } = &target.structures.entries[1].structure else {
            panic!("built entry");
        };
        assert_eq!(
            keyed.level_names(),
            vec![vec!["x".to_owned()], vec!["y".to_owned()]]
        );
        assert!(
            matches!(&keyed.ty, syn::Type::Path(path) if path.path.segments.last().unwrap().ident == "PrefixRun")
        );
    }
}
