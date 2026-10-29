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
    target_program: &ra::Program,
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
    schedule.occurrence_paths((0..source.cq.query.body.len()).map(vo::syntax::Occurrence::new));
    let mut relations = HashMap::new();
    for (clause, atom) in &analysis.atoms {
        let declaration = declarations[&symbol_name(&atom.relation)];
        let mapping = declaration
            .columns
            .iter()
            .zip(&atom.args)
            .filter_map(|(c, a)| cq::rust::variable(a).map(|v| (c.name.clone(), v.clone())))
            .collect::<Vec<_>>();
        let constants = super::occurrence_constants(declaration, atom);
        relations.insert(
            *clause,
            schedule.atom(*clause, &atom.relation, &mapping, constants.as_ref()),
        );
    }
    let hypergraph = cq::hypergraph::Analysis::of(&source.cq);
    let mut prefix = None;
    let mut heading = HashSet::new();
    let mut reductions = Vec::new();
    let mut grouped = HashSet::new();
    let mut local_predicates = HashSet::new();
    let mut placed_negations = HashSet::new();
    for (step, introduces) in source.valuation.steps.iter().zip(&analysis.introduces) {
        match step {
            vo::Step::Block(..) => unreachable!("block schedule checked separately"),
            vo::Step::Atom(atom, _) => {
                if grouped.contains(&atom.clause()) {
                    continue;
                }
                if let Some(branch) = source.valuation.join_branches.as_ref().and_then(|groups| {
                    groups
                        .branches
                        .iter()
                        .find(|b| b.clauses().contains(&atom.clause()))
                }) {
                    let members = branch.clauses();
                    let own: std::collections::BTreeSet<_> = members
                        .iter()
                        .flat_map(|c| analysis.atoms[c].variables().map(symbol_name))
                        .collect();
                    let mut result = None;
                    for clause in &members {
                        let right = relations[clause];
                        result =
                            Some(result.map_or(right, |left| schedule.join(left, right, false)));
                    }
                    for (clause, item) in source.cq.query.body.iter().enumerate() {
                        let theta = match item {
                            cq::BodyItem::Equality(e)
                                if e.variables().all(|v| own.contains(&symbol_name(v))) =>
                            {
                                ra::Theta {
                                    equalities: std::iter::once(e.clone()).collect(),
                                    constraints: Default::default(),
                                }
                            }
                            cq::BodyItem::Constraint(c)
                                if c.variables().all(|v| own.contains(&symbol_name(v))) =>
                            {
                                let mut theta = ra::Theta::default();
                                theta.push_constraint(c.clone());
                                theta
                            }
                            _ => continue,
                        };
                        result = Some(schedule.select(result.unwrap(), &theta));
                        local_predicates.insert(clause);
                    }
                    let right = schedule.project(
                        result.unwrap(),
                        &branch.columns.iter().cloned().collect::<Vec<_>>(),
                    );
                    prefix = Some(prefix.map_or(right, |left| schedule.join(left, right, false)));
                    grouped.extend(members);
                    heading.extend(branch.columns.iter().map(symbol_name));
                    continue;
                }
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
                if source.cq.query.body.iter().enumerate().any(|(c, item)| {
                    local_predicates.contains(&c)
                        && match item {
                            cq::BodyItem::Equality(e) => e == equality,
                            cq::BodyItem::Constraint(c) => c.equality().as_ref() == Some(equality),
                            _ => false,
                        }
                }) {
                    continue;
                }
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
                let equality = constraint.equality();
                if source.cq.query.body.iter().enumerate().any(|(c, item)| {
                    local_predicates.contains(&c)
                        && match item {
                            cq::BodyItem::Constraint(predicate) => predicate == constraint,
                            cq::BodyItem::Equality(e) => equality.as_ref() == Some(e),
                            _ => false,
                        }
                }) {
                    continue;
                }
                let input = prefix.unwrap_or_else(|| schedule.unit());
                let mut theta = ra::Theta::default();
                theta.push_constraint(constraint.clone());
                prefix = Some(schedule.select(input, &theta));
            }
            vo::Step::Negation(negation, _) => {
                let clause = source
                    .cq
                    .query
                    .body
                    .iter()
                    .enumerate()
                    .find_map(|(clause, item)| {
                        (!placed_negations.contains(&clause)
                            && matches!(item, cq::BodyItem::Negation(n) if n == negation))
                        .then_some(clause)
                    })
                    .expect("VO negation coverage");
                let bound =
                    cq::blocks::bound_before(&source.cq.query.body, clause, &Default::default());
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(super::negation::obligation(
                    &mut schedule,
                    declarations[&symbol_name(&negation.atom.relation)],
                    negation,
                    &vo::syntax::Occurrence::new(clause),
                    &bound,
                    input,
                ));
                placed_negations.insert(clause);
            }
            vo::Step::Guard(filter, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.guard(input, filter));
            }
            vo::Step::Let(binding, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.bind(input, binding));
            }
            vo::Step::For(binding, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.for_bind(input, binding));
            }
        }
        heading.extend(introduces.iter().cloned());
    }
    let attributes: Vec<Ident> = source.cq.query.valuation_variables().cloned().collect();
    let input = prefix.unwrap_or_else(|| schedule.unit());
    let mut output = schedule.project(input, &attributes);
    if let Some(group) = &source.cq.query.group {
        output = schedule.group(
            output,
            &group.operator(source.cq.query.head.relation.clone()),
        );
    }
    schedule.finish(output, &source.cq.query.head);
    schedule.check(target, &reductions).or_else(|message| {
        // An under-canon certificate may recover a primary expansion witness
        // from the supplied target itself. No source plan is compiled or
        // selected here, and the original named reductions remain required.
        schedule
            .check_dead_expansions(target_program, &reductions)
            .map_err(|_| message)
    })
}
