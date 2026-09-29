//! Physical → Structures: hosts restate the policy, host keys and sorted
//! pipes answer reads in place, everything else gets a typed structure.
use cq_lang_physical as physical;
use cq_lang_physical::policy::Policy;
use cq_lang_ra as ra;
use cq_lang_structures as structures;
use cq_structure_selection::{check, compile};
use quote::{ToTokens, quote};

fn chain(attrs: proc_macro2::TokenStream) -> ra::Module {
    syn::parse_quote! {
        struct Chain;
        relation L(a: i32);
        #attrs
        relation R(b: i32, a: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(b, a), S(b, c);
        relational {
            l = rename L {a -> a};
            r = rename R {b -> b, a -> a};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            x = semijoin j1 with l;
            p = project x keep {a, c};
            output p as answer(a, c);
        }
    }
}

fn physical(plan: &ra::Module, program: proc_macro2::TokenStream) -> physical::Module {
    let ix = cq_index_policy::compile(plan).unwrap();
    syn::parse2(quote! { #ix physical { #program } }).unwrap()
}

fn hash_chain() -> proc_macro2::TokenStream {
    quote! {
        l = let rename; r = alias rename; s = let rename;
        j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
        x = pipe semi[a] j1, l; p = pipe canon project; return p;
    }
}

fn entries(module: &structures::Module) -> String {
    module
        .entries
        .iter()
        .map(|e| e.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Compile under `policy`; the entries as text, and the correspondence.
fn select(source: &physical::Module, policy: proc_macro2::TokenStream) -> (String, bool) {
    let policy: Policy = syn::parse2(policy).unwrap();
    let target = compile(source, &policy).unwrap();
    (entries(&target), check(source, &target, &policy))
}

/// The expected entries, spelled as the IR spells them.
fn expected(source: &physical::Module, entries: proc_macro2::TokenStream) -> String {
    let module: structures::Module =
        syn::parse2(quote! { #source structures { #entries } }).unwrap();
    self::entries(&module)
}

#[test]
fn hosts_restate_the_policy_and_host_keys_answer_reads_through_renames() {
    // R keyed on `a` answers j0's probe of r in place; l gets one key set
    // for the semijoin and s a hash index for the probe.
    let source = physical(&chain(quote! {}), hash_chain());
    let (text, ok) = select(
        &source,
        quote! { policy { L canon; R keyed [a] ordered canon; S; } },
    );
    assert!(ok);
    assert_eq!(
        text,
        expected(
            &source,
            quote! {
                L canon; R keyed [a] ordered canon; S;
                l = ::std::collections::HashSet<_> [a];
                s = ::cq_storage::HashIndex<_, _> [b];
            }
        )
    );
}

#[test]
fn a_host_key_on_other_columns_does_not_answer_and_the_read_of_an_alias_is_reported() {
    // R keyed on `b` does not answer a probe on `a`; r is an alias, so
    // nothing can be built over it and the contract reports the read.
    let source = physical(&chain(quote! {}), hash_chain());
    let policy: Policy = syn::parse_quote! { policy { L; R keyed [b]; S; } };
    let error = compile(&source, &policy).unwrap_err().to_string();
    assert!(error.contains("`j0` reads `r` on [\"a\"]"), "{error}");
}

#[test]
fn merge_orders_both_sides_except_a_pipe_sorted_by_its_own_merge() {
    // j0 is materialized and merged on b by j1; l serves both the merge on
    // a and the semijoin on a with one ordered structure.
    let source = physical(
        &chain(quote! {}),
        quote! {
            l = let rename; r = let rename; s = let rename;
            j0 = let merge[a] l, r; j1 = pipe merge[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return p;
        },
    );
    let (text, ok) = select(&source, quote! { policy { L; R; S; join = merge; } });
    assert!(ok);
    assert_eq!(
        text,
        expected(
            &source,
            quote! {
                L; R; S;
                l = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [a];
                r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [a];
                s = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b];
                j0 = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b];
            }
        )
    );

    // A star on `a`: j0 is a pipe sorted on a by its own merge, so j1's
    // merge on a reads the stream; ordered host keys answer the rest.
    let star: ra::Module = syn::parse_quote! {
        struct Star;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation T(a: i32, c: i32);
        answer(a, b, c) :- L(a), R(a, b), T(a, c);
        relational {
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            t = rename T {a -> a, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with t;
            output j1 as answer(a, b, c);
        }
    };
    let source = physical(
        &star,
        quote! {
            l = alias rename; r = alias rename; t = let rename;
            j0 = pipe merge[a] l, r; j1 = pipe canon merge[a] j0, t; return j1;
        },
    );
    let (text, ok) = select(
        &source,
        quote! { policy { L keyed [a] ordered; R keyed [a] ordered; T; join = merge; } },
    );
    assert!(ok);
    assert_eq!(
        text,
        expected(
            &source,
            quote! {
                L keyed [a] ordered; R keyed [a] ordered; T;
                t = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [a];
            }
        )
    );
}

#[test]
fn a_range_needs_an_ordered_key() {
    let plan: ra::Module = syn::parse_quote! {
        struct Bounded;
        relation R(a: i32, b: i32);
        answer(a, b) :- R(a, b), if (a > 1);
        relational {
            r = rename R {a -> a, b -> b};
            f = select r if (a > 1);
            output f as answer(a, b);
        }
    };
    let program = quote! { r = let rename; f = pipe range[a]; return f; };
    let source = physical(&plan, program);
    // An unordered host key on `a` does not answer the range: build one.
    let (text, ok) = select(&source, quote! { policy { R keyed [a] canon; } });
    assert!(ok);
    assert_eq!(
        text,
        expected(
            &source,
            quote! {
                R keyed [a] canon;
                r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [a];
            }
        )
    );
    // An ordered one does.
    let (text, ok) = select(&source, quote! { policy { R keyed [a] ordered canon; } });
    assert!(ok);
    assert_eq!(
        text,
        expected(&source, quote! { R keyed [a] ordered canon; })
    );
}

#[test]
fn policy_key_columns_are_names_and_every_host_is_listed() {
    let source = physical(&chain(quote! {}), hash_chain());
    let policy: Policy = syn::parse_quote! { policy { L; R keyed [z]; S; } };
    let err = compile(&source, &policy).unwrap_err();
    assert_eq!(err.to_string(), "`R` has no column `z`");
    let policy: Policy = syn::parse_quote! { policy { L; R; } };
    let err = compile(&source, &policy).unwrap_err();
    assert_eq!(err.to_string(), "policy must list host relation `S`");
}

#[test]
fn index_advice_names_any_rust_type_for_structures_built_over_that_relation() {
    let source = physical(
        &chain(quote! { #[index(my_crate::Trie)] }),
        quote! {
            l = let rename; r = let rename; s = let rename;
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return p;
        },
    );
    let (text, ok) = select(&source, quote! { policy { L; R; S; } });
    assert!(ok);
    assert_eq!(
        text,
        expected(
            &source,
            quote! {
                L; R; S;
                l = ::std::collections::HashSet<_> [a];
                r = my_crate::Trie [a];
                s = ::cq_storage::HashIndex<_, _> [b];
            }
        )
    );
}

#[test]
fn a_non_canonical_return_is_the_contract_s_diagnostic_not_a_built_structure() {
    let source = physical(
        &chain(quote! {}),
        quote! {
            l = let rename; r = alias rename; s = let rename;
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe project; return p;
        },
    );
    let policy: Policy = syn::parse_quote! { policy { L; R keyed [a]; S; } };
    let error = compile(&source, &policy).unwrap_err().to_string();
    assert!(error.contains("returned `p` is not canonical"), "{error}");
}

#[allow(unused_imports, dead_code)]
mod cq_index_policy {
    pub use ::cq_index_policy::*;
    pub fn compile(source: &cq_lang_ra::Module) -> syn::Result<cq_lang_ix::Module> {
        Ok(cq_lang_ix::Module::new(
            source.clone(),
            ::cq_index_policy::compile(source)?,
        ))
    }
    pub fn check(source: &cq_lang_ra::Module, target: &cq_lang_ix::Module) -> bool {
        target.relational_plan == *source && ::cq_index_policy::check(source, &target.section)
    }
}

#[allow(unused_imports, dead_code)]
mod cq_structure_selection {
    pub use ::cq_structure_selection::*;
    pub fn compile(
        source: &cq_lang_physical::Module,
        policy: &cq_lang_physical::policy::Policy,
    ) -> syn::Result<cq_lang_structures::Module> {
        Ok(cq_lang_structures::Module::new(
            source.clone(),
            ::cq_structure_selection::compile(source, policy)?,
        ))
    }
    pub fn check(
        source: &cq_lang_physical::Module,
        target: &cq_lang_structures::Module,
        policy: &cq_lang_physical::policy::Policy,
    ) -> bool {
        target.physical == *source
            && ::cq_structure_selection::check(source, &target.section, policy)
    }
}
