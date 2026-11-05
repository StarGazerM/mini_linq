//! Requested placement of one negation (ADM-InPlan advice `known(at(level k))`).
//!
//! The requested boundary follows the k-th top-level positive read (1-based,
//! reductions excluded), with required bindings available there. I2 still
//! drains the negation at its earliest ready boundary; the VO records that
//! corrected placement rather than delaying a ready filter.
use super::{BodyItem, cq, valuation};

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

    /// Formal rule O-KnownAt: Validate readiness at the requested positive-read boundary, then drain the selected negation to its earliest ready boundary under I2.
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
        // Include bindings and filters following the requested read so a
        // computed head term can supply the negation before I2 drainage.
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
        super::drain_ready_checks(program).map_err(|err| {
            syn::Error::new_spanned(
                &self.relation,
                format!(
                    "query `{}`: known placement precedes the binding of its variables: {err}",
                    program.cq.name,
                ),
            )
        })?;
        valuation::contract::check(program).map(drop)
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
