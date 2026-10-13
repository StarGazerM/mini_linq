//! Exact branch construction and a separate structural shape obligation.
use super::*;

impl State<'_> {
    pub(super) fn branch_local(&self, variables: BTreeSet<String>) -> bool {
        !variables.is_empty() && self.join_branches.is_some_and(|groups| groups.branches.iter().any(|branch| {
            let members = branch.clauses();
            members.is_subset(&self.joined_branch_atoms) && variables.is_subset(&members.iter().flat_map(|c| self.atoms[c].variables().map(symbol_name)).collect())
        }))
    }
    /// Formal rule E-JoinBranch: Materialize each independent exact atom join once, project its interface, and join the prefix against that result.
    pub(super) fn join_branch(&mut self, branch: &valuation::JoinBranch) {
        let members = branch.clauses();
        // Root the connected member graph at its first scheduled occurrence.
        // Grow a deterministic spanning tree, then build each child subtree
        // before probing it from its parent. Unlike reversing a flat sequence,
        // this never creates a Cartesian Supplier/Nation × Part intermediate.
        let ordered: Vec<_> = self.steps.iter().filter_map(|step| match step {
            valuation::Step::Atom(a, _) if members.contains(&a.clause()) => Some(a.clause()),
            _ => None,
        }).collect();
        let root = ordered[0];
        let mut pending: BTreeSet<_> = members.iter().copied().filter(|c| *c != root).collect();
        let mut frontier = vec![root];
        let mut children: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut next = 0;
        while next < frontier.len() {
            let parent = frontier[next];
            next += 1;
            let attached: Vec<_> = ordered.iter().copied().filter(|c| pending.contains(c) && !self.hypergraph.shared(parent, *c).is_empty()).collect();
            for child in &attached { pending.remove(child); }
            frontier.extend(attached.iter().copied());
            children.insert(parent, attached);
        }
        assert!(pending.is_empty(), "checked connected branch");
        fn build(state: &mut State<'_>, clause: usize, children: &BTreeMap<usize, Vec<usize>>) -> Ident {
            let mut left = state.relation(clause);
            for &child in &children[&clause] {
                let right = build(state, child, children);
                left = state.emit(ra::Operator::NaturalJoin(Box::new(ra::NaturalJoin {
                    natural_join_token: Default::default(), left, with_token: Default::default(), right,
                })));
            }
            left
        }
        let mut prefix = Some(build(self, root, &children));
        // Equality-class aliases have already been solved by `relation`. Keep
        // each canonical interface column once, even when CQ spells aliases.
        let own: BTreeSet<_> = members.iter().flat_map(|c| self.atoms[c].variables().map(symbol_name)).collect();
        // Predicates spanning two branch atoms cannot be pushed to either
        // rename. Evaluate them on the exact branch join before projection.
        let constraints: Vec<_> = self.constraints.iter().filter(|(c, targets)| targets.is_disjoint(&members) && c.variables().map(symbol_name).collect::<BTreeSet<_>>().is_subset(&own))
            .map(|(c, _)| { let mut c = c.clone(); c.map_terms(&mut |term| { if let cq::constraint::Term::Variable(v) = term { *v = self.canonical(v); } }); c }).collect();
        if !constraints.is_empty() { prefix = Some(self.select_constraints(prefix.unwrap(), constraints)); }
        let equalities: Vec<_> = self.equalities.iter().filter(|(e, targets)| targets.is_disjoint(&members) && e.variables().map(symbol_name).collect::<BTreeSet<_>>().is_subset(&own))
            .map(|(e, _)| ra::Equality { left: self.canonical(&e.left), right: match &e.right {
                ra::EqualityValue::Variable(v) => ra::EqualityValue::Variable(self.canonical(v)),
                other => other.clone(),
            }, ..e.clone() }).filter(|e| !matches!(&e.right, ra::EqualityValue::Variable(v) if symbol_name(v) == symbol_name(&e.left))).collect();
        if !equalities.is_empty() { prefix = Some(self.select(prefix.unwrap(), equalities)); }
        let columns: BTreeMap<_, _> = branch.columns.iter().map(|v| {
            let canonical = self.canonical(v);
            (symbol_name(&canonical), canonical)
        }).collect();
        let right = self.project(prefix.expect("nonempty checked join branch"), columns.values().cloned().collect());
        self.prefix = Some(match self.prefix.take() {
            None => right,
            Some(left) => self.emit(ra::Operator::NaturalJoin(Box::new(ra::NaturalJoin {
                natural_join_token: Default::default(), left, with_token: Default::default(), right,
            }))),
        });
        self.heading.extend(columns.into_keys());
        self.read_clauses.extend(members.iter().copied());
        self.joined_branch_atoms.extend(members);
    }
}

/// Formal rule CK-JoinBranch: Require an exact projected branch DAG for every VO atom group in addition to global occurrence, predicate, and Before correspondence.
pub(super) fn check(source: &valuation::Program, target: &ra::Program) -> syn::Result<()> {
    let Some(groups) = &source.valuation.join_branches else { return Ok(()); };
    let canonical = aliases(&source.cq);
    let mut live = BTreeSet::from([symbol_name(&target.plan.output.input)]);
    for definition in target.plan.definitions.iter().rev() {
        if !live.contains(&symbol_name(&definition.result)) { continue; }
        let inputs: Vec<_> = match &definition.operator {
            ra::Operator::NaturalJoin(j) => vec![&j.left, &j.right],
            ra::Operator::Semijoin(j) => vec![&j.left, &j.right],
            ra::Operator::Anti(j) => vec![&j.left, &j.right],
            ra::Operator::Select(s) => vec![&s.input],
            ra::Operator::Project(p) => vec![&p.input],
            ra::Operator::Guard(g) => vec![&g.input],
            ra::Operator::Bind(b) => vec![&b.input],
            ra::Operator::Union(u) => u.operands.iter().collect(),
            _ => Vec::new(),
        };
        live.extend(inputs.into_iter().map(symbol_name));
    }
    let mut lineage: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut projections = Vec::new();
    for definition in &target.plan.definitions {
        let occurrences = match &definition.operator {
            ra::Operator::Rename(r) => BTreeSet::from([r.occurrence.clause()]),
            ra::Operator::NaturalJoin(j) => lineage[&symbol_name(&j.left)].union(&lineage[&symbol_name(&j.right)]).copied().collect(),
            ra::Operator::Select(s) => lineage[&symbol_name(&s.input)].clone(),
            ra::Operator::Semijoin(s) => lineage[&symbol_name(&s.left)].clone(),
            ra::Operator::Project(p) => {
                let atoms = lineage[&symbol_name(&p.input)].clone();
                if live.contains(&symbol_name(&definition.result)) {
                    projections.push((atoms.clone(), p.attributes.iter().map(symbol_name).collect::<BTreeSet<_>>()));
                }
                atoms
            }
            _ => BTreeSet::new(),
        };
        lineage.insert(symbol_name(&definition.result), occurrences);
    }
    for branch in &groups.branches {
        let columns: BTreeSet<_> = branch.columns.iter().map(|v| canonical.get(&symbol_name(v)).map(symbol_name).unwrap_or_else(|| symbol_name(v))).collect();
        if !projections.iter().any(|(atoms, heading)| *atoms == branch.clauses() && *heading == columns) {
            return Err(syn::Error::new_spanned(branch, "RA must contain the exact projected independent join branch recorded in VO"));
        }
    }
    Ok(())
}
