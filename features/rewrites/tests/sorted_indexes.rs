use cq_lang_structures::{self as ir, Structure, symbol_name};
use quote::quote;

/// `z` is bound by the root scan, `a` only by the first probe. `c` is read on
/// `[z]` (probe) and `[a, z]` (semijoin); `d` on `[a, z]` (probe). Names are
/// chosen so alphabetical order (`a` before `z`) differs from binding order.
fn program() -> ir::Program {
    let physical: ir::physical::Program = syn::parse_quote! { physical {
        relation A(z:u32); relation C(z:u32,a:u32); relation D(a:u32,z:u32,w:u32);
        x = pipe rename A@0 {z -> z};
        c = let rename C@1 {z -> z, a -> a};
        d = let rename D@2 {a -> a, z -> z, w -> w};
        p = pipe probe first [z] x, c;
        s = pipe semi eager [a, z] p, c;
        q = pipe canon probe first [a, z] s, d;
        return q as answer(z, a, w);
    } };
    cq_structure_selection::compile(&physical).unwrap()
}

fn layouts(program: &ir::Program, result: &str) -> Vec<(Vec<Vec<String>>, String)> {
    program
        .structures
        .entries
        .iter()
        .filter(|entry| symbol_name(&entry.name) == result)
        .filter_map(|entry| match &entry.structure {
            Structure::Built { keyed, .. } => Some((
                keyed.levels.iter().map(|l| l.columns.iter().map(symbol_name).collect()).collect(),
                quote::ToTokens::to_token_stream(&keyed.ty).to_string().replace(' ', ""),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn default_layouts_cover_reads_in_binding_order() {
    let source = program();
    let target = cq_rewrite_passes::sorted_indexes(&source, quote!()).unwrap();
    ir::check_rewrite(&source, &target).unwrap();
    assert_eq!(target.physical, source.physical, "Physical is unchanged");
    // A payload probe must read a whole layout, so the probe on [z] cannot
    // share the semijoin's [z][a]: two layouts, the semijoin's a set.
    assert_eq!(
        layouts(&target, "c"),
        [
            (vec![vec!["z".into()]], "::cq_storage::HashIndex<'_,_,_>".into()),
            (vec![vec!["z".into()], vec!["a".into()]], "::cq_storage::Trie<'_,_,::cq_storage::HashSet<_>>".into()),
        ]
    );
    // The composite read is ordered by binding, not alphabetically.
    assert_eq!(
        layouts(&target, "d"),
        [(vec![vec!["z".into()], vec!["a".into()]], "::cq_storage::Trie<'_,_,::cq_storage::HashIndex<'_,_,_>>".into())]
    );
    // Idempotent on its own output.
    assert_eq!(cq_rewrite_passes::sorted_indexes(&target, quote!()).unwrap(), target);
}

#[test]
fn an_explicit_layout_overrides_the_default_but_must_serve_every_read() {
    let source = program();
    let target = cq_rewrite_passes::sorted_indexes(&source, quote!(D => [a][z])).unwrap();
    assert_eq!(layouts(&target, "d")[0].0, [vec!["a".to_string()], vec!["z".to_string()]]);
    assert_eq!(layouts(&target, "c").len(), 2, "other results keep the default");
    // `c` is probed on [z]; one layout [a][z] cannot serve that read.
    assert!(cq_rewrite_passes::sorted_indexes(&source, quote!(C => [a][z])).is_err());
    // Levels must be single columns of the result.
    assert!(cq_rewrite_passes::sorted_indexes(&source, quote!(D => [a, z])).is_err());
    assert!(cq_rewrite_passes::sorted_indexes(&source, quote!(D => [a][missing])).is_err());
    // A choice naming nothing built is an error, not ignored.
    assert!(cq_rewrite_passes::sorted_indexes(&source, quote!(Nothing => [z])).is_err());
}

#[test]
fn chain_cover_is_minimum_and_checked() {
    use std::collections::BTreeSet;
    let demands: BTreeSet<Vec<u8>> = [vec![1], vec![1, 2], vec![3], vec![1, 2, 3], vec![2, 3]].into_iter().collect();
    let chains = cq_rewrite_passes::chain_cover(&demands, |_| true);
    cq_rewrite_passes::check_chain_cover(&demands, &chains).unwrap();
    // The poset's width is 2 ({1} and {3} are incomparable): two chains.
    assert_eq!(chains.len(), 2);
    // A closed demand ends its chain: {1,2} may not extend to {1,2,3}.
    let closed = cq_rewrite_passes::chain_cover(&demands, |key| key != &vec![1, 2]);
    cq_rewrite_passes::check_chain_cover(&demands, &closed).unwrap();
    assert!(closed.iter().all(|chain| {
        let mut prefix: Vec<u8> = Vec::new();
        chain.iter().enumerate().all(|(depth, level)| {
            prefix.extend(level);
            prefix.sort();
            prefix != [1, 2] || depth + 1 == chain.len()
        })
    }));
    assert!(cq_rewrite_passes::check_chain_cover(&demands, &[vec![vec![1, 2, 3]]]).is_err());
}
