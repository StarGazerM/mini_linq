//! Producer-owned planning side tables transported beside, never inside, IR.
use std::collections::BTreeMap;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Ident, Token, parse::{Parse, ParseStream, Parser}};

#[derive(Clone, Debug, Default)]
pub struct Planning {
    pub reads: BTreeMap<String, Vec<usize>>,
    pub steps: BTreeMap<String, Vec<usize>>,
}

impl Parse for Planning {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<super::kw::planning_tables>()?;
        let body;
        syn::braced!(body in input);
        let table = |expected: &str| -> syn::Result<BTreeMap<String, Vec<usize>>> {
            let name: Ident = body.parse()?;
            if name != expected { return Err(syn::Error::new_spanned(name, "unexpected planning table")); }
            let entries;
            syn::braced!(entries in body);
            let mut result = BTreeMap::new();
            while !entries.is_empty() {
                let key: syn::LitStr = entries.parse()?;
                let path;
                syn::bracketed!(path in entries);
                let value = path.parse_terminated(|input| input.parse::<syn::LitInt>()?.base10_parse(), Token![,])?.into_iter().collect();
                entries.parse::<Token![;]>()?;
                if result.insert(key.value(), value).is_some() {
                    return Err(syn::Error::new_spanned(key, "duplicate planning table key"));
                }
            }
            Ok(result)
        };
        let reads = table("reads")?;
        let steps = table("steps")?;
        if !body.is_empty() { return Err(body.error("unexpected planning table contents")); }
        Ok(Self { reads, steps })
    }
}
impl ToTokens for Planning {
    fn to_tokens(&self, output: &mut TokenStream) {
        fn entries(table: &BTreeMap<String, Vec<usize>>) -> impl Iterator<Item = TokenStream> + '_ {
            table.iter().map(|(key, path)| {
                let path = path.iter().map(|index| syn::Index::from(*index));
                quote!(#key [#(#path),*];)
            })
        }
        let reads = entries(&self.reads);
        let steps = entries(&self.steps);
        output.extend(quote!(planning_tables { reads { #(#reads)* } steps { #(#steps)* } }));
    }
}

struct Selector {
    relation: Ident,
    occurrence: Option<cq_syntax::Occurrence>,
}
impl Parse for Selector {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let relation = input.parse()?;
        let occurrence = if input.peek(Token![@]) {
            input.parse::<Token![@]>()?;
            Some(input.parse()?)
        } else { None };
        Ok(Self { relation, occurrence })
    }
}

/// Formal rule Smallest-Resolve: Resolve only this invocation's authored selectors through carried first-read and Steps tables; refuse unresolved or ambiguous selectors.
pub fn resolve_smallest(arguments: TokenStream, planning: Option<&Planning>) -> syn::Result<TokenStream> {
    let parser = |input: ParseStream<'_>| -> syn::Result<TokenStream> {
        let mut sites = Vec::new();
        while !input.is_empty() {
            let keyword: Ident = input.parse()?;
            if keyword != "smallest" { return Err(syn::Error::new_spanned(keyword, "expected smallest site")); }
            let content;
            syn::parenthesized!(content in input);
            let selectors = content.parse_terminated(Selector::parse, Token![,])?;
            let names = selectors.iter().map(|selector| {
                let fail = || syn::Error::new_spanned(&selector.relation, "Smallest-Resolve: unresolved or ambiguous selector");
                let relation = cq_syntax::symbol_name(&selector.relation);
                let key = match &selector.occurrence {
                    Some(path) => format!("{relation}@{path}"),
                    None => relation,
                };
                let Some(planning) = planning else {
                    return if selector.occurrence.is_none() { Ok(selector.relation.clone()) } else { Err(fail()) };
                };
                let Some(read) = planning.reads.get(&key) else {
                    return if selector.occurrence.is_none() && planning.steps.contains_key(&key) {
                        Ok(selector.relation.clone())
                    } else { Err(fail()) };
                };
                let mut names = planning.steps.iter().filter(|(_, path)| *path == read).map(|(name, _)| name);
                let name = names.next().ok_or_else(fail)?;
                if names.next().is_some() { return Err(fail()); }
                syn::parse_str::<Ident>(name).map_err(|_| fail())
            }).collect::<syn::Result<Vec<_>>>()?;
            sites.push(quote!(smallest(#(#names),*)));
            if !input.is_empty() { input.parse::<Token![,]>()?; }
        }
        Ok(quote!(#(#sites),*))
    };
    parser.parse2(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_arguments_resolve_from_roundtripped_nested_tables() {
        let planning: Planning = syn::parse_quote!(planning_tables {
            reads { "R" [1]; "R@0" [1]; "S@2.1.0" [3, 1, 2]; }
            steps { "r0" [1]; "r7" [3, 1, 2]; }
        });
        let planning: Planning = syn::parse2(planning.to_token_stream()).unwrap();
        let resolved = resolve_smallest(quote!(smallest(R, S@2.1.0)), Some(&planning)).unwrap();
        assert_eq!(resolved.to_string(), quote!(smallest(r0, r7)).to_string());
        let raw = resolve_smallest(quote!(smallest(r#R, r#S@2.1.0)), Some(&planning)).unwrap();
        assert_eq!(raw.to_string(), resolved.to_string());
        let raw_definition = resolve_smallest(quote!(smallest(r#r0)), Some(&planning)).unwrap();
        assert_eq!(raw_definition.to_string(), quote!(smallest(r#r0)).to_string());
        for arguments in [
            quote!(smallest(Missing)), quote!(smallest(R@1)),
            quote!(smallest(S)), quote!(smallest(S@2.0.0)),
            quote!(smallest(S@2.1)), quote!(smallest(S@2.1.0(x))),
        ] {
            assert!(resolve_smallest(arguments, Some(&planning)).is_err());
        }
        assert!(resolve_smallest(quote!(smallest(R@0)), None).is_err());
        assert_eq!(resolve_smallest(quote!(smallest(r0, r7)), None).unwrap().to_string(),
            quote!(smallest(r0, r7)).to_string());
    }

    #[test]
    fn missing_or_ambiguous_inverse_steps_refuse() {
        let mut planning = Planning::default();
        planning.reads.insert("R@0".into(), vec![1]);
        assert!(resolve_smallest(quote!(smallest(R@0)), Some(&planning)).is_err());
        planning.steps.insert("r0".into(), vec![1]);
        planning.steps.insert("r1".into(), vec![1]);
        assert!(resolve_smallest(quote!(smallest(R@0)), Some(&planning)).is_err());
    }
}
