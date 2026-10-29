use cq_lang_structures::{self as ir, ProviderCapability, Structure};
use cq_rewrite_passes::{expand_lookups, join_filter, retain_lookups, sorted_runs};
use cq_structure_selection::retype;
use quote::quote;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

// An externally authored exact provider with neither a builtin name nor cursor.
struct CustomProvider<K, V>(BTreeMap<K, Vec<V>>);
type ProviderAlias<K, V> = CustomProvider<K, V>;

impl<K: Ord, V, I: Iterator<Item = (K, V)>> cq_storage::Build<I> for CustomProvider<K, V> {
    fn build(source: I) -> Self {
        let mut groups: BTreeMap<K, Vec<V>> = BTreeMap::new();
        for (key, value) in source {
            groups.entry(key).or_default().push(value);
        }
        Self(groups)
    }
}

#[derive(Clone, Copy)]
struct CustomCursor<'a, V> {
    remaining: &'a [V],
}
impl<V: Copy> Iterator for CustomCursor<'_, V> {
    type Item = V;
    fn next(&mut self) -> Option<V> {
        let (first, rest) = self.remaining.split_first()?;
        self.remaining = rest;
        Some(*first)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining.len(), Some(self.remaining.len()))
    }
}
impl<V: Copy> ExactSizeIterator for CustomCursor<'_, V> {}
impl<'a, K: Ord, V: Copy> cq_storage::IndexRead for &'a CustomProvider<K, V> {
    type Key = K;
    type Item = V;
    type Cursor = CustomCursor<'a, V>;
    fn lookup(&self, key: &K) -> Self::Cursor {
        CustomCursor {
            remaining: self.0.get(key).map_or(&[], Vec::as_slice),
        }
    }
}
impl<K: Ord, V> cq_storage::ExactMembership<K> for CustomProvider<K, V> {
    fn contains(&self, key: &K) -> bool {
        self.0.contains_key(key)
    }
}
impl<K, V> cq_storage::Keys for CustomProvider<K, V> {
    type Key = K;
    type Iter<'a>
        = std::collections::btree_map::Keys<'a, K, Vec<V>>
    where
        Self: 'a;
    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.0.keys()
    }
    fn key_count(&self) -> usize {
        self.0.len()
    }
}

fn source() -> ir::Program {
    cq_structure_selection::compile(&syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,r;
        p = pipe probe first [x] s,r;
        out = pipe canon project p keep {x,y};
        return out as answer(x,y);
    }})
    .unwrap()
}

#[test]
fn explicit_support_composes_and_retyping_cannot_leave_stale_promises() {
    let source = source();
    let native = ir::contract::check(&source).unwrap();
    assert!(native.provider_support("r", &["x".into()], None, ProviderCapability::Keys));
    assert!(!native.provider_support("r", &["x".into()], None, ProviderCapability::Retain));
    let custom = retype(
        &source,
        quote!(r => ProviderAlias<_, _> supports [expand, keys]),
    )
    .unwrap();
    let facts = ir::contract::check(&custom).unwrap();
    assert!(facts.provider_support("r", &["x".into()], None, ProviderCapability::Retain));
    assert_eq!(custom.physical, source.physical);
    let retained = retain_lookups(&custom, quote!()).unwrap();
    let staged = expand_lookups(&custom, quote!(2)).unwrap();
    let combined = expand_lookups(&retained, quote!(2)).unwrap();
    let reordered = retain_lookups(&staged, quote!()).unwrap();
    let combined_facts = ir::contract::check(&combined).unwrap();
    let reordered_facts = ir::contract::check(&reordered).unwrap();
    assert_eq!(combined_facts.shares, reordered_facts.shares);
    assert_eq!(combined_facts.witnesses, reordered_facts.witnesses);
    assert_eq!(combined_facts.expands, reordered_facts.expands);
    for plan in [&retained, &staged, &combined] {
        assert!(retype(plan, quote!(r => CustomProvider<_, _>)).is_err());
        retype(
            plan,
            quote!(r => CustomProvider<_, _> supports [expand, keys]),
        )
        .unwrap();
    }
    let cleared = retype(&custom, quote!(r => CustomProvider<_, _>)).unwrap();
    assert_eq!(retain_lookups(&cleared, quote!()).unwrap(), cleared);
    assert_eq!(expand_lookups(&cleared, quote!(2)).unwrap(), cleared);
    assert!(cleared.structures.entries.iter().all(|entry| !matches!(
        &entry.structure, Structure::Built { keyed, .. } if keyed.supports.is_some()
    )));
    assert!(retype(&source, quote!(dedup => BTreeSet<_> supports [keys])).is_err());
    let retained_only =
        retype(&source, quote!(r => CustomProvider<_, _> supports [retain])).unwrap();
    assert_eq!(
        expand_lookups(&retained_only, quote!(2)).unwrap(),
        retained_only
    );
    assert!(
        ir::contract::check(&retain_lookups(&retained_only, quote!()).unwrap())
            .unwrap()
            .witnesses
            .contains_key("s")
    );
}

#[test]
fn implementation_selection_replaces_support_for_its_actual_layout() {
    let source = source();
    for selected in [
        sorted_runs(&source, quote!()).unwrap(),
        cq_rewrite_passes::radix_runs(&source, quote!()).unwrap(),
    ] {
        let facts = ir::contract::check(&selected).unwrap();
        for capability in [
            ProviderCapability::Retain,
            ProviderCapability::Expand,
            ProviderCapability::Keys,
        ] {
            assert!(facts.provider_support("r", &["x".into()], None, capability));
        }
        assert_eq!(selected.physical, source.physical);
    }
    // Rebuilding a hash layout replaces the old packet, even when its original
    // spelling still matches the selector's implementation catalog.
    let declared = retype(
        &source,
        quote!(r => ::cq_storage::HashIndex<'_, _, _> supports [expand, keys]),
    )
    .unwrap();
    let rebuilt = cq_rewrite_passes::sorted_indexes(&declared, quote!()).unwrap();
    let facts = ir::contract::check(&rebuilt).unwrap();
    assert!(facts.provider_support("r", &["x".into()], None, ProviderCapability::Keys));
    assert!(!facts.provider_support("r", &["x".into()], None, ProviderCapability::Expand));
}

#[test]
fn keys_advice_uses_declarations_instead_of_names() {
    let source = source();
    for plan in [
        source.clone(),
        sorted_runs(&source, quote!()).unwrap(),
        retype(&source, quote!(r => ProviderAlias<_, _> supports [keys])).unwrap(),
    ] {
        let filtered = join_filter(&plan, quote!(l)).unwrap();
        assert!(!ir::contract::check(&filtered).unwrap().join_filters["l"].is_empty());
        assert!(retype(&filtered, quote!(r => ProviderAlias<_, _>)).is_err());
    }
    let undeclared = retype(&source, quote!(r => ProviderAlias<_, _>)).unwrap();
    assert!(join_filter(&undeclared, quote!(l)).is_err());
}

#[test]
fn builtin_spelling_without_support_does_not_enable_advice() {
    let sorted = sorted_runs(&source(), quote!()).unwrap();
    let unrecorded = retype(&sorted, quote!(r => ::cq_storage::SortedRun<_, _>)).unwrap();
    assert_eq!(retain_lookups(&unrecorded, quote!()).unwrap(), unrecorded);
    assert_eq!(expand_lookups(&unrecorded, quote!(2)).unwrap(), unrecorded);
    assert!(join_filter(&unrecorded, quote!(l)).is_err());
}

macro_rules! query {
    ($provider:ident, $left:expr, $right:expr, $visits:ident; $($pipeline:tt)*) => {
        mini_linq::passes::select! {
            pipeline {
                cq_rewrites::retype(r => $provider<_, _> supports [expand, keys]),
                $($pipeline)*
            };
            physical {
                relation L(a:i64) = ($left); relation R(a:i64,b:i64) = ($right);
                l = pipe rename L@0 {a -> x};
                r = let rename R@1 {a -> x,b -> y};
                s = pipe semi eager [x] l,r;
                p = pipe probe first [x] s,r;
                observed = pipe guard p if ({ $visits.borrow_mut().push((*x, *y)); true });
                out = pipe canon project observed keep {x,y};
                return out as answer(x,y);
            }
        }
    };
}

macro_rules! execution_case {
    ($name:ident, $provider:ident; $($advice:tt)*) => {
        #[test]
        fn $name() {
            for (left, right) in [
                (vec![], vec![]),
                (vec![(0,), (1,)], vec![]),
                (vec![(3,), (1,), (2,), (1,), (i64::MIN,), (i64::MAX,)],
                 vec![(1, 10), (3, 30), (1, 11), (1, 10), (i64::MIN, -1), (i64::MAX, 1)]),
            ] {
                let expected: Vec<_> = left.iter().flat_map(|&(key,)| {
                    right.iter().filter_map(move |&(other, value)| (key == other).then_some((key, value)))
                }).collect();
                let visits = &RefCell::new(Vec::new());
                let actual: BTreeSet<_> = query!($provider, &left, &right, visits;
                    $($advice)*
                    mini_linq::passes::sequence, mini_linq::passes::sequential,
                    mini_linq::passes::translate_pull
                ).map(|(x, y)| (*x, *y)).collect();
                assert_eq!(actual, expected.iter().copied().collect());
                assert_eq!(*visits.borrow(), expected);
                let default: BTreeSet<_> = mini_linq::compile! {
                    pipeline {
                        structures = [
                            cq_rewrites::retype(payload => $provider<_, _> supports [expand, keys]),
                            $($advice)*
                        ];
                    }
                    struct CustomQuery;
                    relation L(a:i64) = (&left);
                    relation R(a:i64,b:i64) = (&right);
                    answer(a,b) :- L(a), R(a,b);
                }.map(|(x, y)| (*x, *y)).collect();
                assert_eq!(default, actual);
            }
        }
    };
}

execution_case!(custom_immediate, CustomProvider;);
execution_case!(custom_retained, CustomProvider; cq_rewrites::retain_lookups,);
execution_case!(custom_staged, CustomProvider; cq_rewrites::expand_lookups(2),);
execution_case!(custom_combined, CustomProvider; cq_rewrites::retain_lookups, cq_rewrites::expand_lookups(2),);
execution_case!(alias_immediate, ProviderAlias;);
execution_case!(alias_retained, ProviderAlias; cq_rewrites::retain_lookups,);
execution_case!(alias_staged, ProviderAlias; cq_rewrites::expand_lookups(2),);
execution_case!(alias_combined, ProviderAlias; cq_rewrites::expand_lookups(2), cq_rewrites::retain_lookups,);

#[test]
fn unit_key_support_preserves_cartesian_occurrences() {
    let left = [(2_i64,), (1,), (2,)];
    let right = [(10_i64,), (11,), (10,)];
    let visits = &RefCell::new(Vec::new());
    let answers: BTreeSet<_> = mini_linq::passes::sequence! {
        pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation L(a:i64) = (&left); relation R(b:i64) = (&right);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {b -> y};
            s = pipe semi eager [] l,r;
            p = pipe probe first [] s,r;
            observed = pipe guard p if ({ visits.borrow_mut().push((*x,*y)); true });
            out = pipe canon project observed keep {x,y};
            return out as answer(x,y);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r = ProviderAlias<_,_> [] supports [expand];
            s share = lookup r [] first s force p;
            s repr = tuples witness { r [] at s as _ };
            p stage = batch 2 expand;
            out canon = ::std::collections::HashSet<_>;
        }
    }
    .map(|(x, y)| (*x, *y))
    .collect();
    let expected: Vec<_> = left
        .iter()
        .flat_map(|&(x,)| right.iter().map(move |&(y,)| (x, y)))
        .collect();
    assert_eq!(*visits.borrow(), expected);
    assert_eq!(answers, expected.into_iter().collect());
}
