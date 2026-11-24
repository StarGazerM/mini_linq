//! State the fixed block equations as the structural reference. Each block
//! arm is visited in place; there is no mapping search or compilation.
//! Branches project to the common heading. Fallback negatives refer to the
//! original branches, while positive reductions remain separate references.

use super::structural::Emitter;
use cq_lang_ra as ra;
use cq_lang_vo::{self as vo, cq};
use cq_syntax::symbol_name;
use std::collections::{BTreeMap, BTreeSet};
use syn::Ident;
use vo::syntax as cq_syntax;

pub(super) fn reference(source: &cq::Program, steps: &[vo::Step]) -> ra::Program {
    let mut schedule = Emitter::default();
    let mut obligation = Body {
        schedule: &mut schedule,
        source,
        body: &source.query.body,
        steps,
        context: BTreeSet::new(),
        heading: BTreeSet::new(),
        prefix: None,
        relations: BTreeMap::new(),
        reads: BTreeSet::new(),
        correlated: BTreeSet::new(),
        placed_rust: BTreeSet::new(),
    };
    obligation.visit();
    let input = obligation.seed();
    schedule.finish(source, input)
}

struct Body<'a, 's> {
    schedule: &'s mut Emitter,
    source: &'a cq::Program,
    body: &'a cq_syntax::CommaList<cq::BodyItem>,
    steps: &'a [vo::Step],
    context: BTreeSet<String>,
    heading: BTreeSet<String>,
    prefix: Option<usize>,
    relations: BTreeMap<usize, usize>,
    reads: BTreeSet<usize>,
    correlated: BTreeSet<usize>,
    placed_rust: BTreeSet<usize>,
}
impl Body<'_, '_> {
    fn seed(&mut self) -> usize {
        self.prefix.unwrap_or_else(|| {
            let unit = self.schedule.unit();
            self.prefix = Some(unit);
            unit
        })
    }
    fn relation(&mut self, clause: usize) -> usize {
        if let Some(result) = self.relations.get(&clause) {
            return *result;
        }
        let result = match &self.body[clause] {
            cq::BodyItem::Positive { atom } => {
                let declaration = self
                    .source
                    .inputs()
                    .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
                    .expect("contracted atom");
                let mapping = declaration
                    .columns
                    .iter()
                    .zip(&atom.args)
                    .filter_map(|(c, a)| cq::rust::variable(a).map(|v| (c.name.clone(), v.clone())))
                    .collect::<Vec<_>>();
                let constants = super::occurrence_constants(declaration, atom);
                self.schedule.atom(&atom.relation, &mapping, constants)
            }
            cq::BodyItem::Block(block) => {
                let prefix = cq::blocks::bound_before(self.body, clause, &self.context);
                let facts = block.analyze(&prefix);
                let planned = self
                    .steps
                    .iter()
                    .find_map(|step| match step {
                        vo::Step::Block(b, _) if b.source.clause() == clause => Some(b),
                        _ => None,
                    })
                    .expect("VO block coverage");
                let correlated = planned.placement == vo::blocks::Placement::Correlated;
                let over = correlated.then(|| self.seed());
                // The finite domain is the current prefix projected to F: VO
                // binds F before the block and before any reduction reading it.
                let domain = (!correlated && block.needs_domain(&facts)).then(|| {
                    let input = self.seed();
                    self.schedule
                        .project(input, &facts.free.values().cloned().collect::<Vec<_>>())
                });
                let common: BTreeSet<_> = facts
                    .free
                    .keys()
                    .chain(facts.interface.keys())
                    .cloned()
                    .collect();
                let mut originals = Vec::new();
                let mut operands = Vec::new();
                let columns = facts.free.values().cloned().collect::<Vec<_>>();
                for (branch, plan) in block.branches.iter().zip(&planned.branches)
                {
                    let mut input = over.or(domain);
                    if correlated && block.kind == cq::blocks::Kind::Fallback {
                        for previous in &originals {
                            input = Some(self.schedule.anti(
                                input.expect("correlated seed"),
                                *previous,
                                &columns,
                            ));
                        }
                    }
                    let mut child = Body {
                        schedule: self.schedule,
                        source: self.source,
                        body: &branch.body,
                        steps: &plan.steps,
                        context: if correlated {
                            prefix.clone()
                        } else {
                            facts.free.keys().cloned().collect()
                        },
                        heading: if correlated {
                            self.heading.clone()
                        } else if domain.is_some() {
                            facts.free.keys().cloned().collect()
                        } else {
                            BTreeSet::new()
                        },
                        prefix: input,
                        relations: BTreeMap::new(),
                        reads: BTreeSet::new(),
                        correlated: BTreeSet::new(),
                        placed_rust: BTreeSet::new(),
                    };
                    child.visit();
                    let input = child.seed();
                    let kept: BTreeSet<_> = if correlated {
                        self.heading.union(&common).cloned().collect()
                    } else {
                        common.clone()
                    };
                    let attributes = kept
                        .iter()
                        .map(|v| Ident::new(v, proc_macro2::Span::call_site()))
                        .collect::<Vec<_>>();
                    let original = self.schedule.project(input, &attributes);
                    let mut positive = original;
                    if !correlated && block.kind == cq::blocks::Kind::Fallback {
                        for previous in &originals {
                            positive = self.schedule.anti(positive, *previous, &columns);
                        }
                    }
                    originals.push(original);
                    operands.push(positive);
                }
                let origin = ra::BlockOrigin {
                        block_token: Default::default(),
                        free_token: Default::default(),
                        free_bracket: Default::default(),
                        free: facts.free.values().cloned().collect(),
                        interface_token: Default::default(),
                        interface_bracket: Default::default(),
                        interface: facts.interface.values().cloned().collect(),
                        fallback: (block.kind == cq::blocks::Kind::Fallback).then(Default::default),
                        over: None,
                        domain: None,
                };
                if correlated {
                    self.correlated.insert(clause);
                }
                self.schedule.union(&operands, origin, over, domain)
            }
            _ => unreachable!("VO relation occurrence"),
        };
        self.relations.insert(clause, result);
        result
    }
    fn read(&mut self, atom: &vo::Subatom) {
        self.reads.insert(atom.clause());
        let mut right = self.relation(atom.clause());
        let partial = atom.variables().count() != atom.columns.len();
        let check = atom
            .variables()
            .all(|(_, v)| self.heading.contains(&symbol_name(v)));
        if partial && !check {
            right = self.schedule.project(
                right,
                &atom.variables().map(|(_, v)| v.clone()).collect::<Vec<_>>(),
            );
        }
        self.prefix = Some(match self.prefix {
            Some(left) => self.schedule.join(left, right, partial && check),
            None => right,
        });
        self.heading
            .extend(atom.variables().map(|(_, v)| symbol_name(v)));
    }
    fn environment(
        &mut self,
        guard: Option<&cq_syntax::Filter>,
        binding: Option<&cq_syntax::Let>,
        generator: Option<&cq_syntax::For>,
    ) -> ra::Environment {
        let clause = self
            .body
            .iter()
            .enumerate()
            .find(|(c, item)| {
                !self.placed_rust.contains(c)
                    && match item {
                        cq::BodyItem::Filter(value) => guard == Some(value),
                        cq::BodyItem::Let(value) => binding == Some(&**value),
                        cq::BodyItem::For(value) => generator == Some(&**value),
                        _ => false,
                    }
            })
            .map(|(c, _)| c)
            .expect("VO Rust source slot");
        self.placed_rust.insert(clause);
        let scope = cq::blocks::bound_before(self.body, clause, &self.context);
        let names = self.body[clause]
            .rust_step()
            .expect("VO Rust source slot")
            .free_variables
            .dependencies(&scope);
        ra::Environment {
            scope_token: Default::default(),
            bracket: Default::default(),
            variables: names
                .iter()
                .map(|name| Ident::new(name, proc_macro2::Span::call_site()))
                .collect(),
        }
    }
    fn visit(&mut self) {
        for step in self.steps {
            match step {
                vo::Step::Atom(a, _) => self.read(a),
                vo::Step::Block(b, _) => {
                    let node = self.relation(b.source.clause());
                    if self.correlated.contains(&b.source.clause()) {
                        self.reads.insert(b.source.clause());
                        self.prefix = Some(node);
                        self.heading
                            .extend(b.source.variables().map(|(_, v)| symbol_name(v)));
                    } else {
                        self.read(&b.source);
                    }
                }
                vo::Step::Equality(e, _) => {
                    let input = self.seed();
                    self.prefix = Some(self.schedule.select(
                        input,
                        ra::Theta {
                            equalities: std::iter::once(e.clone()).collect(),
                            constraints: Default::default(),
                        },
                    ));
                }
                vo::Step::Constraint(c, _) => {
                    let input = self.seed();
                    let mut theta = ra::Theta::default();
                    theta.push_constraint(c.clone());
                    self.prefix = Some(self.schedule.select(input, theta));
                }
                vo::Step::Restrict(r, _) => {
                    let occurrence = self.relation(r.clause());
                    let mut theta = ra::Theta::default();
                    theta.push_constraint(r.predicate.clone());
                    let restricted = self.schedule.select(occurrence, theta);
                    self.relations.insert(r.clause(), restricted);
                }
                vo::Step::Negation(negation, _) => {
                    let clause = self
                        .body
                        .iter()
                        .enumerate()
                        .find_map(|(clause, item)| {
                            (!self.reads.contains(&clause)
                                && matches!(item, cq::BodyItem::Negation(n) if n == negation))
                            .then_some(clause)
                        })
                        .expect("VO negation coverage");
                    let declaration = self
                        .source
                        .inputs()
                        .find(|d| symbol_name(&d.name) == symbol_name(&negation.atom.relation))
                        .expect("contracted negative input");
                    let bound = cq::blocks::bound_before(self.body, clause, &self.context);
                    let input = self.seed();
                    self.prefix = Some(super::negation::obligation(
                        self.schedule,
                        declaration,
                        negation,
                        &bound,
                        input,
                    ));
                    self.reads.insert(clause);
                }
                vo::Step::Guard(g, _) => {
                    let input = self.seed();
                    let environment = self.environment(Some(g), None, None);
                    self.prefix = Some(self.schedule.guard(input, g, Some(environment)));
                }
                vo::Step::Let(b, _) => {
                    let input = self.seed();
                    let environment = self.environment(None, Some(b), None);
                    self.prefix = Some(self.schedule.bind(input, b, Some(environment)));
                    self.heading
                        .extend(cq::rust::bindings(&b.pattern).iter().map(symbol_name));
                }
                vo::Step::For(b, _) => {
                    let input = self.seed();
                    let environment = self.environment(None, None, Some(b));
                    self.prefix = Some(self.schedule.for_bind(input, b, Some(environment)));
                    self.heading
                        .extend(cq::rust::bindings(&b.pattern).iter().map(symbol_name));
                }
                vo::Step::Reduce(r, _) => {
                    let vo::Reduction::With(_, support) = &r.reduction;
                    let left = self.relation(r.clause());
                    let right = self.relation(support.clause());
                    // Equal spellings are natural keys; distinct equality aliases
                    // require explicit matches rather than an existence-only semi.
                    let hypergraph = cq::hypergraph::Analysis::of(self.source);
                    let target_variables: Vec<_> = match &self.body[r.clause()] {
                        cq::BodyItem::Positive { atom } => atom.variables().cloned().collect(),
                        cq::BodyItem::Block(b) => {
                            let prefix = self.body.iter().take(r.clause()).cloned().collect();
                            let names = cq::blocks::bound(&prefix)
                                .into_keys()
                                .chain(self.context.iter().cloned())
                                .collect();
                            let facts = b.analyze(&names);
                            facts
                                .free
                                .values()
                                .chain(facts.interface.values())
                                .cloned()
                                .collect()
                        }
                        _ => unreachable!("reduction target"),
                    };
                    let mut equalities = Vec::new();
                    for v in &target_variables {
                        for (_, s) in support.variables() {
                            if v != s && hypergraph.equivalent(&symbol_name(v), &symbol_name(s)) {
                                equalities.push(ra::Equality {
                                    left: v.clone(),
                                    eq_token: Default::default(),
                                    right: ra::EqualityValue::Variable(s.clone()),
                                });
                            }
                        }
                    }

                    let node = if equalities.is_empty() {
                        self.schedule.join(left, right, true)
                    } else {
                        let joined = self.schedule.join(left, right, false);
                        let selected = self.schedule.select(
                            joined,
                            ra::Theta {
                                equalities: equalities.into_iter().collect(),
                                constraints: Default::default(),
                            },
                        );
                        self.schedule.project(selected, &target_variables)
                    };
                    if self.reads.contains(&r.clause()) {
                        if let Some(prefix) = self.prefix {
                            self.prefix = Some(self.schedule.join(prefix, node, true));
                        }
                    }
                    self.relations.insert(r.clause(), node);
                }
            }
        }
    }
}
