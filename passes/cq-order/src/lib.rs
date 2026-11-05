//! Schedule set-valued comprehensions into valuations.
//! Rust lets, generators and guards are opaque, pure and total by premise.
//! They are placed solely by their free-variable dependencies and are never
//! recognized, split or rewritten as relational predicates.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! Ordering plans each block recursively.
//! A Rust-free block is one hyperedge over its free/interface heading at its CQ clause index.
//! A Rust block requires its free variables and provides its interface after all arms.
//! A Rust-free block that takes a finite prefix domain also requires its free variables,
//! provided by the scope's seed or its earlier clauses: a hoisted arm without a finite
//! domain sees its free columns but no seed provides them.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Union siblings are unordered; fallback retains its relational anti steps.
//! Only fallback construction proves union arms disjoint; literal values prove nothing.
//!
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_cq as cq;
use cq_lang_cq::syntax::symbol_name;
use cq_lang_cq::{BodyItem, rust};
use cq_lang_vo as valuation;
use proc_macro2::Span;
use syn::LitInt;

pub mod evidence;
pub mod known;
pub mod le;
mod restrict;

pub use known::Known;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub heuristic: Heuristic,
    pub split: Split,
}

/// Which clause comes next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Heuristic {
    /// Connected growth from the strongest static relational filter.
    Filtered,
    #[default]
    Written,
    /// Formal rule S-WrittenConnected: Retain ready written clause preference, drain I2 checks, and replace only a disconnected introducing choice by the first ready connected atom.
    WrittenConnected,
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
    atom: Option<cq::Atom>,
    rust: bool,
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
        self.atom.is_some() && !self.rust
    }
}

/// Formal rule D-Atom: Atom provides its columns and requires let-supplied columns.
/// Formal rule D-Guard: Guard requires dependencies on the written prefix.
/// Formal rule D-Bind: Binding provides pattern names and requires RHS prefix dependencies.
/// Formal rule D-For: Generator provides pattern names and requires RHS prefix dependencies.
fn clauses<'a>(body: &'a cq::CommaList<BodyItem>, context: &BTreeSet<String>) -> Vec<Clause<'a>> {
    let mut let_bound = BTreeSet::new();
    let mut written = context.clone();
    for item in body {
        match item {
            BodyItem::Let(binding) => {
                let names = rust::bindings(&binding.pattern)
                    .iter()
                    .map(symbol_name)
                    .collect::<Vec<_>>();
                let_bound.extend(names.iter().cloned());
                written.extend(names);
            }
            BodyItem::For(binding) => {
                let names = rust::bindings(&binding.pattern)
                    .iter()
                    .map(symbol_name)
                    .collect::<Vec<_>>();
                let_bound.extend(names.iter().cloned());
                written.extend(names);
            }
            BodyItem::Positive { atom } => written.extend(atom.variables().map(symbol_name)),
            BodyItem::Block(block) => {
                let facts = block.analyze(&written);
                if facts.rust {
                    let_bound.extend(facts.interface.keys().cloned());
                }
                written.extend(facts.interface.into_keys());
            }
            _ => (),
        }
    }
    let mut prefix = context.clone();
    body.iter()
        .map(|item| {
            let (provides, requires) = match item {
                BodyItem::Block(block) => {
                    let facts = block.analyze(&prefix);
                    let provides: Vec<_> = if facts.rust {
                        facts.interface.keys().cloned().collect()
                    } else {
                        facts
                            .free
                            .keys()
                            .chain(facts.interface.keys())
                            .cloned()
                            .collect()
                    };
                    // A Rust block, or a hoisted block taking a finite prefix
                    // domain, waits for its free columns.
                    let domain = !facts.rust && block.needs_domain(&facts);
                    let requires = facts
                        .free
                        .keys()
                        .filter(|name| facts.rust || domain || let_bound.contains(*name))
                        .cloned()
                        .collect();
                    (provides, requires)
                }
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
                BodyItem::Negation(negation) => (
                    Vec::new(),
                    negation
                        .atom
                        .variables()
                        .map(symbol_name)
                        .filter(|name| prefix.contains(name))
                        .collect(),
                ),
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
                BodyItem::For(binding) => (
                    rust::bindings(&binding.pattern)
                        .iter()
                        .map(symbol_name)
                        .collect(),
                    rust::free_variables(&binding.expression).dependencies(&prefix),
                ),
                BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {
                    unreachable!("normal form")
                }
            };
            let rust = item.rust_step().is_some()
                || matches!(item, BodyItem::Block(block) if block.analyze(&prefix).rust);
            let atom = match item {
                BodyItem::Positive { atom } => Some(atom.clone()),
                BodyItem::Block(block) => {
                    let facts = block.analyze(&prefix);
                    Some(cq::Atom {
                        relation: syn::Ident::new("block", Span::call_site()),
                        paren_token: Default::default(),
                        args: facts
                            .free
                            .values()
                            .chain(facts.interface.values())
                            .map(|v| -> syn::Expr { syn::parse_quote!(#v) })
                            .collect(),
                    })
                }
                _ => None,
            };
            prefix.extend(provides.iter().cloned());
            Clause {
                atom,
                rust,
                item,
                provides,
                requires,
            }
        })
        .collect()
}

/// Formal rule C-Ready: Require precisely the clause's free-variable inputs.
fn ready(clauses: &[Clause<'_>], bound: &BTreeSet<String>, clause: usize) -> bool {
    clauses[clause].requires.is_subset(bound)
}

/// Structural rank; the single final drainage producer records I2 moves.
type Rank = (bool, bool, usize, usize);

/// Choose introducing order by structural rank, then drain ready checks once.
/// Readiness and connectivity see only what the seed and earlier clauses
/// supply, not visible-but-unsupplied names in a seedless hoisted arm.
fn schedule(
    query: &cq::Query,
    clauses: &[Clause<'_>],
    seed: &BTreeSet<String>,
    rank: impl Fn(usize, &BTreeSet<String>) -> Rank,
    connect_written: bool,
) -> syn::Result<Vec<usize>> {
    let mut provided = seed.clone();
    let mut pending: Vec<usize> = (0..clauses.len()).collect();
    let mut sequence = Vec::with_capacity(clauses.len());
    while !pending.is_empty() {
        let choice = pending
            .iter()
            .copied()
            .filter(|&c| ready(clauses, &provided, c))
            .min_by_key(|&c| rank(c, &provided));
        let Some(mut choice) = choice else {
            return Err(syn::Error::new_spanned(query, "no clause is ready"));
        };
        if connect_written && clauses[choice].is_atom()
            && !clauses[choice].provides.is_empty()
            && clauses[choice].provides.iter().all(|name| !provided.contains(name))
        {
            if let Some(connected) = pending.iter().copied().find(|&clause| {
                clauses[clause].is_atom() && ready(clauses, &provided, clause)
                    && clauses[clause].provides.iter().any(|name| provided.contains(name))
            }) {
                choice = connected;
            }
        }
        pending.retain(|&c| c != choice);
        provided.extend(clauses[choice].provides.iter().cloned());
        sequence.push(choice);
    }
    Ok(sequence)
}

/// After I2 drainage, prioritize ready bindings, then connected atoms,
/// then the narrowest heading, then the earliest clause.
/// Connectivity counts only names actually shared after normalization: an
/// unsolved (protected) equality class is not a join key the lowering can
/// realize, so it does not make an occurrence look connected.
/// Formal rule S-Connected: Drain ready nonintroducing clauses before any introduction, then rank atoms by shared names and clause index.
/// Formal rule S-Greedy: After I2 drainage, GreedyWidth ranks connected atoms before union arity and written position.
fn prefix_rank<'a>(
    clauses: &'a [Clause<'a>],
    ranking: Ranking,
) -> impl Fn(usize, &BTreeSet<String>) -> Rank + 'a {
    move |clause, bound| {
        let facts = &clauses[clause];
        if !facts.is_atom() {
            let priority = usize::from(!matches!(
                facts.item,
                BodyItem::Constraint(_) | BodyItem::Negation(_)
            ));
            return (false, false, priority, clause);
        }
        let shared = facts.provides.iter().filter(|name| bound.contains(*name)).count();
        let width = match ranking {
            Ranking::Connected => 0,
            Ranking::GreedyWidth => bound.len() + facts.provides.len() - shared,
        };
        (true, shared == 0, width, clause)
    }
}

/// Formal rule S-Filtered: Drain ready checks before any introduction, then rank atoms by connectivity, local filter strength, and written position.
fn filtered_rank<'a>(
    clauses: &'a [Clause<'a>],
    filters: &'a BTreeMap<usize, u8>,
) -> impl Fn(usize, &BTreeSet<String>) -> Rank + 'a {
    move |clause, bound| {
        let facts = &clauses[clause];
        if !facts.is_atom() {
            return (
                false,
                false,
                usize::from(!matches!(
                    facts.item,
                    BodyItem::Constraint(_) | BodyItem::Negation(_)
                )),
                clause,
            );
        }
        let connected = facts.provides.iter().any(|name| bound.contains(name));
        (
            true,
            !connected,
            usize::from(filters.get(&clause).copied().unwrap_or(3)),
            clause,
        )
    }
}

/// Connected atoms precede disconnected atoms; elimination position breaks ties.
/// Formal rule S-Elimination: Rank ready atoms by connectivity, earliest elimination variable, then clause index.
fn elimination(
    source: &cq::Program,
    clauses: &[Clause<'_>],
    seed: &BTreeSet<String>,
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
    let sequence = schedule(&source.query, clauses, seed, |clause, bound| {
        if clauses[clause].is_atom() {
            let connected = clauses[clause].provides.iter().any(|name| bound.contains(name));
            (true, !connected, earliest[clause], clause)
        } else {
            (
                false,
                false,
                usize::from(!matches!(
                    clauses[clause].item,
                    BodyItem::Constraint(_) | BodyItem::Negation(_)
                )),
                clause,
            )
        }
    }, false)?;
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
            clause: LitInt::new(&clause.to_string(), Span::call_site()).into(),
            paren: Default::default(),
            columns,
            check: None,
            moved_after: None,
        },
        Default::default(),
    )
}

/// Formal rule O-Coarse: Copy guards and lets; emit one full subatom per chosen atom.
fn coarse(clauses: &[Clause<'_>], sequence: &[usize]) -> Vec<valuation::Step> {
    sequence
        .iter()
        .map(|&clause| match clauses[clause].item {
            BodyItem::Positive { .. } | BodyItem::Block(_) => subatom(
                clause,
                clauses[clause].atom.as_ref().expect("atom or block"),
                |_| true,
            ),
            BodyItem::Equality(equality) => {
                valuation::Step::Equality(equality.clone(), Default::default())
            }
            BodyItem::Constraint(c) => valuation::Step::Constraint(c.clone(), Default::default()),
            BodyItem::Negation(n) => valuation::Step::Negation(n.clone(), Default::default()),
            BodyItem::Filter(filter) => valuation::Step::Guard(filter.clone(), Default::default()),
            BodyItem::Let(binding) => valuation::Step::Let(binding.clone(), Default::default()),
            BodyItem::For(binding) => valuation::Step::For(binding.clone(), Default::default()),
            BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {
                unreachable!("normal form")
            }
        })
        .collect()
}

/// The steps of a fine plan under construction: guards and bindings are
/// placed as soon as their variables are provided.
struct Fine<'a> {
    clauses: &'a [Clause<'a>],
    bound: BTreeSet<String>,
    /// What the seed and the emitted steps provide; see [`schedule`].
    provided: BTreeSet<String>,
    /// Guards, bindings and deferred holding atoms not yet placed.
    waiting: Vec<usize>,
    steps: Vec<valuation::Step>,
}

impl Fine<'_> {
    fn place_ready(&mut self) {
        loop {
            let Some(clause) = self
                .waiting
                .iter()
                .copied()
                .filter(|&c| ready(self.clauses, &self.provided, c))
                .min_by_key(|&c| {
                    (
                        usize::from(!matches!(
                            self.clauses[c].item,
                            BodyItem::Constraint(_) | BodyItem::Negation(_)
                        )),
                        c,
                    )
                })
            else {
                break;
            };
            self.waiting.retain(|&c| c != clause);
            if self.clauses[clause].is_atom() {
                self.subatom(clause, self.clauses[clause].atom.as_ref().expect("atom heading"));
                continue;
            }
            self.bound
                .extend(self.clauses[clause].provides.iter().cloned());
            self.provided
                .extend(self.clauses[clause].provides.iter().cloned());
            self.steps.push(match self.clauses[clause].item {
                BodyItem::Equality(equality) => {
                    valuation::Step::Equality(equality.clone(), Default::default())
                }
                BodyItem::Constraint(c) => {
                    valuation::Step::Constraint(c.clone(), Default::default())
                }
                BodyItem::Negation(n) => valuation::Step::Negation(n.clone(), Default::default()),
                BodyItem::Filter(filter) => {
                    valuation::Step::Guard(filter.clone(), Default::default())
                }
                BodyItem::Let(binding) => valuation::Step::Let(binding.clone(), Default::default()),
                BodyItem::For(binding) => valuation::Step::For(binding.clone(), Default::default()),
                BodyItem::Block(_) => subatom(
                    clause,
                    self.clauses[clause].atom.as_ref().expect("block heading"),
                    |_| true,
                ),
                _ => unreachable!("equalities and Rust steps wait"),
            });
        }
    }

    /// The clause's subatom over its bound columns.
    fn subatom(&mut self, clause: usize, atom: &cq::Atom) {
        if !ready(self.clauses, &self.provided, clause) {
            if !self.waiting.contains(&clause) {
                self.waiting.push(clause);
            }
            return;
        }
        self.waiting.retain(|&pending| pending != clause);
        let step = subatom(clause, atom, |v| self.bound.contains(v));
        if let valuation::Step::Atom(subatom, _) = &step {
            self.provided
                .extend(subatom.variables().map(|(_, v)| symbol_name(v)));
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
    context: &BTreeSet<String>,
    seed: &BTreeSet<String>,
) -> syn::Result<Vec<valuation::Step>> {
    let atoms: Vec<usize> = sequence
        .iter()
        .copied()
        .filter(|&c| clauses[c].is_atom())
        .collect();
    let let_bound: BTreeSet<&String> = clauses
        .iter()
        .filter(|c| c.rust || matches!(c.item, BodyItem::Let(_)))
        .flat_map(|c| &c.provides)
        .collect();
    let mut plan = Fine {
        clauses,
        bound: context.clone(),
        provided: seed.clone(),
        waiting: (0..clauses.len())
            .filter(|&c| !clauses[c].is_atom())
            .collect(),
        steps: Vec::new(),
    };
    for &clause in &atoms {
        if clauses[clause]
            .atom
            .as_ref()
            .expect("atom heading")
            .variables()
            .next()
            .is_none()
        {
            plan.subatom(clause, clauses[clause].atom.as_ref().expect("atom heading"));
        }
    }
    let mut remaining: Vec<&String> = variables.iter().collect();
    while !remaining.is_empty() {
        plan.place_ready();
        let Some(next) = remaining.iter().enumerate().filter(|(_, v)| {
            plan.bound.contains(**v)
                || !let_bound.contains(*v)
                    && atoms.iter().any(|&clause| {
                        clauses[clause].provides.contains(*v)
                            && ready(clauses, &plan.provided, clause)
                    })
        }).min_by_key(|(position, v)| {
            let connected = atoms.iter().any(|&clause| {
                clauses[clause].provides.contains(*v)
                    && ready(clauses, &plan.provided, clause)
                    && clauses[clause].provides.iter().any(|name| plan.provided.contains(name))
            });
            (!connected, *position)
        }).map(|(position, _)| position) else {
            return Err(syn::Error::new_spanned(
                query,
                "no variable can be bound next",
            ));
        };
        let variable = remaining.remove(next);
        let holding: Vec<_> = atoms
            .iter()
            .copied()
            .filter(|&c| clauses[c].provides.contains(variable))
            .collect();
        let binder = if !plan.bound.contains(variable) {
            let Some(binder) = holding
                .iter()
                .copied()
                .filter(|&c| ready(clauses, &plan.provided, c))
                .min_by_key(|&c| !clauses[c].provides.iter().any(|name| plan.provided.contains(name)))
            else {
                return Err(syn::Error::new_spanned(
                    query,
                    format!("`{variable}` is not a variable of the query"),
                ));
            };
            plan.bound.insert(variable.clone());
            plan.subatom(binder, clauses[binder].atom.as_ref().expect("atom heading"));
            plan.place_ready();
            Some(binder)
        } else {
            None
        };
        for check in holding.into_iter().filter(|&c| Some(c) != binder) {
            plan.subatom(check, clauses[check].atom.as_ref().expect("atom heading"));
            plan.place_ready();
        }
    }
    plan.place_ready();
    Ok(plan.steps)
}

/// Split Written atoms in place; the assembled schedule subsequently drains checks.
fn written_fine(
    clauses: &[Clause<'_>],
    sequence: &[usize],
    context: &BTreeSet<String>,
) -> Vec<valuation::Step> {
    let mut steps = Vec::new();
    let mut bound = context.clone();
    for &clause in sequence {
        let facts = &clauses[clause];
        if let Some(atom) = facts.atom.as_ref().filter(|_| !facts.rust) {
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

/// Order the normal-form query under `options`, record its occurrence
/// restrictions (O-Restrict), then check the VO contract.
/// Formal rule S-Written: Written retains introducing body order, drains ready checks, and rejects I1/I3 violations.
pub fn compile(source: &cq::Program, options: &Options) -> syn::Result<valuation::Program> {
    let mut program = compile_unrestricted(source, options)?;
    restrict(&mut program)?;
    Ok(program)
}

/// The ordered schedule with every predicate at its prefix position. A
/// planner that reschedules prefix steps does so here, then calls [`restrict`].
pub fn compile_unrestricted(
    source: &cq::Program,
    options: &Options,
) -> syn::Result<valuation::Program> {
    cq::normal::check(source)?;
    let mut program = assemble(source, options)?;
    drain_ready_checks(&mut program)?;
    valuation::contract::check(&program)?;
    Ok(program)
}

/// Assemble an intermediate schedule before I2 drainage and VO-Order checking.
fn assemble(source: &cq::Program, options: &Options) -> syn::Result<valuation::Program> {
    let steps = compile_body(source, options, &BTreeSet::new(), &BTreeSet::new(), None)?;
    Ok(valuation::Program {
        cq: source.clone(),
        valuation: valuation::Valuation {
            valuation_token: Default::default(),
            evidence: None,
            ordering: source.ordering.clone(),
            brace: Default::default(),
            steps,
        },
    })
}

/// Drain ready nonintroducing steps before every introduction, preserving
/// introducing choices and exact projected subatoms. This is producer logic;
/// the independent VO-Order checker validates the resulting schedule.
pub fn drain_ready_checks(program: &mut valuation::Program) -> syn::Result<()> {
    fn scope(steps: &mut Vec<valuation::Step>, analysis: &valuation::Analysis) {
        for step in steps.iter_mut() {
            if let valuation::Step::Block(block, _) = step {
                for (branch, facts) in block.branches.iter_mut()
                    .zip(&analysis.branches[&block.source.clause()])
                {
                    scope(&mut branch.steps, facts);
                }
            }
        }
        // Formal rule O-EmptyCheck: omit a zero-column preliminary read
        // exactly when the same occurrence later introduces columns.
        let mut order: Vec<_> = (0..steps.len()).filter(|&index| {
            !matches!(&steps[index], valuation::Step::Atom(atom, _)
                if atom.variables().next().is_none()
                    && steps.iter().enumerate().skip(index + 1).any(|(later, step)|
                        matches!(step, valuation::Step::Atom(next, _)
                            if next.clause == atom.clause
                                && !analysis.introduces[later].is_empty())))
        }).collect();
        let mut index = 0;
        steps.retain(|_| {
            let keep = order.binary_search(&index).is_ok();
            index += 1;
            keep
        });
        let mut bound = analysis.provided_before.first().cloned().unwrap_or_default();
        for position in 0..steps.len() {
            if !analysis.order[order[position]].preprocessing {
                if let Some(next) = (position..steps.len()).find(|&next| {
                    let facts = &analysis.order[order[next]];
                    !facts.preprocessing && facts.ready(&bound) && !facts.introduces(&bound)
                }) {
                    // I2 moves an existing check, not its provider phase.
                    // Newly adjacent introductions must not turn it into an
                    // alternate supplier during Physical selection.
                    let crossed = (position..next).filter_map(|crossed| {
                        if analysis.introduces[order[crossed]].is_empty() {
                            return None;
                        }
                        match &steps[crossed] {
                            valuation::Step::Atom(atom, _) => Some(atom.clause.clone()),
                            _ => None,
                        }
                    }).collect::<Vec<_>>();
                    let moved_after = (!crossed.is_empty()).then(|| cq::syntax::algebra::MovedAfter {
                        after_token: Default::default(),
                        bracket: Default::default(),
                        occurrences: crossed.into_iter().collect(),
                    });
                    // Removing a separating check must not make the following
                    // standalone check an alternate candidate provider.
                    if next != position && next + 1 < steps.len()
                        && analysis.introduces[order[next + 1]].is_empty()
                    {
                        if let valuation::Step::Atom(following, _) = &mut steps[next + 1] {
                            following.check = Some(Default::default());
                        }
                    }
                    if next != position || matches!(&steps[next],
                        valuation::Step::Atom(atom, _) if atom.variables().next().is_none())
                    {
                        if let valuation::Step::Atom(atom, _) = &mut steps[next] {
                            atom.check = Some(Default::default());
                            if moved_after.is_some() {
                                atom.moved_after = moved_after;
                            }
                        }
                    }
                    steps[position..=next].rotate_right(1);
                    order[position..=next].rotate_right(1);
                }
            }
            bound.extend(analysis.order[order[position]].provides.iter().cloned());
        }
    }
    let analysis = valuation::order::analyze(program)?;
    scope(&mut program.valuation.steps, &analysis);
    Ok(())
}

/// Record the occurrence restrictions (O-Restrict) of an unrestricted
/// schedule, then check the VO contract.
pub fn restrict(program: &mut valuation::Program) -> syn::Result<()> {
    restrict::restrict(program);
    valuation::contract::check(program).map(drop)
}

/// Record Written/Coarse with per-prefix block arms and earliest-ready checks.
/// It records no restriction: every predicate stays a prefix step.
/// Formal rule O-WrittenBlocks: Written-reference construction records Correlated on every block recursively.
pub fn compile_written(source: &cq::Program) -> syn::Result<valuation::Program> {
    fn correlate(steps: &mut [valuation::Step]) {
        for step in steps {
            if let valuation::Step::Block(block, _) = step {
                block.placement = valuation::blocks::Placement::Correlated;
                for branch in &mut block.branches {
                    correlate(&mut branch.steps);
                }
            }
        }
    }
    let mut program = compile_unrestricted(source, &Options::default())?;
    correlate(&mut program.valuation.steps);
    drain_ready_checks(&mut program)?;
    valuation::contract::check(&program)?;
    Ok(program)
}

/// A top-level positive atom selected by relation, optionally qualified by its
/// zero-based normal-body clause index (the same occurrence used by `R@0` in RA).
/// `block@i` selects a whole top-level block without reordering its branches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderAtom {
    pub relation: syn::Ident,
    pub occurrence: Option<usize>,
    /// Explicit subatom columns; absent means the complete occurrence.
    pub columns: Option<Vec<syn::Ident>>,
}

impl syn::parse::Parse for OrderAtom {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let relation = input.parse()?;
        let occurrence = if input.peek(syn::Token![@]) {
            input.parse::<syn::Token![@]>()?;
            Some(input.parse::<LitInt>()?.base10_parse()?)
        } else {
            None
        };
        let columns = if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            Some(
                content
                    .parse_terminated(|input| input.parse::<syn::Ident>(), syn::Token![,])?
                    .into_iter()
                    .collect(),
            )
        } else {
            None
        };
        Ok(Self {
            relation,
            occurrence,
            columns,
        })
    }
}

impl OrderAtom {
    /// Resolve the selector in an already normalized top-level body.
    pub fn resolve(&self, source: &cq::Program) -> syn::Result<usize> {
        self.resolve_in(&source.query.body)
    }

    /// Resolve the selector against one body's clause numbering. Callers that
    /// order bodies outside a CQ program (such as Datalog rules) share the
    /// same `Relation` / `Relation@clause` / `block@clause` meaning.
    pub fn resolve_in(&self, body: &cq::CommaList<BodyItem>) -> syn::Result<usize> {
        let relation = &self.relation;
        if let Some(index) = self.occurrence {
            let matches = match body.get(index) {
                Some(BodyItem::Positive { atom }) => atom.relation == *relation,
                Some(BodyItem::Block(_)) => relation == "block",
                _ => false,
            };
            if !matches {
                return Err(syn::Error::new_spanned(
                    relation,
                    if relation == "block" {
                        format!("advice selector block@{index} does not name a top-level block")
                    } else {
                        format!(
                            "advice selector {relation}@{index} does not name that top-level positive atom"
                        )
                    },
                ));
            }
            Ok(index)
        } else {
            let mut occurrences = body.iter().enumerate().filter_map(|(i, item)| {
                matches!(item, BodyItem::Positive { atom } if atom.relation == *relation)
                    .then_some(i)
            });
            let first = occurrences.next();
            if first.is_none() || occurrences.next().is_some() {
                return Err(syn::Error::new_spanned(
                    relation,
                    "advice needs one top-level occurrence; qualify repeated relations as Relation@clause",
                ));
            }
            Ok(first.unwrap())
        }
    }

    /// Reduction endpoints are positive atoms, not whole block frontiers.
    pub fn positive_clause(&self, source: &cq::Program) -> syn::Result<usize> {
        if self.columns.is_some() {
            return Err(syn::Error::new_spanned(
                &self.relation,
                "reduction endpoints select whole occurrences, not subatom columns",
            ));
        }
        let clause = self.resolve(source)?;
        if !matches!(source.query.body[clause], BodyItem::Positive { .. }) {
            return Err(syn::Error::new_spanned(
                &self.relation,
                "reduction advice requires a top-level positive atom, not a block",
            ));
        }
        Ok(clause)
    }
}

/// Author a top-level occurrence or subatom order without rewriting the query.
/// Unqualified relations must occur exactly once; `R@i` selects the positive
/// atom named R at normal-body clause i, including non-atom clauses in the
/// numbering. `block@i` selects a block as a unit. Unnamed introducing atoms
/// retain written priority after named atoms/blocks. Ready nonintroducing
/// checks drain first; illegal readiness or connectivity choices are rejected.
/// `R@i(cols)` retains exactly those columns: a bound-only partial read is
/// a lookup, otherwise a partial read may introduce one variable (Fine).
/// Named partial occurrences must be explicitly completed by a full read.
pub fn compile_explicit(
    source: &cq::Program,
    relations: &[OrderAtom],
) -> syn::Result<valuation::Program> {
    cq::normal::check(source)?;
    compile_steps(source, relations).map_err(|error| {
        syn::Error::new_spanned(
            &source.name,
            format!("query `{}`: {error}", source.name),
        )
    })
}

/// Formal rule O-StepAdvice: Preserve authored introductions, drain ready checks, and check VO-Order plus selected CK-Le phases.
fn compile_steps(source: &cq::Program, selectors: &[OrderAtom]) -> syn::Result<valuation::Program> {
    let context = BTreeSet::new();
    let clauses = clauses(&source.query.body, &context);
    let mut program = assemble(source, &Options::default())?;
    let mut ordinary: Vec<_> = program.valuation.steps.into_iter().map(Some).collect();
    let mut pending = Vec::new();
    let mut named = BTreeSet::new();
    for selector in selectors {
        let clause = selector.resolve(source)?;
        named.insert(clause);
        let mut step = ordinary[clause].as_ref().expect("source clause").clone();
        if let Some(columns) = &selector.columns {
            let valuation::Step::Atom(atom, _) = &mut step else {
                return Err(syn::Error::new_spanned(
                    &selector.relation,
                    "subatom advice requires a positive atom, not a block",
                ));
            };
            let mut selected = BTreeSet::new();
            for column in columns {
                let name = symbol_name(column);
                if !atom.variables().any(|(_, v)| symbol_name(v) == name) {
                    return Err(syn::Error::new_spanned(
                        column,
                        "order subatom column does not belong to this occurrence",
                    ));
                }
                if !selected.insert(name) {
                    return Err(syn::Error::new_spanned(
                        column,
                        "duplicate order subatom column",
                    ));
                }
            }
            atom.columns = atom
                .columns
                .iter()
                .map(|column| match column {
                    valuation::Column::Variable(v) if selected.contains(&symbol_name(v)) => {
                        column.clone()
                    }
                    _ => valuation::Column::Dropped(Default::default()),
                })
                .collect();
        }
        pending.push((clause, step, selector.columns.is_some()));
    }
    // Partial occurrences must be completed explicitly. Only entirely unnamed
    // occurrences retain the old written-priority completion.
    for (clause, facts) in clauses.iter().enumerate() {
        if facts.atom.is_some() && !named.contains(&clause) {
            pending.push((clause, ordinary[clause].take().unwrap(), false));
        }
    }
    let mut waiting: BTreeSet<_> = clauses
        .iter()
        .enumerate()
        .filter_map(|(i, facts)| facts.atom.is_none().then_some(i))
        .collect();
    let mut bound = BTreeSet::new();
    let mut steps = Vec::new();
    let mut lookups = BTreeSet::new();
    let mut fine = BTreeSet::new();
    let mut completed = BTreeSet::new();
    let partial_advice = selectors.iter().any(|selector| selector.columns.is_some());
    for (clause, step, explicit_columns) in pending {
        let demand = partial_advice.then(|| match &step {
            valuation::Step::Atom(atom, _) => {
                atom.variables().map(|(_, v)| symbol_name(v)).collect::<BTreeSet<_>>()
            }
            _ => clauses[clause].requires.clone(),
        });
        place_ready_advice(
            &clauses,
            &mut ordinary,
            &mut waiting,
            &mut bound,
            &mut steps,
            demand.as_ref(),
        );
        let dependencies_ready = match &step {
            valuation::Step::Atom(atom, _) => clauses[clause]
                .requires
                .iter()
                .filter(|name| atom.variables().any(|(_, v)| symbol_name(v) == **name))
                .all(|name| bound.contains(name)),
            _ => ready(&clauses, &bound, clause),
        };
        if !dependencies_ready {
            return Err(syn::Error::new_spanned(
                &source.query,
                "order advice conflicts with clause dependencies",
            ));
        }
        match &step {
            valuation::Step::Atom(atom, _) => {
                let introduced: Vec<_> = atom
                    .variables()
                    .map(|(_, v)| symbol_name(v))
                    .filter(|name| !bound.contains(name))
                    .collect();
                if completed.contains(&clause) {
                    return Err(syn::Error::new_spanned(
                        atom,
                        "order advice places a lookup or another expansion after expansion",
                    ));
                }
                let partial = atom.variables().count() < atom.columns.len();
                if explicit_columns && partial && introduced.is_empty() && !fine.contains(&clause) {
                    lookups.insert(clause);
                } else if explicit_columns && partial {
                    if introduced.len() > 1 {
                        return Err(syn::Error::new_spanned(
                            atom,
                            "fine order subatoms may introduce at most one new variable; lookup columns must already be bound",
                        ));
                    }
                    fine.insert(clause);
                }
                if !partial {
                    completed.insert(clause);
                }
                bound.extend(introduced);
            }
            valuation::Step::Block(_, _) => {
                if !completed.insert(clause) {
                    return Err(syn::Error::new_spanned(
                        &source.query,
                        "duplicate block occurrence in order advice",
                    ));
                }
                bound.extend(clauses[clause].provides.iter().cloned());
            }
            _ => unreachable!("advice selects positive atoms or blocks"),
        }
        steps.push(step);
    }
    place_ready_advice(
        &clauses,
        &mut ordinary,
        &mut waiting,
        &mut bound,
        &mut steps,
        None,
    );
    if !waiting.is_empty() {
        return Err(syn::Error::new_spanned(
            &source.query,
            "order advice leaves clause dependencies unbound",
        ));
    }
    program.valuation.steps = steps;
    drain_ready_checks(&mut program)?;
    restrict::restrict(&mut program);
    le::check_advice(source, &program, &lookups)?;
    Ok(program)
}

/// Place ready unadvised clauses before the next advised step. Filters
/// (constraints, negations, guards) go as early as possible. A Rust binding is
/// pure and total, so it may sit anywhere after its free variables and before
/// its first use (WF-VO): it waits until the next advised step demands one of
/// its bindings, or until the end. This keeps advised steps adjacent, e.g. an
/// intersection site, when a head copy would otherwise separate them.
fn place_ready_advice(
    clauses: &[Clause<'_>],
    ordinary: &mut [Option<valuation::Step>],
    waiting: &mut BTreeSet<usize>,
    bound: &mut BTreeSet<String>,
    steps: &mut Vec<valuation::Step>,
    demand: Option<&BTreeSet<String>>,
) {
    loop {
        let next = waiting
            .iter()
            .copied()
            .filter(|&clause| ready(clauses, bound, clause))
            .filter(|&clause| {
                let provides = &clauses[clause].provides;
                demand.is_none_or(|demand| {
                    provides.is_empty() || provides.iter().any(|name| demand.contains(name))
                })
            })
            .min_by_key(|&clause| {
                (
                    usize::from(!matches!(
                        clauses[clause].item,
                        BodyItem::Constraint(_) | BodyItem::Negation(_)
                    )),
                    clause,
                )
            });
        let Some(clause) = next else { break };
        waiting.remove(&clause);
        bound.extend(clauses[clause].provides.iter().cloned());
        steps.push(ordinary[clause].take().expect("unplaced source clause"));
    }
}

/// Formal rule O-BlockPlacement: Ordinary ordering hoists Rust-free blocks and correlates blocks containing Rust steps.
/// `context` is the scope's visible lexical context; `seed` the columns its
/// seed provides (all of `context`, except in a hoisted arm without a finite
/// domain, which starts from none).
fn compile_body(
    source: &cq::Program,
    options: &Options,
    context: &BTreeSet<String>,
    seed: &BTreeSet<String>,
    parent: Option<(valuation::Occurrence, usize)>,
) -> syn::Result<Vec<valuation::Step>> {
    let query = &source.query;
    let clauses = clauses(&query.body, context);
    let hypergraph = cq::hypergraph::Analysis::of_body(&query.body, context);
    let (sequence, mut variables) = match options.heuristic {
        Heuristic::Filtered => {
            let local = cq::hypergraph::local_predicates(&query.body);
            let filters = cq::hypergraph::filter_ranks(
                &query.body,
                &hypergraph,
                local
                    .iter()
                    .flat_map(cq::hypergraph::LocalPredicate::restrictions),
            );
            let sequence = schedule(
                query,
                &clauses,
                seed,
                filtered_rank(&clauses, &filters),
                false,
            )?;
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Written => {
            let sequence: Vec<usize> = (0..clauses.len()).collect();
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::WrittenConnected => {
            let sequence = schedule(
                query, &clauses, seed, |clause, _| (false, false, 0, clause), true,
            )?;
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Prefix(ranking) => {
            let sequence = schedule(
                query,
                &clauses,
                seed,
                prefix_rank(&clauses, ranking),
                false,
            )?;
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Elimination => elimination(source, &clauses, seed)?,
    };
    // Elimination removes leaves first. Enumerating in that direction joins
    // their independent projections before any connecting variable is bound.
    // Reverse it for Fine; Coarse retains the earliest-variable atom ranking.
    if options.heuristic == Heuristic::Elimination && options.split == Split::Fine {
        variables.reverse();
    }
    let written_preference = matches!(options.heuristic, Heuristic::Written | Heuristic::WrittenConnected);
    if options.split == Split::Fine && !written_preference {
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
        Split::Fine if written_preference => written_fine(&clauses, &sequence, context),
        Split::Fine => fine(query, &clauses, &sequence, &variables, context, seed)?,
    };
    let mut prefix = context.clone();
    let mut written_contexts = Vec::new();
    for facts in &clauses {
        written_contexts.push(prefix.clone());
        prefix.extend(facts.provides.iter().cloned());
    }
    let mut result = Vec::new();
    for step in steps {
        let valuation::Step::Atom(mut atom, semi) = step else {
            result.push(step);
            continue;
        };
        let clause = atom.clause();
        atom.clause = match &parent {
            Some((path, branch)) => path.child(*branch, clause),
            None => valuation::Occurrence::new(clause),
        };
        if let BodyItem::Block(block) = clauses[clause].item {
            let facts = block.analyze(&written_contexts[clause]);
            let context = facts.free.keys().cloned().collect();
            let seed = if facts.rust || block.needs_domain(&facts) {
                &context
            } else {
                &BTreeSet::new()
            };
            let mut branches = Vec::new();
            for (j, branch) in block.branches.iter().enumerate() {
                let mut branch_source = source.clone();
                branch_source.query.body = branch.body.clone();
                branches.push(valuation::blocks::Branch {
                    brace: branch.brace,
                    steps: compile_body(
                        &branch_source,
                        options,
                        &context,
                        seed,
                        Some((atom.clause.clone(), j)),
                    )?,
                });
            }
            result.push(valuation::Step::Block(
                valuation::blocks::Block {
                    source: atom,
                    placement: if facts.rust {
                        valuation::blocks::Placement::Correlated
                    } else {
                        valuation::blocks::Placement::Hoisted
                    },
                    kind: block.kind,
                    branches,
                },
                semi,
            ));
        } else {
            result.push(valuation::Step::Atom(atom, semi));
        }
    }
    Ok(result)
}

/// Check normal form, unchanged CQ, split granularity and explicit order.
/// Structural rankings are producer choices, not replayed checker obligations.
/// Formal rule CK-Order: Validate unchanged CQ, split, coverage, VO-Order, and Written introducing order.
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
    check_steps(
        source,
        &source.query.body,
        &target.valuation.steps,
        &analysis,
        options,
    )
}

fn check_steps(
    source: &cq::Program,
    body: &cq::CommaList<BodyItem>,
    steps: &[valuation::Step],
    analysis: &valuation::Analysis,
    options: &Options,
) -> syn::Result<()> {
    let mut seen = BTreeSet::new();
    let mut previous = None;
    for (step, introduced) in steps.iter().zip(&analysis.introduces) {
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
            valuation::Step::Atom(a, _)
            | valuation::Step::Block(valuation::blocks::Block { source: a, .. }, _) => {
                if let valuation::Step::Block(block, _) = step {
                    let BodyItem::Block(written) = &body[a.clause()] else {
                        unreachable!("VO source block");
                    };
                    for (j, arm) in block.branches.iter().enumerate() {
                        check_steps(
                            source,
                            &written.branches[j].body,
                            &arm.steps,
                            &analysis.branches[&a.clause()][j],
                            options,
                        )?;
                    }
                }
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
                // Written constrains introductions; I2 may move checks earlier.
                if options.heuristic == Heuristic::Written && !introduced.is_empty() {
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
            | valuation::Step::Negation(..)
            | valuation::Step::Guard(..)
            | valuation::Step::Let(..)
            | valuation::Step::For(..)
            | valuation::Step::Restrict(..) => {}
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
    check_restricted(body, steps)
}

/// Formal rule CK-Restrict: A written predicate keeps a prefix step for every written instance of its payload or for none; WF-Restrict places one without steps through its restrictions.
fn check_restricted(body: &cq::CommaList<BodyItem>, steps: &[valuation::Step]) -> syn::Result<()> {
    for item in body {
        let predicate = match item {
            BodyItem::Equality(equality) => cq::Constraint::from_equality(equality),
            BodyItem::Constraint(constraint) => constraint.clone(),
            _ => continue,
        };
        let written = body
            .iter()
            .filter(|other| valuation::restrict::states(other, &predicate))
            .count();
        let kept = steps
            .iter()
            .filter(|step| valuation::restrict::step_states(step, &predicate))
            .count();
        if kept != 0 && kept != written {
            return Err(syn::Error::new_spanned(
                item,
                "a restricted predicate drops the prefix steps of all its written instances or none",
            ));
        }
    }
    Ok(())
}
