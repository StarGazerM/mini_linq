//! Syntax-preserving administrative macro arguments; query semantics stay in cq-ir.

use std::collections::BTreeMap;

use cq_compiler::PhysicalOptions;
use cq_ir::{relational_plan as ra, symbol_name};
use quote::ToTokens;
use syn::{parse::Parse, token};

mod kw {
    syn::custom_keyword!(storage);
    syn::custom_keyword!(access);
    syn::custom_keyword!(iterator);
    syn::custom_keyword!(key_provider);
    syn::custom_keyword!(key_providers);
    syn::custom_keyword!(materialize);
    syn::custom_keyword!(scan);
    syn::custom_keyword!(keys);
    syn::custom_keyword!(join);
    syn::custom_keyword!(inputs);
    syn::custom_keyword!(results);
    syn::custom_keyword!(scans);
    syn::custom_keyword!(key_sets);
    syn::custom_keyword!(joins);
}

/// Administrative choices for the direct RA-to-Access edge. Their contextual
/// validity is checked separately from their derived parser.
#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub struct PhysicalInput {
    #[syn(repeat, peek = syn::Ident)]
    directives: Vec<Directive>,
    pub source: ra::Module,
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
enum Directive {
    #[parse(peek = kw::storage)]
    Storage(PathDirective<kw::storage>),
    #[parse(peek = kw::access)]
    Access(PathDirective<kw::access>),
    #[parse(peek = kw::iterator)]
    Iterator(PathDirective<kw::iterator>),
    #[parse(peek = kw::key_provider)]
    KeyProvider(PathDirective<kw::key_provider>),
    #[parse(peek = kw::key_providers)]
    KeyProviders(MapDirective<kw::key_providers>),
    #[parse(peek = kw::materialize)]
    Materialize(PathDirective<kw::materialize>),
    #[parse(peek = kw::scan)]
    Scan(PathDirective<kw::scan>),
    #[parse(peek = kw::keys)]
    Keys(PathDirective<kw::keys>),
    #[parse(peek = kw::join)]
    Join(PathDirective<kw::join>),
    #[parse(peek = kw::inputs)]
    Inputs(MapDirective<kw::inputs>),
    #[parse(peek = kw::results)]
    Results(MapDirective<kw::results>),
    #[parse(peek = kw::scans)]
    Scans(MapDirective<kw::scans>),
    #[parse(peek = kw::key_sets)]
    KeySets(MapDirective<kw::key_sets>),
    #[parse(peek = kw::joins)]
    Joins(MapDirective<kw::joins>),
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
struct PathDirective<K: Parse + ToTokens> {
    keyword: K,
    path: syn::Path,
    semi_token: token::Semi,
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
struct MapDirective<K: Parse + ToTokens> {
    keyword: K,
    #[syn(braced)]
    brace: token::Brace,
    #[syn(in = brace, repeat)]
    bindings: Vec<Binding>,
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
struct Binding {
    name: syn::Ident,
    fat_arrow_token: token::FatArrow,
    path: syn::Path,
    semi_token: token::Semi,
}

impl PhysicalInput {
    pub fn options(&self) -> syn::Result<PhysicalOptions> {
        let mut scalar = BTreeMap::new();
        let mut maps = BTreeMap::new();
        for directive in &self.directives {
            let scalar_entry = match directive {
                Directive::Storage(value) => Some(("storage", &value.path)),
                Directive::Access(value) => Some(("access", &value.path)),
                Directive::Iterator(value) => Some(("iterator", &value.path)),
                Directive::KeyProvider(value) => Some(("key_provider", &value.path)),
                Directive::Materialize(value) => Some(("materialize", &value.path)),
                Directive::Scan(value) => Some(("scan", &value.path)),
                Directive::Keys(value) => Some(("keys", &value.path)),
                Directive::Join(value) => Some(("join", &value.path)),
                _ => None,
            };
            if let Some((name, path)) = scalar_entry {
                if scalar.insert(name, path.clone()).is_some() {
                    return Err(syn::Error::new_spanned(
                        directive,
                        "duplicate physical directive",
                    ));
                }
                continue;
            }
            let (name, bindings) = match directive {
                Directive::Inputs(value) => ("inputs", &value.bindings),
                Directive::KeyProviders(value) => ("key_providers", &value.bindings),
                Directive::Results(value) => ("results", &value.bindings),
                Directive::Scans(value) => ("scans", &value.bindings),
                Directive::KeySets(value) => ("key_sets", &value.bindings),
                Directive::Joins(value) => ("joins", &value.bindings),
                _ => unreachable!("scalar directives handled above"),
            };
            if maps.contains_key(name) {
                return Err(syn::Error::new_spanned(
                    directive,
                    "duplicate physical directive",
                ));
            }
            let mut entries = BTreeMap::new();
            for binding in bindings {
                if entries
                    .insert(symbol_name(&binding.name), binding.path.clone())
                    .is_some()
                {
                    return Err(syn::Error::new_spanned(
                        &binding.name,
                        "duplicate physical binding",
                    ));
                }
            }
            maps.insert(name, entries);
        }
        let mut required = |name| {
            scalar.remove(name).ok_or_else(|| {
                syn::Error::new_spanned(
                    &self.source,
                    format!("missing physical directive `{name}`"),
                )
            })
        };
        Ok(PhysicalOptions {
            storage: required("storage")?,
            access: required("access")?,
            iterator: required("iterator")?,
            key_provider: required("key_provider")?,
            key_providers: maps.remove("key_providers").unwrap_or_default(),
            materialize: required("materialize")?,
            scan: required("scan")?,
            keys: required("keys")?,
            join: required("join")?,
            inputs: maps.remove("inputs").unwrap_or_default(),
            results: maps.remove("results").unwrap_or_default(),
            scans: maps.remove("scans").unwrap_or_default(),
            key_sets: maps.remove("key_sets").unwrap_or_default(),
            joins: maps.remove("joins").unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn source() -> ra::Module {
        syn::parse_quote! {
            pub struct P;
            relation R(k:i64);
            answer(k) :- R(k);
            relational {
                r = rename R {k -> k};
                output r as answer(k);
            }
        }
    }

    fn required() -> proc_macro2::TokenStream {
        quote! {
            storage crate::storage;
            access crate::access;
            iterator crate::iterator;
            key_provider crate::key_provider;
            materialize crate::rows;
            scan crate::reduce;
            keys crate::Keys;
            join crate::pairs;
        }
    }

    #[test]
    fn configured_ra_round_trips_without_an_intermediate_language() {
        let source = source();
        let tokens = quote! {
            inputs { r#R => crate::rows; }
            join crate::pairs;
            storage crate::storage;
            results {}
            keys crate::Keys;
            access crate::access;
            iterator crate::iterator;
            key_provider crate::key_provider;
            materialize crate::rows;
            scan crate::reduce;
            #source
        };
        let input: PhysicalInput = syn::parse2(tokens.clone()).unwrap();
        input.options().unwrap();
        assert_eq!(input.to_token_stream().to_string(), tokens.to_string());
    }

    #[test]
    fn missing_required_directives_are_configuration_errors() {
        let source = source();
        let input: PhysicalInput = syn::parse_quote!(#source);
        let error = input.options().err().unwrap();
        assert!(
            error
                .to_string()
                .contains("missing physical directive `storage`")
        );
    }

    #[test]
    fn unknown_directives_report_the_declared_keyword_alternatives() {
        let source = source();
        let required = required();
        let error = syn::parse2::<PhysicalInput>(quote! {
            #required mystery crate::other; #source
        })
        .unwrap_err();
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("expected one of"), "{diagnostic}");
        for keyword in ["storage", "inputs", "scans", "key_sets", "joins"] {
            assert!(diagnostic.contains(keyword), "{diagnostic}");
        }
    }

    #[test]
    fn duplicate_scalar_and_scope_directives_are_rejected() {
        let source = source();
        let required = required();
        for duplicate in [
            quote!(storage other::storage;),
            quote!(inputs {} inputs {}),
            quote!(joins {} joins {}),
        ] {
            let input: PhysicalInput = syn::parse2(quote!(#required #duplicate #source)).unwrap();
            let error = input.options().err().unwrap();
            assert!(error.to_string().contains("duplicate physical directive"));
        }
    }

    #[test]
    fn binding_duplicates_are_normalized_within_each_scope() {
        let source = source();
        let required = required();
        let duplicate: PhysicalInput = syn::parse_quote! {
            #required inputs { R => a::rows; r#R => b::rows; } #source
        };
        assert!(
            duplicate
                .options()
                .err()
                .unwrap()
                .to_string()
                .contains("duplicate physical binding")
        );

        let separate: PhysicalInput = syn::parse_quote! {
            #required
            inputs { R => a::rows; }
            results { R => b::rows; }
            #source
        };
        let options = separate.options().unwrap();
        assert!(options.inputs.contains_key("R"));
        assert!(options.results.contains_key("R"));
        assert!(options.scans.is_empty());
        assert!(options.key_sets.is_empty());
        assert!(options.joins.is_empty());
    }
}
