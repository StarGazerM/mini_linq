//! Independent legality checking over binding facts, never scheduler replay.
use std::collections::BTreeSet;

use crate::{Analysis, Program, Step};

/// A recorded step's binding semantics. Preprocessing is not a stream step.
#[derive(Clone, Debug, Default)]
pub struct StepFacts {
    pub required: BTreeSet<String>,
    pub provides: BTreeSet<String>,
    pub atom: bool,
    pub preprocessing: bool,
}

impl StepFacts {
    pub fn ready(&self, bound: &BTreeSet<String>) -> bool {
        self.required.is_subset(bound)
    }

    pub fn introduces(&self, bound: &BTreeSet<String>) -> bool {
        !self.provides.is_subset(bound)
    }

    pub fn connected(&self, bound: &BTreeSet<String>) -> bool {
        self.atom && !self.provides.is_disjoint(bound)
    }
}

/// Establish coverage and binding facts before a producer repairs ordering.
/// Unlike `contract::check`, this deliberately does not establish VO-Order.
pub fn analyze(program: &Program) -> syn::Result<Analysis> {
    crate::cq::normal::check(&program.cq)?;
    crate::analyze(program)
}

/// Establish binding facts for a CQ occurrence proof reference, not a recorded
/// VO. This checks normal form and scope/coverage without imposing an order.
pub fn analyze_reference(cq: &crate::cq::Program, steps: &[Step]) -> syn::Result<Analysis> {
    crate::cq::normal::check(cq)?;
    crate::analyze_reference(cq, steps)
}

/// Formal rule VO-Order: Independently check I1–I3 in every recorded scope.
/// Written and authored plans have no exemption from these legality rules.
/// Only binding facts decide readiness, introductions and connectivity; neither
/// Rust syntax/types nor a producer's ranking or scheduler are consulted.
pub fn check(program: &Program, analysis: &Analysis) -> syn::Result<()> {
    check_steps(&program.valuation.steps, analysis)
}

fn check_steps(steps: &[Step], analysis: &Analysis) -> syn::Result<()> {
    for (index, (step, facts)) in steps.iter().zip(&analysis.order).enumerate() {
        let bound = &analysis.provided_before[index];
        // Formal rule VO-EmptyCheck: a later introduction of the same
        // occurrence already establishes its nonemptiness.
        if let Step::Atom(atom, _) = step {
            if atom.variables().next().is_none()
                && steps.iter().enumerate().skip(index + 1).any(|(later, step)|
                    matches!(step, Step::Atom(next, _)
                        if next.clause == atom.clause && !analysis.introduces[later].is_empty()))
            {
                return crate::error(atom, "redundant zero-column check before occurrence introduction");
            }
        }
        // Formal rule VO-Order-I1: every required binding is actually supplied.
        if !facts.ready(bound) {
            let missing = facts.required.difference(bound).collect::<Vec<_>>();
            return crate::error(step, format!("VO-Order I1: unavailable bindings {missing:?}"));
        }
        if !facts.preprocessing && facts.introduces(bound) {
            for (later, candidate) in analysis.order.iter().enumerate().skip(index + 1) {
                if candidate.preprocessing || !candidate.ready(bound) {
                    continue;
                }
                // Formal rule VO-Order-I2: no ready filter crosses any introduction.
                // This includes lets; filters at the same boundary are unordered.
                if !candidate.introduces(bound) {
                    return crate::error(
                        &steps[later],
                        format!("VO-Order I2: ready nonintroducing step crosses introduction at step {index}"),
                    );
                }
                // Formal rule VO-Order-I3: connected ready atoms precede disconnected atoms.
                if facts.atom && !facts.connected(bound) && candidate.connected(bound) {
                    return crate::error(
                        step,
                        format!("VO-Order I3: disconnected atom precedes ready connected atom at step {later}"),
                    );
                }
            }
        }
        if let Step::Block(block, _) = step {
            for (branch, facts) in block.branches.iter().zip(&analysis.branches[&block.source.clause()]) {
                check_steps(&branch.steps, facts)?;
            }
        }
    }
    Ok(())
}
