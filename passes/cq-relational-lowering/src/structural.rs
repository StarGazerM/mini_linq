use std::collections::{BTreeSet, HashMap, HashSet};

use cq_lang_ra as ra;
use cq_lang_vo::{self as vo, cq, symbol_name};
use quote::format_ident;
use syn::Ident;

/// The structural reference `RL(s)`: the supplied VO schedule stated as an RA
/// program, one definition per RL rule application. It never runs the
/// compiler, solves aliases, chooses an order, or discovers an atom mapping;
/// the RA rewrite check compares it with the target (CK-RLStructure).
pub(super) fn reference(
    source: &cq::Program,
    steps: &[vo::Step],
    analysis: &vo::Analysis,
) -> ra::Program {
    if steps.iter().any(|step| matches!(step, vo::Step::Block(..))) {
        return super::block_contract::reference(source, steps);
    }
    let declarations: HashMap<_, _> = source
        .inputs()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut schedule = Emitter::default();
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
            schedule.atom(&atom.relation, &mapping, constants),
        );
    }
    let hypergraph = cq::hypergraph::Analysis::of(source);
    let mut prefix = None;
    let mut heading = HashSet::new();
    let mut placed_negations = HashSet::new();
    for (step, introduces) in steps.iter().zip(&analysis.introduces) {
        match step {
            vo::Step::Block(..) => unreachable!("block schedule stated separately"),
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
                    ra::Theta {
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
            }
            vo::Step::Restrict(restrict, _) => {
                let mut theta = ra::Theta::default();
                theta.push_constraint(restrict.predicate.clone());
                let restricted = schedule.select(relations[&restrict.clause()], theta);
                relations.insert(restrict.clause(), restricted);
            }
            vo::Step::Equality(equality, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.select(
                    input,
                    ra::Theta {
                        equalities: std::iter::once(equality.clone()).collect(),
                        constraints: Default::default(),
                    },
                ));
            }
            vo::Step::Constraint(constraint, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                let mut theta = ra::Theta::default();
                theta.push_constraint(constraint.clone());
                prefix = Some(schedule.select(input, theta));
            }
            vo::Step::Negation(negation, _) => {
                let clause = source
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
                    cq::blocks::bound_before(&source.query.body, clause, &Default::default());
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(super::negation::obligation(
                    &mut schedule,
                    declarations[&symbol_name(&negation.atom.relation)],
                    negation,
                    &bound,
                    input,
                ));
                placed_negations.insert(clause);
            }
            vo::Step::Guard(filter, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.guard(input, filter, None));
            }
            vo::Step::Let(binding, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.bind(input, binding, None));
            }
            vo::Step::For(binding, _) => {
                let input = prefix.unwrap_or_else(|| schedule.unit());
                prefix = Some(schedule.for_bind(input, binding, None));
            }
        }
        heading.extend(introduces.iter().cloned());
    }
    let input = prefix.unwrap_or_else(|| schedule.unit());
    schedule.finish(source, input)
}

/// RA definitions of the structural reference, named `__rl<k>` by index.
#[derive(Default)]
pub(super) struct Emitter {
    definitions: Vec<ra::Definition>,
}

impl Emitter {
    fn name(index: usize) -> Ident {
        format_ident!("__rl{index}")
    }

    fn push(&mut self, operator: ra::Operator) -> usize {
        let index = self.definitions.len();
        self.definitions.push(ra::Definition {
            result: Self::name(index),
            eq_token: Default::default(),
            operator,
            semi_token: Default::default(),
        });
        index
    }

    pub(super) fn unit(&mut self) -> usize {
        self.push(ra::Operator::Unit(ra::Unit {
            unit_token: Default::default(),
        }))
    }

    pub(super) fn atom(
        &mut self,
        relation: &Ident,
        mapping: &[(Ident, Ident)],
        constants: Option<ra::OccurrenceConstants>,
    ) -> usize {
        self.push(ra::Operator::Rename(Box::new(ra::Rename {
            rename_token: Default::default(),
            relation: relation.clone(),
            mapping_brace: Default::default(),
            mapping: mapping
                .iter()
                .map(|(source, target)| ra::AttributeRename {
                    source: source.clone(),
                    minus_token: Default::default(),
                    gt_token: Default::default(),
                    target: target.clone(),
                })
                .collect(),
            constants,
        })))
    }

    pub(super) fn project(&mut self, input: usize, attributes: &[Ident]) -> usize {
        self.push(ra::Operator::Project(Box::new(ra::Project {
            project_token: Default::default(),
            input: Self::name(input),
            keep_token: Default::default(),
            attributes_brace: Default::default(),
            attributes: attributes.iter().cloned().collect(),
        })))
    }

    pub(super) fn join(&mut self, left: usize, right: usize, semijoin: bool) -> usize {
        let (left, right) = (Self::name(left), Self::name(right));
        self.push(if semijoin {
            ra::Operator::Semijoin(Box::new(ra::Semijoin {
                semijoin_token: Default::default(),
                left,
                with_token: Default::default(),
                right,
            }))
        } else {
            ra::Operator::NaturalJoin(Box::new(ra::NaturalJoin {
                natural_join_token: Default::default(),
                left,
                with_token: Default::default(),
                right,
            }))
        })
    }

    pub(super) fn select(&mut self, input: usize, condition: ra::Theta) -> usize {
        if condition.equalities.is_empty() && condition.constraints.is_empty() {
            return input;
        }
        self.push(ra::Operator::Select(Box::new(ra::Select {
            select_token: Default::default(),
            input: Self::name(input),
            if_token: Default::default(),
            condition_paren: Default::default(),
            condition,
        })))
    }

    pub(super) fn anti(&mut self, left: usize, right: usize, columns: &[Ident]) -> usize {
        self.push(ra::Operator::Anti(Box::new(ra::Anti {
            anti_token: Default::default(),
            bracket: Default::default(),
            columns: columns.iter().cloned().collect(),
            left: Self::name(left),
            comma: Default::default(),
            right: Self::name(right),
        })))
    }

    pub(super) fn guard(
        &mut self,
        input: usize,
        filter: &cq::Filter,
        environment: Option<ra::Environment>,
    ) -> usize {
        self.push(ra::Operator::Guard(Box::new(ra::Guard {
            guard_token: Default::default(),
            input: Self::name(input),
            if_token: filter.if_token,
            capture: filter.capture,
            condition_paren: Default::default(),
            condition: filter.condition.clone(),
            environment,
        })))
    }

    pub(super) fn bind(
        &mut self,
        input: usize,
        binding: &cq::Let,
        environment: Option<ra::Environment>,
    ) -> usize {
        self.push(ra::Operator::Bind(Box::new(ra::Bind {
            bind_token: Default::default(),
            input: Self::name(input),
            with_token: Default::default(),
            for_token: None,
            pattern: binding.pattern.clone(),
            annotation: binding.annotation.clone(),
            eq_token: binding.eq_token,
            capture: binding.capture,
            expression_paren: Default::default(),
            expression: binding.expression.clone(),
            environment,
        })))
    }

    pub(super) fn for_bind(
        &mut self,
        input: usize,
        binding: &cq::For,
        environment: Option<ra::Environment>,
    ) -> usize {
        self.push(ra::Operator::Bind(Box::new(ra::Bind {
            bind_token: Default::default(),
            input: Self::name(input),
            with_token: Default::default(),
            for_token: Some(binding.for_token),
            pattern: binding.pattern.clone(),
            annotation: None,
            eq_token: Default::default(),
            capture: None,
            expression_paren: Default::default(),
            expression: binding.expression.clone(),
            environment,
        })))
    }

    /// A block union with its origin; `over` and `domain` name earlier definitions.
    pub(super) fn union(
        &mut self,
        operands: &[usize],
        mut origin: ra::BlockOrigin,
        over: Option<usize>,
        domain: Option<usize>,
    ) -> usize {
        origin.over = over.map(|prefix| ra::Correlation {
            over_token: Default::default(),
            prefix: Self::name(prefix),
        });
        origin.domain = domain.map(|input| ra::Domain {
            domain_token: Default::default(),
            input: Self::name(input),
        });
        self.push(ra::Operator::Union(Box::new(ra::Union {
            union_token: Default::default(),
            operands: operands.iter().map(|operand| Self::name(*operand)).collect(),
            origin: Some(origin),
        })))
    }

    /// RL-Finish: project the valuation variables, group a grouped head, and
    /// keep only definitions the output reaches.
    pub(super) fn finish(mut self, source: &cq::Program, input: usize) -> ra::Program {
        let attributes: Vec<Ident> = source.query.valuation_variables().cloned().collect();
        let mut output = self.project(input, &attributes);
        if let Some(group) = &source.query.group {
            output = self.push(ra::Operator::Group(Box::new(group.operator(Self::name(output)))));
        }
        let mut live = BTreeSet::from([output]);
        for index in (0..=output).rev() {
            if !live.contains(&index) {
                continue;
            }
            let names: Vec<&Ident> = match &self.definitions[index].operator {
                ra::Operator::Unit(_) | ra::Operator::Rename(_) => Vec::new(),
                ra::Operator::Union(u) => u
                    .operands
                    .iter()
                    .chain(u.origin.iter().flat_map(|o| {
                        o.over.iter().map(|c| &c.prefix).chain(o.domain.iter().map(|d| &d.input))
                    }))
                    .collect(),
                ra::Operator::Anti(a) => vec![&a.left, &a.right],
                ra::Operator::NaturalJoin(j) => vec![&j.left, &j.right],
                ra::Operator::Semijoin(j) => vec![&j.left, &j.right],
                ra::Operator::Group(g) => vec![&g.input],
                ra::Operator::Project(p) => vec![&p.input],
                ra::Operator::Select(s) => vec![&s.input],
                ra::Operator::Bind(b) => vec![&b.input],
                ra::Operator::Guard(g) => vec![&g.input],
            };
            live.extend(names.into_iter().map(|name| {
                symbol_name(name)
                    .strip_prefix("__rl")
                    .and_then(|index| index.parse::<usize>().ok())
                    .expect("reference operand")
            }));
        }
        let definitions = std::mem::take(&mut self.definitions)
            .into_iter()
            .enumerate()
            .filter_map(|(index, definition)| live.contains(&index).then_some(definition))
            .collect();
        ra::Program {
            relational_token: Default::default(),
            relational_brace: Default::default(),
            plan: ra::Plan {
                declarations: source.declarations.clone(),
                captures: source.query.captures.clone(),
                definitions,
                output: ra::Output {
                    output_token: Default::default(),
                    input: Self::name(output),
                    as_token: Default::default(),
                    head: source.query.head.clone(),
                    semi_token: Default::default(),
                },
            },
        }
    }
}
