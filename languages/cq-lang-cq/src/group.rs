//! Typed normalized grouped heads. Valuation columns are captured before
//! normalization introduces anonymous witnesses, and survive every planner.
use crate::{CommaList, Program, symbol_name};
use cq_syntax::algebra::{AggregateBinding, AggregateKind, Group, kw};
use std::collections::BTreeSet;
use syn::{Ident, token};

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct HeadGroup {
    pub group_token: kw::group,
    #[syn(bracketed)]
    pub valuation_bracket: token::Bracket,
    #[syn(in = valuation_bracket)]
    #[parse(CommaList::parse_terminated)]
    pub valuation: CommaList<Ident>,
    pub by_token: kw::by,
    #[syn(bracketed)]
    pub keys_bracket: token::Bracket,
    #[syn(in = keys_bracket)]
    #[parse(CommaList::parse_terminated)]
    pub keys: CommaList<Ident>,
    #[syn(braced)]
    pub brace_token: token::Brace,
    #[syn(in = brace_token)]
    #[parse(parse_aggregates)]
    #[to_tokens(|tokens, bindings| quote::TokenStreamExt::append_all(tokens, bindings))]
    pub aggregates: Vec<AggregateBinding>,
}

fn parse_aggregates(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<AggregateBinding>> {
    let mut bindings = Vec::new();
    while !input.is_empty() {
        bindings.push(input.parse()?);
    }
    Ok(bindings)
}

impl HeadGroup {
    pub fn operator(&self, input: Ident) -> Group {
        Group {
            group_token: self.group_token,
            input,
            by_token: self.by_token,
            bracket_token: self.keys_bracket,
            keys: self.keys.clone(),
            brace_token: self.brace_token,
            aggregates: self.aggregates.clone(),
        }
    }

    pub fn check(&self, program: &Program) -> syn::Result<()> {
        let schemas = program
            .inputs()
            .map(|d| (symbol_name(&d.name), d))
            .collect();
        let types = crate::blocks::types(&schemas, &program.query.body, &Default::default())?;
        let mut input = std::collections::BTreeMap::new();
        for variable in &self.valuation {
            let name = symbol_name(variable);
            let ty = types.get(&name).ok_or_else(|| {
                syn::Error::new_spanned(variable, "group valuation column is not bound")
            })?;
            if input.insert(name, ty.clone()).is_some() {
                return Err(syn::Error::new_spanned(
                    variable,
                    "group valuation columns must be distinct",
                ));
            }
        }
        self.operator(Ident::new("valuation", proc_macro2::Span::call_site()))
            .check(&input)?;
        let available: BTreeSet<_> = self
            .keys
            .iter()
            .chain(self.aggregates.iter().map(|a| &a.name))
            .map(symbol_name)
            .collect();
        let output = program.output().ok_or_else(|| {
            syn::Error::new_spanned(
                &program.query.head,
                "aggregated output requires a result declaration",
            )
        })?;
        for aggregate in &self.aggregates {
            if types.contains_key(&symbol_name(&aggregate.name)) {
                return Err(syn::Error::new_spanned(
                    &aggregate.name,
                    "aggregate output shadows a body variable",
                ));
            }
        }
        let mut used = BTreeSet::new();
        for (expression, column) in program.query.head.args.iter().zip(&output.columns) {
            let variable = crate::rust::variable(expression)
                .filter(|v| available.contains(&symbol_name(v)))
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        expression,
                        "grouped output must name a key or aggregate result",
                    )
                })?;
            used.insert(symbol_name(variable));
            if let Some(aggregate) = self.aggregates.iter().find(|a| a.name == *variable) {
                if aggregate.ty != column.ty {
                    return Err(syn::Error::new_spanned(
                        column,
                        "aggregate type differs from its result declaration",
                    ));
                }
            }
        }
        if used != available {
            return Err(syn::Error::new_spanned(
                &program.query.head,
                "grouped output must retain every key and aggregate result",
            ));
        }
        Ok(())
    }
}

/// Recognize only unqualified aggregate head calls, not ordinary qualified Rust calls.
pub fn aggregate(
    expression: &syn::Expr,
) -> syn::Result<Option<(AggregateKind, Option<&syn::Expr>)>> {
    let syn::Expr::Call(call) = expression else {
        return Ok(None);
    };
    let Some(name) = crate::rust::variable(&call.func) else {
        return Ok(None);
    };
    let kind = match symbol_name(name).as_str() {
        "count" => AggregateKind::Count,
        "sum" => AggregateKind::Sum,
        "min" => AggregateKind::Min,
        "max" => AggregateKind::Max,
        _ => return Ok(None),
    };
    let arity = usize::from(kind != AggregateKind::Count);
    if call.args.len() != arity {
        return Err(syn::Error::new_spanned(
            call,
            "count takes no argument; sum/min/max take exactly one",
        ));
    }
    Ok(Some((kind, call.args.first())))
}
