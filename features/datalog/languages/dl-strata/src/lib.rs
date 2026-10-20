//! DL-Strata records exact dependency-ordered SCCs over DL-Source.
//! Its meaning is sequential composition of the source component fixpoints.
pub use cq_lang_datalog_source as source;
use source::{BodyItem, Graph, ensure};
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::BTreeMap;
use syn::{Ident, Token, parse::{Parse, ParseStream}};
pub mod kw { syn::custom_keyword!(strata); syn::custom_keyword!(component); }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program { pub source: source::Program, pub components: Vec<Vec<Ident>> }
impl Parse for Program {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let source = input.parse()?;
        input.parse::<kw::strata>()?;
        let body; syn::braced!(body in input);
        let mut components = Vec::new();
        while !body.is_empty() {
            body.parse::<kw::component>()?;
            let members; syn::bracketed!(members in body);
            components.push(members.parse_terminated(Ident::parse, Token![,])?.into_iter().collect());
            body.parse::<Token![;]>()?;
        }
        Ok(Self { source, components })
    }
}
impl ToTokens for Program {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let source = &self.source;
        let components = self.components.iter().map(|m| quote!(component [#(#m),*];));
        tokens.extend(quote!(#source strata { #(#components)* }));
    }
}
/// Formal rule DL-Strata: Certify the exact SCC cover and dependency order of the visible graph.
fn check_strata(program: &Program, g: &Graph) -> syn::Result<()> {
    let p = &program.source;
    let components = &program.components;
    let mut positions = BTreeMap::new();
    for (c, members) in components.iter().enumerate() {
        ensure(!members.is_empty(), &p.name, "empty SCC")?;
        for member in members {
            let i = g.index(member)?;
            ensure(
                p.declarations[i].value.is_none() && positions.insert(i, c).is_none(),
                member,
                "SCCs must cover IDB relations exactly",
            )?;
        }
    }
    for (i, d) in p.declarations.iter().enumerate() {
        ensure(
            d.value.is_some() || positions.contains_key(&i),
            d,
            "missing IDB SCC",
        )?;
    }
    for (&i, &ci) in &positions {
        for (&j, &cj) in &positions {
            ensure(
                (ci == cj) == (i == j || (g.reach[i][j] && g.reach[j][i])),
                &p.name,
                "recorded component is not an SCC",
            )?;
        }
    }
    for &(h, b, strict) in &g.edges {
        if let (Some(&h), Some(&b)) = (positions.get(&h), positions.get(&b)) {
            ensure(
                if strict { b < h } else { b <= h },
                &p.name,
                "SCCs are not in dependency order",
            )?;
        }
    }
    Ok(())
}
#[derive(Clone, Debug)]
pub struct Analysis {
    pub graph: Graph,
    pub positions: BTreeMap<String, usize>,
    pub dependencies: Vec<(usize, usize, bool)>,
    pub components: Vec<Vec<usize>>,
    pub component: Vec<Option<usize>>,
    pub changing_positions: Vec<Vec<usize>>,
}
pub mod contract {
    use super::*;
    pub fn check(program: &Program) -> syn::Result<Analysis> {
        let source::Analysis {graph, positions, dependencies} = source::contract::check(&program.source)?;
        check_strata(program, &graph)?;
        let components: Vec<Vec<usize>> = program
            .components
            .iter()
            .map(|members| {
                members
                    .iter()
                    .map(|id| positions[&symbol_name(id)])
                    .collect()
            })
            .collect();
        let mut component = vec![None; program.source.declarations.len()];
        for (index, members) in components.iter().enumerate() {
            for &member in members {
                component[member] = Some(index);
            }
        }
        let changing_positions = program.source
            .rules
            .iter()
            .map(|query| {
                let head = positions[&symbol_name(&query.head.relation)];
                query
                    .body
                    .iter()
                    .enumerate()
                    .filter_map(|(index, item)| {
                        if let BodyItem::Positive { atom } = item {
                            let input = positions[&symbol_name(&atom.relation)];
                            if component[head].is_some() && component[head] == component[input] {
                                return Some(index);
                            }
                        }
                        None
                    })
                    .collect()
            })
            .collect();
        Ok(Analysis { graph, positions, dependencies, components, component, changing_positions })
    }
}
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?; contract::check(target)?;
    ensure(source == target, target, "rewrite changed recorded strata")
}
impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
