use cq_lang_structures::{self as ir, Structure, physical};
use cq_rewrite_passes::{join_filter, retype};
use quote::{ToTokens, quote};

fn source() -> ir::Program {
    let p: physical::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        s = pipe range select r if (z > 1);
        b = let project s keep {x,z};
        j = pipe probe first [x] l,b;
        out = pipe canon project j keep {x,y,z}; return out as answer(x,y,z);
    }};
    cq_structure_selection::compile(&p).unwrap()
}

#[test]
fn advisory_insertion_and_retyping_preserve_every_other_decision() {
    let before = source();
    assert!(
        ir::contract::check(&before)
            .unwrap()
            .join_filters
            .is_empty()
    );
    let added = join_filter(&before, quote!(l)).unwrap();
    assert_eq!(added.physical, before.physical);
    assert_eq!(join_filter(&added, quote!(l)).unwrap(), added);
    for args in [
        quote!(l join_filter => FilterAlias),
        quote!(join_filter => FilterAlias),
    ] {
        let changed = retype(&added, args).unwrap();
        assert_eq!(changed.physical, before.physical);
        let facts = ir::contract::check(&changed).unwrap();
        assert_eq!(facts.join_filters["l"].len(), 1);
        assert_eq!(
            facts.join_filters["l"][0].ty,
            syn::parse_quote!(FilterAlias)
        );
        assert_eq!(facts.join_filters["l"][0].covering, "j");
        assert_eq!(
            syn::parse2::<ir::Program>(changed.to_token_stream()).unwrap(),
            changed
        );
        assert_eq!(
            &changed.structures.entries[..before.structures.entries.len()],
            &before.structures.entries[..]
        );
    }
    assert!(retype(&before, quote!(l join_filter => FilterAlias)).is_err());
    assert!(retype(&added, quote!(b join_filter => FilterAlias)).is_err());
    assert!(join_filter(&before, quote!(r)).is_err());
}

#[test]
fn contract_rejects_missing_cover_rust_between_and_anti() {
    let good = join_filter(&source(), quote!(l)).unwrap();
    for mutation in 0..4 {
        let mut bad = good.clone();
        match mutation {
            0 => {
                for entry in &mut bad.structures.entries {
                    if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
                        filter.covering = syn::parse_quote!(missing);
                    }
                }
            }
            1 => {
                let defs = &mut bad.physical.program.definitions;
                let i = defs.iter().position(|d| d.name == "j").unwrap();
                defs.insert(
                    i,
                    syn::parse_quote!(g = pipe guard l if (panic!("must run"));),
                );
                if let physical::Operator::Probe(_, k) = &mut defs[i + 1].operator {
                    k.left = syn::parse_quote!(g);
                }
            }
            2 => {
                let d = bad
                    .physical
                    .program
                    .definitions
                    .iter_mut()
                    .find(|d| d.name == "j")
                    .unwrap();
                d.operator = syn::parse_quote!(anti eager [x] l,b);
            }
            _ => {
                // A filter may not become the sole check of the join constraint.
                bad.physical
                    .program
                    .definitions
                    .retain(|d| d.name == "l" || d.name == "out");
            }
        }
        if mutation >= 2 {
            let out = bad
                .physical
                .program
                .definitions
                .iter_mut()
                .find(|d| d.name == "out")
                .unwrap();
            let input = if mutation == 2 { quote!(j) } else { quote!(l) };
            out.operator = syn::parse2(quote!(project #input keep {x,y})).unwrap();
            bad.physical.program.head.args = syn::parse_quote!(x, y);
        }
        physical::contract::check(&bad.physical).expect("mutation remains valid Physical");
        assert!(ir::contract::check(&bad).is_err(), "mutation {mutation}");
    }
}

#[test]
fn explicit_advice_can_cover_a_total_unfiltered_build() {
    let mut p = source().physical;
    let s = p
        .program
        .definitions
        .iter_mut()
        .find(|d| d.name == "s")
        .unwrap();
    s.operator = syn::parse_quote!(project r keep {x,z});
    let selected = cq_structure_selection::compile_with(&p, quote!(positional(L, R))).unwrap();
    assert!(
        ir::contract::check(&selected)
            .unwrap()
            .join_filters
            .is_empty()
    );
    let advised = join_filter(&selected, quote!(l)).unwrap();
    assert_eq!(advised.physical, selected.physical);
    assert_eq!(ir::contract::check(&advised).unwrap().join_filters.len(), 1);
}

#[test]
fn routed_exact_provider_retyping_preserves_join_filter_obligations() {
    let before = join_filter(&source(), quote!(l)).unwrap();
    let expected = ir::contract::check(&before).unwrap().join_filters;
    for ty in [
        quote!(::cq_storage::sharded::Sharded<::cq_storage::sharded::Index<_, _>>),
        quote!(::cq_storage::sharded::Sharded<::cq_storage::SortedRun<_, _>>),
    ] {
        let changed = retype(&before, quote!(b => #ty)).unwrap();
        assert_eq!(changed.physical, before.physical);
        let facts = ir::contract::check(&changed).unwrap();
        assert_eq!(facts.join_filters, expected);
        assert_eq!(ir::join_filter_candidates(&changed, &facts, "j").len(), 1);
        assert_eq!(changed, syn::parse2(changed.to_token_stream()).unwrap());
    }
}

#[test]
fn routed_filter_requires_an_enumerable_exact_native_family() {
    let before = join_filter(&source(), quote!(l)).unwrap();
    for ty in [
        quote!(::cq_storage::sharded::Sharded<MembershipOnly>),
        quote!(::cq_storage::sharded::Sharded<::cq_storage::HashIndex<'_, _, _>>),
        quote!(::cq_storage::sharded::Sharded<::cq_storage::sharded::Index<_, _>, UnknownRouter>),
        quote!(
            ::cq_storage::sharded::Sharded<
                ::cq_storage::sharded::Sharded<::cq_storage::sharded::Index<_, _>>,
            >
        ),
    ] {
        assert!(retype(&before, quote!(b => #ty)).is_err(), "{ty}");
    }
}
