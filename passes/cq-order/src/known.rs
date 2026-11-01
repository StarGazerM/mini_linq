//! Pinned placement of one negation (ADM-InPlan advice `known(at(level k))`).
//!
//! The ordinary planners place a ready negation as soon as its variables are
//! bound. Advice may pin it later: immediately after the k-th top-level
//! positive read (1-based, reductions excluded). WF-VO independently checks
//! that its variables are bound there.
use super::{BodyItem, cq, valuation};

/// `Rel@i at k`: the negation at normal-body clause `i` follows the k-th positive read.
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

    /// Formal rule O-KnownAt: Move the selected top-level negation after the k-th positive read (past the bindings and filters directly following it, before the next read); WF-VO checks its variables are bound there.
    pub fn place(&self, program: &mut valuation::Program) -> syn::Result<()> {
        let negation = self.negation(&program.cq)?.clone();
        let steps = &mut program.valuation.steps;
        let Some(current) = steps
            .iter()
            .position(|step| matches!(step, valuation::Step::Negation(n, _) if *n == negation))
        else {
            return Err(syn::Error::new_spanned(
                &self.relation,
                "known placement selects a negation absent from the top-level plan",
            ));
        };
        let step = steps.remove(current);
        // Bindings and filters ready after the k-th read (e.g. the `let`
        // naming a computed head term) stay ahead of the negation.
        let mut at = self.position(steps)?;
        while matches!(
            steps.get(at),
            Some(
                valuation::Step::Let(..)
                    | valuation::Step::Equality(..)
                    | valuation::Step::Constraint(..)
                    | valuation::Step::Guard(..)
            )
        ) {
            at += 1;
        }
        steps.insert(at, step);
        if valuation::contract::check(program).is_err() {
            return Err(syn::Error::new_spanned(
                &self.relation,
                "known placement precedes the binding of its variables",
            ));
        }
        Ok(())
    }

    /// Check that the selected negation sits immediately after the k-th positive read.
    pub fn check(&self, program: &valuation::Program) -> syn::Result<()> {
        let negation = self.negation(&program.cq)?;
        let steps = &program.valuation.steps;
        let at = self.position(steps)?;
        let placed = steps[at..]
            .iter()
            .take_while(|step| !matches!(step, valuation::Step::Atom(..)))
            .any(|step| matches!(step, valuation::Step::Negation(n, _) if n == negation));
        if !placed {
            return Err(syn::Error::new_spanned(
                &self.relation,
                format!(
                    "negation {}@{} is not placed after positive read {}",
                    self.relation, self.clause, self.level
                ),
            ));
        }
        Ok(())
    }
}
