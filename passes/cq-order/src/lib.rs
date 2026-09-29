//! CQ-NF → VO: the order in which a query's valuations are built.
//!
//! This pass decides one thing: the sequence of steps — which clause each
//! subatom draws on and which of its columns it binds, and where each guard
//! and binding sits. A [`Heuristic`] fixes the clause order and a [`Split`]
//! the granularity; nothing else is chosen here.
//!
//! - `Written` keeps the normal-form body order; guards and bindings stay
//!   where the body has them.
//! - `Prefix(ranking)` repeats the clause-prefix choice: among the clauses
//!   whose inputs are bound, a guard or binding first, then an atom sharing
//!   a bound variable, then (`GreedyWidth`, `Auto`) the narrowest heading
//!   after the join, then the earliest clause.
//! - `Elimination` takes the variable order of `cq::hypergraph::Analysis`
//!   (a perfect elimination order when the query is acyclic) and orders the
//!   atoms by the position of their earliest variable.
//!
//! `Coarse` writes one full subatom per atom occurrence (a binary join
//! plan). `Fine` writes one variable at a time (generic join): the first
//! atom of the variable binds it over the already-bound columns, every
//! other atom holding it checks, and guards and bindings follow the step
//! that binds their last variable.
//!
//! Readiness is availability only: an atom waits for the bindings that
//! supply its columns, a guard or binding for the variables it uses, and
//! guards (bindings) keep their query order among themselves as VO
//! requires. Normal form has no expression arguments; availability is
//! derived from the complete query carried by the resulting VO program.
//! [`check`] independently validates preservation and split obligations;
//! Prefix/Elimination rankings remain fixture choices. Under Written the
//! subatoms that introduce variables follow the normal body; Fine may check a
//! later clause first.

use std::collections::BTreeSet;

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

/// Whether the clause may be placed: its inputs are bound and, for a guard
/// or binding, it is the next of its kind in query order.
/// Formal rule C-Ready: Require inputs and next pending guard or let of the same kind.
fn ready(
    clauses: &[Clause<'_>],
    pending: &[usize],
    bound: &BTreeSet<String>,
    clause: usize,
) -> bool {
    clauses[clause].requires.is_subset(bound)
        && match clauses[clause].item {
            BodyItem::Positive { .. } => true,
            BodyItem::Filter(_) => {
                pending
                    .iter()
                    .find(|&&c| matches!(clauses[c].item, BodyItem::Filter(_)))
                    == Some(&clause)
            }
            BodyItem::Let(_) => {
                pending
                    .iter()
                    .find(|&&c| matches!(clauses[c].item, BodyItem::Let(_)))
                    == Some(&clause)
            }
            BodyItem::Negation(_) | BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {
                unreachable!("normal form")
            }
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
) -> impl Fn(usize, &BTreeSet<String>) -> Rank + 'a {
    move |clause, bound| {
        let facts = &clauses[clause];
        if !facts.is_atom() {
            return (false, false, 0, clause);
        }
        let shared = facts.provides.iter().filter(|v| bound.contains(*v)).count();
        let width = match ranking {
            Ranking::Connected => 0,
            Ranking::GreedyWidth | Ranking::Auto => bound.len() + facts.provides.len() - shared,
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
            (false, false, 0, clause)
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
    steps: Vec<valuation::Step>,
}

impl Fine<'_> {
    fn place_ready(&mut self) {
        while let Some(clause) = self
            .waiting
            .iter()
            .copied()
            .find(|&c| ready(self.clauses, &self.waiting, &self.bound, c))
        {
            self.waiting.retain(|&c| c != clause);
            self.bound
                .extend(self.clauses[clause].provides.iter().cloned());
            self.steps.push(match self.clauses[clause].item {
                BodyItem::Filter(filter) => {
                    valuation::Step::Guard(filter.clone(), Default::default())
                }
                BodyItem::Let(binding) => valuation::Step::Let(binding.clone(), Default::default()),
                BodyItem::Positive { .. }
                | BodyItem::Negation(_)
                | BodyItem::Aggregate(_)
                | BodyItem::Extension { .. } => {
                    unreachable!("guards and bindings wait")
                }
            });
        }
    }

    /// The clause's subatom over its bound columns.
    fn subatom(&mut self, clause: usize, atom: &cq::Atom) {
        let step = subatom(clause, atom, |v| self.bound.contains(v));
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
        steps: Vec::new(),
    };
    let mut remaining: Vec<&String> = variables.iter().collect();
    while !remaining.is_empty() {
        plan.place_ready();
        let Some(next) = remaining
            .iter()
            .position(|v| !let_bound.contains(v) || plan.bound.contains(*v))
        else {
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

/// Order the normal-form query under `options`, then check the VO contract.
/// Formal rule S-Written: Written uses the complete normal-form body sequence.
pub fn compile(source: &cq::Program, options: &Options) -> syn::Result<valuation::Program> {
    cq::normal::check(source)?;
    let query = &source.query;
    let clauses = clauses(query);
    let (sequence, variables) = match options.heuristic {
        Heuristic::Written => {
            let sequence: Vec<usize> = (0..clauses.len()).collect();
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Prefix(ranking) => {
            let sequence = schedule(query, &clauses, prefix_rank(&clauses, ranking))?;
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Elimination => elimination(source, &clauses)?,
    };
    let steps = match options.split {
        Split::Coarse => coarse(&clauses, &sequence),
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
            valuation::Step::Guard(..) | valuation::Step::Let(..) => {}
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
