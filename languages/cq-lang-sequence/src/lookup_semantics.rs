//! Narrow certificates for Lookup/Expand reexpression. Evidence is never read.
//! Natural joins and semijoins are conjunctive queries; every other operator is
//! an opaque named relation whose syntax and input support must be preserved.
use std::collections::{BTreeMap, HashMap, HashSet};

use cq_lang_structures::physical::{self, Operator};
use cq_syntax::symbol_name;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct Query {
    head: BTreeMap<u32, usize>,
    atoms: Vec<(u32, Vec<usize>)>,
    variables: usize,
}

pub(super) type Queries = BTreeMap<u32, std::rc::Rc<Query>>;
#[derive(PartialEq, Eq, Hash)]
enum NodeKey {
    Join {
        semi: bool,
        left: u32,
        right: u32,
        columns: Vec<u32>,
    },
    Union {
        operands: Vec<u32>,
        columns: Vec<u32>,
    },
    Alias(u32),
    Opaque(Vec<u32>),
}
type ProgramKey = Vec<(u32, NodeKey)>;
type SearchState = (Vec<usize>, Vec<(usize, usize)>);

#[derive(Default)]
pub(super) struct Context {
    names: std::cell::RefCell<HashMap<String, u32>>,
    programs: std::cell::RefCell<HashMap<ProgramKey, std::rc::Rc<Queries>>>,
    equalities: std::cell::RefCell<HashMap<Query, HashMap<Query, bool>>>,
}

impl Context {
    fn intern(&self, name: &str) -> u32 {
        let mut names = self.names.borrow_mut();
        if let Some(&id) = names.get(name) {
            return id;
        }
        let next = u32::try_from(names.len()).expect("support name inventory exceeds u32");
        *names.entry(name.to_owned()).or_insert(next)
    }

    fn key(&self, program: &physical::Program, analysis: &physical::Analysis) -> ProgramKey {
        program
            .program
            .definitions
            .iter()
            .map(|definition| {
                let name = symbol_name(&definition.name);
                let operator = match &definition.operator {
                    Operator::Probe(_, keyed)
                    | Operator::Merge(_, keyed)
                    | Operator::Semi(_, keyed) => NodeKey::Join {
                        semi: matches!(definition.operator, Operator::Semi(..)),
                        left: self.intern(&symbol_name(&keyed.left)),
                        right: self.intern(&symbol_name(&keyed.right)),
                        columns: keyed
                            .columns
                            .iter()
                            .map(|column| self.intern(&symbol_name(column)))
                            .collect(),
                    },
                    Operator::Union(union) => NodeKey::Union {
                        operands: union
                            .operands
                            .iter()
                            .map(|name| self.intern(&symbol_name(name)))
                            .collect(),
                        columns: analysis.headings[&name]
                            .iter()
                            .map(|column| self.intern(column))
                            .collect(),
                    },
                    Operator::Project(project)
                        if analysis.headings[&name]
                            == analysis.headings[&symbol_name(&project.input)] =>
                    {
                        // Keeping the complete checked heading is identity,
                        // including a separately named provider's retained rows.
                        NodeKey::Alias(self.intern(&symbol_name(&project.input)))
                    }
                    _ => NodeKey::Opaque(
                        analysis.headings[&name]
                            .iter()
                            .map(|column| self.intern(column))
                            .collect(),
                    ),
                };
                (self.intern(&name), operator)
            })
            .collect()
    }

    pub(super) fn queries(
        &self,
        program: &physical::Program,
        analysis: &physical::Analysis,
    ) -> std::rc::Rc<Queries> {
        let key = self.key(program, analysis);
        if let Some(queries) = self.programs.borrow().get(&key) {
            return queries.clone();
        }
        let queries = std::rc::Rc::new(queries(&key, self));
        self.programs.borrow_mut().insert(key, queries.clone());
        queries
    }

    pub(super) fn equal_in(&self, queries: &Queries, left: &str, right: &str) -> bool {
        queries
            .get(&self.intern(left))
            .zip(queries.get(&self.intern(right)))
            .is_some_and(|(left, right)| equal(left, right, self))
    }

    pub(super) fn same_result(
        &self,
        source: &Queries,
        left: &str,
        target: &Queries,
        right: &str,
    ) -> bool {
        source
            .get(&self.intern(left))
            .zip(target.get(&self.intern(right)))
            .is_some_and(|(left, right)| equal(left, right, self))
    }
}

fn queries(program: &ProgramKey, context: &Context) -> Queries {
    let mut queries: Queries = BTreeMap::new();
    for (name, operator) in program {
        // Aliases and equal-support alternatives share a proved query rather
        // than copying all existential witnesses into another graph node.
        if let NodeKey::Alias(input) = operator {
            queries.insert(*name, queries[input].clone());
            continue;
        }
        if let NodeKey::Union { operands, .. } = operator
            && let Some(representative) = operands
                .iter()
                .map(|operand| &queries[operand])
                .min_by_key(|query| (query.atoms.len(), query.variables))
            && operands
                .iter()
                .all(|arm| equal(representative, &queries[arm], context))
        {
            // Prefer the smaller certified construction: an absorbed optional
            // semijoin need not survive as redundant witnesses at every join.
            queries.insert(*name, representative.clone());
            continue;
        }
        let mut query =
            match operator {
                NodeKey::Join {
                    semi,
                    left,
                    right,
                    columns,
                } => {
                    let mut left = (*queries[left]).clone();
                    let right = &queries[right];
                    let mut variables: Vec<_> =
                        (left.variables..left.variables + right.variables).collect();
                    for column in columns {
                        variables[right.head[column]] = left.head[column];
                    }
                    left.variables += right.variables;
                    left.atoms.extend(right.atoms.iter().map(|(atom, args)| {
                        (*atom, args.iter().map(|id| variables[*id]).collect())
                    }));
                    if !semi {
                        left.head.extend(
                            right
                                .head
                                .iter()
                                .map(|(column, id)| (*column, variables[*id])),
                        );
                    }
                    left
                }
                NodeKey::Opaque(columns) | NodeKey::Union { columns, .. } => Query {
                    head: columns
                        .iter()
                        .enumerate()
                        .map(|(i, column)| (*column, i))
                        .collect(),
                    atoms: vec![(*name, (0..columns.len()).collect())],
                    variables: columns.len(),
                },
                NodeKey::Alias(_) => unreachable!("aliases share their input query"),
            };
        query.atoms.sort_unstable();
        query.atoms.dedup();
        queries.insert(*name, std::rc::Rc::new(query));
    }
    queries
}

fn maps_into(from: &Query, into: &Query) -> bool {
    // Reject an absent relation before backtracking through repeated atoms.
    // A late missing atom otherwise explores every irrelevant assignment.
    if from.head.keys().ne(into.head.keys())
        || from.atoms.iter().any(|(relation, arguments)| {
            !into.atoms.iter().any(|(candidate, columns)| {
                candidate == relation && columns.len() == arguments.len()
            })
        })
    {
        return false;
    }
    let mut substitution = vec![None; from.variables];
    for (column, variable) in &from.head {
        substitution[*variable] = Some(into.head[column]);
    }
    let mut relations: HashMap<u32, Vec<&[usize]>> = HashMap::new();
    for (name, arguments) in &into.atoms {
        relations.entry(*name).or_default().push(arguments);
    }
    let mut candidates: Vec<Vec<&[usize]>> = from
        .atoms
        .iter()
        .map(|(name, arguments)| {
            relations[name]
                .iter()
                .copied()
                .filter(|candidate| {
                    candidate.len() == arguments.len()
                        && arguments.iter().enumerate().all(|(i, variable)| {
                            arguments[..i].iter().enumerate().all(|(j, earlier)| {
                                earlier != variable || candidate[i] == candidate[j]
                            })
                        })
                })
                .collect()
        })
        .collect();
    // Generalized arc consistency: every variable value needs a supporting
    // tuple in every incident atom. This rejects incompatible join keys before
    // enumerating choices for unrelated existential witnesses.
    loop {
        let mut domains: Vec<Option<HashSet<usize>>> = substitution
            .iter()
            .map(|value| value.map(|value| HashSet::from([value])))
            .collect();
        for ((_, arguments), choices) in from.atoms.iter().zip(&candidates) {
            for (position, &variable) in arguments.iter().enumerate() {
                let values: HashSet<_> = choices.iter().map(|tuple| tuple[position]).collect();
                match &mut domains[variable] {
                    Some(domain) => domain.retain(|value| values.contains(value)),
                    domain @ None => *domain = Some(values),
                }
            }
        }
        let mut changed = false;
        for ((_, arguments), choices) in from.atoms.iter().zip(&mut candidates) {
            let before = choices.len();
            choices.retain(|tuple| {
                arguments
                    .iter()
                    .zip(*tuple)
                    .all(|(&variable, value)| domains[variable].as_ref().unwrap().contains(value))
            });
            if choices.is_empty() {
                return false;
            }
            changed |= choices.len() != before;
        }
        if !changed {
            break;
        }
    }
    fn compatible(
        arguments: &[usize],
        candidate: &[usize],
        substitution: &[Option<usize>],
    ) -> bool {
        arguments.iter().zip(candidate).all(|(&variable, &value)| {
            substitution[variable].is_none_or(|previous| previous == value)
        })
    }
    fn search(
        remaining: &mut [usize],
        from: &Query,
        candidates: &[Vec<&[usize]>],
        substitution: &mut [Option<usize>],
        assigned: &mut Vec<usize>,
        failed: &mut HashSet<SearchState>,
        fuel: &mut usize,
    ) -> bool {
        if remaining.is_empty() {
            return true;
        }
        if *fuel == 0 {
            return false;
        }
        *fuel -= 1;
        // Forget assignments that no remaining atom reads. Independent witness
        // choices then share one failed suffix instead of multiplying searches.
        let mut atoms = remaining.to_vec();
        atoms.sort_unstable();
        let mut live: Vec<_> = atoms
            .iter()
            .flat_map(|&atom| from.atoms[atom].1.iter().copied())
            .collect();
        live.sort_unstable();
        live.dedup();
        let state = (
            atoms,
            live.into_iter()
                .filter_map(|variable| substitution[variable].map(|value| (variable, value)))
                .collect(),
        );
        if failed.contains(&state) {
            return false;
        }
        // Fail-first constraint ordering avoids exploring independent repeated
        // atoms before discovering that a bound key has no compatible match.
        let mut best = (0, usize::MAX);
        for (position, &atom) in remaining.iter().enumerate() {
            let arguments = &from.atoms[atom].1;
            let choices = candidates[atom]
                .iter()
                .filter(|candidate| compatible(arguments, candidate, substitution))
                .count();
            if choices == 0 {
                return false;
            }
            if choices < best.1 {
                best = (position, choices);
            }
        }
        remaining.swap(0, best.0);
        let atom = remaining[0];
        let arguments = &from.atoms[atom].1;
        for &candidate in &candidates[atom] {
            if !compatible(arguments, candidate, substitution) {
                continue;
            }
            let mark = assigned.len();
            let mut matches = true;
            for (&variable, &value) in arguments.iter().zip(candidate) {
                match substitution[variable] {
                    Some(previous) if previous != value => {
                        matches = false;
                        break;
                    }
                    Some(_) => {}
                    None => {
                        substitution[variable] = Some(value);
                        assigned.push(variable);
                    }
                }
            }
            if matches
                && search(
                    &mut remaining[1..],
                    from,
                    candidates,
                    substitution,
                    assigned,
                    failed,
                    fuel,
                )
            {
                return true;
            }
            while assigned.len() > mark {
                substitution[assigned.pop().unwrap()] = None;
            }
        }
        remaining.swap(0, best.0);
        failed.insert(state);
        false
    }
    let mut remaining: Vec<_> = (0..from.atoms.len()).collect();
    search(
        &mut remaining,
        from,
        &candidates,
        &mut substitution,
        &mut Vec::new(),
        &mut HashSet::new(),
        // This is a bounded certificate search, not an unrestricted CQ solver.
        // Exhaustion rejects the proposed equivalence; it never accepts it.
        &mut from.atoms.len().saturating_mul(4),
    )
}

fn canonical(query: &Query) -> (Vec<usize>, Vec<(u32, Vec<usize>)>) {
    let mut variables = BTreeMap::new();
    let mut rename = |variable| {
        let next = variables.len();
        *variables.entry(variable).or_insert(next)
    };
    let head = query.head.values().copied().map(&mut rename).collect();
    let atoms = query
        .atoms
        .iter()
        .map(|(name, arguments)| (*name, arguments.iter().copied().map(&mut rename).collect()))
        .collect();
    (head, atoms)
}

fn equal(left: &Query, right: &Query, context: &Context) -> bool {
    if left.head.keys().ne(right.head.keys()) {
        return false;
    }
    if left.head == right.head && left.atoms == right.atoms {
        return true;
    }
    if let Some(&result) = context
        .equalities
        .borrow()
        .get(left)
        .and_then(|row| row.get(right))
    {
        return result;
    }
    let result =
        canonical(left) == canonical(right) || (maps_into(left, right) && maps_into(right, left));
    let mut cache = context.equalities.borrow_mut();
    let row = cache.entry(left.clone()).or_default();
    row.insert(right.clone(), result);
    result
}

fn opaque<'a>(
    program: &'a physical::Program,
    queries: &Queries,
    context: &Context,
) -> BTreeMap<u32, &'a physical::Definition> {
    program
        .program
        .definitions
        .iter()
        .filter(|definition| {
            let name = context.intern(&symbol_name(&definition.name));
            let query = &queries[&name];
            query.atoms.len() == 1 && query.atoms[0].0 == name
        })
        .map(|d| (context.intern(&symbol_name(&d.name)), d))
        .collect()
}

/// Preserve Rust syntax and input support without pinning a written-order prefix.
pub(super) fn reexpression(
    source: &physical::Program,
    source_analysis: &physical::Analysis,
    target: &physical::Program,
    target_analysis: &physical::Analysis,
    left: &Queries,
    right: &Queries,
    context: &Context,
) -> bool {
    let a = &source.program;
    let b = &target.program;
    if a.declarations != b.declarations
        || a.head != b.head
        || source_analysis.discharged_host_sets != target_analysis.discharged_host_sets
        || source_analysis.consumer_canonical != target_analysis.consumer_canonical
    {
        return false;
    }
    let old = opaque(source, left, context);
    let new = opaque(target, right, context);
    if old.keys().ne(new.keys()) {
        return false;
    }
    for (name, definition) in old {
        let other = new[&name];
        let mut operator = other.operator.clone();
        let old_inputs = definition.operator.operands();
        let new_inputs = operator.operands();
        if old_inputs.len() != new_inputs.len()
            || old_inputs.iter().zip(&new_inputs).any(|(a, b)| {
                !equal(
                    &left[&context.intern(&symbol_name(a))],
                    &right[&context.intern(&symbol_name(b))],
                    context,
                )
            })
        {
            return false;
        }
        // Only the input edge of unary, total relational operators may change.
        match (&definition.operator, &mut operator) {
            (Operator::Project(a), Operator::Project(b)) => b.input = a.input.clone(),
            (Operator::Select(a), Operator::Select(b)) => b.input = a.input.clone(),
            _ => {}
        }
        if definition.operator != operator {
            return false;
        }
    }
    equal(
        &left[&context.intern(&symbol_name(&a.returned))],
        &right[&context.intern(&symbol_name(&b.returned))],
        context,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_witnesses_do_not_multiply_an_unsatisfiable_suffix() {
        let mut from = Query {
            head: BTreeMap::new(),
            atoms: (0..30).map(|i| (0, vec![2 * i, 2 * i + 1])).collect(),
            variables: 63,
        };
        from.atoms
            .extend([(0, vec![60, 61]), (0, vec![61, 62]), (0, vec![62, 60])]);
        let into = Query {
            head: BTreeMap::new(),
            atoms: vec![(0, vec![0, 1]), (0, vec![1, 0])],
            variables: 2,
        };
        assert!(!maps_into(&from, &into));
        from.atoms.pop();
        assert!(maps_into(&from, &into));
        from.atoms.push((0, vec![62, 62]));
        assert!(!maps_into(&from, &into));
    }

    #[test]
    fn missing_atom_rejects_before_enumerating_repeated_relation_assignments() {
        let mut from = Query {
            head: BTreeMap::from([(1, 0)]),
            atoms: (1..40).map(|variable| (0, vec![0, variable])).collect(),
            variables: 40,
        };
        let into = Query {
            head: BTreeMap::from([(1, 0)]),
            atoms: vec![(0, vec![0, 1]), (0, vec![0, 2])],
            variables: 3,
        };
        assert!(maps_into(&from, &into));
        from.atoms.push((0, vec![39, 0]));
        assert!(!maps_into(&from, &into));
        from.atoms.pop();
        from.atoms.push((2, vec![0]));
        assert!(!maps_into(&from, &into));
    }

    #[test]
    fn interned_union_graph_retains_opaque_heading_and_equal_arm_support() {
        let context = Context::default();
        let mut graph = vec![
            (0, NodeKey::Opaque(vec![3])),
            (1, NodeKey::Opaque(vec![3])),
            (
                2,
                NodeKey::Union {
                    operands: vec![0, 1],
                    columns: vec![3],
                },
            ),
        ];
        let distinct = queries(&graph, &context);
        assert_eq!(distinct[&2].head, BTreeMap::from([(3, 0)]));
        assert_eq!(distinct[&2].atoms, vec![(2, vec![0])]);
        graph[2].1 = NodeKey::Union {
            operands: vec![0, 0],
            columns: vec![3],
        };
        let repeated = queries(&graph, &context);
        assert!(equal(&repeated[&0], &repeated[&2], &context));
        assert!(std::rc::Rc::ptr_eq(&repeated[&0], &repeated[&2]));
    }

    #[test]
    fn equivalent_union_arms_share_the_smallest_proved_support() {
        let context = Context::default();
        let mut graph = vec![
            (0, NodeKey::Opaque(vec![10])),
            (1, NodeKey::Opaque(vec![10, 11])),
            (
                2,
                NodeKey::Join {
                    semi: true,
                    left: 0,
                    right: 1,
                    columns: vec![10],
                },
            ),
            (
                3,
                NodeKey::Join {
                    semi: false,
                    left: 2,
                    right: 1,
                    columns: vec![10],
                },
            ),
            (
                4,
                NodeKey::Join {
                    semi: false,
                    left: 0,
                    right: 1,
                    columns: vec![10],
                },
            ),
            (
                5,
                NodeKey::Union {
                    operands: vec![3, 4],
                    columns: vec![10, 11],
                },
            ),
        ];
        for name in 6..70 {
            graph.push((name, NodeKey::Alias(name - 1)));
        }
        let checked = queries(&graph, &context);
        assert_eq!(checked[&3].atoms.len(), 3);
        assert_eq!(checked[&4].atoms.len(), 2);
        assert!(equal(&checked[&3], &checked[&4], &context));
        for name in 5..70 {
            assert!(std::rc::Rc::ptr_eq(&checked[&name], &checked[&4]));
        }
    }
}
