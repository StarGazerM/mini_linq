//! An independent theta-join surface lowered to a complete IX program.
use cq_lang_ix::{
    self as ix, ra,
    syntax::{self, symbol_name},
};
use quote::quote;
use std::collections::BTreeSet;
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
};
mod kw {
    syn::custom_keyword!(theta_join);
    syn::custom_keyword!(with);
    syn::custom_keyword!(output);
}
struct Binding {
    host: Ident,
    mapping: syntax::CommaList<ra::AttributeRename>,
}
impl Parse for Binding {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let host = input.parse()?;
        let content;
        syn::braced!(content in input);
        let mapping = syntax::CommaList::parse_terminated(&content)?;
        Ok(Self { host, mapping })
    }
}
pub struct Query {
    ra: ra::Program,
}
impl Parse for Query {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let declarations = syntax::parse_declarations(input)?;
        input.parse::<kw::theta_join>()?;
        let left: Binding = input.parse()?;
        input.parse::<kw::with>()?;
        let right: Binding = input.parse()?;
        input.parse::<Token![if]>()?;
        let condition: syn::Expr = input.parse()?;
        input.parse::<Token![;]>()?;
        input.parse::<kw::output>()?;
        let head: syntax::Atom = input.parse()?;
        input.parse::<Token![;]>()?;
        let (lh, lm, rh, rm) = (&left.host, &left.mapping, &right.host, &right.mapping);
        let kept = head.variables();
        let ra: ra::Program = syn::parse2(quote! {
            relational {
                #(#declarations)*
                l = rename #lh { #lm };
                r = rename #rh { #rm };
                j = natural_join l with r;
                s = select j if (#condition);
                p = project s keep { #(#kept),* };
                output p as #head;
            }
        })?;
        let info = ra::contract::check(&ra)?;
        for (column, ty) in &info["l"].types {
            if info["r"].types.get(column).is_some_and(|right| right != ty) {
                return Err(syn::Error::new_spanned(
                    &head,
                    format!("theta_join attribute `{column}` has unequal types"),
                ));
            }
        }
        Ok(Self { ra })
    }
}
impl Query {
    /// Derive demands here; downstream compilation uses the standard IX API.
    pub fn indexes(&self) -> syn::Result<ix::Program> {
        let info = ra::contract::check(&self.ra)?;
        let mut indexes = syntax::SemicolonList::new();
        let mut seen = BTreeSet::new();
        for d in &self.ra.plan.definitions {
            let ra::Operator::NaturalJoin(join) = &d.operator else {
                continue;
            };
            let columns: Vec<_> = info[&symbol_name(&join.right)]
                .heading
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    info[&symbol_name(&join.left)]
                        .heading
                        .contains(c)
                        .then_some(i)
                })
                .collect();
            if seen.insert((symbol_name(&join.right), columns.clone())) {
                indexes.push(ix::IndexDefinition {
                    name: Ident::new(&format!("theta_key_{}", indexes.len()), join.right.span()),
                    eq_token: Default::default(),
                    requirement: ix::IndexRequirement {
                        at_token: Default::default(),
                        relation: join.right.clone(),
                        bracket_token: Default::default(),
                        columns: columns.into_iter().map(syn::Index::from).collect(),
                    },
                });
                indexes.push_punct(Default::default());
            }
        }
        let program = ix::Program {
            ra: self.ra.clone(),
            indexes_token: Default::default(),
            indexes_brace: Default::default(),
            indexes,
        };
        ix::contract::check(&program)?;
        Ok(program)
    }
}
