//! Independent relational-region proof. A tableau records existential rows,
//! literal equalities and the opaque Rust relations already traversed. Projection
//! and semijoin keep independent existential witnesses. Memoized DAG results
//! share alpha-normalized bodies; witnesses are folded only with a certified
//! output-fixing homomorphism. Containment uses indexed, memoized search. Leibniz
//! equality unifies terms; functional extensionality makes projection sound
//! before Rust whenever its free variables remain in the heading.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate as ra;
use quote::ToTokens;

use crate::symbol_name;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Term {
    Variable(usize),
    Literal(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Atom {
    relation: String,
    arguments: Vec<Term>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Tableau {
    heading: BTreeMap<String, Term>,
    atoms: Rc<BTreeSet<Atom>>,
    impossible: bool,
}

#[derive(Default)]
struct Context {
    next_variable: usize,
    joins: BTreeMap<(Tableau, Tableau, bool), Tableau>,
    bodies: BTreeSet<Rc<BTreeSet<Atom>>>,
}

impl Context {
    fn share(&mut self, mut table: Tableau) -> Tableau {
        table.normalize();
        if let Some(atoms) = self.bodies.get(&table.atoms) {
            table.atoms = atoms.clone();
        } else {
            self.bodies.insert(table.atoms.clone());
        }
        table
    }

    fn fresh(&mut self) -> Term {
        let variable = self.next_variable;
        self.next_variable += 1;
        Term::Variable(variable)
    }
}

impl Tableau {
    fn column(&mut self, name: String, context: &mut Context) -> Term {
        self.heading
            .entry(name)
            .or_insert_with(|| context.fresh())
            .clone()
    }

    fn unify(&mut self, left: Term, right: Term) -> Option<(Term, Term)> {
        if left == right {
            return None;
        }
        let (replace, keep) = match (left, right) {
            (Term::Literal(_), Term::Literal(_)) => {
                self.impossible = true;
                return None;
            }
            (literal @ Term::Literal(_), variable) => (variable, literal),
            (variable, other) => (variable, other),
        };
        for value in self.heading.values_mut() {
            if *value == replace {
                *value = keep.clone();
            }
        }
        self.atoms = Rc::new(
            self.atoms
                .iter()
                .cloned()
                .map(|mut atom| {
                    for term in &mut atom.arguments {
                        if *term == replace {
                            *term = keep.clone();
                        }
                    }
                    atom
                })
                .collect(),
        );
        Some((replace, keep))
    }

    fn equality(&mut self, equality: &ra::Equality) {
        let left = self.heading[&symbol_name(&equality.left)].clone();
        match &equality.right {
            ra::EqualityValue::Variable(variable) => {
                let right = self.heading[&symbol_name(variable)].clone();
                self.unify(left, right);
            }
            ra::EqualityValue::Literal(literal) => {
                // Keep literal predicates syntactic. Inferring contradiction
                // from differing token spellings (1 versus 0x1) is unsound.
                Rc::make_mut(&mut self.atoms).insert(Atom {
                    relation: "#literal".into(),
                    arguments: vec![left, Term::Literal(literal.to_token_stream().to_string())],
                });
            }
        }
    }

    /// Treat the complete boolean predicate as a total relation symbol. This
    /// conservative proof preserves its syntax and all its column arguments;
    /// it does not assume implication between distinct comparisons.
    /// Formal rule CT-Constraint: Preserve the boolean predicate symbol and all its tableau arguments.
    fn constraint(&mut self, c: &ra::syntax::Constraint) {
        let arguments = c
            .variables()
            .map(|v| self.heading[&symbol_name(v)].clone())
            .collect();
        let mut shape = c.clone();
        shape.map_terms(&mut |t| {
            if let ra::syntax::constraint::Term::Variable(v) = t {
                *v = syn::Ident::new("__column", v.span());
            }
        });
        Rc::make_mut(&mut self.atoms).insert(Atom {
            relation: format!("#constraint {}", shape.to_token_stream()),
            arguments,
        });
    }

    fn freshen(&self, context: &mut Context) -> Self {
        let mut renaming = BTreeMap::<usize, Term>::new();
        let mut rename = |term: &Term| match term {
            Term::Variable(variable) => renaming
                .entry(*variable)
                .or_insert_with(|| context.fresh())
                .clone(),
            literal => literal.clone(),
        };
        let heading = self
            .heading
            .iter()
            .map(|(name, term)| (name.clone(), rename(term)))
            .collect();
        let atoms = self
            .atoms
            .iter()
            .map(|atom| Atom {
                relation: atom.relation.clone(),
                arguments: atom.arguments.iter().map(&mut rename).collect(),
            })
            .collect();
        Self {
            heading,
            atoms: Rc::new(atoms),
            impossible: self.impossible,
        }
    }

    fn join(&self, right: &Self, semijoin: bool, context: &mut Context) -> Self {
        let key = (self.clone(), right.clone(), semijoin);
        if let Some(result) = context.joins.get(&key) {
            return result.clone();
        }
        let common = self
            .heading
            .keys()
            .filter(|name| right.heading.contains_key(*name))
            .cloned()
            .collect();
        // Idempotence/subsumption is certified before any freshening. The
        // operands are memoized DAG results, not recursively unfolded trees.
        let redundant_right = (semijoin
            || right
                .heading
                .keys()
                .all(|name| self.heading.contains_key(name)))
            && contained(self, right, &common);
        let redundant_left = !semijoin
            && self
                .heading
                .keys()
                .all(|name| right.heading.contains_key(name))
            && contained(right, self, &common);
        if redundant_right || redundant_left {
            let result = if redundant_right {
                self.clone()
            } else {
                right.clone()
            };
            context.joins.insert(key, result.clone());
            return result;
        }
        let right = right.freshen(context);
        let mut joined = self.clone();
        joined.impossible |= right.impossible;
        Rc::make_mut(&mut joined.atoms).extend(right.atoms.iter().cloned());
        let mut equalities = Vec::new();
        for (name, term) in right.heading {
            match joined.heading.get(&name) {
                Some(left) => equalities.push((left.clone(), term)),
                None if !semijoin => {
                    joined.heading.insert(name, term);
                }
                None => {}
            }
        }
        while let Some((left, right)) = equalities.pop() {
            if let Some((replace, keep)) = joined.unify(left, right) {
                for (left, right) in &mut equalities {
                    if *left == replace {
                        *left = keep.clone();
                    }
                    if *right == replace {
                        *right = keep.clone();
                    }
                }
            }
        }
        let joined = context.share(joined);
        context.joins.insert(key, joined.clone());
        joined
    }

    /// Fold only certified redundant witnesses, fixing every visible column.
    /// The homomorphic image is a subtableau, so containment holds in both
    /// directions. A projection never shares a hidden variable by name alone.
    fn normalize(&mut self) {
        let columns = self.heading.keys().cloned().collect();
        loop {
            let mut image = None;
            let mut counts = BTreeMap::new();
            for atom in self.atoms.iter() {
                *counts
                    .entry((&atom.relation, atom.arguments.len()))
                    .or_insert(0) += 1;
            }
            for atom in self.atoms.iter() {
                if counts[&(&atom.relation, atom.arguments.len())] < 2 {
                    continue;
                }
                let mut smaller = self.clone();
                Rc::make_mut(&mut smaller.atoms).remove(atom);
                if let Some(mapping) = homomorphism(&smaller, self, &columns) {
                    let atoms: BTreeSet<_> = self
                        .atoms
                        .iter()
                        .map(|a| Atom {
                            relation: a.relation.clone(),
                            arguments: a
                                .arguments
                                .iter()
                                .map(|t| match t {
                                    Term::Variable(v) => mapping[v].clone(),
                                    Term::Literal(_) => t.clone(),
                                })
                                .collect(),
                        })
                        .collect();
                    debug_assert!(atoms.is_subset(&smaller.atoms));
                    image = Some(atoms);
                    break;
                }
            }
            match image {
                Some(atoms) => self.atoms = Rc::new(atoms),
                None => break,
            }
        }
        // Alpha-normalize local IDs so identical DAG results can be interned.
        let mut renaming = BTreeMap::new();
        let mut rename = |term: &Term| match term {
            Term::Variable(v) => {
                let next = renaming.len();
                renaming.entry(*v).or_insert(Term::Variable(next)).clone()
            }
            Term::Literal(_) => term.clone(),
        };
        for term in self.heading.values_mut() {
            *term = rename(term);
        }
        self.atoms = Rc::new(
            self.atoms
                .iter()
                .map(|atom| Atom {
                    relation: atom.relation.clone(),
                    arguments: atom.arguments.iter().map(&mut rename).collect(),
                })
                .collect(),
        );
    }

    fn project(&mut self, keep: &BTreeSet<String>) {
        self.heading.retain(|name, _| keep.contains(name));
    }

    fn rust(
        &mut self,
        ordinal: usize,
        expression: &syn::Expr,
        bindings: &[syn::Ident],
        context: &mut Context,
    ) {
        let environment = self.heading.keys().cloned().collect();
        let mut arguments: Vec<_> = ra::syntax::rust::free_variables(expression)
            .dependencies(&environment)
            .iter()
            .map(|name| self.heading[name].clone())
            .collect();
        for name in bindings {
            let term = context.fresh();
            self.heading.insert(symbol_name(name), term.clone());
            arguments.push(term);
        }
        Rc::make_mut(&mut self.atoms).insert(Atom {
            relation: format!("#rust{ordinal}"),
            arguments,
        });
    }
}

struct ChainNode {
    payload: String,
    input: Tableau,
}

fn payload(item: &impl ToTokens) -> String {
    // Equality ignores spans; raw identifier spelling is retained consistently
    // between the source payload and its structurally preserved target node.
    item.to_token_stream().to_string()
}

fn program_tableau(target: &ra::Program, context: &mut Context) -> (Tableau, Vec<ChainNode>) {
    let mut results = BTreeMap::<String, Tableau>::new();
    let mut chain = Vec::new();
    for definition in &target.plan.definitions {
        let table = match &definition.operator {
            ra::Operator::Unit(_) => Tableau::default(),
            ra::Operator::Rename(rename) => {
                let mut table = Tableau::default();
                let declaration = target
                    .plan
                    .declarations
                    .iter()
                    .find(|d| symbol_name(&d.name) == symbol_name(&rename.relation))
                    .unwrap();
                let arguments = declaration
                    .columns
                    .iter()
                    .map(|column| {
                        let mapping = rename
                            .mapping
                            .iter()
                            .find(|m| symbol_name(&m.source) == symbol_name(&column.name))
                            .unwrap();
                        table.column(symbol_name(&mapping.target), context)
                    })
                    .collect();
                Rc::make_mut(&mut table.atoms).insert(Atom {
                    relation: symbol_name(&rename.relation),
                    arguments,
                });
                table
            }
            ra::Operator::NaturalJoin(join) => results[&symbol_name(&join.left)].join(
                &results[&symbol_name(&join.right)],
                false,
                context,
            ),
            ra::Operator::Semijoin(join) => results[&symbol_name(&join.left)].join(
                &results[&symbol_name(&join.right)],
                true,
                context,
            ),
            ra::Operator::Project(project) => {
                let mut table = results[&symbol_name(&project.input)].clone();
                let keep: BTreeSet<_> = project.attributes.iter().map(symbol_name).collect();
                table.project(&keep);
                table
            }
            ra::Operator::Select(select) => {
                let mut table = results[&symbol_name(&select.input)].clone();
                for c in &select.condition.constraints {
                    table.constraint(c);
                }
                for equality in &select.condition.equalities {
                    table.equality(equality);
                }
                table
            }
            ra::Operator::Guard(guard) => {
                let mut table = results[&symbol_name(&guard.input)].clone();
                let item = ra::syntax::Filter {
                    if_token: guard.if_token,
                    condition: guard.condition.clone(),
                };
                chain.push(ChainNode {
                    payload: payload(&item),
                    input: table.clone(),
                });
                table.rust(chain.len() - 1, &guard.condition, &[], context);
                table
            }
            ra::Operator::Bind(bind) => {
                let mut table = results[&symbol_name(&bind.input)].clone();
                let item = bind.binding();
                chain.push(ChainNode {
                    payload: payload(&item),
                    input: table.clone(),
                });
                table.rust(
                    chain.len() - 1,
                    &bind.expression,
                    &ra::syntax::rust::bindings(&bind.pattern),
                    context,
                );
                table
            }
        };
        results.insert(symbol_name(&definition.result), context.share(table));
    }
    (
        results
            .remove(&symbol_name(&target.plan.output.input))
            .unwrap(),
        chain,
    )
}

/// Indexed conjunctive-query homomorphism search. Only failed frontier states
/// are memoized: assignments that no remaining atom mentions cannot affect the
/// rest of the proof. Repeated variables and literals are checked exactly.
fn homomorphism(
    target: &Tableau,
    source: &Tableau,
    columns: &BTreeSet<String>,
) -> Option<BTreeMap<usize, Term>> {
    fn assign(from: &Term, to: &Term, mapping: &mut BTreeMap<usize, Term>) -> bool {
        match from {
            Term::Literal(_) => from == to,
            Term::Variable(variable) => match mapping.get(variable) {
                Some(old) => old == to,
                None => {
                    mapping.insert(*variable, to.clone());
                    true
                }
            },
        }
    }
    let mut mapping = BTreeMap::new();
    for column in columns {
        if !assign(
            &source.heading[column],
            &target.heading[column],
            &mut mapping,
        ) {
            return None;
        }
    }
    if source.atoms.is_subset(&target.atoms)
        && mapping.iter().all(|(v, t)| t == &Term::Variable(*v))
    {
        for atom in source.atoms.iter() {
            for term in &atom.arguments {
                if let Term::Variable(v) = term {
                    mapping.insert(*v, term.clone());
                }
            }
        }
        return Some(mapping);
    }
    let mut index = BTreeMap::<(&str, usize), Vec<&Atom>>::new();
    let mut positions = BTreeMap::<(&str, usize, usize, &Term), Vec<&Atom>>::new();
    for atom in target.atoms.iter() {
        index
            .entry((&atom.relation, atom.arguments.len()))
            .or_default()
            .push(atom);
        for (position, term) in atom.arguments.iter().enumerate() {
            positions
                .entry((&atom.relation, atom.arguments.len(), position, term))
                .or_default()
                .push(atom);
        }
    }
    let atoms: Vec<_> = source.atoms.iter().collect();
    type State = (Vec<usize>, Vec<(usize, Term)>);
    struct Search<'a> {
        atoms: Vec<&'a Atom>,
        candidates: Vec<Vec<&'a Atom>>,
        failures: BTreeSet<State>,
    }
    impl Search<'_> {
        fn compatible(atom: &Atom, candidate: &Atom, mapping: &BTreeMap<usize, Term>) -> bool {
            let mut local = BTreeMap::new();
            atom.arguments
                .iter()
                .zip(&candidate.arguments)
                .all(|(from, to)| match from {
                    Term::Literal(_) => from == to,
                    Term::Variable(v) => match mapping.get(v).or_else(|| local.get(v)) {
                        Some(old) => old == to,
                        None => {
                            local.insert(*v, to.clone());
                            true
                        }
                    },
                })
        }

        fn search(
            &mut self,
            remaining: Vec<usize>,
            mapping: BTreeMap<usize, Term>,
        ) -> Option<BTreeMap<usize, Term>> {
            if remaining.is_empty() {
                return Some(mapping);
            }
            let live: BTreeSet<_> = remaining
                .iter()
                .flat_map(|i| {
                    self.atoms[*i].arguments.iter().filter_map(|t| match t {
                        Term::Variable(v) => Some(*v),
                        Term::Literal(_) => None,
                    })
                })
                .collect();
            let state = (
                remaining.clone(),
                mapping
                    .iter()
                    .filter(|(v, _)| live.contains(v))
                    .map(|(v, t)| (*v, t.clone()))
                    .collect(),
            );
            if self.failures.contains(&state) {
                return None;
            }
            let mut choices = Vec::new();
            let mut domains = BTreeMap::<usize, BTreeSet<Term>>::new();
            for i in &remaining {
                let candidates: Vec<_> = self.candidates[*i]
                    .iter()
                    .copied()
                    .filter(|candidate| Self::compatible(self.atoms[*i], candidate, &mapping))
                    .collect();
                if candidates.is_empty() {
                    self.failures.insert(state);
                    return None;
                }
                // Arc consistency: a shared variable must have a candidate
                // value in every atom where it appears. This only prunes
                // impossible mappings; it never identifies witnesses.
                for (position, term) in self.atoms[*i].arguments.iter().enumerate() {
                    if let Term::Variable(v) = term {
                        let values: BTreeSet<_> = candidates
                            .iter()
                            .map(|c| c.arguments[position].clone())
                            .collect();
                        domains
                            .entry(*v)
                            .and_modify(|old| old.retain(|t| values.contains(t)))
                            .or_insert(values);
                    }
                }
                choices.push((*i, candidates));
            }
            if domains.values().any(BTreeSet::is_empty) {
                self.failures.insert(state);
                return None;
            }
            for (i, candidates) in &mut choices {
                candidates.retain(|c| {
                    self.atoms[*i]
                        .arguments
                        .iter()
                        .zip(&c.arguments)
                        .all(|(from, to)| match from {
                            Term::Variable(v) => domains[v].contains(to),
                            Term::Literal(_) => from == to,
                        })
                });
            }
            let (i, candidates) = choices
                .into_iter()
                .min_by_key(|(_, candidates)| candidates.len())
                .unwrap();
            let rest: Vec<_> = remaining.into_iter().filter(|other| *other != i).collect();
            for candidate in candidates {
                let mut next = mapping.clone();
                for (from, to) in self.atoms[i].arguments.iter().zip(&candidate.arguments) {
                    if !assign(from, to, &mut next) {
                        unreachable!("candidate compatibility was checked");
                    }
                }
                if let Some(result) = self.search(rest.clone(), next) {
                    return Some(result);
                }
            }
            self.failures.insert(state);
            None
        }
    }
    let mut candidates = Vec::new();
    for atom in &atoms {
        let mut rows = index.get(&(atom.relation.as_str(), atom.arguments.len()))?;
        for (position, term) in atom.arguments.iter().enumerate() {
            let bound = match term {
                Term::Literal(_) => Some(term),
                Term::Variable(v) => mapping.get(v),
            };
            if let Some(bound) = bound {
                let filtered = positions.get(&(
                    atom.relation.as_str(),
                    atom.arguments.len(),
                    position,
                    bound,
                ))?;
                if filtered.len() < rows.len() {
                    rows = filtered;
                }
            }
        }
        candidates.push(
            rows.iter()
                .copied()
                .filter(|candidate| Search::compatible(atom, candidate, &mapping))
                .collect(),
        );
    }
    let remaining = (0..atoms.len()).collect();
    Search {
        atoms,
        candidates,
        failures: BTreeSet::new(),
    }
    .search(remaining, mapping)
}

/// Is `target` contained in `source`, fixing the named output columns? A
/// homomorphism from the source tableau into the target tableau proves this.
fn contained(target: &Tableau, source: &Tableau, columns: &BTreeSet<String>) -> bool {
    if target.impossible {
        return true;
    }
    if source.impossible {
        return false;
    }
    homomorphism(target, source, columns).is_some()
}

/// Prove an RA′ rewrite from its relational tableaux and ordered Rust chain.
/// Every target Rust input must retain its source-before constraints; final
/// output relations must contain one another with named output columns fixed.
pub fn check_rewrite(source: &ra::Program, target: &ra::Program) -> syn::Result<()> {
    ra::contract::check(source)?;
    ra::contract::check(target)?;
    if source.plan.declarations != target.plan.declarations
        || source.plan.output.head != target.plan.output.head
    {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            "RA rewrite changes declarations or output head",
        ));
    }
    let mut context = Context::default();
    let (source_output, source_chain) = program_tableau(source, &mut context);
    let (target_output, target_chain) = program_tableau(target, &mut context);
    compare(
        source_output,
        source_chain,
        target_output,
        target_chain,
        target,
    )
}

fn compare(
    source_output: Tableau,
    source_chain: Vec<ChainNode>,
    target_output: Tableau,
    target_chain: Vec<ChainNode>,
    target: &ra::Program,
) -> syn::Result<()> {
    let error = |message: &str| {
        syn::Error::new_spanned(
            &target.plan.output,
            format!("result `{}`: kernel {message}", target.plan.output.input),
        )
    };
    if source_chain.len() != target_chain.len()
        || source_chain
            .iter()
            .zip(&target_chain)
            .any(|(s, t)| s.payload != t.payload)
    {
        return Err(error(
            "Rust chain differs from written source order, payload or bindings",
        ));
    }
    for (ordinal, (source, target)) in source_chain.iter().zip(&target_chain).enumerate() {
        let common = source
            .input
            .heading
            .keys()
            .filter(|name| target.input.heading.contains_key(*name))
            .cloned()
            .collect();
        if !contained(&target.input, &source.input, &common) {
            return Err(error(&format!(
                "Rust step {ordinal} lost a source-before relational constraint"
            )));
        }
    }
    let columns = source_output.heading.keys().cloned().collect();
    if !contained(&target_output, &source_output, &columns)
        || !contained(&source_output, &target_output, &columns)
    {
        return Err(error("output relational tableau differs from source"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(vertices: usize, mask: usize) -> Tableau {
        Tableau {
            atoms: Rc::new(
                (0..vertices * vertices)
                    .filter(|i| mask & (1 << i) != 0)
                    .map(|i| Atom {
                        relation: "E".into(),
                        arguments: vec![Term::Variable(i / vertices), Term::Variable(i % vertices)],
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn indexed_memoized_search_matches_exhaustive_small_graph_mappings() {
        // Independently enumerate every function from three source vertices
        // to two target vertices for every pair of directed edge subsets.
        // Loops exercise repeated variables; disconnected graphs exercise
        // forgetting assignments outside the remaining search frontier.
        for source_mask in 0..512 {
            let source = graph(3, source_mask);
            for target_mask in 0..16 {
                let target = graph(2, target_mask);
                let expected = (0..8).any(|mapping| {
                    source.atoms.iter().all(|atom| {
                        let image = Atom {
                            relation: atom.relation.clone(),
                            arguments: atom
                                .arguments
                                .iter()
                                .map(|t| {
                                    let Term::Variable(v) = t else { unreachable!() };
                                    Term::Variable((mapping >> v) & 1)
                                })
                                .collect(),
                        };
                        target.atoms.contains(&image)
                    })
                });
                assert_eq!(
                    homomorphism(&target, &source, &BTreeSet::new()).is_some(),
                    expected,
                    "source={source_mask} target={target_mask}"
                );
            }
        }
    }

    #[test]
    fn repeated_dag_operands_share_their_tableau_instead_of_unfolding() {
        let source = graph(3, 0b101010101);
        let mut context = Context {
            next_variable: 3,
            ..Default::default()
        };
        let source = context.share(source);
        let mut result = source.clone();
        for _ in 0..64 {
            result = result.join(&result, true, &mut context);
            assert!(Rc::ptr_eq(&result.atoms, &source.atoms));
        }
        assert_eq!(context.joins.len(), 1);
    }
}
