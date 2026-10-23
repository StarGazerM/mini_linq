//! Schedule `T X = bottom + Set X` comprehensions into valuations.
//! Rust lets, finite set-valued generators and guards are opaque partial functions and keep written relative
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

//! Ordering plans each block recursively.
//! A Rust-free block is one hyperedge over its free/interface heading at its CQ clause index.
//! A Rust block is pinned by its branch Before requirements and provides its interface only after all arms.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_cq as cq;
use cq_lang_cq::syntax::symbol_name;
use cq_lang_cq::{BodyItem, rust};
use cq_lang_vo as valuation;
use proc_macro2::Span;
use syn::LitInt;

mod branches;

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
                    let requires = facts
                        .free
                        .keys()
                        .filter(|name| facts.rust || let_bound.contains(*name))
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

/// Formal rule C-Ready: Require inputs, next Rust chain step, and every written relational predecessor.
fn ready(
    clauses: &[Clause<'_>],
    pending: &[usize],
    bound: &BTreeSet<String>,
    clause: usize,
) -> bool {
    clauses[clause].requires.is_subset(bound)
        && (!clauses[clause].rust
            || (pending.iter().copied().filter(|&c| clauses[c].rust).min() == Some(clause)
                && !pending.iter().any(|&c| {
                    c < clause
                        && (clauses[c].atom.is_some()
                            || matches!(
                                clauses[c].item,
                                BodyItem::Equality(_)
                                    | BodyItem::Constraint(_)
                                    | BodyItem::Negation(_)
                            ))
                })))
}

/// A rank orders the ready clauses; the least is placed next. Guards and
/// bindings rank `(false, ..)`, so they go before any atom.
type Rank = (bool, bool, usize, usize);

/// Place the clauses one at a time, each the least ready clause by `rank`.
fn schedule(
    query: &cq::Query,
    clauses: &[Clause<'_>],
    context: &BTreeSet<String>,
    rank: impl Fn(usize, &BTreeSet<String>) -> Rank,
) -> syn::Result<Vec<usize>> {
    let mut bound = context.clone();
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
            let priority = usize::from(!matches!(
                facts.item,
                BodyItem::Constraint(_) | BodyItem::Negation(_)
            ));
            return (false, false, priority, clause);
        }
        let canonical = |name: &String| canonical_names.get(name).unwrap_or(name).clone();
        let supplied: BTreeSet<_> = facts.provides.iter().map(canonical).collect();
        let available: BTreeSet<_> = bound.iter().map(canonical).collect();
        let shared = supplied.intersection(&available).count();
        let width = match ranking {
            Ranking::Connected => 0,
            Ranking::GreedyWidth => available.union(&supplied).count(),
        };
        (true, shared == 0, width, clause)
    }
}

/// Formal rule S-Filtered: Rank ready occurrences by connectivity, local filter strength, then written position.
fn filtered_rank<'a>(
    clauses: &'a [Clause<'a>],
    graph: &'a cq::hypergraph::Analysis,
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
        let connected = facts
            .provides
            .iter()
            .any(|name| bound.iter().any(|b| graph.equivalent(name, b)));
        (
            true,
            !connected,
            usize::from(filters.get(&clause).copied().unwrap_or(3)),
            clause,
        )
    }
}

/// Atoms in the order of their earliest variable in the elimination order.
/// Formal rule S-Elimination: Rank ready atoms by earliest elimination variable then clause index.
fn elimination(
    source: &cq::Program,
    clauses: &[Clause<'_>],
    context: &BTreeSet<String>,
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
    let sequence = schedule(&source.query, clauses, context, |clause, _| {
        if clauses[clause].is_atom() {
            (true, false, earliest[clause], clause)
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
            clause: LitInt::new(&clause.to_string(), Span::call_site()).into(),
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
            self.bound
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
    context: &BTreeSet<String>,
) -> syn::Result<Vec<valuation::Step>> {
    let atoms: Vec<usize> = sequence
        .iter()
        .copied()
        .filter(|&c| clauses[c].is_atom())
        .collect();
    let let_bound: BTreeSet<&String> = clauses
        .iter()
        .filter(|c| c.rust)
        .flat_map(|c| &c.provides)
        .collect();
    let mut plan = Fine {
        clauses,
        bound: context.clone(),
        waiting: (0..clauses.len())
            .filter(|&c| !clauses[c].is_atom())
            .collect(),
        incomplete_atoms: atoms.iter().copied().collect(),
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
            plan.subatom(binder, clauses[binder].atom.as_ref().expect("atom heading"));
            plan.place_ready();
        }
        for check in holding {
            plan.subatom(check, clauses[check].atom.as_ref().expect("atom heading"));
        }
    }
    plan.place_ready();
    Ok(plan.steps)
}

/// Written splits each atom in place; it performs no cross-clause reordering.
fn written_fine(clauses: &[Clause<'_>], context: &BTreeSet<String>) -> Vec<valuation::Step> {
    let mut steps = Vec::new();
    let mut bound = context.clone();
    for (clause, facts) in clauses.iter().enumerate() {
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

/// Order the normal-form query under `options`, then check the VO contract.
/// Formal rule S-Written: Written uses the complete normal-form body sequence.
pub fn compile(source: &cq::Program, options: &Options) -> syn::Result<valuation::Program> {
    cq::normal::check(source)?;
    let steps = compile_body(source, options, &BTreeSet::new(), None)?;
    let program = valuation::Program {
        cq: source.clone(),
        valuation: valuation::Valuation {
            valuation_token: Default::default(),
            join_branches: if options.heuristic == Heuristic::Filtered
                && options.split == Split::Coarse
            {
                branches::choose(source)
            } else {
                None
            },
            brace: Default::default(),
            steps,
        },
    };
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
        Ok(Self { relation, occurrence })
    }
}

/// Author a coarse top-level relation order without rewriting the query.
/// Unqualified relations must occur exactly once; `R@i` selects the positive
/// atom named R at normal-body clause i, including non-atom clauses in the
/// numbering. `block@i` selects a block as a unit. Unnamed clauses retain
/// written priority after named atoms/blocks. Opaque Rust still obeys readiness
/// and Before; nested blocks keep their internal order.
pub fn compile_explicit(
    source: &cq::Program,
    relations: &[OrderAtom],
) -> syn::Result<valuation::Program> {
    cq::normal::check(source)?;
    let context = BTreeSet::new();
    let clauses = clauses(&source.query.body, &context);
    let mut priorities = BTreeMap::new();
    for (priority, selector) in relations.iter().enumerate() {
        let relation = &selector.relation;
        let occurrence = if let Some(index) = selector.occurrence {
            let matches = match source.query.body.get(index) {
                Some(BodyItem::Positive { atom }) => atom.relation == *relation,
                Some(BodyItem::Block(_)) => relation == "block",
                _ => false,
            };
            if !matches {
                return Err(syn::Error::new_spanned(
                    relation,
                    if relation == "block" {
                        format!("order advice block@{index} does not name a top-level block")
                    } else {
                        format!("order advice {relation}@{index} does not name that top-level positive atom")
                    },
                ));
            }
            index
        } else {
            let mut occurrences = source.query.body.iter().enumerate().filter_map(|(i, item)| {
                matches!(item, BodyItem::Positive { atom } if atom.relation == *relation)
                    .then_some(i)
            });
            let first = occurrences.next();
            if first.is_none() || occurrences.next().is_some() {
                return Err(syn::Error::new_spanned(
                    relation,
                    "order advice needs one top-level occurrence; qualify repeated relations as Relation@clause",
                ));
            }
            first.unwrap()
        };
        if priorities.insert(occurrence, priority).is_some() {
            return Err(syn::Error::new_spanned(
                relation,
                "duplicate atom occurrence in order advice",
            ));
        }
    }
    let sequence = schedule(&source.query, &clauses, &context, |clause, _| {
        (
            clauses[clause].is_atom(),
            false,
            priorities
                .get(&clause)
                .copied()
                .unwrap_or(relations.len() + clause),
            clause,
        )
    })?;
    if !sequence
        .iter()
        .filter_map(|clause| priorities.get(clause).copied())
        .eq(0..relations.len())
    {
        return Err(syn::Error::new_spanned(
            &source.query,
            "order advice conflicts with clause dependencies",
        ));
    }
    let mut program = compile(source, &Options::default())?;
    // Coarse emits exactly one step per source clause, including block steps.
    let mut steps: Vec<_> = program.valuation.steps.into_iter().map(Some).collect();
    program.valuation.steps = sequence
        .into_iter()
        .map(|i| steps[i].take().unwrap())
        .collect();
    check(
        source,
        &program,
        &Options {
            heuristic: Heuristic::Prefix(Ranking::Connected),
            split: Split::Coarse,
        },
    )?;
    Ok(program)
}

fn compile_body(
    source: &cq::Program,
    options: &Options,
    context: &BTreeSet<String>,
    parent: Option<(valuation::Occurrence, usize)>,
) -> syn::Result<Vec<valuation::Step>> {
    let query = &source.query;
    let clauses = clauses(&query.body, context);
    let hypergraph = cq::hypergraph::Analysis::of_body(&query.body, context);
    let (sequence, mut variables) = match options.heuristic {
        Heuristic::Filtered => {
            let mut filters = cq::hypergraph::filter_ranks(&query.body, &hypergraph);
            if parent.is_none() && options.split == Split::Coarse {
                if let Some(groups) = branches::choose(source) {
                    for branch in groups.branches {
                        let members = branch.clauses();
                        let rank = members.iter().map(|c| filters[c]).min().unwrap();
                        for clause in members {
                            filters.insert(clause, rank);
                        }
                    }
                }
            }
            let sequence = schedule(
                query,
                &clauses,
                context,
                filtered_rank(&clauses, &hypergraph, &filters),
            )?;
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Written => {
            let sequence: Vec<usize> = (0..clauses.len()).collect();
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Prefix(ranking) => {
            let sequence = schedule(
                query,
                &clauses,
                context,
                prefix_rank(&clauses, ranking, &hypergraph.canonical_names),
            )?;
            let variables = binding_order(&clauses, &sequence);
            (sequence, variables)
        }
        Heuristic::Elimination => elimination(source, &clauses, context)?,
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
        Split::Fine if options.heuristic == Heuristic::Written => written_fine(&clauses, context),
        Split::Fine => fine(query, &clauses, &sequence, &variables, context)?,
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
            let context = if facts.rust {
                written_contexts[clause].clone()
            } else {
                facts.free.keys().cloned().collect()
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
                        Some((atom.clause.clone(), j)),
                    )?,
                });
            }
            result.push(valuation::Step::Block(
                valuation::blocks::Block {
                    source: atom,
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
/// Prefix and elimination rankings are expansion choices, not obligations.
/// Formal rule CK-Order: Validate unchanged CQ, split, coverage, Rust Before, Written order, Filtered coarse greedy choices and static branch groups.
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
    let expected_branches =
        if options.heuristic == Heuristic::Filtered && options.split == Split::Coarse {
            branches::choose(source)
        } else {
            None
        };
    if target.valuation.join_branches != expected_branches {
        return Err(syn::Error::new_spanned(
            &target.valuation,
            "join branches must match the static dimension-ear policy",
        ));
    }
    check_steps(
        &source.query.body,
        &target.valuation.steps,
        &analysis,
        options,
        target.valuation.join_branches.as_ref(),
    )
}

fn check_steps(
    body: &cq::CommaList<BodyItem>,
    steps: &[valuation::Step],
    analysis: &valuation::Analysis,
    options: &Options,
    join_branches: Option<&valuation::JoinBranches>,
) -> syn::Result<()> {
    if options.heuristic == Heuristic::Written
        || options.heuristic == Heuristic::Filtered && options.split == Split::Coarse
    {
        let mut written = Vec::new();
        let mut previous_atom = None;
        for step in steps {
            match step {
                valuation::Step::Atom(atom, _)
                | valuation::Step::Block(valuation::blocks::Block { source: atom, .. }, _) => {
                    if previous_atom != Some(atom.clause()) {
                        written.push(atom.clause());
                    }
                    previous_atom = Some(atom.clause());
                }
                valuation::Step::Equality(equality, _) => {
                    let position = body.iter().enumerate().find(|(i,item)| !written.contains(i) && matches!(item, BodyItem::Equality(expected) if expected == equality)).map(|(i,_)| i).expect("VO equality correspondence");
                    written.push(position);
                    previous_atom = None;
                }
                valuation::Step::Constraint(constraint, _) => {
                    let position = body.iter().enumerate().find(|(i, item)|
                        !written.contains(i) && matches!(item, BodyItem::Constraint(expected) if expected == constraint)
                    ).map(|(i, _)| i).expect("VO constraint correspondence");
                    written.push(position);
                    previous_atom = None;
                }
                valuation::Step::Negation(negation, _) => {
                    let position = body.iter().enumerate().find(|(i, item)|
                        !written.contains(i) && matches!(item, BodyItem::Negation(expected) if expected == negation)
                    ).map(|(i, _)| i).expect("VO negation correspondence");
                    written.push(position);
                    previous_atom = None;
                }
                valuation::Step::Guard(filter, _) => {
                    let position = body
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
                    let position = body
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
                valuation::Step::For(binding, _) => {
                    let position = body
                        .iter()
                        .enumerate()
                        .find(|(i, item)| {
                            !written.contains(i)
                                && matches!(item, BodyItem::For(expected) if expected == binding)
                        })
                        .map(|(i, _)| i)
                        .expect("VO generator correspondence");
                    written.push(position);
                    previous_atom = None;
                }
                valuation::Step::Reduce(..) => {}
            }
        }
        if options.heuristic == Heuristic::Written && written != (0..body.len()).collect::<Vec<_>>()
        {
            let position = written
                .iter()
                .enumerate()
                .find(|(i, clause)| **clause != *i)
                .map(|(_, clause)| *clause)
                .unwrap_or(body.len());
            return Err(syn::Error::new_spanned(
                &quote::quote!(#(#steps)*),
                format!("clause #{position}: Written requires every clause in normal-body order"),
            ));
        }
        if options.heuristic == Heuristic::Filtered {
            let context: BTreeSet<_> = analysis
                .bound_before
                .first()
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            let facts = clauses(body, &context);
            let graph = cq::hypergraph::Analysis::of_body(body, &context);
            let mut filters = cq::hypergraph::filter_ranks(body, &graph);
            if let Some(groups) = join_branches {
                // The independent branch is one filtered build. Check each
                // member's priority against every predicate in that branch.
                for branch in &groups.branches {
                    let members = branch.clauses();
                    let strongest = members.iter().map(|c| filters[c]).min().unwrap();
                    for clause in members {
                        filters.insert(clause, strongest);
                    }
                }
            }
            let mut bound = context;
            let mut pending: Vec<_> = (0..body.len()).collect();
            for clause in written {
                // Independently check the greedy choice, not compiler output equality.
                let rank = filtered_rank(&facts, &graph, &filters);
                if pending
                    .iter()
                    .copied()
                    .filter(|&c| ready(&facts, &pending, &bound, c))
                    .any(|c| rank(c, &bound) < rank(clause, &bound))
                {
                    return Err(syn::Error::new_spanned(
                        &body[clause],
                        format!(
                            "clause #{clause}: Filtered requires the strongest ready connected occurrence"
                        ),
                    ));
                }
                pending.retain(|&c| c != clause);
                bound.extend(facts[clause].provides.iter().cloned());
            }
        }
    }
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
                            &written.branches[j].body,
                            &arm.steps,
                            &analysis.branches[&a.clause()][j],
                            options,
                            None,
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
            | valuation::Step::Negation(..)
            | valuation::Step::Guard(..)
            | valuation::Step::Let(..)
            | valuation::Step::For(..) => {}
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
