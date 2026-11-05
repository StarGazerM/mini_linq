//! Lookup/expand is an authored VO schedule, not a new algebra operator.
//! Expansions are coarse: exactly one full correlated row after each lookup.
//! CK-Le checks this authored schedule before checked RA simplification may
//! erase an expansion's unobservable columns under canonical answer semantics.
//! Rust steps retain their source payloads and depend only on their free
//! variables; neither source order nor earlier unrelated expansions constrain them.
use std::collections::{BTreeMap, BTreeSet};

use crate::{cq, vo};
use cq::symbol_name;
use cq_order::le::check;

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
            // An empty preliminary probe cannot constrain a later introduction:
            // that occurrence's expansion already establishes nonemptiness.
            if partial.variables().next().is_none() && !atom.provides.is_empty() {
                continue;
            }
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
        // Demand may skip an unrelated connected ear (for example a guard
        // demanding a disconnected aggregate). I3 still requires finishing
        // ready connected growth before starting that disconnected component.
        if !self.atoms[&clause].provides.is_subset(&self.bound)
            && self.atoms[&clause].key.is_disjoint(&self.bound)
            && self.atoms[&clause].provides.is_disjoint(&self.bound)
        {
            while let Some(connected) = self.atoms.iter().find_map(|(&pending, atom)| {
                (!atom.expanded
                    && atom.key.is_subset(&self.bound)
                    && !atom.provides.is_subset(&self.bound)
                    && (!atom.key.is_disjoint(&self.bound)
                        || !atom.provides.is_disjoint(&self.bound)))
                .then_some(pending)
            }) {
                self.expand(connected);
            }
        }
        self.lookups();
        let atom = self.atoms.get_mut(&clause).expect("pending atom");
        self.steps
            .push(vo::Step::Atom(atom.full.clone(), Default::default()));
        atom.expanded = true;
        // The full expansion also discharges an omitted empty lookup.
        atom.lookup = true;
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

// Formal rule Plan-Le: Emit ready ear lookups and postpone coarse expansions to demand, without bypassing ready connected expansions.
pub(super) fn compile(source: &vo::Program) -> syn::Result<vo::Program> {
    let analysis = vo::order::analyze(source)?;
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
        // A drained zero-column guard may precede the stream, but does not
        // replace the written outer occurrence or own an expansion phase.
        let outer = body.iter().position(|item|
            matches!(item, cq::BodyItem::Positive { .. } | cq::BodyItem::Block(_)));
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
        let mut placed_rust = BTreeSet::new();
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
                        .find_map(|(clause, item)| {
                            let identical = match (&step, item) {
                                (vo::Step::Guard(a, _), cq::BodyItem::Filter(b)) => a == b,
                                (vo::Step::Let(a, _), cq::BodyItem::Let(b)) => a == b,
                                (vo::Step::For(a, _), cq::BodyItem::For(b)) => a == b,
                                _ => false,
                            };
                            (identical && !placed_rust.contains(&clause))
                                .then(|| (clause, item.rust_step().expect("Rust step")))
                        })
                        .expect("checked Rust expression");
                    placed_rust.insert(clause);
                    let environment = analysis.bound_before[index].iter().cloned().collect();
                    state.demand(&rust.free_variables.dependencies(&environment));
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
                vo::Step::Restrict(..) => unreachable!("LE precedes O-Restrict"),
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
    // Drain retained checks; empty preliminary probes of later introducers
    // are omitted by the phase producer, not interpreted as root streams.
    cq_order::drain_ready_checks(&mut target)?;
    check(&source.cq, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_guard_demand_finishes_ready_connected_expansions_first() {
        let source: vo::Program = syn::parse_quote! {
            struct MaximumRevenue;
            relation Revenue(supplier:i32,revenue:i32);
            relation Supplier(supplier:i32,phone:i32);
            relation Phone(phone:i32,label:i32);
            relation Maximum(maximum:i32);
            relation answer(supplier:i32,label:i32,revenue:i32);
            answer(supplier,label,revenue) :-
                Revenue(supplier,revenue), Supplier(supplier,phone),
                Phone(phone,label), Maximum(maximum), if revenue == maximum;
            valuation {
                Revenue@0(supplier,revenue);
                Supplier@1(supplier,phone);
                Phone@2(phone,label);
                Maximum@3(maximum);
                if revenue == maximum;
            }
        };
        vo::contract::check(&source).unwrap();
        let target = compile(&source).unwrap();
        let analysis = vo::contract::check(&target).unwrap();
        let introducing: Vec<_> = target.valuation.steps.iter()
            .zip(&analysis.introduces)
            .filter_map(|(step, introduced)| match step {
                vo::Step::Atom(atom, _) if !introduced.is_empty() => Some(atom.clause()),
                _ => None,
            })
            .collect();
        assert_eq!(introducing, [0, 1, 2, 3]);
        assert!(target.valuation.steps.iter().all(|step|
            !matches!(step, vo::Step::Atom(atom, _) if atom.variables().next().is_none())));
        check(&source.cq, &target).unwrap();
    }

    #[test]
    fn ready_membership_expansion_precedes_an_introducing_let() {
        let source: vo::Program = syn::parse_quote! {
            struct Q;
            relation R(x:i32); relation S(x:i32);
            relation answer(x:i32,y:i32);
            answer(x,y) :- R(x), let y:i32 = value(x), S(x);
            valuation { R@0(x); let y:i32 = value(x); S@2(x); }
        };
        let target = compile(&source).unwrap();
        assert!(matches!(&target.valuation.steps[0], vo::Step::Atom(a, _) if a.clause() == 0));
        assert!(matches!(&target.valuation.steps[1], vo::Step::Atom(a, _) if a.clause() == 2));
        assert!(matches!(&target.valuation.steps[2], vo::Step::Atom(a, _) if a.clause() == 2));
        assert!(matches!(&target.valuation.steps[3], vo::Step::Let(..)));
        check(&source.cq, &target).unwrap();
    }

    #[test]
    fn reordered_rust_payloads_keep_dependencies_and_duplicate_occurrences() {
        let source: vo::Program = syn::parse_quote! {
            struct Q;
            relation R(x:i32); relation S(y:i32);
            relation answer(x:i32,y:i32,z:i32,w:i32);
            answer(x,y,z,w) :- R(x), S(y), if keep(y),
                let z:i32 = *y + 1, for w in values(z), if keep(x), if keep(x);
            valuation {
                R@0(x);
                if keep(x);
                if keep(x);
                S@1(y);
                let z:i32 = *y + 1;
                for w in values(z);
                if keep(y);
            }
        };
        let target = compile(&source).unwrap();
        check(&source.cq, &target).unwrap();
        let analysis = vo::contract::check(&target).unwrap();
        let guards: Vec<_> = target
            .valuation
            .steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| matches!(step, vo::Step::Guard(..)).then_some(index))
            .collect();
        assert_eq!(guards.len(), 3);
        for &index in &guards[..2] {
            assert!(analysis.bound_before[index].iter().any(|name| name == "x"));
            assert!(!analysis.bound_before[index].iter().any(|name| name == "y"));
        }
        assert!(
            analysis.bound_before[guards[2]]
                .iter()
                .any(|name| name == "y")
        );
    }
}
