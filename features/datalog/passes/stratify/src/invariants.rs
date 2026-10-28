//! Opt-in loop-invariant materialization with a structural cardinality bound.
//!
//! Only a maximal frozen side containing one positive atom and bound negative
//! filters is selected. Its result has at most as many tuples as that atom.
//! Multi-positive joins (including ddisasm rule 77) are deliberately excluded:
//! connectedness alone does not bound their fanout. The side must follow and
//! share a key with a changing occurrence; opaque Rust clauses are not moved.
use crate::ensure;
use cq_lang_datalog_source::{BodyItem, Invariant};
use cq_lang_datalog_strata::{Program, contract};
use cq_syntax::{rust, symbol_name};
use quote::{ToTokens, quote};
use std::collections::BTreeSet;

pub fn compile(source: &Program) -> syn::Result<Program> {
    let analysis = contract::check(source)?;
    ensure(
        source.source.invariants.is_empty(),
        source,
        "invariant materialization requires fresh strata before seminaive",
    )?;
    let mut target = source.clone();
    for (rule, query) in source.source.rules.iter().enumerate() {
        let head = analysis.positions[&symbol_name(&query.head.relation)];
        if analysis.changing_positions[rule].is_empty() || !analysis.graph.reach[head][head] {
            continue;
        }
        if query
            .body
            .iter()
            .any(|item| !matches!(item, BodyItem::Positive { .. } | BodyItem::Negation(_)))
        {
            continue;
        }
        // A coarser authored partition may iterate acyclic dependencies together.
        // Do not certify those inputs as genuinely recursive source occurrences.
        if analysis.changing_positions[rule].iter().any(|&position| {
            let BodyItem::Positive { atom } = &query.body[position] else {
                unreachable!()
            };
            let input = analysis.positions[&symbol_name(&atom.relation)];
            input != head && !analysis.graph.reach[input][head]
        }) {
            continue;
        }
        let component = analysis.component[head];
        let frozen = |id: &syn::Ident| {
            let i = analysis.positions[&symbol_name(id)];
            analysis.component[i].is_none() || analysis.component[i] < component
        };
        let positives: Vec<_> = query
            .body
            .iter()
            .enumerate()
            .filter_map(|(i, item)| match item {
                BodyItem::Positive { atom } if frozen(&atom.relation) => Some((i, atom)),
                _ => None,
            })
            .collect();
        let [(position, atom)] = positives.as_slice() else {
            continue;
        };
        let position = *position;
        let vars: Vec<_> = atom.variables().collect();
        let names: BTreeSet<_> = vars.iter().map(|v| symbol_name(v)).collect();
        if vars.is_empty() || vars.len() != names.len() {
            continue;
        }
        let mut prior = BTreeSet::new();
        for item in query.body.iter().take(position) {
            if let BodyItem::Positive { atom } = item {
                prior.extend(atom.variables().map(symbol_name));
            }
        }
        if !analysis.changing_positions[rule]
            .iter()
            .any(|&i| i < position)
            || !names.iter().any(|v| prior.contains(v))
        {
            continue;
        }
        let mut positions = vec![position];
        for (i, item) in query.body.iter().enumerate() {
            if let BodyItem::Negation(n) = item {
                if frozen(&n.atom.relation)
                    && n.atom.variables().all(|v| names.contains(&symbol_name(v)))
                {
                    positions.push(i);
                }
            }
        }
        if positions.len() == 1 {
            continue;
        }
        // Reject names appearing anywhere, including Rust expressions and schemas.
        let text = target.to_token_stream().to_string();
        let helper = (0..)
            .map(|n| {
                syn::Ident::new(
                    &format!("__cq_datalog_invariant_{n}"),
                    proc_macro2::Span::mixed_site(),
                )
            })
            .find(|id| {
                !text
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|word| id == word)
            })
            .unwrap();
        let input = &source.source.declarations[analysis.positions[&symbol_name(&atom.relation)]];
        let columns: Vec<_> = atom
            .args
            .iter()
            .zip(input.columns.iter())
            .filter_map(|(arg, col)| {
                rust::variable(arg).map(|v| {
                    let ty = &col.ty;
                    quote!(#v: #ty)
                })
            })
            .collect();
        target
            .source
            .declarations
            .push(syn::parse2(quote!(relation #helper(#(#columns),*);))?);
        let body: Vec<_> = positions.iter().map(|&i| &query.body[i]).collect();
        let heads: Vec<_> = atom
            .args
            .iter()
            .zip(input.columns.iter())
            .filter_map(|(arg, column)| {
                rust::variable(arg)
                    .map(|v| cq_lang_datalog_source::invariants::owned_column(v, &column.ty))
            })
            .collect();
        target
            .source
            .rules
            .push(syn::parse2(quote!(#helper(#(#heads),*) :- #(#body),*;))?);
        target.source.labels.push(None);
        let replacement: BodyItem = syn::parse2(quote!(#helper(#(#vars),*)))?;
        target.source.rules[rule].body = query
            .body
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                if i == position {
                    Some(replacement.clone())
                } else if positions.contains(&i) {
                    None
                } else {
                    Some(item.clone())
                }
            })
            .collect();
        let consumer = target
            .components
            .iter()
            .position(|members| members.contains(&query.head.relation))
            .unwrap();
        target.components.insert(consumer, vec![helper.clone()]);
        target
            .source
            .models
            .insert(symbol_name(&helper), syn::parse_quote!(set));
        target.source.invariants.push(Invariant {
            rule,
            helper,
            source: query.clone(),
        });
    }
    check(source, &target)?;
    Ok(target)
}

/// Independent certificate checking, not compiler replay.
pub fn check(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    ensure(
        source.source.invariants.is_empty() || source.source.invariants == target.source.invariants,
        target,
        "invariant scheduling changed existing certificates",
    )?;
    ensure(
        cq_lang_datalog_strata::invariants::erase(source)?
            == cq_lang_datalog_strata::invariants::erase(target)?,
        target,
        "invariant scheduling changed source rules, declarations or strata",
    )
}
