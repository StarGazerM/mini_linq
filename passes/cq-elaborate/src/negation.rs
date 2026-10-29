//! E-Neg's positional relation, shared by lowering and structural obligations.
//! Raw and ordinary identifier spellings share relation and uniqueness-key identities.
use std::collections::BTreeSet;

use cq_lang_ra as ra;
use cq_lang_vo::{cq, symbol_name, syntax};
use syn::Ident;
use syntax::constraint::{Comparison, Term};

pub(super) struct Layout {
    pub mapping: Vec<(Ident, Ident)>,
    pub condition: ra::Theta,
    pub keys: Vec<Ident>,
}

pub(super) fn layout(
    declaration: &syntax::RelationDecl,
    negation: &cq::Negation,
    bound: &BTreeSet<String>,
    canonical: impl Fn(&Ident) -> Ident,
) -> Layout {
    let mut reserved = bound.clone();
    reserved.extend(negation.atom.variables().map(symbol_name));
    let mut keys = Vec::new();
    let mut mapping = Vec::new();
    let mut condition = ra::Theta::default();
    for (position, (column, argument)) in declaration
        .columns
        .iter()
        .zip(&negation.atom.args)
        .enumerate()
    {
        let variable = syntax::rust::variable(argument);
        let term = variable.map(|v| {
            if bound.contains(&symbol_name(v)) {
                Term::Variable(canonical(v))
            } else {
                Term::Closed(syn::parse_quote!(#v))
            }
        });
        if let Some(Term::Variable(v)) = &term {
            if !keys.iter().any(|key| symbol_name(key) == symbol_name(v)) {
                keys.push(v.clone());
                mapping.push((column.name.clone(), v.clone()));
                continue;
            }
        }
        // Repeated variables need separate columns until their equality has
        // been checked; wildcard and constant columns disappear at projection.
        let mut name = format!("__cq_neg_{position}");
        while !reserved.insert(name.clone()) {
            name.push('_');
        }
        let fresh = Ident::new(&name, column.name.span());
        mapping.push((column.name.clone(), fresh.clone()));
        if !matches!(argument, syn::Expr::Infer(_)) {
            condition.push_constraint(syntax::Constraint::Compare {
                left: Term::Variable(fresh),
                comparison: Comparison::Eq(Default::default()),
                right: term.unwrap_or_else(|| Term::Literal(Box::new(argument.clone()))),
            });
        }
    }
    Layout {
        mapping,
        condition,
        keys,
    }
}

pub(super) fn obligation(
    schedule: &mut ra::contract::Schedule,
    declaration: &syntax::RelationDecl,
    negation: &cq::Negation,
    path: &syntax::Occurrence,
    bound: &BTreeSet<String>,
    left: usize,
) -> usize {
    let layout = layout(declaration, negation, bound, Clone::clone);
    let mut right = schedule.atom_path(path, &negation.atom.relation, &layout.mapping, None);
    let filtered =
        !layout.condition.equalities.is_empty() || !layout.condition.constraints.is_empty();
    if filtered {
        right = schedule.select(right, &layout.condition);
    }
    if layout.keys.len() != layout.mapping.len() && filtered {
        right = schedule.project(right, &layout.keys);
    }
    schedule.anti(left, right, &layout.keys)
}
