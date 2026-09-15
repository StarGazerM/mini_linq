//! Independent CQ -> RA edge relation. This reads actual definitions; it never
//! invokes the scheduling policy or the rule implementation that emits RA.
use crate::cq_to_relational::{self, JoinPlanReport, Rule};
use cq_ir::{cq, relational_plan as ra};

pub(crate) fn check(
    source: &cq::Module,
    target: &ra::Module,
    proof: &JoinPlanReport,
) -> syn::Result<()> {
    cq::contract::check(source)?;
    ra::contract::check(target)?;
    require(
        target.program == source.program,
        "planning changed the source annotation",
    )?;
    let clauses = cq_to_relational::dependencies::ClauseDependencies::analyze(source)?;
    let mut prefix = Vec::new();
    for (clause, item) in source.program.query.body.iter().enumerate() {
        if let cq::BodyItem::Positive { atom } = item {
            for (argument, _) in atom
                .args
                .iter()
                .enumerate()
                .filter(|(_, arg)| cq::rust::literal(arg))
            {
                prefix.push(Rule::Constant { clause, argument });
            }
        }
    }
    require(
        proof.derivation.starts_with(&prefix) && proof.derivation.last() == Some(&Rule::Finish),
        "missing constant normalization or Finish",
    )?;
    let order = proof.derivation[prefix.len()..proof.derivation.len() - 1]
        .iter()
        .map(|rule| match rule {
            Rule::Match { clause } | Rule::Bind { clause } | Rule::Guard { clause } => {
                match (rule, source.program.query.body.get(*clause)) {
                    (Rule::Match { .. }, Some(cq::BodyItem::Positive { .. }))
                    | (Rule::Guard { .. }, Some(cq::BodyItem::Filter(_)))
                    | (Rule::Bind { .. }, Some(cq::BodyItem::Let(_))) => Ok(*clause),
                    _ => Err(syn::Error::new_spanned(source, "invalid clause derivation")),
                }
            }
            _ => Err(syn::Error::new_spanned(source, "invalid clause derivation")),
        })
        .collect::<syn::Result<Vec<_>>>()?;
    super::ordered::check_scheduled_in(source, target, &order, &clauses)?;
    let peak = ra::check_in(target.program.inputs(), &target.plan)?
        .values()
        .map(|r| r.heading.len())
        .max()
        .unwrap_or(0);
    require(
        proof.analysis.is_empty()
            && proof.join_tree.is_none()
            && proof.peak_arity == peak
            && proof.options.max_arity.is_none_or(|limit| peak <= limit),
        "Rust clauses require a legal complete derivation and exact width",
    )?;
    Ok(())
}

fn require(premise: bool, message: &str) -> syn::Result<()> {
    if premise { Ok(()) } else { Err(error(message)) }
}
fn error(message: &str) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}
