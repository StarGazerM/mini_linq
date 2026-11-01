//! Recorded planning evidence: base-relation row counts and asserted keys,
//! with the provenance of the numbers.
//!
//! ```text
//! evidence from "tpch-gpu sf1 manifest" {
//!     count Lineitem = 6001215;
//!     key Orders [o_orderkey];
//! }
//! ```
//!
//! Evidence never carries semantics: it only chooses among plans that WF-VO
//! and the ordering checkers accept on their own, so a wrong count or key
//! changes which checked plan runs, never an answer. Subjects are declared
//! relations; key columns are that relation's declared column names. The
//! grammar is the `count`/`key` subset of Sequence evidence.

use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitInt, LitStr, Token, braced, bracketed};

use std::collections::{BTreeMap, BTreeSet};

pub mod kw {
    syn::custom_keyword!(evidence);
    syn::custom_keyword!(from);
    syn::custom_keyword!(count);
    syn::custom_keyword!(key);
}

/// Counts and keys supplied to the planner, and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub provenance: LitStr,
    pub facts: Vec<Fact>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fact {
    /// Base rows of a declared relation (an estimate source, not a premise).
    Count { relation: Ident, rows: u64 },
    /// The relation has at most one row per value of these declared columns.
    Key {
        relation: Ident,
        columns: Vec<Ident>,
    },
}

impl Evidence {
    /// The recorded row count of a relation; absence is unknown.
    pub fn count(&self, relation: &str) -> Option<u64> {
        self.facts.iter().find_map(|fact| match fact {
            Fact::Count {
                relation: name,
                rows,
            } if symbol_name(name) == relation => Some(*rows),
            _ => None,
        })
    }

    /// The recorded keys of a relation as declared column positions.
    pub fn keys(&self, relation: &cq_syntax::RelationDecl) -> Vec<Vec<usize>> {
        let name = symbol_name(&relation.name);
        self.facts
            .iter()
            .filter_map(|fact| match fact {
                Fact::Key {
                    relation: subject,
                    columns,
                } if symbol_name(subject) == name => Some(
                    columns
                        .iter()
                        .map(|column| {
                            relation
                                .columns
                                .iter()
                                .position(|declared| declared.name == *column)
                                .expect("checked evidence key column")
                        })
                        .collect(),
                ),
                _ => None,
            })
            .collect()
    }

    /// Formal rule WF-Evidence: Evidence names its provenance, declared relations and their declared columns, with one count per relation and distinct nonempty keys.
    pub fn check(&self, program: &crate::cq::Program) -> syn::Result<()> {
        if self.provenance.value().trim().is_empty() {
            return Err(syn::Error::new_spanned(
                &self.provenance,
                "evidence must record its provenance",
            ));
        }
        let relations: BTreeMap<_, _> = program
            .inputs()
            .map(|declaration| (symbol_name(&declaration.name), declaration))
            .collect();
        let mut counted = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for fact in &self.facts {
            let (Fact::Count { relation, .. } | Fact::Key { relation, .. }) = fact;
            let Some(declaration) = relations.get(&symbol_name(relation)) else {
                return Err(syn::Error::new_spanned(
                    relation,
                    "evidence names no declared relation",
                ));
            };
            match fact {
                Fact::Count { relation, .. } => {
                    if !counted.insert(symbol_name(relation)) {
                        return Err(syn::Error::new_spanned(
                            relation,
                            "evidence records two counts for one relation",
                        ));
                    }
                }
                Fact::Key { relation, columns } => {
                    let mut names = BTreeSet::new();
                    for column in columns {
                        if !declaration
                            .columns
                            .iter()
                            .any(|declared| declared.name == *column)
                        {
                            return Err(syn::Error::new_spanned(
                                column,
                                "evidence key column is not declared by its relation",
                            ));
                        }
                        if !names.insert(symbol_name(column)) {
                            return Err(syn::Error::new_spanned(
                                column,
                                "duplicate evidence key column",
                            ));
                        }
                    }
                    if names.is_empty() || !keys.insert((symbol_name(relation), names)) {
                        return Err(syn::Error::new_spanned(
                            relation,
                            "evidence keys must be nonempty and distinct",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn parse_optional(input: ParseStream<'_>) -> syn::Result<Option<Evidence>> {
    if input.peek(kw::evidence) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

impl Parse for Evidence {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::evidence>()?;
        input.parse::<kw::from>()?;
        let provenance = input.parse()?;
        let body;
        braced!(body in input);
        let mut facts = Vec::new();
        while !body.is_empty() {
            facts.push(body.parse()?);
        }
        Ok(Self { provenance, facts })
    }
}

impl Parse for Fact {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        let fact = if lookahead.peek(kw::count) {
            input.parse::<kw::count>()?;
            let relation = input.parse()?;
            input.parse::<Token![=]>()?;
            Self::Count {
                relation,
                rows: input.parse::<LitInt>()?.base10_parse()?,
            }
        } else if lookahead.peek(kw::key) {
            input.parse::<kw::key>()?;
            let relation = input.parse()?;
            let body;
            bracketed!(body in input);
            Self::Key {
                relation,
                columns: Punctuated::<Ident, Token![,]>::parse_terminated(&body)?
                    .into_iter()
                    .collect(),
            }
        } else {
            return Err(lookahead.error());
        };
        input.parse::<Token![;]>()?;
        Ok(fact)
    }
}

impl ToTokens for Evidence {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let (provenance, facts) = (&self.provenance, &self.facts);
        tokens.extend(quote! { evidence from #provenance { #(#facts)* } });
    }
}

impl ToTokens for Fact {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Count { relation, rows } => {
                let rows = LitInt::new(&rows.to_string(), proc_macro2::Span::call_site());
                quote! { count #relation = #rows; }
            }
            Self::Key { relation, columns } => quote! { key #relation [#(#columns),*]; },
        });
    }
}
