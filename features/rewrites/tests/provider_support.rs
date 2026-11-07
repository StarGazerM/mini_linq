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

/// Advice declaring the test's custom exact provider token.
fn declared() -> proc_macro2::TokenStream {
    quote!(providers {
        custom_provider = CustomProvider<_, _> {
            reads [exact, membership], supports [retain, expand, keys], build typed
        };
    })
}

fn source() -> (ir::physical::Program, ir::Program) {
    let physical = syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L {a -> x};
        r = let rename R {a -> x,b -> y};
        s = pipe semi eager [x] l,r;
        p = pipe probe first [x] s,r;
        out = let canon project p keep {x,y};
        return out as answer(x,y);
    }};
    let structures = cq_structure_selection::compile(&physical).unwrap();
    (physical, structures)
}

#[test]
fn explicit_support_composes_and_retyping_cannot_leave_stale_promises() {
    let (physical, source) = source();
    let native = ir::contract::check(&physical, &source).unwrap();
    assert!(native.provider_support("r", &["x".into()], None, ProviderCapability::Keys));
    assert!(!native.provider_support("r", &["x".into()], None, ProviderCapability::Retain));
    let declared = declared();
    let custom = retype(&physical, 
        &source,
        quote!(#declared r => @custom_provider as ProviderAlias<_, _> supports [expand, keys]),
    )
    .unwrap();
    let facts = ir::contract::check(&physical, &custom).unwrap();
    assert!(facts.provider_support("r", &["x".into()], None, ProviderCapability::Retain));
    let retained = retain_lookups(&physical, &custom, quote!()).unwrap();
    let staged = expand_lookups(&physical, &custom, quote!(2)).unwrap();
    let combined = expand_lookups(&physical, &retained, quote!(2)).unwrap();
    let reordered = retain_lookups(&physical, &staged, quote!()).unwrap();
    let combined_facts = ir::contract::check(&physical, &combined).unwrap();
    let reordered_facts = ir::contract::check(&physical, &reordered).unwrap();
    assert_eq!(combined_facts.shares, reordered_facts.shares);
    assert_eq!(combined_facts.witnesses, reordered_facts.witnesses);
    assert_eq!(combined_facts.expands, reordered_facts.expands);
    for plan in [&retained, &staged, &combined] {
        assert!(retype(&physical, plan, quote!(r => @custom_provider as CustomProvider<_, _>)).is_err());
        retype(&physical, 
            plan,
            quote!(r => @custom_provider as CustomProvider<_, _> supports [expand, keys]),
        )
        .unwrap();
    }
    let cleared = retype(&physical, 
        &custom,
        quote!(r => @custom_provider as CustomProvider<_, _>),
    )
    .unwrap();
    assert_eq!(retain_lookups(&physical, &cleared, quote!()).unwrap(), cleared);
    assert_eq!(expand_lookups(&physical, &cleared, quote!(2)).unwrap(), cleared);
    assert!(cleared.structures.entries.iter().all(|entry| !matches!(
        &entry.structure, Structure::Built { keyed, .. } if keyed.supports.is_some()
    )));
    assert!(
        retype(&physical, 
            &source,
            quote!(dedup => @btree_set as BTreeSet<_> supports [keys])
        )
        .is_err()
    );
    let retained_only = retype(&physical, 
        &source,
        quote!(#declared r => @custom_provider as CustomProvider<_, _> supports [retain]),
    )
    .unwrap();
    assert_eq!(
        expand_lookups(&physical, &retained_only, quote!(2)).unwrap(),
        retained_only
    );
    assert!(
        ir::contract::check(&physical, &retain_lookups(&physical, &retained_only, quote!()).unwrap())
            .unwrap()
            .witnesses
            .contains_key("s")
    );
}

#[test]
fn implementation_selection_replaces_support_for_its_actual_layout() {
    let (physical, source) = source();
    for selected in [
        sorted_runs(&physical, &source, quote!()).unwrap(),
        cq_rewrite_passes::radix_runs(&physical, &source, quote!()).unwrap(),
    ] {
        let facts = ir::contract::check(&physical, &selected).unwrap();
        for capability in [
            ProviderCapability::Retain,
            ProviderCapability::Expand,
            ProviderCapability::Keys,
        ] {
            assert!(facts.provider_support("r", &["x".into()], None, capability));
        }
    }
    // Rebuilding a hash layout replaces the old packet, even when its original
    // spelling still matches the selector's implementation catalog. Support
    // beyond the token's declaration is rejected (STR-Provider).
    assert!(
        retype(&physical, 
            &source,
            quote!(r => @hash_index as ::cq_storage::HashIndex<'_, _, _> supports [expand, keys]),
        )
        .is_err()
    );
    let declared = retype(&physical, 
        &source,
        quote!(r => @hash_index as ::cq_storage::HashIndex<'_, _, _> supports [keys]),
    )
    .unwrap();
    let rebuilt = cq_rewrite_passes::sorted_indexes(&physical, &declared, quote!()).unwrap();
    let facts = ir::contract::check(&physical, &rebuilt).unwrap();
    assert!(facts.provider_support("r", &["x".into()], None, ProviderCapability::Keys));
    assert!(!facts.provider_support("r", &["x".into()], None, ProviderCapability::Expand));
}

#[test]
fn keys_advice_uses_declarations_instead_of_names() {
    let (physical, source) = source();
    let declared = declared();
    for plan in [
        source.clone(),
        sorted_runs(&physical, &source, quote!()).unwrap(),
        retype(&physical, 
            &source,
            quote!(#declared r => @custom_provider as ProviderAlias<_, _> supports [keys]),
        )
        .unwrap(),
    ] {
        let filtered = join_filter(&physical, &plan, quote!(l)).unwrap();
        assert!(!ir::contract::check(&physical, &filtered).unwrap().join_filters["l"].is_empty());
        assert!(
            retype(&physical, 
                &filtered,
                quote!(#declared r => @custom_provider as ProviderAlias<_, _>)
            )
            .is_err()
        );
    }
    let undeclared = retype(&physical, 
        &source,
        quote!(#declared r => @custom_provider as ProviderAlias<_, _>),
    )
    .unwrap();
    assert!(join_filter(&physical, &undeclared, quote!(l)).is_err());
}

#[test]
fn builtin_spelling_without_support_does_not_enable_advice() {
    let (physical, source) = source();
    let sorted = sorted_runs(&physical, &source, quote!()).unwrap();
    let unrecorded = retype(&physical, 
        &sorted,
        quote!(r => @sorted_run as ::cq_storage::SortedRun<_, _>),
    )
    .unwrap();
    assert_eq!(retain_lookups(&physical, &unrecorded, quote!()).unwrap(), unrecorded);
    assert_eq!(expand_lookups(&physical, &unrecorded, quote!(2)).unwrap(), unrecorded);
    assert!(join_filter(&physical, &unrecorded, quote!(l)).is_err());
}

macro_rules! query {
    ($provider:ident, $left:expr, $right:expr, $visits:ident; $($pipeline:tt)*) => {
        mini_linq::passes::select! {
            pipeline {
                cq_rewrites::retype(
                    providers {
                        custom_provider = CustomProvider<_, _> {
                            reads [exact, membership], supports [retain, expand, keys], build typed
                        };
                    }
                    r => @custom_provider as $provider<_, _> supports [expand, keys]
                ),
                $($pipeline)*
            };
            physical {
                relation L(a:i64) = ($left); relation R(a:i64,b:i64) = ($right);
                l = pipe rename L {a -> x};
                r = let rename R {a -> x,b -> y};
                s = pipe semi eager [x] l,r;
                p = pipe probe first [x] s,r;
                observed = pipe guard p if ({ $visits.borrow_mut().push((*x, *y)); true });
                out = let canon project observed keep {x,y};
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
                            cq_rewrites::retype(
                                providers {
                                    custom_provider = CustomProvider<_, _> {
                                        reads [exact, membership],
                                        supports [retain, expand, keys],
                                        build typed
                                    };
                                }
                                payload => @custom_provider as $provider<_, _> supports [expand, keys]
                            ),
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
fn whole_relation_reads_preserve_cartesian_occurrences() {
    let left = [(2_i64,), (1,), (2,)];
    let right = [(10_i64,), (11,), (10,)];
    let visits = &RefCell::new(Vec::new());
    let answers: BTreeSet<_> = mini_linq::passes::sequence! {
        pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation L(a:i64) = (&left); relation R(b:i64) = (&right);
            l = pipe rename L {a -> x};
            r = let rename R {b -> y};
            s = pipe semi eager [] l,r;
            p = pipe probe first [] s,r;
            observed = pipe guard p if ({ visits.borrow_mut().push((*x,*y)); true });
            out = let canon project observed keep {x,y};
            return out as answer(x,y);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @host_scan as ::core::iter::Map<_,_> [];
            r host scan = @host_scan as ::core::iter::Map<_,_> [];
            r rows = @vec as ::std::vec::Vec<_>;
            out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
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
