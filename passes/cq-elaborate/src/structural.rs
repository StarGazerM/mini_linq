use std::collections::{HashMap, HashSet};

use cq_lang_ra::{self as ra, contract::Schedule};
use cq_lang_vo::{self as vo, cq, symbol_name};
use syn::Ident;

/// State the supplied VO schedule as structural obligations. This does not run
/// the compiler, solve aliases, choose an order, or discover any atom mapping.
pub(super) fn check(
    source: &vo::Program,
    analysis: &vo::Analysis,
    target: &Schedule,
) -> Result<(), String> {
    if source
        .valuation
        .steps
        .iter()
        .any(|step| matches!(step, vo::Step::Block(..)))
    {
        return super::block_contract::check(source, target);
    }
    let declarations: HashMap<_, _> = source
        .cq
        .inputs()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut schedule = Schedule::default();
    let mut relations = HashMap::new();
    for (clause, atom) in &analysis.atoms {
        let declaration = declarations[&symbol_name(&atom.relation)];
        let mapping = declaration
            .columns
            .iter()
            .zip(atom.variables())
            .map(|(c, v)| (c.name.clone(), v.clone()))
            .collect::<Vec<_>>();
        relations.insert(*clause, schedule.atom(*clause, &atom.relation, &mapping));
    }
    let hypergraph = cq::hypergraph::Analysis::of(&source.cq);
    let mut prefix = None;
    let mut heading = HashSet::new();
    let mut reductions = Vec::new();
    for (step, introduces) in source.valuation.steps.iter().zip(&analysis.introduces) {
        match step {
            vo::Step::Block(..) => unreachable!("block schedule checked separately"),
            vo::Step::Atom(atom, _) => {
                let mut right = relations[&atom.clause()];
                let partial = atom.variables().count() != atom.columns.len();
                let check = atom
                    .variables()
                    .all(|(_, v)| heading.contains(&symbol_name(v)));
                if partial && !check {
                    right = schedule.project(
                        right,
                        &atom.variables().map(|(_, v)| v.clone()).collect::<Vec<_>>(),
                    );
                }
                prefix = Some(match prefix {
                    Some(left) => schedule.join(left, right, partial && check),
                    None => right,
                });
            }
            vo::Step::Reduce(reduce, _) => {
                let vo::Reduction::With(_, support) = &reduce.reduction;
                let left = relations[&reduce.clause()];
                let right = schedule.project(
                    relations[&support.clause()],
                    &support
                        .variables()
                        .map(|(_, v)| v.clone())
                        .collect::<Vec<_>>(),
                );
                let joined = schedule.join(left, right, false);
                let equalities = analysis.atoms[&reduce.clause()]
                    .variables()
                    .flat_map(|left| {
                        support
                            .variables()
                            .filter(|(_, right)| {
                                hypergraph.equivalent(&symbol_name(left), &symbol_name(right))
                            })
                            .map(|(_, right)| ra::Equality {
                                left: left.clone(),
                                eq_token: Default::default(),
                                right: ra::EqualityValue::Variable(right.clone()),
                            })
                    })
                    .collect();
                let matched = schedule.select(
                    joined,
                    &ra::Theta {
                        equalities,
                        constraints: Default::default(),
                    },
                );
                let attributes = analysis.atoms[&reduce.clause()]
                    .variables()
                    .cloned()
                    .collect::<Vec<_>>();
                let reduced = schedule.project(matched, &attributes);
                relations.insert(reduce.clause(), reduced);
                reductions.push((reduce.clause(), support.clause()));
            }
            vo::Step::Equality(equality, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.select(
                    input,
                    &ra::Theta {
                        equalities: std::iter::once(equality.clone()).collect(),
                        constraints: Default::default(),
                    },
                ));
            }
            vo::Step::Constraint(constraint, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.select(
                    input,
                    &ra::Theta {
                        equalities: Default::default(),
                        constraints: std::iter::once(constraint.clone()).collect(),
                    },
                ));
            }
            vo::Step::Guard(filter, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.guard(input, filter));
            }
            vo::Step::Let(binding, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.bind(input, binding));
            }
        }
        heading.extend(introduces.iter().cloned());
    }
    let attributes: Vec<Ident> = source.cq.query.head.variables().cloned().collect();
    let input = prefix.unwrap_or_else(|| schedule.unit());
    let output = schedule.project(input, &attributes);
    schedule.finish(output, &source.cq.query.head);
    schedule.check(target, &reductions)
}
