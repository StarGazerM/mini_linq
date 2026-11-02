//! O-Restrict: record where relational predicates run.
//!
//! Every H-Local predicate (shared with H-FilterRank and the reductions)
//! becomes `restrict R@i where P;` on each target occurrence, at the head of
//! its scope in H-Local order. A written predicate whose every written
//! instance has a target has no prefix step; implied domains exist only as
//! restrictions.

use std::collections::BTreeSet;

use cq_lang_cq as cq;
use cq_lang_cq::BodyItem;
use cq_lang_vo as valuation;

/// Formal rule O-Restrict: Restrict every H-Local target occurrence by its predicate at the head of the scope, and omit the prefix steps of a written predicate whose every written instance restricts some occurrence.
pub(crate) fn restrict(program: &mut valuation::Program) {
    let steps = std::mem::take(&mut program.valuation.steps);
    program.valuation.steps = scope(&program.cq.query.body, &BTreeSet::new(), steps, None);
}

fn scope(
    body: &cq::CommaList<BodyItem>,
    context: &BTreeSet<String>,
    steps: Vec<valuation::Step>,
    parent: Option<(valuation::Occurrence, usize)>,
) -> Vec<valuation::Step> {
    let path = |clause: usize| match &parent {
        Some((path, branch)) => path.child(*branch, clause),
        None => valuation::Occurrence::new(clause),
    };
    let local = cq::hypergraph::local_predicates(body);
    let mut result = Vec::with_capacity(steps.len());
    for predicate in &local {
        for &target in &predicate.targets {
            let BodyItem::Positive { atom } = &body[target] else {
                unreachable!("H-Local targets positive atoms");
            };
            result.push(valuation::Step::Restrict(
                valuation::Restrict {
                    restrict_token: Default::default(),
                    relation: atom.relation.clone(),
                    at: Default::default(),
                    clause: path(target),
                    where_token: Default::default(),
                    predicate: predicate.predicate.clone(),
                },
                Default::default(),
            ));
        }
    }
    // A prefix step is omitted when every H-Local entry carrying its
    // payload restricts some occurrence; variable identities always stay.
    let covered = |step: &valuation::Step| {
        let mut carriers = local
            .iter()
            .filter(|entry| valuation::restrict::step_states(step, &entry.predicate))
            .peekable();
        carriers.peek().is_some() && carriers.all(|entry| !entry.targets.is_empty())
    };
    for step in steps {
        match step {
            valuation::Step::Constraint(..) | valuation::Step::Equality(..) if covered(&step) => {}
            valuation::Step::Block(mut block, semi) => {
                let clause = block.source.clause();
                let BodyItem::Block(written) = &body[clause] else {
                    unreachable!("VO block clause");
                };
                let prefix = cq::blocks::bound_before(body, clause, context);
                let facts = written.analyze(&prefix);
                let arm_context: BTreeSet<String> =
                    if block.placement == valuation::Placement::Hoisted {
                        facts.free.keys().cloned().collect()
                    } else {
                        prefix
                    };
                for (j, (arm, source)) in
                    block.branches.iter_mut().zip(&written.branches).enumerate()
                {
                    arm.steps = scope(
                        &source.body,
                        &arm_context,
                        std::mem::take(&mut arm.steps),
                        Some((block.source.clause.clone(), j)),
                    );
                }
                result.push(valuation::Step::Block(block, semi));
            }
            step => result.push(step),
        }
    }
    result
}
