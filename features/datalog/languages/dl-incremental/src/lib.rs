//! DL-Incremental records a semi-naive changing-position cover.
//! Delta rounds read frozen lower components and preserve written Rust clauses.
//! Each variant selects one changing positive occurrence; every such occurrence
//! is covered at least once. Repeated variants are admissible for set relations.
//! Only `seminaive_covering` is supported; no naive or exact scheme is implied.
//! An occurrence-order certificate may permute the first contiguous positive
//! prefix. Delta positions always refer to source order; erasure restores that
//! order before comparison and never crosses a Rust or negative clause.
use cq_datalog_support::{Fresh, ensure};
pub use cq_lang_datalog_strata as strata;
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use source::{BodyItem, Graph, Query, QueryChecker};
use std::collections::{BTreeMap, BTreeSet};
pub use strata::source;
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
};
pub mod kw {
    syn::custom_keyword!(incremental);
    syn::custom_keyword!(scheme);
    syn::custom_keyword!(seminaive_covering);
    syn::custom_keyword!(delta);
    syn::custom_keyword!(variant);
    syn::custom_keyword!(seed);
    syn::custom_keyword!(order);
    syn::custom_keyword!(authored);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub strata: strata::Program,
    pub variants: Vec<Variant>,
    pub deltas: Vec<Delta>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub rule: usize,
    pub delta: Option<usize>,
    /// Source-body positions in execution order; only the first positive prefix may move.
    pub order: Vec<usize>,
    /// The order is authored advice, not a driver policy: the hole's CQ plan
    /// keeps it as an explicit atom order instead of choosing its own.
    pub authored: bool,
    pub query: Query,
}
/// A fresh delta relation inheriting the named IDB's ordered columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    pub relation: Ident,
    pub name: Ident,
}
fn number(input: ParseStream<'_>) -> syn::Result<usize> {
    input.parse::<syn::LitInt>()?.base10_parse()
}
impl Parse for Program {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let strata: strata::Program = input.parse()?;
        input.parse::<kw::incremental>()?;
        let body;
        syn::braced!(body in input);
        body.parse::<kw::scheme>()?;
        body.parse::<kw::seminaive_covering>()?;
        body.parse::<Token![;]>()?;
        let mut deltas = Vec::new();
        while body.peek(kw::delta) {
            body.parse::<kw::delta>()?;
            let relation = body.parse()?;
            body.parse::<Token![=]>()?;
            let name = body.parse()?;
            body.parse::<Token![;]>()?;
            deltas.push(Delta { relation, name });
        }
        let mut variants = Vec::new();
        while !body.is_empty() {
            body.parse::<kw::variant>()?;
            let rule = number(&body)?;
            let delta = if body.peek(kw::seed) {
                body.parse::<kw::seed>()?;
                None
            } else {
                body.parse::<kw::delta>()?;
                Some(number(&body)?)
            };
            let order = if body.peek(kw::order) {
                body.parse::<kw::order>()?;
                let positions;
                syn::bracketed!(positions in body);
                positions
                    .parse_terminated(syn::LitInt::parse, Token![,])?
                    .iter()
                    .map(syn::LitInt::base10_parse)
                    .collect::<syn::Result<Vec<_>>>()?
            } else {
                (0..strata
                    .source
                    .rules
                    .get(rule)
                    .map_or(0, |query| query.body.len()))
                    .collect()
            };
            let authored = body.peek(kw::authored);
            if authored {
                body.parse::<kw::authored>()?;
            }
            let query;
            syn::braced!(query in body);
            variants.push(Variant {
                rule,
                delta,
                order,
                authored,
                query: query.parse()?,
            });
            if !query.is_empty() {
                return Err(query.error("unexpected variant text"));
            }
        }
        Ok(Self {
            strata,
            variants,
            deltas,
        })
    }
}
impl ToTokens for Program {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let strata = &self.strata;
        let variants = &self.variants;
        let deltas = &self.deltas;
        let variants = variants.iter().map(|v| {
            let rule = syn::LitInt::new(&v.rule.to_string(), proc_macro2::Span::call_site());
            let query = &v.query;
            let kind = v
                .delta
                .map(|p| {
                    let p = syn::LitInt::new(&p.to_string(), proc_macro2::Span::call_site());
                    quote!(delta #p)
                })
                .unwrap_or(quote!(seed));
            let order = v.order.iter().map(|position| {
                syn::LitInt::new(&position.to_string(), proc_macro2::Span::call_site())
            });
            let authored = v.authored.then(|| quote!(authored));
            quote!(variant #rule #kind order [#(#order),*] #authored { #query })
        });
        let deltas = deltas.iter().map(|d| {
            let Delta { relation, name } = d;
            quote!(delta #relation = #name;)
        });
        tokens.extend(
            quote!(#strata incremental { scheme seminaive_covering; #(#deltas)* #(#variants)* }),
        );
    }
}
/// Check a supplied permutation, without replaying the order-selection heuristic.
pub fn variant_order<'a>(query: &Query, order: &'a [usize]) -> syn::Result<&'a [usize]> {
    let length = query.body.len();
    ensure(
        order.len() == length
            && order.iter().copied().collect::<BTreeSet<_>>() == (0..length).collect(),
        query,
        "variant order must cover every source occurrence exactly once",
    )?;
    let first = query
        .body
        .iter()
        .position(|item| matches!(item, BodyItem::Positive { .. }))
        .unwrap_or(length);
    let end = first
        + query
            .body
            .iter()
            .skip(first)
            .take_while(|item| matches!(item, BodyItem::Positive { .. }))
            .count();
    ensure(
        order.iter().enumerate().all(|(position, &source)| {
            if (first..end).contains(&position) {
                (first..end).contains(&source)
            } else {
                position == source
            }
        }),
        query,
        "variant order must preserve Before constraints outside the first positive prefix",
    )?;
    Ok(order)
}
/// Formal rule DL-Delta: check variant erasure and coverage of changing positions.
fn check_variants(program: &Program, analysis: &Analysis) -> syn::Result<()> {
    let p = &program.strata.source;
    let variants = &program.variants;
    let mut seen = BTreeSet::new();
    let mut declarations = p.declarations.clone();
    for delta in &program.deltas {
        let mut declaration =
            p.declarations[analysis.positions[&symbol_name(&delta.relation)]].clone();
        declaration.name = delta.name.clone();
        declarations.push(declaration);
    }
    let mut checker = QueryChecker::new(p, &declarations);
    for v in variants {
        ensure(v.rule < p.rules.len(), p, "invalid variant rule index")?;
        seen.insert((v.rule, v.delta));
        let positions = &analysis.changing_positions[v.rule];
        ensure(
            match v.delta {
                Some(i) => positions.contains(&i),
                None => positions.is_empty(),
            },
            p,
            "incorrect seed/delta classification",
        )?;
        // Check complete declared schemas and CQ scope independently of erasure.
        checker.check(&v.query)?;
        let order = variant_order(&p.rules[v.rule], &v.order)?;
        ensure(
            v.query.body.len() == order.len(),
            &v.query,
            "variant changed body coverage",
        )?;
        let mut erased = v.query.clone();
        if let Some(i) = v.delta {
            let original = &p.rules[v.rule].body[i];
            let BodyItem::Positive { atom: original } = original else {
                unreachable!()
            };
            let position = order
                .iter()
                .position(|&position| position == i)
                .expect("complete order");
            let Some(BodyItem::Positive { atom }) = erased.body.iter_mut().nth(position) else {
                return Err(syn::Error::new_spanned(p, "missing delta atom"));
            };
            ensure(
                atom.relation == analysis.deltas[&symbol_name(&original.relation)],
                atom.clone(),
                "delta read name changed",
            )?;
            atom.relation = original.relation.clone();
        }
        let mut body: Vec<_> = erased.body.into_iter().collect();
        let mut positions = order.to_vec();
        for target in 0..positions.len() {
            while positions[target] != target {
                let source = positions[target];
                body.swap(target, source);
                positions.swap(target, source);
            }
        }
        erased.body = body.into_iter().collect();
        let expected = &p.rules[v.rule];
        ensure(
            &erased == expected,
            &v.query,
            "variant erasure differs from the rule",
        )?;
    }
    for i in 0..p.rules.len() {
        let positions = &analysis.changing_positions[i];
        if positions.is_empty() {
            ensure(seen.contains(&(i, None)), p, "missing seed variant")?;
        }
        for position in positions {
            ensure(
                seen.contains(&(i, Some(*position))),
                p,
                "missing changing-position variant",
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
    pub deltas: BTreeMap<String, Ident>,
}
pub mod contract {
    use super::*;
    pub fn check(program: &Program) -> syn::Result<Analysis> {
        let strata::Analysis {
            graph,
            positions,
            dependencies,
            components,
            component,
            changing_positions,
        } = strata::contract::check(&program.strata)?;
        let p = &program.strata.source;
        let mut deltas = BTreeMap::new();
        let mut fresh = Fresh::new(p);
        for delta in &program.deltas {
            let index = graph.index(&delta.relation)?;
            ensure(
                p.declarations[index].value.is_none(),
                &delta.relation,
                "delta declarations must name IDB relations",
            )?;
            ensure(
                deltas
                    .insert(symbol_name(&delta.relation), delta.name.clone())
                    .is_none(),
                &delta.relation,
                "duplicate delta declaration for relation",
            )?;
            ensure(
                fresh.reserve(&delta.name),
                &delta.name,
                "delta relation name must be fresh in the complete source",
            )?;
        }
        for declaration in &p.declarations {
            ensure(
                declaration.value.is_some() || deltas.contains_key(&symbol_name(&declaration.name)),
                declaration,
                "missing IDB delta declaration",
            )?;
        }
        let analysis = Analysis {
            graph,
            positions,
            dependencies,
            components,
            component,
            changing_positions,
            deltas,
        };
        check_variants(program, &analysis)?;
        Ok(analysis)
    }
}
/// Preserve the immediate Strata program while independently admitting decisions.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    check_rewrite_admitted(source, target)
}
// The stage runner has already checked both language contracts.
fn check_rewrite_admitted(source: &Program, target: &Program) -> syn::Result<()> {
    ensure(
        source.strata == target.strata,
        target,
        "incremental rewrite changed the strata program",
    )
}
/// The rewrite callback assumes `cq_stage::run` checked both contracts.
/// Standalone callers must use the public free [`check_rewrite`] function.
impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite_admitted(source, target)
    }
}
