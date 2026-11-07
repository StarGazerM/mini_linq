//! Requested placement of one negation (ADM-InPlan advice `known(at(level k))`).
//!
//! The requested boundary follows the k-th top-level positive read (1-based,
//! reductions excluded), with required bindings available there. I2 still
//! drains the negation at its earliest ready boundary; the VO records that
//! corrected placement rather than delaying a ready filter.
use std::collections::BTreeSet;

use super::{BodyItem, cq, symbol_name, valuation};

/// `Rel@i at k`: request a ready boundary after read k, subject to I2 drainage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    pub relation: syn::Ident,
    pub clause: usize,
    pub level: usize,
}

impl syn::parse::Parse for Known {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let relation = input.parse()?;
        input.parse::<syn::Token![@]>()?;
        let clause = input.parse::<syn::LitInt>()?.base10_parse()?;
        let at: syn::Ident = input.parse()?;
        if at != "at" {
            return Err(syn::Error::new_spanned(
                at,
                "expected `Relation@clause at level`",
            ));
        }
        let level = input.parse::<syn::LitInt>()?;
        Ok(Self {
            relation,
            clause,
            level: level.base10_parse()?,
        })
    }
}

impl Known {
    /// The selected negation of the normal-form body.
    pub fn negation<'a>(&self, source: &'a cq::Program) -> syn::Result<&'a cq::Negation> {
        match source.query.body.get(self.clause) {
            Some(BodyItem::Negation(negation)) if negation.atom.relation == self.relation => {
                Ok(negation)
            }
            _ => Err(syn::Error::new_spanned(
                &self.relation,
                format!(
                    "known selector {}@{} does not name that top-level negation",
                    self.relation, self.clause
                ),
            )),
        }
    }

    fn position(&self, steps: &[valuation::Step]) -> syn::Result<usize> {
        let mut reads = 0;
        for (index, step) in steps.iter().enumerate() {
            if matches!(step, valuation::Step::Atom(..)) {
                reads += 1;
                if reads == self.level {
                    return Ok(index + 1);
                }
            }
        }
        Err(syn::Error::new_spanned(
            &self.relation,
            format!("known placement names no positive read {}", self.level),
        ))
    }

    /// Front: resolve the request against the plan (O-KnownResolve), then
    /// move the negation and drain (total).
    /// Formal rule O-KnownAt: Move the resolved negation to the requested boundary, then drain it to its earliest ready boundary under I2; WF-VO and the boundary are checked after reduction.
    pub fn place(&self, program: &mut valuation::Program) -> syn::Result<()> {
        let (current, at) = self.resolve(program)?;
        let step = program.valuation.steps.remove(current);
        program.valuation.steps.insert(at, step);
        super::drain(program);
        Ok(())
    }

    /// The negation's step and its requested position in the plan without
    /// it: after read k and the bindings and filters that follow it.
    /// Formal rule O-KnownResolve: The selector names a top-level negation of a plan with WF-VO binding facts, read k exists, and the negation's logical variables are bound at the requested boundary.
    fn resolve(&self, program: &valuation::Program) -> syn::Result<(usize, usize)> {
        let analysis = valuation::order::analyze(program)?;
        let negation = self.negation(&program.cq)?;
        let steps = &program.valuation.steps;
        let current = steps
            .iter()
            .position(|step| matches!(step, valuation::Step::Negation(n, _) if n == negation))
            .expect("WF-VO-Predicates places each top-level negation in the top-level scope");
        // Positions in the plan without the negation, mapped back to `steps`.
        let original = |at: usize| if at < current { at } else { at + 1 };
        let mut at = self.position(steps)?;
        if at > current {
            at -= 1;
        }
        while matches!(
            steps.get(original(at)),
            Some(
                valuation::Step::Let(..)
                    | valuation::Step::Equality(..)
                    | valuation::Step::Constraint(..)
                    | valuation::Step::Guard(..)
            )
        ) {
            at += 1;
        }
        // The negation introduces nothing, so removing it changes no bound set.
        let index = original(at);
        let bound: BTreeSet<&String> = match analysis.bound_before.get(index) {
            Some(bound) => bound.iter().collect(),
            None => analysis.bound_before.iter().zip(&analysis.introduces).last()
                .map(|(bound, introduced)| bound.iter().chain(introduced).collect())
                .unwrap_or_default(),
        };
        let logical = cq::blocks::bound(&program.cq.query.body);
        if let Some(variable) = negation.atom.variables().find(|variable| {
            let name = symbol_name(variable);
            logical.contains_key(&name) && !bound.contains(&name)
        }) {
            return Err(syn::Error::new_spanned(
                &self.relation,
                format!(
                    "query `{}`: known placement precedes the binding of `{variable}`",
                    program.cq.name,
                ),
            ));
        }
        Ok((current, at))
    }

    /// Check VO-Order and that drainage did not exceed the requested read boundary.
    pub fn check(&self, program: &valuation::Program) -> syn::Result<()> {
        valuation::contract::check(program)?;
        let negation = self.negation(&program.cq)?;
        let steps = &program.valuation.steps;
        let at = self.position(steps)?;
        let placed = steps.iter()
            .position(|step| matches!(step, valuation::Step::Negation(n, _) if n == negation))
            .expect("checked source negation");
        if placed > at && steps[at..placed].iter().any(|step| matches!(step, valuation::Step::Atom(..))) {
            return Err(syn::Error::new_spanned(
                &self.relation,
                format!(
                    "negation {}@{} exceeds the requested boundary after positive read {}",
                    self.relation, self.clause, self.level
                ),
            ));
        }
        Ok(())
    }
}
