//! Administrative macro configuration, separate from the source language.
//!
//! Prefix directives are serialized only by macro entry points. The compiler
//! receives the source and ordinary options separately. This recursive derived
//! syntax accepts any directive order without putting configuration in cq-ir.

use cq_compiler::{AccessOptions, ProviderOptions};
use quote::ToTokens;
use syn::{Token, parse::Parse, parse::ParseStream, token};

mod kw {
    syn::custom_keyword!(storage);
    syn::custom_keyword!(iterator);
    syn::custom_keyword!(keys);
    syn::custom_keyword!(work);
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

#[derive(Default)]
struct Configuration<'a> {
    storage: Option<&'a syn::Path>,
    iterator: Option<&'a syn::Path>,
    keys: Option<&'a syn::Path>,
    work: Option<&'a syn::Expr>,
}

impl<S: Parse + ToTokens> Input<S> {
    pub fn source(&self) -> &S {
        match self {
            Self::Source { source } => source,
            Self::Storage { rest, .. }
            | Self::Iterator { rest, .. }
            | Self::Keys { rest, .. }
            | Self::Work { rest, .. } => rest.source(),
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

    fn required(&self, value: Option<&syn::Path>, name: &str) -> syn::Result<syn::Path> {
        value.cloned().ok_or_else(|| {
            syn::Error::new_spanned(
                self.source(),
                format!("missing physical configuration `{name}`"),
            )
        })
    }

    pub fn provider_options(&self) -> syn::Result<ProviderOptions> {
        let configuration = self.configuration()?;
        if let Some(path) = configuration.iterator.or(configuration.keys) {
            return Err(syn::Error::new_spanned(
                path,
                "provider configuration admits only `storage` and optional `work`",
            ));
        }
        Ok(ProviderOptions {
            storage: self.required(configuration.storage, "storage")?,
            work: configuration.work.cloned(),
        })
    }

    pub fn access_options(&self) -> syn::Result<AccessOptions> {
        let configuration = self.configuration()?;
        Ok(AccessOptions {
            storage: self.required(configuration.storage, "storage")?,
            iterator: self.required(configuration.iterator, "iterator")?,
            keys: configuration.keys.cloned(),
            work: configuration.work.cloned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_ir::physical_provider::Operation;
    use quote::quote;

    #[test]
    fn provider_source_is_independent_of_optional_configuration() {
        let plain: Input<Operation> = syn::parse_quote! {
            storage crate::storage;
            scan (rows)
        };
        let observed: Input<Operation> = syn::parse_quote! {
            work (&metrics! { local, counters });
            storage crate::storage;
            scan (rows)
        };
        assert_eq!(plain.source(), observed.source());
        assert!(plain.provider_options().unwrap().work.is_none());
        assert!(observed.provider_options().unwrap().work.is_some());
        assert_eq!(
            observed.to_token_stream().to_string(),
            syn::parse2::<Input<Operation>>(observed.to_token_stream())
                .unwrap()
                .to_token_stream()
                .to_string()
        );
    }

    #[test]
    fn iterator_configuration_accepts_each_order_without_changing_source() {
        let tokens = quote! {
            work (&counters);
            keys crate::Keys;
            iterator crate::collect;
            storage crate::data;
            scan (rows)
        };
        let input: Input<Operation> = syn::parse2(tokens.clone()).unwrap();
        let options = input.access_options().unwrap();
        assert_eq!(options.storage, syn::parse_quote!(crate::data));
        assert_eq!(options.iterator, syn::parse_quote!(crate::collect));
        assert_eq!(options.keys, Some(syn::parse_quote!(crate::Keys)));
        assert_eq!(options.work, Some(syn::parse_quote!(&counters)));
        assert_eq!(input.to_token_stream().to_string(), tokens.to_string());
        let input: Input<Operation> = syn::parse_quote! {
            iterator crate::collect;
            storage crate::data;
            scan (rows)
        };
        let options = input.access_options().unwrap();
        assert!(options.keys.is_none());
        assert!(options.work.is_none());
    }

    #[test]
    fn duplicates_and_irrelevant_or_missing_configuration_are_rejected() {
        for duplicate in [
            quote!(storage second::data;),
            quote!(iterator second::collect;),
            quote!(keys second::Keys;),
            quote!(work (&other);),
        ] {
            let input: Input<Operation> = syn::parse2(quote! {
                storage crate::data; iterator crate::collect;
                keys crate::Keys; work (&counters);
                #duplicate
                scan (rows)
            })
            .unwrap();
            assert!(
                input
                    .access_options()
                    .unwrap_err()
                    .to_string()
                    .contains("duplicate")
            );
        }
        let input: Input<Operation> = syn::parse_quote!(scan(rows));
        assert!(
            input
                .provider_options()
                .unwrap_err()
                .to_string()
                .contains("storage")
        );
        let input: Input<Operation> = syn::parse_quote!(storage crate::data; scan (rows));
        assert!(
            input
                .access_options()
                .unwrap_err()
                .to_string()
                .contains("iterator")
        );
        for irrelevant in [quote!(iterator crate::collect;), quote!(keys crate::Keys;)] {
            let input: Input<Operation> = syn::parse2(quote! {
                storage crate::data; #irrelevant scan (rows)
            })
            .unwrap();
            assert!(
                input
                    .provider_options()
                    .unwrap_err()
                    .to_string()
                    .contains("only")
            );
        }
        assert!(
            syn::parse2::<Input<Operation>>(quote! {
                storage crate::data; unknown crate::other; scan (rows)
            })
            .is_err()
        );
    }

    #[test]
    fn directive_words_remain_available_as_source_definition_names() {
        #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
        struct Definition {
            name: syn::Ident,
            equals: Token![=],
            value: syn::Expr,
            semi: Token![;],
        }
        for name in ["storage", "iterator", "keys", "work"] {
            let name: syn::Ident = syn::parse_str(name).unwrap();
            let input: Input<Definition> = syn::parse2(quote! {
                storage crate::data; #name = 1;
            })
            .unwrap();
            assert_eq!(input.source().name, name);
            input.provider_options().unwrap();
        }
    }
}
