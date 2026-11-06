//! Occurrence restrictions: `restrict R@path where P;`.
//!
//! A `where P;` step filters the valuation prefix. A restriction instead
//! filters one clause's own relation before the scope reads, reduces or
//! supports with it, so a plan states that a predicate runs on an occurrence
//! (a keyed or ranged read of that relation). The predicate is a written
//! relational predicate of the scope (a literal equality in constraint form)
//! or one of its E-Domain implied domains, and every predicate variable is a
//! column of the restricted atom: `sigma_P(R) join prefix = sigma_P(R join
//! prefix)`. A written predicate without a prefix step must restrict at least
//! one occurrence, which is then its only placement.

use std::collections::BTreeSet;

use syn::{Ident, Token};

use super::{Occurrence, Step, cq, symbol_name};

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Restrict {
    pub restrict_token: super::kw::restrict,
    pub relation: Ident,
    pub at: Token![@],
    pub clause: Occurrence,
    pub where_token: Token![where],
    pub predicate: cq::Constraint,
}

impl Restrict {
    pub fn clause(&self) -> usize {
        self.clause.clause()
    }

    pub fn variables(&self) -> BTreeSet<String> {
        self.predicate.variables().map(symbol_name).collect()
    }
}

/// Whether `predicate` is the written relational predicate of `item`: the
/// constraint itself, or the same equality in constraint form.
pub fn states(item: &cq::BodyItem, predicate: &cq::Constraint) -> bool {
    match item {
        cq::BodyItem::Equality(equality) => predicate.equality().as_ref() == Some(equality),
        cq::BodyItem::Constraint(constraint) => {
            constraint == predicate
                || constraint.equality().is_some() && constraint.equality() == predicate.equality()
        }
        _ => false,
    }
}

/// Whether a prefix step carries the written predicate `predicate`.
pub fn step_states(step: &Step, predicate: &cq::Constraint) -> bool {
    match step {
        Step::Equality(equality, _) => predicate.equality().as_ref() == Some(equality),
        Step::Constraint(constraint, _) => {
            states(&cq::BodyItem::Constraint(constraint.clone()), predicate)
        }
        _ => false,
    }
}

/// Formal rule WF-Restrict: A restriction names a positive atom occurrence of its scope, carries a written relational predicate or one of its implied domains over that atom's columns, and precedes every other step of the scope.
///
/// Returns the written predicate clauses whose predicate this restriction
/// carries (an implied domain carries none).
/// Formal rule WF-VO-Restrictions: Restrict only a scoped positive occurrence with a written or implied column predicate before all other steps.
pub(super) fn check(
    body: &cq_syntax::CommaList<cq::BodyItem>,
    restrict: &Restrict,
    expected_path: &Occurrence,
    started: bool,
) -> syn::Result<BTreeSet<usize>> {
    let fail = |message: &str| super::error(restrict, message);
    let clause = restrict.clause();
    if restrict.clause != *expected_path {
        return fail("restriction occurrence escaped its source branch path");
    }
    let Some(cq::BodyItem::Positive { atom }) = body.iter().nth(clause) else {
        return fail("only a positive atom occurrence can be restricted");
    };
    if symbol_name(&atom.relation) != symbol_name(&restrict.relation) {
        return fail(&format!("clause #{clause} is `{}`", atom.relation));
    }
    if started {
        return fail("a restriction precedes every other step of its scope");
    }
    let columns: BTreeSet<_> = atom.variables().map(symbol_name).collect();
    if restrict.variables().is_empty() || !restrict.variables().is_subset(&columns) {
        return fail("a restriction uses only columns of its occurrence");
    }
    if restrict
        .predicate
        .equality()
        .is_some_and(|equality| matches!(equality.right, cq::EqualityValue::Variable(_)))
    {
        return fail("a variable identity is a prefix step, not a restriction");
    }
    let mut placed = BTreeSet::new();
    let mut implied = false;
    for (position, item) in body.iter().enumerate() {
        if states(item, &restrict.predicate) {
            placed.insert(position);
        } else if let cq::BodyItem::Constraint(constraint) = item {
            implied |= constraint.implied_domains().contains(&restrict.predicate);
        }
    }
    if placed.is_empty() && !implied {
        return fail(
            "a restriction carries a written relational predicate or one of its implied domains",
        );
    }
    Ok(placed)
}
