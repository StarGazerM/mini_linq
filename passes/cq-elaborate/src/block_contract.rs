//! State the fixed block equations as structural obligations. Each occurrence
//! has its given positional path; there is no mapping search or compilation.
//! Branches project to the common heading. Fallback negatives refer to the
//! original branches, while positive reductions remain separate references.

use cq_lang_ra::{self as ra, contract::Schedule};
use cq_lang_vo::{self as vo, cq};
use cq_syntax::{Occurrence, symbol_name};
use std::collections::{BTreeMap, BTreeSet};
use syn::Ident;
use vo::syntax as cq_syntax;

fn paths(
    body: &cq_syntax::CommaList<cq::BodyItem>,
    parent: Option<(&Occurrence, usize)>,
    result: &mut Vec<Occurrence>,
) {
    for (c, item) in body.iter().enumerate() {
        let path = parent.map_or_else(|| Occurrence::new(c), |(p, j)| p.child(j, c));
        match item {
            cq::BodyItem::Positive { .. } => result.push(path),
            cq::BodyItem::Block(block) => {
                result.push(path.clone());
                for (j, b) in block.branches.iter().enumerate() {
                    paths(&b.body, Some((&path, j)), result);
                }
            }
            _ => (),
        }
    }
}

pub(super) fn check(source: &vo::Program, target: &Schedule) -> Result<(), String> {
    let mut schedule = Schedule::default();
    let mut occurrences = Vec::new();
    paths(&source.cq.query.body, None, &mut occurrences);
    schedule.occurrence_paths(occurrences);
    let mut obligation = Body {
        schedule: &mut schedule,
        source: &source.cq,
        body: &source.cq.query.body,
        steps: &source.valuation.steps,
        parent: None,
        context: BTreeSet::new(),
        heading: BTreeSet::new(),
        prefix: None,
        relations: BTreeMap::new(),
        reads: BTreeSet::new(),
        correlated: BTreeSet::new(),
        placed_rust: BTreeSet::new(),
    };
    obligation.visit();
    let input = obligation
        .prefix
        .unwrap_or_else(|| obligation.schedule.unit());
    let output = obligation.schedule.project(
        input,
        &source
            .cq
            .query
            .head
            .variables()
            .cloned()
            .collect::<Vec<_>>(),
    );
    obligation.schedule.finish(output, &source.cq.query.head);
    schedule.check(target, &[])
}

struct Body<'a, 's> {
    schedule: &'s mut Schedule,
    source: &'a cq::Program,
    body: &'a cq_syntax::CommaList<cq::BodyItem>,
    steps: &'a [vo::Step],
    parent: Option<(Occurrence, usize)>,
    context: BTreeSet<String>,
    heading: BTreeSet<String>,
    prefix: Option<usize>,
    relations: BTreeMap<usize, usize>,
    reads: BTreeSet<usize>,
    correlated: BTreeSet<usize>,
    placed_rust: BTreeSet<usize>,
}
impl Body<'_, '_> {
    fn path(&self, clause: usize) -> Occurrence {
        self.parent
            .as_ref()
            .map_or_else(|| Occurrence::new(clause), |(p, j)| p.child(*j, clause))
    }
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
                    .find(|d| d.name == atom.relation)
                    .expect("contracted atom");
                let mapping = declaration
                    .columns
                    .iter()
                    .zip(atom.variables())
                    .map(|(c, v)| (c.name.clone(), v.clone()))
                    .collect::<Vec<_>>();
                self.schedule
                    .atom_path(&self.path(clause), &atom.relation, &mapping)
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
                let over = facts.rust.then(|| self.seed());
                let domain = (!facts.rust
                    && block.branches.iter().any(|branch| {
                        let generated: BTreeSet<_> = cq::blocks::walk(&branch.body)
                            .into_iter()
                            .filter_map(cq::BodyItem::atom)
                            .flat_map(cq_syntax::Atom::variables)
                            .map(symbol_name)
                            .collect();
                        facts.free.keys().any(|name| !generated.contains(name))
                    }))
                .then(|| {
                    let saved = self.prefix;
                    if !facts.free.keys().all(|name| self.heading.contains(name)) {
                        let mut input = None;
                        for (c, item) in self.body.iter().take(clause).enumerate() {
                            match item {
                                cq::BodyItem::Positive { .. } => {
                                    let right = self.relation(c);
                                    input = Some(match input {
                                        None => right,
                                        Some(left) => self.schedule.join(left, right, false),
                                    });
                                }
                                cq::BodyItem::Equality(eq) => {
                                    if let Some(left) = input {
                                        input = Some(self.schedule.select(
                                            left,
                                            &ra::Theta {
                                                equalities: [eq.clone()].into_iter().collect(),
                                                constraints: Default::default(),
                                            },
                                        ));
                                    }
                                }
                                cq::BodyItem::Constraint(c) => {
                                    if let Some(left) = input {
                                        input = Some(self.schedule.select(
                                            left,
                                            &ra::Theta {
                                                equalities: Default::default(),
                                                constraints: [c.clone()].into_iter().collect(),
                                            },
                                        ));
                                    }
                                }
                                _ => (),
                            }
                        }
                        self.prefix = input;
                    }
                    let input = self.seed();
                    self.prefix = saved;
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
                for (j, (branch, plan)) in block.branches.iter().zip(&planned.branches).enumerate()
                {
                    let mut input = over.or(domain);
                    if facts.rust && block.kind == cq::blocks::Kind::Fallback {
                        for previous in &originals {
                            input = Some(self.schedule.anti(
                                input.expect("correlated seed"),
                                *previous,
                                &columns,
                            ));
                        }
                    }
                    let child_path = self.path(clause);
                    let mut child = Body {
                        schedule: self.schedule,
                        source: self.source,
                        body: &branch.body,
                        steps: &plan.steps,
                        parent: Some((child_path, j)),
                        context: if facts.rust {
                            prefix.clone()
                        } else {
                            facts.free.keys().cloned().collect()
                        },
                        heading: if facts.rust {
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
                    let kept: BTreeSet<_> = if facts.rust {
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
                    if !facts.rust && block.kind == cq::blocks::Kind::Fallback {
                        for previous in &originals {
                            positive = self.schedule.anti(positive, *previous, &columns);
                        }
                    }
                    originals.push(original);
                    operands.push(positive);
                }
                let union = ra::Union {
                    union_token: Default::default(),
                    operands: Default::default(),
                    origin: Some(ra::BlockOrigin {
                        block_token: Default::default(),
                        at: Default::default(),
                        occurrence: self.path(clause),
                        free_token: Default::default(),
                        free_bracket: Default::default(),
                        free: facts.free.values().cloned().collect(),
                        interface_token: Default::default(),
                        interface_bracket: Default::default(),
                        interface: facts.interface.values().cloned().collect(),
                        fallback: (block.kind == cq::blocks::Kind::Fallback).then(Default::default),
                        over: None,
                        domain: None,
                    }),
                };
                if facts.rust {
                    self.correlated.insert(clause);
                }
                self.schedule.union_frame(&operands, &union, over, domain)
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
        let names = cq::blocks::bound_before(self.body, clause, &self.context);
        ra::Environment {
            path: Some(self.path(clause)),
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
                        &ra::Theta {
                            equalities: std::iter::once(e.clone()).collect(),
                            constraints: Default::default(),
                        },
                    ));
                }
                vo::Step::Constraint(c, _) => {
                    let input = self.seed();
                    self.prefix = Some(self.schedule.select(
                        input,
                        &ra::Theta {
                            equalities: Default::default(),
                            constraints: std::iter::once(c.clone()).collect(),
                        },
                    ));
                }
                vo::Step::Guard(g, _) => {
                    let input = self.seed();
                    let environment = self.environment(Some(g), None, None);
                    self.prefix = Some(self.schedule.guard_in(input, g, Some(&environment)));
                }
                vo::Step::Let(b, _) => {
                    let input = self.seed();
                    let environment = self.environment(None, Some(b), None);
                    self.prefix = Some(self.schedule.bind_in(input, b, Some(&environment)));
                    self.heading
                        .extend(cq::rust::bindings(&b.pattern).iter().map(symbol_name));
                }
                vo::Step::For(b, _) => {
                    let input = self.seed();
                    let environment = self.environment(None, None, Some(b));
                    self.prefix = Some(self.schedule.for_in(input, b, Some(&environment)));
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
                            &ra::Theta {
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
