//! Lookup/expand is an authored VO schedule, not a new algebra operator.
//! Expansions are coarse: exactly one full correlated row after each lookup.
//! CK-Le checks this authored schedule before checked RA simplification may
//! erase an expansion's unobservable columns under canonical answer semantics.
use std::collections::{BTreeMap, BTreeSet};

use crate::{cq, vo};
use cq::symbol_name;

// Formal rule CK-Le: Check occurrence phases and column ownership independently of scheduling.
pub(super) fn check(normal: &cq::Program, target: &vo::Program) -> syn::Result<()> {
    if normal != &target.cq {
        return Err(syn::Error::new_spanned(
            target,
            "LE must preserve normalized CQ",
        ));
    }
    let analysis = vo::contract::check(target)?;
    fn scope(
        body: &cq::CommaList<cq::BodyItem>,
        steps: &[vo::Step],
        analysis: &vo::Analysis,
        context: &BTreeSet<String>,
    ) -> syn::Result<()> {
        let mut written = context.clone();
        let mut ownership = BTreeMap::new();
        let outer = body.iter().position(|item| {
            matches!(item, cq::BodyItem::Positive { .. } | cq::BodyItem::Block(_))
        });
        for (clause, item) in body.iter().enumerate() {
            match item {
                cq::BodyItem::Positive { atom } => {
                    let columns: BTreeSet<_> = atom.variables().map(symbol_name).collect();
                    ownership.insert(
                        clause,
                        columns
                            .difference(&written)
                            .cloned()
                            .collect::<BTreeSet<_>>(),
                    );
                    written.extend(columns);
                }
                cq::BodyItem::Let(_) | cq::BodyItem::For(_) => written.extend(
                    item.rust_step()
                        .expect("Rust binding")
                        .bindings
                        .iter()
                        .map(symbol_name),
                ),
                cq::BodyItem::Block(block) => {
                    written.extend(block.analyze(&written).interface.into_keys())
                }
                _ => (),
            }
        }
        let first = steps.iter().find_map(|step| match step {
            vo::Step::Atom(a, _) => Some(a.clause()),
            vo::Step::Block(b, _) => Some(b.source.clause()),
            _ => None,
        });
        if first != outer {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "LE must retain the written outer stream",
            ));
        }
        let mut phases: BTreeMap<usize, usize> = BTreeMap::new();
        for (index, step) in steps.iter().enumerate() {
            match step {
                vo::Step::Atom(atom, _) => {
                    let phase = phases.entry(atom.clause()).or_default();
                    let stream = Some(atom.clause()) == outer;
                    let full = stream || *phase == 1;
                    if *phase >= if stream { 1 } else { 2 }
                        || (full && atom.variables().count() != atom.columns.len())
                        || (!full && !analysis.introduces[index].is_empty())
                        || (full
                            && analysis.introduces[index]
                                .iter()
                                .cloned()
                                .collect::<BTreeSet<_>>()
                                != ownership[&atom.clause()])
                    {
                        return Err(syn::Error::new_spanned(
                            atom,
                            "LE requires a non-introducing lookup followed by one coarse expansion with written column ownership",
                        ));
                    }
                    *phase += 1;
                }
                vo::Step::Block(block, _) => {
                    let cq::BodyItem::Block(source) = &body[block.source.clause()] else {
                        unreachable!("checked VO block")
                    };
                    let bound = analysis.bound_before[index].iter().cloned().collect();
                    let facts = source.analyze(&bound);
                    let context = if facts.rust {
                        bound
                    } else {
                        facts.free.into_keys().collect()
                    };
                    for (branch, (source, facts)) in block.branches.iter().zip(
                        source
                            .branches
                            .iter()
                            .zip(&analysis.branches[&block.source.clause()]),
                    ) {
                        scope(&source.body, &branch.steps, facts, &context)?;
                    }
                }
                _ => (),
            }
        }
        for (&clause, atom) in &analysis.atoms {
            if body[clause].atom().is_none() {
                continue;
            }
            if phases.get(&clause).copied() != Some(if Some(clause) == outer { 1 } else { 2 }) {
                return Err(syn::Error::new_spanned(
                    atom,
                    "LE occurrence is missing its lookup or expansion",
                ));
            }
        }
        // WF-VO independently establishes all first-use and Rust Before obligations,
        // full correlated rows, block interfaces, and fallback ancestry.
        Ok(())
    }
    scope(
        &normal.query.body,
        &target.valuation.steps,
        &analysis,
        &BTreeSet::new(),
    )
}

struct Atom {
    full: vo::Subatom,
    key: BTreeSet<String>,
    provides: BTreeSet<String>,
    lookup: bool,
    expanded: bool,
}

struct Scope {
    atoms: BTreeMap<usize, Atom>,
    ears: Vec<usize>,
    bound: BTreeSet<String>,
    steps: Vec<vo::Step>,
    started: bool,
}

impl Scope {
    fn lookups(&mut self) {
        if !self.started {
            return;
        }
        for &clause in &self.ears {
            let atom = self.atoms.get_mut(&clause).expect("atom ear");
            // The lookup key is the currently available part of the written
            // join key. A cycle need not wait for its other endpoint.
            if atom.lookup || (!atom.key.is_empty() && atom.key.is_disjoint(&self.bound)) {
                continue;
            }
            let mut partial = atom.full.clone();
            partial.columns = partial
                .columns
                .iter()
                .map(|column| match column {
                    vo::Column::Variable(v) if self.bound.contains(&symbol_name(v)) => {
                        column.clone()
                    }
                    _ => vo::Column::Dropped(Default::default()),
                })
                .collect();
            self.steps.push(vo::Step::Atom(partial, Default::default()));
            atom.lookup = true;
        }
    }

    fn expand(&mut self, clause: usize) {
        if self.atoms[&clause].expanded {
            return;
        }
        let key = self.atoms[&clause].key.clone();
        self.demand(&key);
        self.lookups();
        let atom = self.atoms.get_mut(&clause).expect("pending atom");
        self.steps
            .push(vo::Step::Atom(atom.full.clone(), Default::default()));
        atom.expanded = true;
        self.bound.extend(atom.provides.iter().cloned());
        self.lookups();
    }

    fn demand(&mut self, demand: &BTreeSet<String>) {
        let needed: Vec<_> = self
            .atoms
            .iter()
            .filter_map(|(&clause, atom)| {
                (!atom.expanded && !atom.provides.is_disjoint(demand)).then_some(clause)
            })
            .collect();
        for clause in needed {
            self.expand(clause);
        }
    }
}

// Formal rule Plan-Le: Emit ready ear lookups and postpone coarse expansions to demand or Before boundaries.
pub(super) fn compile(source: &vo::Program) -> syn::Result<vo::Program> {
    let analysis = vo::contract::check(source)?;
    fn scope(
        body: &cq::CommaList<cq::BodyItem>,
        steps: &[vo::Step],
        analysis: &vo::Analysis,
    ) -> Vec<vo::Step> {
        let context: BTreeSet<_> = analysis
            .bound_before
            .first()
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let graph = cq::hypergraph::Analysis::of_body(body, &context);
        let outer = steps.iter().find_map(|step| match step {
            vo::Step::Atom(a, _) => Some(a.clause()),
            vo::Step::Block(b, _) => Some(b.source.clause()),
            _ => None,
        });
        let mut state = Scope {
            atoms: BTreeMap::new(),
            ears: Vec::new(),
            bound: context,
            steps: Vec::new(),
            started: false,
        };
        for (index, step) in steps.iter().enumerate() {
            if let vo::Step::Atom(atom, _) = step {
                if Some(atom.clause()) == outer {
                    continue;
                }
                let bound: BTreeSet<_> = analysis.bound_before[index].iter().cloned().collect();
                state.atoms.insert(
                    atom.clause(),
                    Atom {
                        full: atom.clone(),
                        key: atom
                            .variables()
                            .map(|(_, v)| symbol_name(v))
                            .filter(|v| bound.contains(v))
                            .collect(),
                        provides: analysis.introduces[index].iter().cloned().collect(),
                        lookup: false,
                        expanded: false,
                    },
                );
            }
        }
        // GYO ears first, followed by the Masks tree's cyclic-core clauses.
        for step in &graph.gyo_steps {
            if let cq::hypergraph::GyoStep::Absorb { child, .. } = step {
                if state.atoms.contains_key(child) && !state.ears.contains(child) {
                    state.ears.push(*child);
                }
            }
        }
        for &(clause, _) in &graph.reduction_tree {
            if state.atoms.contains_key(&clause) && !state.ears.contains(&clause) {
                state.ears.push(clause);
            }
        }
        for &clause in state.atoms.keys() {
            if !state.ears.contains(&clause) {
                state.ears.push(clause);
            }
        }
        let mut rust_clause = 0;
        for (index, step) in steps.iter().enumerate() {
            let mut step = step.clone();
            match &mut step {
                vo::Step::Atom(atom, _) if Some(atom.clause()) != outer => {
                    let key = state.atoms[&atom.clause()].key.clone();
                    state.demand(&key);
                    state.lookups();
                    continue;
                }
                vo::Step::Atom(atom, _) => {
                    state
                        .bound
                        .extend(atom.variables().map(|(_, v)| symbol_name(v)));
                    state.started = true;
                }
                vo::Step::Equality(e, _) => state.demand(&e.variables().map(symbol_name).collect()),
                vo::Step::Constraint(c, _) => {
                    state.demand(&c.variables().map(symbol_name).collect())
                }
                vo::Step::Negation(n, _) => {
                    let environment = analysis.bound_before[index].iter().cloned().collect();
                    state.demand(&n.variables(&environment).map(symbol_name).collect());
                }
                vo::Step::Guard(_, _) | vo::Step::Let(_, _) | vo::Step::For(_, _) => {
                    let (clause, rust) = body
                        .iter()
                        .enumerate()
                        .skip(rust_clause)
                        .find_map(|(c, item)| item.rust_step().map(|r| (c, r)))
                        .expect("checked Rust chain");
                    rust_clause = clause + 1;
                    let environment = analysis.bound_before[index].iter().cloned().collect();
                    state.demand(&rust.free_variables.dependencies(&environment));
                    let before: Vec<_> = state
                        .atoms
                        .keys()
                        .copied()
                        .filter(|c| *c < clause)
                        .collect();
                    for clause in before {
                        state.expand(clause);
                    }
                    state.bound.extend(rust.bindings.iter().map(symbol_name));
                }
                vo::Step::Block(block, _) => {
                    let cq::BodyItem::Block(written) = &body[block.source.clause()] else {
                        unreachable!("checked block")
                    };
                    let environment = analysis.bound_before[index].iter().cloned().collect();
                    let facts = written.analyze(&environment);
                    state.demand(&if facts.rust {
                        environment
                    } else {
                        facts.free.into_keys().collect()
                    });
                    for (j, branch) in block.branches.iter_mut().enumerate() {
                        branch.steps = scope(
                            &written.branches[j].body,
                            &branch.steps,
                            &analysis.branches[&block.source.clause()][j],
                        );
                    }
                    state.bound.extend(facts.interface.into_keys());
                    state.started = true;
                }
                vo::Step::Reduce(..) => unreachable!("LE precedes reduction"),
            }
            state.steps.push(step);
            state.lookups();
        }
        let remaining: Vec<_> = state.atoms.keys().copied().collect();
        for clause in remaining {
            state.expand(clause);
        }
        state.steps
    }
    let mut target = source.clone();
    target.valuation.steps = scope(&source.cq.query.body, &source.valuation.steps, &analysis);
    check(&source.cq, &target)?;
    Ok(target)
}
