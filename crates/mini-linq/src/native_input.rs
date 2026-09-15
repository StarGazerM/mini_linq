//! Administrative macro configuration, separate from the source language.
//!
//! Prefix directives are serialized only by macro entry points. The compiler
//! receives the source and ordinary options separately. This recursive derived
//! syntax accepts any directive order without putting configuration in cq-ir.

use quote::ToTokens;
use syn::{Token, parse::Parse, parse::ParseStream, token};

mod kw {
    syn::custom_keyword!(storage);
    syn::custom_keyword!(iterator);
    syn::custom_keyword!(keys);
    syn::custom_keyword!(work);
    syn::custom_keyword!(packing);
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Input<S: Parse + ToTokens> {
    #[parse(peek_func = is_storage)]
    Storage {
        keyword: kw::storage,
        path: syn::Path,
        semi: token::Semi,
        rest: Box<Self>,
    },
    #[parse(peek_func = is_iterator)]
    Iterator {
        keyword: kw::iterator,
        path: syn::Path,
        semi: token::Semi,
        rest: Box<Self>,
    },
    #[parse(peek_func = is_keys)]
    Keys {
        keyword: kw::keys,
        path: syn::Path,
        semi: token::Semi,
        rest: Box<Self>,
    },
    #[parse(peek_func = is_work)]
    Work {
        keyword: kw::work,
        #[syn(parenthesized)]
        paren: token::Paren,
        #[syn(in = paren)]
        value: Box<syn::Expr>,
        semi: token::Semi,
        rest: Box<Self>,
    },
    #[parse(peek_func = is_packing)]
    Packing {
        keyword: kw::packing,
        #[syn(parenthesized)]
        paren: token::Paren,
        #[syn(in = paren)]
        value: Box<syn::Expr>,
        semi: token::Semi,
        rest: Box<Self>,
    },
    #[parse(peek_func = |_| true)]
    Source { source: Box<S> },
}

// A source definition may itself be named `storage`, `iterator`, `keys`, or `work`.
// Its equals sign distinguishes that name from an administrative directive.
fn is_storage(input: ParseStream<'_>) -> bool {
    input.peek(kw::storage) && !input.peek2(Token![=])
}
fn is_iterator(input: ParseStream<'_>) -> bool {
    input.peek(kw::iterator) && !input.peek2(Token![=])
}
fn is_keys(input: ParseStream<'_>) -> bool {
    input.peek(kw::keys) && !input.peek2(Token![=])
}
fn is_work(input: ParseStream<'_>) -> bool {
    input.peek(kw::work) && !input.peek2(Token![=])
}

fn is_packing(input: ParseStream<'_>) -> bool {
    input.peek(kw::packing) && !input.peek2(Token![=])
}

#[derive(Default)]
struct Configuration<'a> {
    storage: Option<&'a syn::Path>,
    iterator: Option<&'a syn::Path>,
    keys: Option<&'a syn::Path>,
    work: Option<&'a syn::Expr>,
    packing: Option<&'a syn::Expr>,
}

impl<S: Parse + ToTokens> Input<S> {
    pub fn source(&self) -> &S {
        match self {
            Self::Source { source } => source,
            Self::Storage { rest, .. }
            | Self::Iterator { rest, .. }
            | Self::Keys { rest, .. }
            | Self::Work { rest, .. }
            | Self::Packing { rest, .. } => rest.source(),
        }
    }

    fn configuration(&self) -> syn::Result<Configuration<'_>> {
        let mut configuration = Configuration::default();
        let mut current = self;
        loop {
            let (duplicate, rest) = match current {
                Self::Storage { path, rest, .. } => {
                    (configuration.storage.replace(path).is_some(), rest)
                }
                Self::Iterator { path, rest, .. } => {
                    (configuration.iterator.replace(path).is_some(), rest)
                }
                Self::Keys { path, rest, .. } => (configuration.keys.replace(path).is_some(), rest),
                Self::Work { value, rest, .. } => {
                    (configuration.work.replace(value).is_some(), rest)
                }
                Self::Packing { value, rest, .. } => {
                    (configuration.packing.replace(value).is_some(), rest)
                }
                Self::Source { .. } => return Ok(configuration),
            };
            if duplicate {
                return Err(syn::Error::new_spanned(
                    current,
                    "duplicate physical configuration directive",
                ));
            }
            current = rest;
        }
    }

    #[cfg(any(feature = "prune-rows", feature = "zone-keys", feature = "block-rows"))]
    fn required(&self, value: Option<&syn::Path>, name: &str) -> syn::Result<syn::Path> {
        value.cloned().ok_or_else(|| {
            syn::Error::new_spanned(
                self.source(),
                format!("missing physical configuration `{name}`"),
            )
        })
    }

    fn configured(&self, allowed: &[&str]) -> syn::Result<Configuration<'_>> {
        let configuration = self.configuration()?;
        for (name, present) in [
            ("storage", configuration.storage.is_some()),
            ("iterator", configuration.iterator.is_some()),
            ("keys", configuration.keys.is_some()),
            ("work", configuration.work.is_some()),
            ("packing", configuration.packing.is_some()),
        ] {
            if present && !allowed.contains(&name) {
                return Err(syn::Error::new_spanned(
                    self,
                    format!("configuration `{name}` does not belong to this implementation"),
                ));
            }
        }
        Ok(configuration)
    }

    #[cfg(feature = "scan-rows")]
    pub fn scan_source(&self) -> syn::Result<&S> {
        // Observed hosts may supply a work expression. Scan performs no work
        // of its own and must leave this expression unexpanded and unevaluated.
        self.configured(&["work"])?;
        Ok(self.source())
    }

    #[cfg(feature = "prune-rows")]
    pub fn prune_options(&self) -> syn::Result<cq_access::candidates::prune::Options> {
        let configuration = self.configured(&["iterator", "work"])?;
        Ok(cq_access::candidates::prune::Options {
            iterator: self.required(configuration.iterator, "iterator")?,
            work: configuration.work.cloned(),
        })
    }

    #[cfg(feature = "exact-keys")]
    pub fn exact_options(&self) -> syn::Result<cq_access::keys::exact::Options> {
        let configuration = self.configured(&["storage", "iterator", "keys", "work"])?;
        Ok(cq_access::keys::exact::Options {
            storage: configuration.storage.cloned(),
            iterator: configuration.iterator.cloned(),
            keys: configuration.keys.cloned(),
            work: configuration.work.cloned(),
        })
    }

    #[cfg(feature = "zone-keys")]
    pub fn zone_options(&self) -> syn::Result<cq_access::keys::zone::Options> {
        let configuration = self.configured(&["storage", "iterator", "keys", "work"])?;
        Ok(cq_access::keys::zone::Options {
            storage: self.required(configuration.storage, "storage")?,
            iterator: configuration.iterator.cloned(),
            keys: configuration.keys.cloned(),
            work: configuration.work.cloned(),
        })
    }

    #[cfg(feature = "hash-witnesses")]
    pub fn hash_witness_options(&self) -> syn::Result<cq_access::witnesses::hash::Options> {
        let configuration = self.configured(&["iterator", "work"])?;
        Ok(cq_access::witnesses::hash::Options {
            iterator: configuration.iterator.cloned(),
            work: configuration.work.cloned(),
        })
    }

    #[cfg(feature = "nested-witnesses")]
    pub fn nested_witness_options(&self) -> syn::Result<cq_access::witnesses::nested::Options> {
        let configuration = self.configured(&["iterator", "work"])?;
        Ok(cq_access::witnesses::nested::Options {
            iterator: configuration.iterator.cloned(),
            work: configuration.work.cloned(),
        })
    }

    #[cfg(feature = "hash-rows")]
    pub fn hash_row_options(&self) -> syn::Result<cq_providers::hash::Options> {
        let configuration = self.configured(&["work"])?;
        Ok(cq_providers::hash::Options {
            work: configuration.work.cloned(),
        })
    }

    #[cfg(feature = "sorted-rows")]
    pub fn sorted_row_options(&self) -> syn::Result<cq_providers::sorted::Options> {
        let configuration = self.configured(&["work"])?;
        Ok(cq_providers::sorted::Options {
            work: configuration.work.cloned(),
        })
    }

    #[cfg(feature = "block-rows")]
    pub fn block_options(&self) -> syn::Result<cq_providers::block::Options> {
        let configuration = self.configured(&["storage", "packing", "work"])?;
        Ok(cq_providers::block::Options {
            storage: self.required(configuration.storage, "storage")?,
            packing: configuration.packing.cloned(),
            work: configuration.work.cloned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_ir::physical_provider::Operation;
    use quote::quote;

    #[cfg(all(
        feature = "block-rows",
        feature = "scan-rows",
        feature = "exact-keys",
        feature = "hash-rows"
    ))]
    #[test]
    fn only_implementation_owned_configuration_is_accepted() {
        let input: Input<Operation> =
            syn::parse_quote!(candidates crate::rows { &rows } (predicate));
        input.scan_source().unwrap();
        let input: Input<Operation> = syn::parse_quote!(rows(&rows));
        input.hash_row_options().unwrap();
        let input: Input<Operation> = syn::parse_quote!(storage crate::data; rows(&rows));
        assert!(input.hash_row_options().is_err());
        input.block_options().unwrap();
        let input: Input<Operation> = syn::parse_quote!(storage crate::data; contains(&keys)([1]));
        let options = input.exact_options().unwrap();
        assert!(options.iterator.is_none());
        cq_access::keys::exact::compile(input.source(), &options).unwrap();
    }

    #[test]
    fn duplicate_directives_and_unknown_options_are_rejected() {
        for duplicate in [
            quote!(storage b;),
            quote!(iterator b;),
            quote!(keys b;),
            quote!(work (b);),
            quote!(packing (b);),
        ] {
            let input: Input<Operation> = syn::parse2(quote! {
                storage a; iterator a; keys a; work (a); packing (a); #duplicate rows(&rows)
            })
            .unwrap();
            assert!(
                input
                    .configuration()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("duplicate")
            );
        }
        assert!(syn::parse2::<Input<Operation>>(quote!(unknown a; rows(&rows))).is_err());
    }

    #[cfg(feature = "block-rows")]
    #[test]
    fn packing_is_administrative_and_round_trips_in_either_order() {
        let source: Operation = syn::parse_quote!(materialize owner value (rows));
        for input in [
            quote!(storage crate::data; packing (unique_size()); #source),
            quote!(packing (unique_size()); storage crate::data; #source),
        ] {
            let parsed: Input<Operation> = syn::parse2(input.clone()).unwrap();
            assert_eq!(parsed.source(), &source);
            assert_eq!(parsed.to_token_stream().to_string(), input.to_string());
            assert!(parsed.block_options().unwrap().packing.is_some());
        }
    }

    #[cfg(feature = "block-rows")]
    #[test]
    fn directive_words_remain_available_as_source_definition_names() {
        #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
        struct Definition {
            name: syn::Ident,
            equals: Token![=],
            value: syn::Expr,
            semi: Token![;],
        }
        for name in ["storage", "iterator", "keys", "work", "packing"] {
            let name: syn::Ident = syn::parse_str(name).unwrap();
            let input: Input<Definition> =
                syn::parse2(quote!(storage crate::data; #name = 1;)).unwrap();
            assert_eq!(input.source().name, name);
            input.block_options().unwrap();
        }
    }
}
