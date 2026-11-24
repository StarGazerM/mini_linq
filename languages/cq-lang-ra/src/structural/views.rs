//! Positive split views as existential conjunctions, not DAG wiring.
use super::*;

#[derive(PartialEq, Eq)]
enum Relation {
    Occurrence(usize),
    Predicate(String),
}

struct Fact {
    relation: Relation,
    columns: Vec<usize>,
}

struct Query {
    facts: Vec<Fact>,
    output: HashMap<String, usize>,
    variables: usize,
}

impl Query {
    fn build(schedule: &Structure) -> Option<Self> {
        fn expand(
            schedule: &Structure,
            index: usize,
            facts: &mut Vec<Fact>,
            equalities: &mut Vec<(usize, usize)>,
            next: &mut usize,
        ) -> Option<HashMap<String, usize>> {
            let node = &schedule.nodes[index];
            let mut heading = match node.kind {
                Kind::Unit => HashMap::new(),
                Kind::Atom(occurrence) => {
                    let mut columns: Vec<_> = node.heading.iter().collect();
                    columns.sort_unstable_by(|a, b| a.1.cmp(b.1));
                    let mut heading = HashMap::new();
                    let mut arguments = Vec::with_capacity(columns.len());
                    for (name, _) in columns {
                        heading.insert(name.clone(), *next);
                        arguments.push(*next);
                        *next += 1;
                    }
                    facts.push(Fact { relation: Relation::Occurrence(occurrence), columns: arguments });
                    heading
                }
                Kind::Project | Kind::Select => {
                    expand(schedule, node.operands[0], facts, equalities, next)?
                }
                Kind::Join(semi) => {
                    let mut left = expand(schedule, node.operands[0], facts, equalities, next)?;
                    let right = expand(schedule, node.operands[1], facts, equalities, next)?;
                    for (name, variable) in right {
                        if let Some(previous) = left.get(&name) {
                            equalities.push((*previous, variable));
                        } else if !semi {
                            left.insert(name, variable);
                        }
                    }
                    left
                }
                _ => return None,
            };
            if matches!(node.kind, Kind::Select) {
                let input = &schedule.nodes[node.operands[0]];
                let variable = |column: &Column| {
                    input.heading.iter().find_map(|(name, c)| (c == column).then(|| heading[name]))
                };
                for (a, b) in &node.equalities {
                    equalities.push((variable(a)?, variable(b)?));
                }
                for predicate in &node.predicates {
                    facts.push(Fact {
                        relation: Relation::Predicate(predicate.shape.clone()),
                        columns: predicate.columns.iter().map(variable).collect::<Option<_>>()?,
                    });
                }
            }
            heading.retain(|name, _| node.heading.contains_key(name));
            Some(heading)
        }
        let mut facts = Vec::new();
        let mut equalities = Vec::new();
        let mut variables = 0;
        let mut output = expand(schedule, schedule.output?, &mut facts, &mut equalities, &mut variables)?;
        let mut uf = UnionFind::new(variables);
        for (a, b) in equalities { uf.union(a, b); }
        for fact in &mut facts {
            for column in &mut fact.columns { *column = uf.label(*column); }
        }
        for column in output.values_mut() { *column = uf.label(*column); }
        Some(Self { facts, output, variables })
    }

    fn maps_to(&self, target: &Self) -> bool {
        if self.output.len() != target.output.len() { return false; }
        let mut mapping = vec![None; self.variables];
        for (name, &source) in &self.output {
            let Some(&destination) = target.output.get(name) else { return false; };
            if mapping[source].is_some_and(|previous| previous != destination) { return false; }
            mapping[source] = Some(destination);
        }
        let mut candidates: Vec<_> = self.facts.iter().map(|fact| {
            (fact, target.facts.iter().filter(|other| other.relation == fact.relation && other.columns.len() == fact.columns.len()).collect::<Vec<_>>())
        }).collect();
        fn compatible(fact: &Fact, other: &Fact, mapping: &[Option<usize>]) -> bool {
            fact.columns.iter().zip(&other.columns).enumerate().all(|(i, (&a, &b))| {
                mapping[a].is_none_or(|previous| previous == b)
                    && (0..i).all(|j| fact.columns[j] != a || other.columns[j] == b)
            })
        }
        fn search(candidates: &mut [(&Fact, Vec<&Fact>)], mapping: &mut [Option<usize>], trail: &mut Vec<usize>) -> bool {
            if candidates.is_empty() { return true; }
            // Choose the most constrained remaining view under the current
            // correlation. A fixed occurrence order can enumerate unrelated
            // witnesses exponentially before noticing a constrained join.
            let mut best = 0;
            let mut count = usize::MAX;
            for (index, (fact, options)) in candidates.iter().enumerate() {
                let available = options.iter().filter(|other| compatible(fact, other, mapping)).count();
                if available == 0 { return false; }
                if available < count {
                    best = index;
                    count = available;
                }
            }
            candidates.swap(0, best);
            let ((fact, options), rest) = candidates.split_first_mut().unwrap();
            for other in options.iter() {
                if !compatible(fact, other, mapping) { continue; }
                let mark = trail.len();
                for (&a, &b) in fact.columns.iter().zip(&other.columns) {
                    if mapping[a].is_none() {
                        mapping[a] = Some(b);
                        trail.push(a);
                    }
                }
                if search(rest, mapping, trail) { return true; }
                for variable in trail.drain(mark..) { mapping[variable] = None; }
            }
            false
        }
        search(&mut candidates, &mut mapping, &mut Vec::with_capacity(self.variables))
    }
}

// Formal rule RA-SplitViews: Mutual containment fixes visible columns and preserves occurrence-labelled row witnesses and their predicates.
pub(super) fn equivalent(source: &Structure, target: &Structure) -> Result<bool, String> {
    match (Query::build(source), Query::build(target)) {
        (Some(source), Some(target)) => Ok(source.maps_to(&target) && target.maps_to(&source)),
        _ => {
            let mut intern = HashMap::new();
            Ok(wiring(source, &mut intern)? == wiring(target, &mut intern)?)
        }
    }
}
