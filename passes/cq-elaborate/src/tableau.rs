//! Independent relational-region proof. A tableau records existential rows,
//! literal equalities and the opaque Rust relations already traversed. Projection
//! and semijoin keep existential witnesses; each right operand gets fresh witness
//! variables. Containment uses finite conjunctive-query homomorphisms. For
//! Rust-observed classes, Eq is an explicit predicate between concrete witnesses;
//! it does not substitute representatives. Projection losses are tracked
//! separately because opaque Rust need not respect the column type's Eq classes.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ra as ra;
use cq_lang_vo::{
    self as valuation,
    cq::{self, BodyItem},
};
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

#[derive(Clone, Debug, Default)]
struct Tableau {
    heading: BTreeMap<String, Term>,
    atoms: BTreeSet<Atom>,
    impossible: bool,
    projected_representatives: BTreeSet<Term>,
}

#[derive(Default)]
struct Context {
    next_variable: usize,
    sensitive: BTreeSet<String>,
}

impl Context {
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
        if self.projected_representatives.remove(&replace) {
            self.projected_representatives.insert(keep.clone());
        }
        self.atoms = std::mem::take(&mut self.atoms)
            .into_iter()
            .map(|mut atom| {
                for term in &mut atom.arguments {
                    if *term == replace {
                        *term = keep.clone();
                    }
                }
                atom
            })
            .collect();
        Some((replace, keep))
    }

    fn equate(
        &mut self,
        left: Term,
        right: Term,
        preserve_representatives: bool,
    ) -> Option<(Term, Term)> {
        if !preserve_representatives {
            return self.unify(left, right);
        }
        if left != right {
            self.atoms.insert(Atom {
                relation: "#eq".into(),
                arguments: vec![left.clone(), right.clone()],
            });
            self.atoms.insert(Atom {
                relation: "#eq".into(),
                arguments: vec![right, left],
            });
        }
        None
    }

    fn equality(&mut self, equality: &ra::Equality, context: &Context) {
        let left = self.heading[&symbol_name(&equality.left)].clone();
        match &equality.right {
            ra::EqualityValue::Variable(variable) => {
                let right = self.heading[&symbol_name(variable)].clone();
                self.equate(
                    left,
                    right,
                    context.sensitive.contains(&symbol_name(&equality.left))
                        || context.sensitive.contains(&symbol_name(variable)),
                );
            }
            ra::EqualityValue::Literal(literal) => {
                // Keep literal predicates syntactic. Inferring contradiction
                // from differing token spellings (1 versus 0x1) is unsound.
                self.atoms.insert(Atom {
                    relation: "#literal".into(),
                    arguments: vec![left, Term::Literal(literal.to_token_stream().to_string())],
                });
            }
        }
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
        let projected_representatives = self
            .projected_representatives
            .iter()
            .map(&mut rename)
            .collect();
        Self {
            heading,
            atoms,
            impossible: self.impossible,
            projected_representatives,
        }
    }

    fn join(&self, right: &Self, semijoin: bool, context: &mut Context) -> Self {
        let right = right.freshen(context);
        let mut joined = self.clone();
        joined.impossible |= right.impossible;
        joined
            .projected_representatives
            .extend(right.projected_representatives);
        joined.atoms.extend(right.atoms);
        let mut equalities = Vec::new();
        for (name, term) in right.heading {
            match joined.heading.get(&name) {
                Some(left) => {
                    equalities.push((left.clone(), term, context.sensitive.contains(&name)))
                }
                None if !semijoin => {
                    joined.heading.insert(name, term);
                }
                None => {}
            }
        }
        while let Some((left, right, sensitive)) = equalities.pop() {
            if let Some((replace, keep)) = joined.equate(left, right, sensitive) {
                for (left, right, _) in &mut equalities {
                    if *left == replace {
                        *left = keep.clone();
                    }
                    if *right == replace {
                        *right = keep.clone();
                    }
                }
            }
        }
        joined
    }

    fn project(&mut self, keep: &BTreeSet<String>, context: &Context) {
        if self.heading.keys().any(|name| !keep.contains(name)) {
            self.projected_representatives.extend(
                self.heading
                    .iter()
                    .filter(|(name, _)| keep.contains(*name) && context.sensitive.contains(*name))
                    .map(|(_, term)| term.clone()),
            );
        }
        self.heading.retain(|name, _| keep.contains(name));
    }

    fn loses_observed_representative(&self, expression: &syn::Expr) -> bool {
        let environment = self.heading.keys().cloned().collect();
        cq::rust::free_variables(expression)
            .dependencies(&environment)
            .iter()
            .any(|name| self.projected_representatives.contains(&self.heading[name]))
    }

    fn rust(
        &mut self,
        ordinal: usize,
        expression: &syn::Expr,
        bindings: &[syn::Ident],
        context: &mut Context,
    ) {
        let environment = self.heading.keys().cloned().collect();
        let mut arguments: Vec<_> = cq::rust::free_variables(expression)
            .dependencies(&environment)
            .iter()
            .map(|name| self.heading[name].clone())
            .collect();
        for name in bindings {
            let term = context.fresh();
            self.heading.insert(symbol_name(name), term.clone());
            arguments.push(term);
        }
        self.atoms.insert(Atom {
            relation: format!("#rust{ordinal}"),
            arguments,
        });
    }
}

struct ChainNode {
    payload: String,
    input: Tableau,
    lost_representative: bool,
}

fn payload(item: &BodyItem) -> String {
    // Equality ignores spans; raw identifier spelling is retained consistently
    // between the source payload and its structurally preserved target node.
    item.to_token_stream().to_string()
}

fn source_tableau(source: &valuation::Program, context: &mut Context) -> (Tableau, Vec<ChainNode>) {
    let mut relations = BTreeMap::<usize, Tableau>::new();
    for (clause, item) in source.cq.query.body.iter().enumerate() {
        let BodyItem::Positive { atom } = item else {
            continue;
        };
        let mut table = Tableau::default();
        let arguments = atom
            .variables()
            .map(|name| table.column(symbol_name(name), context))
            .collect();
        table.atoms.insert(Atom {
            relation: symbol_name(&atom.relation),
            arguments,
        });
        relations.insert(clause, table);
    }
    let hypergraph = cq::hypergraph::Analysis::of(&source.cq);
    let mut table = Tableau::default();
    let mut chain = Vec::new();
    for step in &source.valuation.steps {
        match step {
            valuation::Step::Atom(subatom, _) => {
                let mut input = relations[&subatom.clause()].clone();
                let visible: BTreeSet<_> = subatom
                    .variables()
                    .map(|(_, name)| symbol_name(name))
                    .collect();
                input.project(&visible, context);
                table = table.join(&input, false, context);
            }
            valuation::Step::Equality(equality, _) => table.equality(equality, context),
            valuation::Step::Guard(filter, _) => {
                let item = BodyItem::Filter(filter.clone());
                chain.push(ChainNode {
                    payload: payload(&item),
                    input: table.clone(),
                    lost_representative: table.loses_observed_representative(match &item {
                        BodyItem::Filter(filter) => &filter.condition,
                        BodyItem::Let(binding) => &binding.expression,
                        _ => unreachable!(),
                    }),
                });
                table.rust(chain.len() - 1, &filter.condition, &[], context);
            }
            valuation::Step::Let(binding, _) => {
                let item = BodyItem::Let(binding.clone());
                chain.push(ChainNode {
                    payload: payload(&item),
                    input: table.clone(),
                    lost_representative: table.loses_observed_representative(match &item {
                        BodyItem::Filter(filter) => &filter.condition,
                        BodyItem::Let(binding) => &binding.expression,
                        _ => unreachable!(),
                    }),
                });
                table.rust(
                    chain.len() - 1,
                    &binding.expression,
                    &cq::rust::bindings(&binding.pattern),
                    context,
                );
            }
            valuation::Step::Reduce(reduce, _) => {
                let valuation::Reduction::With(_, support) = &reduce.reduction;
                let left = &relations[&reduce.clause()];
                let mut right = relations[&support.clause()].clone();
                let visible: BTreeSet<_> = support
                    .variables()
                    .map(|(_, name)| symbol_name(name))
                    .collect();
                right.heading.retain(|name, _| visible.contains(name));
                // A reduction compares every listed support column with the
                // target columns in its relational equality class. Private
                // names avoid inventing any extra same-spelling join condition.
                right.heading = right
                    .heading
                    .into_iter()
                    .map(|(name, term)| (format!("#support:{name}"), term))
                    .collect();
                let mut reduced = left.join(&right, false, context);
                for left_name in left.heading.keys() {
                    for right_name in &visible {
                        if hypergraph.equivalent(left_name, right_name) {
                            let lhs = reduced.heading[left_name].clone();
                            let rhs = reduced.heading[&format!("#support:{right_name}")].clone();
                            reduced.equate(
                                lhs,
                                rhs,
                                context.sensitive.contains(left_name)
                                    || context.sensitive.contains(right_name),
                            );
                        }
                    }
                }
                reduced
                    .heading
                    .retain(|name, _| left.heading.contains_key(name));
                relations.insert(reduce.clause(), reduced);
            }
        }
    }
    let head: BTreeSet<_> = source.cq.query.head.variables().map(symbol_name).collect();
    table.heading.retain(|name, _| head.contains(name));
    (table, chain)
}

fn target_tableau(target: &ra::Program, context: &mut Context) -> (Tableau, Vec<ChainNode>) {
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
                table.atoms.insert(Atom {
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
                table.project(&keep, context);
                table
            }
            ra::Operator::Select(select) => {
                let mut table = results[&symbol_name(&select.input)].clone();
                for equality in &select.condition.equalities {
                    table.equality(equality, context);
                }
                table
            }
            ra::Operator::Guard(guard) => {
                let mut table = results[&symbol_name(&guard.input)].clone();
                let item = BodyItem::Filter(cq::Filter {
                    if_token: guard.if_token,
                    condition: guard.condition.clone(),
                });
                chain.push(ChainNode {
                    payload: payload(&item),
                    input: table.clone(),
                    lost_representative: table.loses_observed_representative(match &item {
                        BodyItem::Filter(filter) => &filter.condition,
                        BodyItem::Let(binding) => &binding.expression,
                        _ => unreachable!(),
                    }),
                });
                table.rust(chain.len() - 1, &guard.condition, &[], context);
                table
            }
            ra::Operator::Bind(bind) => {
                let mut table = results[&symbol_name(&bind.input)].clone();
                let item = BodyItem::Let(Box::new(bind.binding()));
                chain.push(ChainNode {
                    payload: payload(&item),
                    input: table.clone(),
                    lost_representative: table.loses_observed_representative(match &item {
                        BodyItem::Filter(filter) => &filter.condition,
                        BodyItem::Let(binding) => &binding.expression,
                        _ => unreachable!(),
                    }),
                });
                table.rust(
                    chain.len() - 1,
                    &bind.expression,
                    &cq::rust::bindings(&bind.pattern),
                    context,
                );
                table
            }
        };
        results.insert(symbol_name(&definition.result), table);
    }
    (
        results
            .remove(&symbol_name(&target.plan.output.input))
            .unwrap(),
        chain,
    )
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
    let mut mapping = BTreeMap::<usize, Term>::new();
    let assign = |from: &Term, to: &Term, mapping: &mut BTreeMap<usize, Term>| match from {
        Term::Literal(_) => from == to,
        Term::Variable(variable) => match mapping.get(variable) {
            Some(old) => old == to,
            None => {
                mapping.insert(*variable, to.clone());
                true
            }
        },
    };
    for column in columns {
        if !assign(
            &source.heading[column],
            &target.heading[column],
            &mut mapping,
        ) {
            return false;
        }
    }
    // Fewest candidates first substantially reduces cyclic-query search.
    let mut atoms: Vec<_> = source.atoms.iter().collect();
    atoms.sort_by_key(|atom| {
        target
            .atoms
            .iter()
            .filter(|candidate| candidate.relation == atom.relation)
            .count()
    });
    fn search(atoms: &[&Atom], target: &Tableau, mapping: BTreeMap<usize, Term>) -> bool {
        let Some((atom, rest)) = atoms.split_first() else {
            return true;
        };
        for candidate in target.atoms.iter().filter(|candidate| {
            candidate.relation == atom.relation && candidate.arguments.len() == atom.arguments.len()
        }) {
            let mut next = mapping.clone();
            let compatible = atom
                .arguments
                .iter()
                .zip(&candidate.arguments)
                .all(|(from, to)| match from {
                    Term::Literal(_) => from == to,
                    Term::Variable(variable) => match next.get(variable) {
                        Some(old) => old == to,
                        None => {
                            next.insert(*variable, to.clone());
                            true
                        }
                    },
                });
            if compatible && search(rest, target, next) {
                return true;
            }
        }
        false
    }
    search(&atoms, target, mapping)
}

pub(crate) fn check(source: &valuation::Program, target: &ra::Program) -> syn::Result<()> {
    let mut context = Context {
        sensitive: crate::rust_sensitive(&source.cq),
        ..Context::default()
    };
    let (source_output, source_chain) = source_tableau(source, &mut context);
    let (target_output, target_chain) = target_tableau(target, &mut context);
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
        if source.lost_representative || target.lost_representative {
            return Err(error(&format!(
                "Rust step {ordinal} follows a projection that may discard its original Eq representative"
            )));
        }
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
