//! Independent source-to-RA relation for Rust clause scheduling. This consumes
//! the target, checks every source clause, and rejects any extra definitions.
use cq_ir::{cq, relational_plan as ra, symbol_name};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

use crate::dependencies::ClauseDependencies;

struct Cursor<'a> {
    definitions: std::slice::Iter<'a, ra::Definition>,
    current: Option<syn::Ident>,
}
impl Cursor<'_> {
    fn accept(&mut self, expected: ra::Operator) -> syn::Result<syn::Ident> {
        let d = self
            .definitions
            .next()
            .ok_or_else(|| err("missing administrative operation"))?;
        if d.operator != expected {
            return Err(syn::Error::new_spanned(
                d,
                "operation does not implement this source clause",
            ));
        }
        Ok(d.result.clone())
    }
    fn input(&mut self) -> syn::Result<syn::Ident> {
        if let Some(id) = &self.current {
            return Ok(id.clone());
        }
        let id = self.accept(syn::parse_quote!(unit))?;
        self.current = Some(id.clone());
        Ok(id)
    }
    fn selection(&mut self, expr: proc_macro2::TokenStream) -> syn::Result<()> {
        let input = self.input()?;
        self.current = Some(self.accept(syn::parse_quote!(select #input if (#expr)))?);
        Ok(())
    }
}
fn err(message: &str) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}

/// A legal schedule preserves each effect boundary and covers every clause once.
/// Availability is additionally checked against each actual RA heading.
fn check_order(analysis: &ClauseDependencies, order: &[usize]) -> syn::Result<()> {
    let body = &analysis.clauses;
    if order.len() != body.len()
        || order.iter().copied().collect::<BTreeSet<_>>() != (0..body.len()).collect()
    {
        return Err(err("schedule must cover every source clause exactly once"));
    }
    let positions: BTreeMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(position, &clause)| (clause, position))
        .collect();
    for (barrier, _) in body
        .iter()
        .enumerate()
        .filter(|(_, clause)| !clause.reorderable)
    {
        if (0..body.len()).any(|other| {
            (other < barrier && positions[&other] > positions[&barrier])
                || (other > barrier && positions[&other] < positions[&barrier])
        }) {
            return Err(err("schedule crossed a Rust evaluation boundary"));
        }
    }
    let mut bound = BTreeSet::new();
    for &clause in order {
        if !body[clause].required.is_subset(&bound) {
            return Err(err("schedule uses a logical input before its producer"));
        }
        bound.extend(body[clause].provided.iter().cloned());
    }
    Ok(())
}

pub(in crate::clause_prefix) fn check(source: &cq::Module, target: &ra::Module) -> syn::Result<()> {
    check_scheduled(
        source,
        target,
        &(0..source.program.query.body.len()).collect::<Vec<_>>(),
    )
}

pub(in crate::clause_prefix) fn check_scheduled(
    source: &cq::Module,
    target: &ra::Module,
    order: &[usize],
) -> syn::Result<()> {
    cq::contract::check(source)?;
    ra::contract::check(target)?;
    if source.program != target.program {
        return Err(err("source program changed"));
    }
    let analysis = ClauseDependencies::analyze(source)?;
    check_scheduled_in(source, target, order, &analysis)
}

/// Check a schedule and its emitted operations after the caller has checked
/// both local contracts and source preservation. The analysis comes from the
/// checker, independently of the lowering that produced the target.
pub(super) fn check_scheduled_in(
    source: &cq::Module,
    target: &ra::Module,
    order: &[usize],
    analysis: &ClauseDependencies,
) -> syn::Result<()> {
    check_order(analysis, order)?;
    let mut cursor = Cursor {
        definitions: target.plan.definitions.iter(),
        current: None,
    };
    let mut bound = BTreeSet::new();
    let mut reserved = crate::clause_prefix::reserved(source);
    let mut constants = BTreeMap::new();
    // Independently consume the constant-to-let normalization prefix.
    for (clause, item) in source.program.query.body.iter().enumerate() {
        if let cq::BodyItem::Positive { atom } = item {
            let declaration = source
                .program
                .inputs()
                .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
                .unwrap();
            for (argument, (expr, column)) in atom.args.iter().zip(&declaration.columns).enumerate()
            {
                if cq::rust::literal(expr) {
                    let name = crate::clause_prefix::fresh(&mut reserved, clause, argument);
                    let ty = &column.ty;
                    let input = cursor.input()?;
                    cursor.current = Some(cursor.accept(syn::parse_quote!(extend #input with {#name: #ty} value ({let #name: #ty = #expr; (#name,)})))?);
                    bound.insert(symbol_name(&name));
                    constants.insert((clause, argument), name);
                }
            }
        }
    }
    for &clause in order {
        let item = &source.program.query.body[clause];
        match item {
            cq::BodyItem::Negation(_) | cq::BodyItem::Aggregate(_) => {
                return Err(err(
                    "negation and aggregation have no implemented lowering contract",
                ));
            }
            cq::BodyItem::Filter(filter) => {
                let expr = &filter.condition;
                cursor.selection(quote!(#expr))?;
            }
            cq::BodyItem::Let(binding) => {
                let input = cursor.input()?;
                let variables = cq::rust::bindings(&binding.pattern);
                let typed = binding.binding_types();
                let columns = typed.iter().map(|(v, ty)| quote!(#v: #ty));
                let pat = &binding.pattern;
                let annotation = &binding.annotation;
                let expr = &binding.expression;
                let result = (!variables.is_empty()).then(|| quote!((#(#variables,)*)));
                cursor.current = Some(cursor.accept(syn::parse_quote!(extend #input with {#(#columns),*} value ({let #pat #annotation = #expr; #result})))?);
                bound.extend(variables.iter().map(symbol_name));
            }
            cq::BodyItem::Positive { atom } => {
                let schema = source
                    .program
                    .inputs()
                    .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
                    .ok_or_else(|| err("undeclared relation"))?;
                let prefix = bound.clone();
                let mut introduced = BTreeSet::new();
                let mut columns = Vec::new();
                let mut tests = Vec::new();
                for (position, (arg, column)) in atom.args.iter().zip(&schema.columns).enumerate() {
                    let attr = if let Some(id) = constants.get(&(clause, position)) {
                        id.clone()
                    } else if let Some(v) = cq::rust::variable(arg) {
                        if introduced.insert(symbol_name(v)) {
                            v.clone()
                        } else {
                            let fresh =
                                crate::clause_prefix::fresh(&mut reserved, clause, position);
                            tests.push(quote!(#fresh == #v));
                            fresh
                        }
                    } else {
                        let fresh = crate::clause_prefix::fresh(&mut reserved, clause, position);
                        if !matches!(arg, syn::Expr::Infer(_)) {
                            let local = introduced.difference(&prefix).cloned().collect();
                            if cq::rust::free_variables(arg)
                                .dependencies(&local)
                                .is_empty()
                            {
                                let input = cursor.input()?;
                                let ty = &column.ty;
                                cursor.current = Some(cursor.accept(syn::parse_quote!(extend #input with {#fresh: #ty} value ((#arg,))))?);
                            } else {
                                tests.push(quote!(#fresh.eq(&(#arg))));
                            }
                        }
                        fresh
                    };
                    bound.insert(symbol_name(&attr));
                    let col = &column.name;
                    columns.push(quote!(#col -> #attr));
                }
                let relation = &atom.relation;
                let right = cursor.accept(syn::parse_quote!(rename #relation {#(#columns),*}))?;
                cursor.current = Some(if let Some(left) = cursor.current.clone() {
                    cursor.accept(syn::parse_quote!(natural_join #left with #right))?
                } else {
                    right
                });
                for test in tests {
                    cursor.selection(test)?;
                }
            }
        }
    }
    let head = &source.program.query.head;
    let needed: BTreeSet<_> = if head.variables().count() < head.args.len() {
        bound.clone()
    } else {
        head.args
            .iter()
            .flat_map(|e| cq::rust::free_variables(e).dependencies(&bound))
            .collect()
    };
    let columns = needed.iter().map(|n| format_ident!("r#{n}"));
    let input = cursor.input()?;
    let result = cursor.accept(syn::parse_quote!(project #input keep {#(#columns),*}))?;
    if cursor.definitions.next().is_some()
        || target.plan.output.input != result
        || target.plan.output.head != source.program.query.head
    {
        return Err(err(
            "output must preserve every head expression and complete the source derivation",
        ));
    }
    Ok(())
}
