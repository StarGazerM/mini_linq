//! Schedule `T X = bottom + Set X` comprehensions into valuations.
//! Rust lets and guards are opaque partial functions and keep written relative
//! order, their free-variable binders, and all their written relational
//! predecessors. Relational steps alone commute and may move earlier across
//! the Rust chain. This sufficient refinement condition permits no new panic
//! on a nonpanicking written reference input. Other effects, including I/O,
//! mutation and evaluation counts, are outside this denotation.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_cq as cq;
use cq_lang_cq::syntax::symbol_name;
use cq_lang_cq::{BodyItem, rust};
use cq_lang_vo as valuation;
use proc_macro2::Span;
use syn::LitInt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub heuristic: Heuristic,
    pub split: Split,
}

/// Which clause comes next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Heuristic {
    #[default]
    Written,
    Prefix(Ranking),
    Elimination,
}

/// How `Prefix` ranks the ready atoms, after ready guards and bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ranking {
    /// An atom sharing a bound variable, then the earliest clause.
    Connected,
    /// Connected, then the narrowest heading after the join.
    GreedyWidth,
    /// The same ranking as `GreedyWidth`.
    Auto,
}

/// How many columns a subatom binds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Split {
    /// One full subatom per atom occurrence.
    #[default]
    Coarse,
    /// One new variable per subatom.
    Fine,
}

/// One normal-form body clause as the scheduler sees it.
struct Clause<'a> {
    item: &'a BodyItem,
    /// The variables the clause binds: atom columns in order, or the
    /// pattern's bindings.
    provides: Vec<String>,
    /// The bound variables the clause waits for: the uses a guard or binding
    /// makes of the written prefix, or the columns of an atom that some
    /// binding supplies.
    requires: BTreeSet<String>,
}

impl Clause<'_> {
    fn is_atom(&self) -> bool {
        matches!(self.item, BodyItem::Positive { .. })
    }
}

/// Formal rule D-Atom: Atom provides its columns and requires let-supplied columns.
/// Formal rule D-Guard: Guard requires dependencies on the written prefix.
/// Formal rule D-Bind: Binding provides pattern names and requires RHS prefix dependencies.
fn clauses(query: &cq::Query) -> Vec<Clause<'_>> {
    let let_bound: BTreeSet<String> = query
        .body
        .iter()
        .filter_map(|item| match item {
            BodyItem::Let(binding) => Some(rust::bindings(&binding.pattern)),
            _ => None,
        })
        .flatten()
        .map(|v| symbol_name(&v))
        .collect();
    let mut prefix = BTreeSet::new();
    query
        .body
        .iter()
        .map(|item| {
            let (provides, requires) = match item {
                BodyItem::Positive { atom } => {
                    let provides: Vec<String> = atom.variables().map(symbol_name).collect();
                    let requires = provides
                        .iter()
                        .filter(|v| let_bound.contains(*v))
                        .cloned()
                        .collect();
                    (provides, requires)
                }
                BodyItem::Equality(equality) => {
                    (Vec::new(), equality.variables().map(symbol_name).collect())
                }
                // Formal rule D-Constraint: A central constraint requires only its query variables.
                BodyItem::Constraint(c) => (Vec::new(), c.variables().map(symbol_name).collect()),
                BodyItem::Filter(filter) => (
                    Vec::new(),
                    rust::free_variables(&filter.condition).dependencies(&prefix),
                ),
                BodyItem::Let(binding) => (
                    rust::bindings(&binding.pattern)
                        .iter()
                        .map(symbol_name)
                        .collect(),
                    rust::free_variables(&binding.expression).dependencies(&prefix),
                ),
                BodyItem::Negation(_) | BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {
                    unreachable!("normal form")
                }
            };
            prefix.extend(provides.iter().cloned());
            Clause {
                item,
                provides,
                requires,
            }
        })
        .collect()
}

/// Formal rule C-Ready: Require inputs, next Rust chain step, and every written relational predecessor.
fn ready(
    clauses: &[Clause<'_>],
    pending: &[usize],
    bound: &BTreeSet<String>,
    clause: usize,
) -> bool {
    clauses[clause].requires.is_subset(bound)
        && match clauses[clause].item {
            BodyItem::Positive { .. } | BodyItem::Equality(_) | BodyItem::Constraint(_) => true,
            BodyItem::Filter(_) | BodyItem::Let(_) => {
                pending
                    .iter()
                    .copied()
                    .filter(|&c| matches!(clauses[c].item, BodyItem::Filter(_) | BodyItem::Let(_)))
                    .min()
                    == Some(clause)
                    && !pending.iter().any(|&c| {
                        c < clause
                            && matches!(
                                clauses[c].item,
                                BodyItem::Positive { .. }
                                    | BodyItem::Equality(_)
                                    | BodyItem::Constraint(_)
                            )
                    })
            }
            _ => unreachable!("normal form"),
        }
}

/// A rank orders the ready clauses; the least is placed next. Guards and
/// bindings rank `(false, ..)`, so they go before any atom.
type Rank = (bool, bool, usize, usize);

/// Place the clauses one at a time, each the least ready clause by `rank`.
fn schedule(
    query: &cq::Query,
    clauses: &[Clause<'_>],
    rank: impl Fn(usize, &BTreeSet<String>) -> Rank,
) -> syn::Result<Vec<usize>> {
    let mut bound = BTreeSet::new();
    let mut pending: Vec<usize> = (0..clauses.len()).collect();
    let mut sequence = Vec::with_capacity(clauses.len());
    while !pending.is_empty() {
        let choice = pending
            .iter()
            .copied()
            .filter(|&c| ready(clauses, &pending, &bound, c))
            .min_by_key(|&c| rank(c, &bound));
        let Some(choice) = choice else {
            return Err(syn::Error::new_spanned(query, "no clause is ready"));
        };
        pending.retain(|&c| c != choice);
        bound.extend(clauses[choice].provides.iter().cloned());
        sequence.push(choice);
    }
    Ok(sequence)
}

/// The clause-prefix rank: a ready guard or binding, then an atom sharing a
/// bound variable, then the narrowest heading, then the earliest clause.
/// Formal rule S-Connected: Ready atoms rank by shared names then clause index.
/// Formal rule S-Greedy: GreedyWidth and Auto rank connected atoms before union arity.
fn prefix_rank<'a>(
    clauses: &'a [Clause<'a>],
    ranking: Ranking,
    canonical_names: &'a BTreeMap<String, String>,
) -> impl Fn(usize, &BTreeSet<String>) -> Rank + 'a {
    move |clause, bound| {
        let facts = &clauses[clause];
        if !facts.is_atom() {
            let priority = usize::from(!matches!(facts.item, BodyItem::Constraint(_)));
            return (false, false, priority, clause);
        }
        let canonical = |name: &String| canonical_names.get(name).unwrap_or(name).clone();
        let supplied: BTreeSet<_> = facts.provides.iter().map(canonical).collect();
        let available: BTreeSet<_> = bound.iter().map(canonical).collect();
        let shared = supplied.intersection(&available).count();
        let width = match ranking {
            Ranking::Connected => 0,
            Ranking::GreedyWidth | Ranking::Auto => available.union(&supplied).count(),
        };
        (true, shared == 0, width, clause)
    }
}

/// Atoms in the order of their earliest variable in the elimination order.
/// Formal rule S-Elimination: Rank ready atoms by earliest elimination variable then clause index.
fn elimination(
    source: &cq::Program,
    clauses: &[Clause<'_>],
) -> syn::Result<(Vec<usize>, Vec<String>)> {
    let order = cq::hypergraph::Analysis::of(source).elimination_order;
    let mut earliest = vec![order.len(); clauses.len()];
    for (clause, facts) in clauses.iter().enumerate().filter(|(_, f)| f.is_atom()) {
        for variable in &facts.provides {
            let Some(position) = order.iter().position(|v| v == variable) else {
                return Err(syn::Error::new_spanned(
                    facts.item,
                    format!("`{variable}` has no place in the elimination order"),
                ));
            };
            earliest[clause] = earliest[clause].min(position);
        }
    }
    let sequence = schedule(&source.query, clauses, |clause, _| {
        if clauses[clause].is_atom() {
            (true, false, earliest[clause], clause)
        } else {
            (
                false,
                false,
                usize::from(!matches!(clauses[clause].item, BodyItem::Constraint(_))),
                clause,
            )
        }
    })?;
    Ok((sequence, order))
}

/// Variables in the order the clause sequence first binds them.
fn binding_order(clauses: &[Clause<'_>], sequence: &[usize]) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    for &clause in sequence {
        for variable in &clauses[clause].provides {
            if !order.contains(variable) {
                order.push(variable.clone());
            }
        }
    }
    order
}

fn subatom(clause: usize, atom: &cq::Atom, keep: impl Fn(&str) -> bool) -> valuation::Step {
    let columns = atom
        .variables()
        .map(|v| {
            if keep(&symbol_name(v)) {
                valuation::Column::Variable(v.clone())
            } else {
                valuation::Column::Dropped(Default::default())
            }
        })
        .collect();
    valuation::Step::Atom(
        valuation::Subatom {
            relation: atom.relation.clone(),
            at: Default::default(),
            clause: LitInt::new(&clause.to_string(), Span::call_site()),
            paren: Default::default(),
            columns,
        },
        Default::default(),
    )
}

/// Formal rule O-Coarse: Copy guards and lets; emit one full subatom per chosen atom.
fn coarse(clauses: &[Clause<'_>], sequence: &[usize]) -> Vec<valuation::Step> {
    sequence
        .iter()
        .map(|&clause| match clauses[clause].item {
            BodyItem::Positive { atom } => subatom(clause, atom, |_| true),
            BodyItem::Equality(equality) => {
                valuation::Step::Equality(equality.clone(), Default::default())
            }
            BodyItem::Constraint(c) => valuation::Step::Constraint(c.clone(), Default::default()),
            BodyItem::Filter(filter) => valuation::Step::Guard(filter.clone(), Default::default()),
            BodyItem::Let(binding) => valuation::Step::Let(binding.clone(), Default::default()),
            BodyItem::Negation(_) | BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {
                unreachable!("normal form")
            }
        })
        .collect()
}

/// The steps of a fine plan under construction: guards and bindings are
/// placed as soon as their variables are bound.
struct Fine<'a> {
    clauses: &'a [Clause<'a>],
    bound: BTreeSet<String>,
    /// Guards and bindings not yet placed, in query order.
    waiting: Vec<usize>,
    incomplete_atoms: BTreeSet<usize>,
    steps: Vec<valuation::Step>,
}

impl Fine<'_> {
    fn place_ready(&mut self) {
        loop {
            let pending: Vec<_> = self
                .waiting
                .iter()
                .copied()
                .chain(self.incomplete_atoms.iter().copied())
                .collect();
            let Some(clause) = self
                .waiting
                .iter()
                .copied()
                .filter(|&c| ready(self.clauses, &pending, &self.bound, c))
                .min_by_key(|&c| {
                    (
                        usize::from(!matches!(self.clauses[c].item, BodyItem::Constraint(_))),
                        c,
                    )
                })
            else {
                break;
            };
            self.waiting.retain(|&c| c != clause);
            self.bound
                .extend(self.clauses[clause].provides.iter().cloned());
            self.steps.push(match self.clauses[clause].item {
                BodyItem::Equality(equality) => {
                    valuation::Step::Equality(equality.clone(), Default::default())
                }
                BodyItem::Constraint(c) => {
                    valuation::Step::Constraint(c.clone(), Default::default())
                }
                BodyItem::Filter(filter) => {
                    valuation::Step::Guard(filter.clone(), Default::default())
                }
                BodyItem::Let(binding) => valuation::Step::Let(binding.clone(), Default::default()),
                _ => unreachable!("equalities and Rust steps wait"),
            });
        }
    }

    /// The clause's subatom over its bound columns.
    fn subatom(&mut self, clause: usize, atom: &cq::Atom) {
        let step = subatom(clause, atom, |v| self.bound.contains(v));
        if atom
            .variables()
            .all(|v| self.bound.contains(&symbol_name(v)))
        {
            self.incomplete_atoms.remove(&clause);
        }
        self.steps.push(step);
    }
}

/// One variable at a time, in `variables` order: its first atom in plan
/// order binds it over the bound columns, every other atom holding it
/// checks; a variable a binding supplies waits for that binding.
/// Formal rule O-Fine: Iterate variables with ready bindings and guards; introduce then check.
fn fine(
    query: &cq::Query,
    clauses: &[Clause<'_>],
    sequence: &[usize],
    variables: &[String],
) -> syn::Result<Vec<valuation::Step>> {
    let atoms: Vec<usize> = sequence
        .iter()
        .copied()
        .filter(|&c| clauses[c].is_atom())
        .collect();
    let let_bound: BTreeSet<&String> = clauses
        .iter()
        .filter(|c| matches!(c.item, BodyItem::Let(_)))
        .flat_map(|c| &c.provides)
        .collect();
    let mut plan = Fine {
        clauses,
        bound: BTreeSet::new(),
        waiting: (0..clauses.len())
            .filter(|&c| !clauses[c].is_atom())
            .collect(),
        incomplete_atoms: atoms.iter().copied().collect(),
        steps: Vec::new(),
    };
    for &clause in &atoms {
        if atom(clauses[clause].item).args.is_empty() {
            plan.subatom(clause, atom(clauses[clause].item));
        }
    }
    let mut remaining: Vec<&String> = variables.iter().collect();
    while !remaining.is_empty() {
        plan.place_ready();
        let Some(next) = remaining.iter().position(|v| {
            if plan.bound.contains(*v) {
                return true;
            }
            !let_bound.contains(v)
        }) else {
            return Err(syn::Error::new_spanned(
                query,
                "no variable can be bound next",
            ));
        };
        let variable = remaining.remove(next);
        let mut holding = atoms
            .iter()
            .copied()
            .filter(|&c| clauses[c].provides.contains(variable));
        if !plan.bound.contains(variable) {
            let Some(binder) = holding.next() else {
                return Err(syn::Error::new_spanned(
                    query,
                    format!("`{variable}` is not a variable of the query"),
                ));
            };
            plan.bound.insert(variable.clone());
            plan.subatom(binder, atom(clauses[binder].item));
            plan.place_ready();
        }
        for check in holding {
            plan.subatom(check, atom(clauses[check].item));
        }
    }
    plan.place_ready();
    Ok(plan.steps)
}

fn atom(item: &BodyItem) -> &cq::Atom {
    item.atom().expect("an atom clause")
}

/// Written splits each atom in place; it performs no cross-clause reordering.
fn written_fine(clauses: &[Clause<'_>]) -> Vec<valuation::Step> {
    let mut steps = Vec::new();
    let mut bound = BTreeSet::new();
    for (clause, facts) in clauses.iter().enumerate() {
        if let BodyItem::Positive { atom } = facts.item {
            let new: Vec<_> = facts
                .provides
                .iter()
                .filter(|v| !bound.contains(*v))
                .cloned()
                .collect();
            if new.is_empty() {
                steps.push(subatom(clause, atom, |_| true));
            } else {
                for variable in new {
                    bound.insert(variable);
                    steps.push(subatom(clause, atom, |v| bound.contains(v)));
                }
            }
        } else {
            steps.extend(coarse(clauses, &[clause]));
            bound.extend(facts.provides.iter().cloned());
        }
    }
    steps
}

/// Order the normal-form query under `options`, then check the VO contract.
/// Formal rule S-Written: Written uses the complete normal-form body sequence.
pub fn compile(source: &cq::Program, options: &Options) -> syn::Result<valuation::Program> {
    cq::normal::check(source)?;
    let query = &source.query;
    let clauses = clauses(query);
    let hypergraph = cq::hypergraph::Analysis::of(source);
    let (sequence, mut variables) = match options.heuristic {
        Heuristic::Written => {
            let sequence: Vec<usize> = (0..clauses.len()).collect();
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Prefix(ranking) => {
            let sequence = schedule(
                query,
                &clauses,
                prefix_rank(&clauses, ranking, &hypergraph.canonical_names),
            )?;
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Elimination => elimination(source, &clauses)?,
    };
    // Elimination removes leaves first. Enumerating in that direction joins
    // their independent projections before any connecting variable is bound.
    // Reverse it for Fine; Coarse retains the earliest-variable atom ranking.
    if options.heuristic == Heuristic::Elimination && options.split == Split::Fine {
        variables.reverse();
    }
    if options.split == Split::Fine && options.heuristic != Heuristic::Written {
        // Enumerate every alias of a relational variable before the next
        // variable class. Normalization must not erase generic-join checks
        // against the other atoms holding that logical variable.
        let mut grouped = Vec::new();
        for variable in &variables {
            let mut aliases: Vec<_> = variables
                .iter()
                .filter(|alias| hypergraph.equivalent(variable, alias))
                .collect();
            // Identity permits any ranked supplier. Bind the class through
            // its earliest scheduled atom before checking the other aliases;
            // name order alone can choose a disconnected supplier instead.
            aliases.sort_by_key(|alias| {
                sequence
                    .iter()
                    .position(|&clause| clauses[clause].provides.contains(alias))
            });
            for alias in aliases {
                if !grouped.contains(alias) {
                    grouped.push(alias.clone());
                }
            }
        }
        variables = grouped;
    }
    let steps = match options.split {
        Split::Coarse => coarse(&clauses, &sequence),
        Split::Fine if options.heuristic == Heuristic::Written => written_fine(&clauses),
        Split::Fine => fine(query, &clauses, &sequence, &variables)?,
    };
    let program = valuation::Program {
        cq: source.clone(),
        valuation: valuation::Valuation {
            valuation_token: Default::default(),
            brace: Default::default(),
            steps,
        },
    };
    valuation::contract::check(&program)?;
    Ok(program)
}

/// Check normal form, unchanged CQ, split granularity and explicit order.
/// Prefix and elimination rankings are expansion choices, not obligations.
/// Formal rule CK-Order: Validate unchanged CQ, split, coverage and Written introducing order.
pub fn check(
    source: &cq::Program,
    target: &valuation::Program,
    options: &Options,
) -> syn::Result<()> {
    cq::normal::check(source)?;
    let analysis = valuation::contract::check(target)?;
    if target.cq != *source {
        return Err(syn::Error::new_spanned(
            &target.cq.name,
            format!(
                "query `{}`: valuation must preserve source CQ `{}`",
                target.cq.name, source.name
            ),
        ));
    }
    if options.heuristic == Heuristic::Written {
        let mut written = Vec::new();
        let mut previous_atom = None;
        for step in &target.valuation.steps {
            match step {
                valuation::Step::Atom(atom, _) => {
                    if previous_atom != Some(atom.clause()) {
                        written.push(atom.clause());
                    }
                    previous_atom = Some(atom.clause());
                }
                valuation::Step::Equality(equality, _) => {
                    let position = source.query.body.iter().enumerate().find(|(i,item)| !written.contains(i) && matches!(item, BodyItem::Equality(expected) if expected == equality)).map(|(i,_)| i).expect("VO equality correspondence");
                    written.push(position);
                    previous_atom = None;
                }
                valuation::Step::Constraint(constraint, _) => {
                    let position = source.query.body.iter().enumerate().find(|(i, item)|
                        !written.contains(i) && matches!(item, BodyItem::Constraint(expected) if expected == constraint)
                    ).map(|(i, _)| i).expect("VO constraint correspondence");
                    written.push(position);
                    previous_atom = None;
                }
                valuation::Step::Guard(filter, _) => {
                    let position = source
                        .query
                        .body
                        .iter()
                        .enumerate()
                        .find(|(i, item)| {
                            !written.contains(i)
                                && matches!(item, BodyItem::Filter(expected) if expected == filter)
                        })
                        .map(|(i, _)| i)
                        .expect("VO chain correspondence");
                    written.push(position);
                    previous_atom = None;
                }
                valuation::Step::Let(binding, _) => {
                    let position = source
                        .query
                        .body
                        .iter()
                        .enumerate()
                        .find(|(i, item)| {
                            !written.contains(i)
                                && matches!(item, BodyItem::Let(expected) if expected == binding)
                        })
                        .map(|(i, _)| i)
                        .expect("VO chain correspondence");
                    written.push(position);
                    previous_atom = None;
                }
                valuation::Step::Reduce(..) => {}
            }
        }
        if written != (0..source.query.body.len()).collect::<Vec<_>>() {
            let position = written
                .iter()
                .enumerate()
                .find(|(i, clause)| **clause != *i)
                .map(|(_, clause)| *clause)
                .unwrap_or(source.query.body.len());
            return Err(syn::Error::new_spanned(
                &target.valuation,
                format!("clause #{position}: Written requires every clause in normal-body order"),
            ));
        }
    }
    let mut seen = BTreeSet::new();
    let mut previous = None;
    for (step, introduced) in target.valuation.steps.iter().zip(&analysis.introduces) {
        match step {
            valuation::Step::Reduce(r, _) => {
                return Err(syn::Error::new_spanned(
                    r,
                    format!(
                        "clause #{} (`{}`): ordering cannot introduce reductions",
                        r.clause(),
                        r.relation
                    ),
                ));
            }
            valuation::Step::Atom(a, _) => {
                let clause = a.clause();
                let first = seen.insert(clause);
                let fail = |message: &str| {
                    syn::Error::new_spanned(
                        a,
                        format!("clause #{clause} (`{}`): {message}", a.relation),
                    )
                };
                if options.split == Split::Coarse
                    && (!first || a.variables().count() != a.columns.len())
                {
                    return Err(fail(
                        "Coarse requires exactly one full subatom per atom clause",
                    ));
                }
                if options.split == Split::Fine && introduced.len() > 1 {
                    return Err(fail("Fine permits at most one new variable per subatom"));
                }
                // Written: the clauses that introduce variables follow the
                // normal body (Coarse: every clause's single subatom; Fine: a
                // check of a later clause may precede, as in R@0, T@2, S@1).
                let introduces = match options.split {
                    Split::Coarse => first,
                    Split::Fine => !introduced.is_empty(),
                };
                if options.heuristic == Heuristic::Written && introduces {
                    if previous.is_some_and(|p| p > clause) {
                        return Err(fail(
                            "Written requires introducing subatoms in normal-body order",
                        ));
                    }
                    previous = Some(clause);
                }
            }
            valuation::Step::Equality(..)
            | valuation::Step::Constraint(..)
            | valuation::Step::Guard(..)
            | valuation::Step::Let(..) => {}
        }
    }
    // The VO variable-coverage check alone is vacuous for nullary atoms.
    for (clause, atom) in &analysis.atoms {
        if !seen.contains(clause) {
            return Err(syn::Error::new_spanned(
                atom,
                format!(
                    "clause #{clause} (`{}`): every atom requires a subatom",
                    atom.relation
                ),
            ));
        }
    }
    Ok(())
}
