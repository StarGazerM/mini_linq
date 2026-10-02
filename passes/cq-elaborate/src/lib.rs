//! VO to RA prime, preserving the opaque Rust chain.
//!
//! The meaning is `T X = bottom + Set X`: one panic aborts evaluation.
//! Each Rust step is a partial function of its free variables; I/O, mutation
//! and evaluation counts are outside the meaning. Relational equalities are
//! Leibniz identities decided by admissible `==`; residual Rust guards remain opaque.
//! Relational constraints may
//! move earlier across a Rust node, shrinking its domain, while Rust nodes retain
//! written order. Identical chain payloads and source-before constraints retained
//! before each node are a sufficient refinement condition. [`check`] proves
//! conjunctive-tableau containment at every supplied-VO chain input and
//! equality of output sets. Leibniz admissibility makes relational equalities
//! identities. Functional extensionality permits alias solving, deduplication
//! and projection before Rust while retaining its free variables.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ra as ra;
use cq_lang_vo as valuation;
use cq_lang_vo::syntax as cq;
use cq_lang_vo::syntax::symbol_name;
use quote::format_ident;
use syn::Ident;

/// Elaborate a valuation plan into named RA, then check the RA contract.
/// Formal rule C-Guard: Append an opaque guard to the prefix, seeding Unit if absent.
pub fn compile(source: &valuation::Program) -> syn::Result<ra::Program> {
    compile_in(source, true)
}

/// Compile the exact written reference, with no relational hoisting or alias
/// solving. The valuation must contain one full atom per written generator and
/// every equality/bind/guard at its written position; reductions are forbidden.
/// This is the executable reference used for partial-expression refinement tests.
pub fn compile_written(source: &valuation::Program) -> syn::Result<ra::Program> {
    valuation::contract::check(source)?;
    use valuation::cq::BodyItem;
    let exact = source.valuation.steps.len() == source.cq.query.body.len()
        && source
            .cq
            .query
            .body
            .iter()
            .zip(&source.valuation.steps)
            .enumerate()
            .all(|(clause, (item, step))| match (item, step) {
                (BodyItem::Positive { atom }, valuation::Step::Atom(subatom, _)) => {
                    subatom.clause() == clause
                        && subatom.relation == atom.relation
                        && subatom.variables().count() == atom.args.len()
                        && subatom
                            .variables()
                            .map(|(_, variable)| symbol_name(variable))
                            .eq(atom.variables().map(symbol_name))
                }
                (BodyItem::Equality(a), valuation::Step::Equality(b, _)) => a == b,
                (BodyItem::Constraint(a), valuation::Step::Constraint(b, _)) => a == b,
                (BodyItem::Filter(a), valuation::Step::Guard(b, _)) => a == b,
                (BodyItem::Let(a), valuation::Step::Let(b, _)) => a == b,
                _ => false,
            });
    if !exact {
        return Err(syn::Error::new_spanned(
            &source.valuation,
            "written reference requires exactly the full written NF steps, without splitting, reductions or reordering",
        ));
    }
    compile_in(source, false)
}

fn compile_in(source: &valuation::Program, relational_hoisting: bool) -> syn::Result<ra::Program> {
    let analysis = valuation::contract::check(source)?;
    let let_bound: BTreeSet<_> = source
        .cq
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            valuation::cq::BodyItem::Let(b) => Some(cq::rust::bindings(&b.pattern)),
            _ => None,
        })
        .flatten()
        .map(|v| symbol_name(&v))
        .collect();
    let targets = |variables: BTreeSet<String>, before: usize| -> BTreeSet<usize> {
        if variables.is_empty() || !variables.is_disjoint(&let_bound) {
            return BTreeSet::new();
        }
        analysis
            .atoms
            .iter()
            .filter(|(clause, atom)| {
                **clause < before
                    && variables.is_subset(&atom.variables().map(symbol_name).collect())
            })
            .map(|(clause, _)| *clause)
            .collect()
    };
    let mut state = State {
        inputs: &source.cq.inputs().cloned().collect::<Vec<_>>(),
        atoms: &analysis.atoms,
        definitions: Vec::new(),
        reserved: source
            .cq
            .declarations
            .iter()
            .map(|declaration| symbol_name(&declaration.name))
            .collect(),
        next_result: 0,
        relations: BTreeMap::new(),
        prefix: None,
        heading: BTreeSet::new(),
        aliases: if relational_hoisting {
            aliases(&source.cq)
        } else {
            BTreeMap::new()
        },
        relational_hoisting,
        constraints: source
            .cq
            .query
            .body
            .iter()
            .enumerate()
            .filter_map(|(clause, item)| match item {
                valuation::cq::BodyItem::Constraint(c) => Some((
                    c.clone(),
                    targets(c.variables().map(symbol_name).collect(), clause),
                )),
                _ => None,
            })
            .collect(),
        equalities: source
            .cq
            .query
            .body
            .iter()
            .enumerate()
            .filter_map(|(clause, item)| match item {
                valuation::cq::BodyItem::Equality(e) => Some((
                    e.clone(),
                    targets(e.variables().map(symbol_name).collect(), clause),
                )),
                _ => None,
            })
            .collect(),
    };
    for (step, introduces) in source.valuation.steps.iter().zip(&analysis.introduces) {
        match step {
            valuation::Step::Atom(subatom, _) => {
                let check = subatom.variables().all(|(_, variable)| {
                    state
                        .heading
                        .contains(&symbol_name(&state.canonical(variable)))
                });
                state.subatom(subatom, check);
            }
            valuation::Step::Reduce(reduce, _) => state.reduce(reduce),
            valuation::Step::Guard(filter, _) => {
                let input = state.prefix_or_unit();
                state.prefix = Some(state.guard(input, filter));
            }
            valuation::Step::Let(binding, _) => state.bind(binding),
            valuation::Step::Equality(equality, _) => state.equality(equality),
            valuation::Step::Constraint(c, _) => state.constraint(c),
        }
        let introduced: Vec<_> = introduces
            .iter()
            .map(|name| {
                state
                    .aliases
                    .get(name)
                    .map(symbol_name)
                    .unwrap_or_else(|| name.clone())
            })
            .collect();
        state.heading.extend(introduced);
    }
    let output = state.finish(&source.cq.query.head);
    let target = ra::Program {
        relational_token: Default::default(),
        relational_brace: Default::default(),
        plan: ra::Plan {
            declarations: source.cq.declarations.clone(),
            definitions: state.definitions,
            output,
        },
    };
    ra::contract::check(&target)?;
    Ok(target)
}

mod kernel;
pub use kernel::{erase, kernel};
mod correspondence;

/// The RA definitions refine the supplied VO schedule, retaining its Rust
/// chain and preceding relational constraints.
/// Formal rule CK-Elaborate: Check local contracts, chain regions and relational tableaux.
pub fn check(source: &valuation::Program, target: &ra::Program) -> syn::Result<()> {
    valuation::contract::check(source)?;
    ra::contract::check(target)?;
    if target.plan.declarations != source.cq.declarations {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            format!(
                "result `{}`: declarations differ from query `{}`",
                target.plan.output.input, source.cq.name
            ),
        ));
    }
    if target.plan.output.head != source.cq.query.head {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            format!(
                "result `{}`: output head differs from `{}`",
                target.plan.output.input, source.cq.query.head.relation
            ),
        ));
    }
    correspondence::check(source, target)?;
    Ok(())
}

/// The definitions so far, each clause's current relation, the prefix
/// result and its heading.
struct State<'a> {
    inputs: &'a [cq::RelationDecl],
    atoms: &'a BTreeMap<usize, cq::Atom>,
    definitions: Vec<ra::Definition>,
    reserved: BTreeSet<String>,
    next_result: usize,
    relations: BTreeMap<usize, Ident>,
    prefix: Option<Ident>,
    heading: BTreeSet<String>,
    aliases: BTreeMap<String, Ident>,
    relational_hoisting: bool,
    equalities: Vec<(ra::Equality, BTreeSet<usize>)>,
    constraints: Vec<(cq::Constraint, BTreeSet<usize>)>,
}

impl State<'_> {
    /// The clause's relation: its rename, emitted at first mention, as
    /// reduced so far.
    /// Formal rule E-Relation: Reuse a clause result or emit its complete positional rename at first use.
    fn relation(&mut self, clause: usize) -> Ident {
        if let Some(result) = self.relations.get(&clause) {
            return result.clone();
        }
        let atom = &self.atoms[&clause];
        let declaration = self
            .inputs
            .iter()
            .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
            .expect("the CQ contract declares every body relation");
        let mapping = declaration
            .columns
            .iter()
            .zip(atom.variables())
            .map(|(column, variable)| ra::AttributeRename {
                source: column.name.clone(),
                minus_token: Default::default(),
                gt_token: Default::default(),
                target: self.canonical(variable),
            })
            .collect();
        let mut result = self.emit(ra::Operator::Rename(Box::new(ra::Rename {
            rename_token: Default::default(),
            relation: atom.relation.clone(),
            mapping_brace: Default::default(),
            mapping,
        })));
        // A literal equality is a keyed relational read of this occurrence.
        // It may move earlier across Rust because it only restricts its domain.
        let literals: Vec<_> = self
            .equalities
            .iter()
            .filter(|(e, targets)| {
                self.relational_hoisting
                    && targets.contains(&clause)
                    && matches!(e.right, ra::EqualityValue::Literal(_))
            })
            .map(|(e, _)| ra::Equality {
                left: self.canonical(&e.left),
                ..e.clone()
            })
            .collect();
        if !literals.is_empty() {
            result = self.select(result, literals);
        }
        // Ordinary relational pushdown: every used column belongs to this
        // occurrence. Group same-column conjuncts into one range selection.
        if self.relational_hoisting {
            let mut groups: Vec<(BTreeSet<String>, Vec<cq::Constraint>)> = Vec::new();
            for (c, targets) in &self.constraints {
                let columns: BTreeSet<_> = c.variables().map(symbol_name).collect();
                if targets.contains(&clause) {
                    let mut predicate = c.clone();
                    predicate.map_terms(&mut |t| {
                        if let cq::constraint::Term::Variable(v) = t {
                            *v = self.canonical(v);
                        }
                    });
                    if let Some((_, constraints)) =
                        groups.iter_mut().find(|(key, _)| *key == columns)
                    {
                        constraints.push(predicate);
                    } else {
                        groups.push((columns, vec![predicate]));
                    }
                }
            }
            for (_, constraints) in groups {
                result = self.select_constraints(result, constraints);
            }
        }
        self.relations.insert(clause, result.clone());
        result
    }

    /// Formal rule E-Check: A partial subatom with no new names semijoins an existing prefix.
    /// Formal rule E-Project: Project a partial subatom unless it is an existing-prefix check.
    /// Formal rule B-Root: Install the first full or projected subatom as the prefix.
    /// Formal rule B-Join: Join a non-check subatom with the current prefix.
    fn subatom(&mut self, subatom: &valuation::Subatom, check: bool) {
        let clause = subatom.clause();
        let mut right = self.relation(clause);
        let partial = subatom.columns.len() != subatom.variables().count();
        // A check binds nothing: it lists every bound column of its clause,
        // which is exactly the heading it shares with the prefix, so it is a
        // semijoin with the clause's own relation. Reading the relation — not
        // a projection of it — lets every prefix check of one clause land on
        // one result, where one curried structure answers them all.
        if partial && check {
            if let Some(left) = self.prefix.take() {
                self.prefix = Some(self.semijoin(left, right));
                return;
            }
        }
        if partial {
            let variables: Vec<&Ident> = self.atoms[&clause].variables().collect();
            let attributes = subatom
                .variables()
                .map(|(position, _)| self.canonical(variables[position]))
                .collect();
            right = self.project(right, attributes);
        }
        self.prefix = Some(match self.prefix.take() {
            None => right,
            Some(left) => self.emit(ra::Operator::NaturalJoin(Box::new(ra::NaturalJoin {
                natural_join_token: Default::default(),
                left,
                with_token: Default::default(),
                right,
            }))),
        });
    }

    /// Formal rule E-ReduceWith: Semijoin current clause relations and replace only the target entry.
    fn reduce(&mut self, reduce: &valuation::Reduce) {
        let clause = reduce.clause();
        let left = self.relation(clause);
        let result = match &reduce.reduction {
            valuation::Reduction::With(_, subatom) => {
                let right = self.relation(subatom.clause());
                self.semijoin(left, right)
            }
        };
        self.relations.insert(clause, result);
    }

    fn semijoin(&mut self, left: Ident, right: Ident) -> Ident {
        self.emit(ra::Operator::Semijoin(Box::new(ra::Semijoin {
            semijoin_token: Default::default(),
            left,
            with_token: Default::default(),
            right,
        })))
    }

    fn canonical(&self, variable: &Ident) -> Ident {
        self.aliases
            .get(&symbol_name(variable))
            .unwrap_or(variable)
            .clone()
    }

    fn select(&mut self, input: Ident, equalities: Vec<ra::Equality>) -> Ident {
        self.emit(ra::Operator::Select(Box::new(ra::Select {
            select_token: Default::default(),
            input,
            if_token: Default::default(),
            condition_paren: Default::default(),
            condition: ra::Theta {
                equalities: equalities.into_iter().collect(),
                constraints: Default::default(),
            },
        })))
    }

    fn select_constraints(&mut self, input: Ident, constraints: Vec<cq::Constraint>) -> Ident {
        self.emit(ra::Operator::Select(Box::new(ra::Select {
            select_token: Default::default(),
            input,
            if_token: Default::default(),
            condition_paren: Default::default(),
            condition: ra::Theta {
                equalities: Default::default(),
                constraints: constraints.into_iter().collect(),
            },
        })))
    }

    /// Formal rule E-Constraint: Push a central constraint onto its complete binder or select the prefix.
    fn constraint(&mut self, constraint: &cq::Constraint) {
        if self.relational_hoisting
            && self
                .constraints
                .iter()
                .filter(|(c, _)| c == constraint)
                .all(|(_, targets)| !targets.is_empty())
        {
            return;
        }
        let mut c = constraint.clone();
        c.map_terms(&mut |t| {
            if let cq::constraint::Term::Variable(v) = t {
                *v = self.canonical(v);
            }
        });
        let input = self.prefix_or_unit();
        self.prefix = Some(self.select_constraints(input, vec![c]));
    }

    fn equality(&mut self, equality: &ra::Equality) {
        if self.relational_hoisting
            && matches!(equality.right, ra::EqualityValue::Literal(_))
            && self
                .equalities
                .iter()
                .filter(|(e, _)| e == equality)
                .all(|(_, targets)| !targets.is_empty())
        {
            return;
        }
        let left = self.canonical(&equality.left);
        let right = match &equality.right {
            ra::EqualityValue::Variable(variable) => {
                let variable = self.canonical(variable);
                if symbol_name(&variable) == symbol_name(&left) {
                    return;
                }
                ra::EqualityValue::Variable(variable)
            }
            other => other.clone(),
        };
        let input = self.prefix_or_unit();
        self.prefix = Some(self.select(
            input,
            vec![ra::Equality {
                left,
                eq_token: equality.eq_token,
                right,
            }],
        ));
    }

    fn guard(&mut self, input: Ident, filter: &cq::Filter) -> Ident {
        self.emit(ra::Operator::Guard(Box::new(ra::Guard {
            guard_token: Default::default(),
            input,
            if_token: filter.if_token,
            condition_paren: Default::default(),
            condition: filter.condition.clone(),
        })))
    }

    /// Formal rule C-Bind: Append an opaque pattern binding to the prefix.
    fn bind(&mut self, binding: &cq::Let) {
        let input = self.prefix_or_unit();
        self.prefix = Some(self.emit(ra::Operator::Bind(Box::new(ra::Bind {
            bind_token: Default::default(),
            input,
            with_token: Default::default(),
            pattern: binding.pattern.clone(),
            annotation: binding.annotation.clone(),
            eq_token: binding.eq_token,
            expression_paren: Default::default(),
            expression: binding.expression.clone(),
        }))));
    }

    /// Retain the normalized head variables and name the output.
    /// Formal rule C-Finish: Project the head variables after the complete chain.
    fn finish(&mut self, head: &cq::Atom) -> ra::Output {
        let retained: BTreeSet<String> = head.variables().map(symbol_name).collect();
        let mut bindings = BTreeMap::new();
        for definition in &self.definitions {
            match &definition.operator {
                ra::Operator::Rename(r) => {
                    for m in &r.mapping {
                        bindings
                            .entry(symbol_name(&m.target))
                            .or_insert_with(|| m.target.clone());
                    }
                }
                ra::Operator::Bind(e) => {
                    for name in cq::rust::bindings(&e.pattern) {
                        bindings.entry(symbol_name(&name)).or_insert(name);
                    }
                }
                _ => (),
            }
        }
        let attributes = retained.iter().map(|v| bindings[v].clone()).collect();
        let input = self.prefix_or_unit();
        let result = self.project(input, attributes);
        ra::Output {
            output_token: Default::default(),
            input: result,
            as_token: Default::default(),
            head: head.clone(),
            semi_token: Default::default(),
        }
    }

    fn project(&mut self, input: Ident, attributes: cq_lang_vo::syntax::CommaList<Ident>) -> Ident {
        self.emit(ra::Operator::Project(Box::new(ra::Project {
            project_token: Default::default(),
            input,
            keep_token: Default::default(),
            attributes_brace: Default::default(),
            attributes,
        })))
    }

    fn prefix_or_unit(&mut self) -> Ident {
        if let Some(prefix) = &self.prefix {
            return prefix.clone();
        }
        let unit = self.emit(ra::Operator::Unit(ra::Unit {
            unit_token: Default::default(),
        }));
        self.prefix = Some(unit.clone());
        unit
    }

    /// Formal rule E-Emit: Append the operator under a fresh rN in emission order.
    fn emit(&mut self, operator: ra::Operator) -> Ident {
        let result = loop {
            let name = format!("r{}", self.next_result);
            self.next_result += 1;
            if self.reserved.insert(name.clone()) {
                break format_ident!("{name}");
            }
        };
        self.definitions.push(ra::Definition {
            result: result.clone(),
            eq_token: Default::default(),
            operator,
            semi_token: Default::default(),
        });
        result
    }
}

/// Solve generator equalities through shared join headings when doing so keeps
/// every opaque expression's names and every atom's distinct rename targets.
fn aliases(source: &valuation::cq::Program) -> BTreeMap<String, Ident> {
    use valuation::cq::BodyItem;
    let atoms: Vec<_> = source
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            BodyItem::Positive { atom } => Some(atom),
            _ => None,
        })
        .collect();
    let let_bound: BTreeSet<_> = source
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            BodyItem::Let(b) => Some(cq::rust::bindings(&b.pattern)),
            _ => None,
        })
        .flatten()
        .map(|v| symbol_name(&v))
        .collect();
    let generators: BTreeSet<_> = atoms
        .iter()
        .flat_map(|atom| atom.variables().map(symbol_name))
        .filter(|v| !let_bound.contains(v))
        .collect();
    let mut protected: BTreeSet<_> = source.query.head.variables().map(symbol_name).collect();
    for item in &source.query.body {
        let expression = match item {
            BodyItem::Filter(filter) => &filter.condition,
            BodyItem::Let(binding) => {
                protected.extend(cq::rust::bindings(&binding.pattern).iter().map(symbol_name));
                &binding.expression
            }
            _ => continue,
        };
        protected.extend(cq::rust::free_variables(expression).dependencies(&generators));
    }
    let mut aliases = BTreeMap::<String, Ident>::new();
    let canonical = |id: &Ident, aliases: &BTreeMap<String, Ident>| {
        aliases.get(&symbol_name(id)).unwrap_or(id).clone()
    };
    for item in &source.query.body {
        let BodyItem::Equality(equality) = item else {
            continue;
        };
        let ra::EqualityValue::Variable(right) = &equality.right else {
            continue;
        };
        if !generators.contains(&symbol_name(&equality.left))
            || !generators.contains(&symbol_name(right))
        {
            continue;
        }
        let left = canonical(&equality.left, &aliases);
        let right = canonical(right, &aliases);
        if left == right {
            continue;
        }
        let (keep, replace) = if protected.contains(&symbol_name(&right)) {
            (right, left)
        } else {
            (left, right)
        };
        if protected.contains(&symbol_name(&replace)) {
            continue;
        }
        let mut proposed = aliases.clone();
        for value in proposed.values_mut() {
            if symbol_name(value) == symbol_name(&replace) {
                *value = keep.clone();
            }
        }
        proposed.insert(symbol_name(&replace), keep);
        if atoms.iter().any(|atom| {
            let names: BTreeSet<_> = atom
                .variables()
                .map(|v| symbol_name(&canonical(v, &proposed)))
                .collect();
            names.len() != atom.args.len()
        }) {
            continue;
        }
        aliases = proposed;
    }
    aliases
}
