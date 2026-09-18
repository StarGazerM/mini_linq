//! Syntax-preserving administrative macro arguments; query semantics stay in cq-ir.

use std::collections::BTreeMap;

use cq_ir::{relational_plan as ra, symbol_name};
use cq_relation_lowering::configured::{ObservedOptions, RelationOptions};
use quote::ToTokens;
use syn::{parse::Parse, token};

mod kw {
    syn::custom_keyword!(counter_storage);
    syn::custom_keyword!(access);
    syn::custom_keyword!(result_rows);
    syn::custom_keyword!(candidates);
    syn::custom_keyword!(keys);
    syn::custom_keyword!(pairs);
    syn::custom_keyword!(inputs);
    syn::custom_keyword!(result_bindings);
    syn::custom_keyword!(candidate_bindings);
    syn::custom_keyword!(key_sets);
    syn::custom_keyword!(pair_bindings);
}

/// Administrative choices for the direct RA-to-Access edge. Their contextual
/// validity is checked separately from their derived parser.
#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub struct RelationInput {
    #[syn(repeat, peek = syn::Ident)]
    directives: Vec<Directive>,
    pub source: ra::Module,
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
enum Directive {
    #[parse(peek = kw::counter_storage)]
    CounterStorage(PathDirective<kw::counter_storage>),
    #[parse(peek = kw::access)]
    Access(PathDirective<kw::access>),
    #[parse(peek = kw::result_rows)]
    ResultRows(PathDirective<kw::result_rows>),
    #[parse(peek = kw::candidates)]
    Candidates(PathDirective<kw::candidates>),
    #[parse(peek = kw::keys)]
    Keys(PathDirective<kw::keys>),
    #[parse(peek = kw::pairs)]
    Pairs(PathDirective<kw::pairs>),
    #[parse(peek = kw::inputs)]
    Inputs(MapDirective<kw::inputs>),
    #[parse(peek = kw::result_bindings)]
    ResultBindings(MapDirective<kw::result_bindings>),
    #[parse(peek = kw::candidate_bindings)]
    CandidateBindings(MapDirective<kw::candidate_bindings>),
    #[parse(peek = kw::key_sets)]
    KeySets(MapDirective<kw::key_sets>),
    #[parse(peek = kw::pair_bindings)]
    PairBindings(MapDirective<kw::pair_bindings>),
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

impl RelationInput {
    pub fn options(&self) -> syn::Result<RelationOptions> {
        let (options, counter_storage) = self.configuration()?;
        if let Some(counter_storage) = counter_storage {
            return Err(syn::Error::new_spanned(
                counter_storage,
                "counter storage belongs to an observed physical host",
            ));
        }
        Ok(options)
    }
    pub fn observed_options(&self) -> syn::Result<ObservedOptions> {
        let (relations, counter_storage) = self.configuration()?;
        let counter_storage = counter_storage.ok_or_else(|| {
            syn::Error::new_spanned(
                &self.source,
                "observed physical host requires counter `counter_storage`",
            )
        })?;
        Ok(ObservedOptions {
            relations,
            counter_storage,
        })
    }
    fn configuration(&self) -> syn::Result<(RelationOptions, Option<syn::Path>)> {
        let mut scalar = BTreeMap::new();
        let mut maps = BTreeMap::new();
        for directive in &self.directives {
            let scalar_entry = match directive {
                Directive::CounterStorage(value) => Some(("counter_storage", &value.path)),
                Directive::Access(value) => Some(("access", &value.path)),
                Directive::ResultRows(value) => Some(("result_rows", &value.path)),
                Directive::Candidates(value) => Some(("candidates", &value.path)),
                Directive::Keys(value) => Some(("keys", &value.path)),
                Directive::Pairs(value) => Some(("pairs", &value.path)),
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
                Directive::ResultBindings(value) => ("result_bindings", &value.bindings),
                Directive::CandidateBindings(value) => ("candidate_bindings", &value.bindings),
                Directive::KeySets(value) => ("key_sets", &value.bindings),
                Directive::PairBindings(value) => ("pair_bindings", &value.bindings),
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
        let counter_storage = scalar.remove("counter_storage");
        Ok((
            RelationOptions {
                access: scalar.remove("access"),
                result_rows: scalar.remove("result_rows"),
                candidates: scalar.remove("candidates"),
                keys: scalar.remove("keys"),
                pairs: scalar.remove("pairs"),
                inputs: maps.remove("inputs").unwrap_or_default(),
                result_bindings: maps.remove("result_bindings").unwrap_or_default(),
                candidate_bindings: maps.remove("candidate_bindings").unwrap_or_default(),
                key_sets: maps.remove("key_sets").unwrap_or_default(),
                pair_bindings: maps.remove("pair_bindings").unwrap_or_default(),
            },
            counter_storage,
        ))
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
            access crate::access;
            result_rows crate::rows;
            candidates crate::reduce;
            keys crate::Keys;
            pairs crate::pairs;
        }
    }

    #[test]
    fn configured_ra_round_trips_without_an_intermediate_language() {
        let source = source();
        let tokens = quote! {
            inputs { r#R => crate::rows; }
            pairs crate::pairs;
            result_bindings {}
            keys crate::Keys;
            access crate::access;
            result_rows crate::rows;
            candidates crate::reduce;
            #source
        };
        let input: RelationInput = syn::parse2(tokens.clone()).unwrap();
        input.options().unwrap();
        assert_eq!(input.to_token_stream().to_string(), tokens.to_string());
    }

    #[test]
    fn unused_defaults_are_absent_from_administrative_configuration() {
        let source = source();
        let input: RelationInput = syn::parse_quote!(#source);
        let options = input.options().unwrap();
        assert!(options.access.is_none());
        assert!(options.result_rows.is_none());
        assert!(options.candidates.is_none());
        assert!(options.keys.is_none());
        assert!(options.pairs.is_none());
    }

    #[test]
    fn unknown_directives_report_the_declared_keyword_alternatives() {
        let source = source();
        let required = required();
        let error = syn::parse2::<RelationInput>(quote! {
            #required mystery crate::other; #source
        })
        .unwrap_err();
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("expected one of"), "{diagnostic}");
        for keyword in [
            "counter_storage",
            "inputs",
            "candidate_bindings",
            "key_sets",
            "pair_bindings",
        ] {
            assert!(diagnostic.contains(keyword), "{diagnostic}");
        }
    }

    #[test]
    fn duplicate_scalar_and_scope_directives_are_rejected() {
        let source = source();
        let required = required();
        for duplicate in [
            quote!(access other::access;),
            quote!(inputs {} inputs {}),
            quote!(pair_bindings {} pair_bindings {}),
        ] {
            let input: RelationInput = syn::parse2(quote!(#required #duplicate #source)).unwrap();
            let error = input.options().err().unwrap();
            assert!(error.to_string().contains("duplicate physical directive"));
        }
    }

    #[test]
    fn observer_configuration_is_owned_by_its_host() {
        let source = source();
        let required = required();
        let plain: RelationInput = syn::parse_quote!(#required #source);
        assert!(plain.options().is_ok());
        assert!(plain.observed_options().is_err());
        let observed: RelationInput =
            syn::parse_quote!(counter_storage crate::counters; #required #source);
        assert!(observed.options().is_err());
        assert!(observed.observed_options().is_ok());
    }

    #[test]
    fn binding_duplicates_are_normalized_within_each_scope() {
        let source = source();
        let required = required();
        let duplicate: RelationInput = syn::parse_quote! {
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

        let separate: RelationInput = syn::parse_quote! {
            #required
            inputs { R => a::rows; }
            result_bindings { R => b::rows; }
            #source
        };
        let options = separate.options().unwrap();
        assert!(options.inputs.contains_key("R"));
        assert!(options.result_bindings.contains_key("R"));
        assert!(options.candidate_bindings.is_empty());
        assert!(options.key_sets.is_empty());
        assert!(options.pair_bindings.is_empty());
    }
}
