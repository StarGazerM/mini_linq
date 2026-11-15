//! Exact valuation choices over the source's first positive prefix: each
//! occurrence is expanded at once, looked up and expanded later, or bound one
//! variable at a time and expanded later. An atom order is the valuation
//! whose occurrences are all expanded at once.
//! Binding semantics come from the checked proof reference, never Rust syntax.
//!
//! The planner never lists the space: [`Prepared::best`] searches it by
//! dynamic programming over states (PLANNING-REDESIGN, Search). The listing
//! [`Prepared::all`] is the test oracle of that search.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use cq_lang_cq as cq;
use cq_lang_vo as vo;
use cq_syntax::symbol_name;

use super::{Candidate, CostStep, IndexDemand, IndexVersion, PartialReads, Read, ReadColumn, Space};
use crate::export::{self, ClauseKind, ExportError, Hole, Origin, PlannerInput, Version};

/// Enumerate the complete source-prefix problem in a checked compiler record
/// over the default [`Space`]. Production VO-Order remains the final check
/// when the caller adopts advice.
pub fn enumerate(input: &PlannerInput, hole: &Hole) -> export::Result<Vec<Candidate>> {
    enumerate_under(input, hole, Space::default())
}

/// [`enumerate`] over a declared space of valuations.
pub fn enumerate_under(input: &PlannerInput, hole: &Hole, space: Space) -> export::Result<Vec<Candidate>> {
    input.check()?;
    if !input.holes.iter().any(|record| record == hole) {
        return Err(ExportError("enumeration hole is not part of the checked input".into()));
    }
    Ok(prepare(input, hole, space)?.all())
}

/// A hole's search problem and its checked incumbent.
pub(crate) struct Prepared {
    problem: Problem,
    /// The checked schedule as recorded, marked current.
    pub(crate) incumbent: Candidate,
    /// Whether some generated valuation is the incumbent.
    generated: bool,
    /// A negative-key helper's recorded canonical demands, which every
    /// valuation of it keeps.
    recorded: Option<BTreeSet<IndexDemand>>,
}

/// The cost entry point has already checked the complete input once.
pub(crate) fn prepare(input: &PlannerInput, hole: &Hole, space: Space) -> export::Result<Prepared> {
    let source: cq::Program = syn::parse_str(&hole.source_cq)?;
    let normal: cq::Program = syn::parse_str(&hole.normalized_cq)?;
    let current = syn::parse::Parser::parse_str(
        |input: syn::parse::ParseStream| vo::Program::parse(input, &normal), &hole.checked_vo,
    )?;
    let prefix = source_prefix(input, hole, &source)?;
    let fixed = fixed_columns(&normal.query.body);
    let mut specs = Vec::with_capacity(hole.candidate_reference.steps.len());
    for (clause, facts) in hole.candidate_reference.steps.iter().enumerate() {
        let item = &normal.query.body[clause];
        let read = if let cq::BodyItem::Positive { atom } = item {
            let occurrence = hole.occurrences.iter()
                .find(|record| record.normalized_path == [clause])
                .ok_or_else(|| ExportError(format!("clause {clause} has no source occurrence")))?;
            let declaration = normal.inputs().find(|decl| symbol_name(&decl.name) == occurrence.input_relation)
                .ok_or_else(|| ExportError(format!("clause {clause} has no input declaration")))?;
            Some(Read {
                source_occurrence: occurrence.source_path[0], normalized_clause: clause,
                logical_relation: occurrence.logical_relation.clone(),
                input_relation: occurrence.input_relation.clone(), version: occurrence.version,
                columns: atom.args.iter().zip(&declaration.columns).zip(&occurrence.columns)
                    .map(|((argument, column), &source_column)| ReadColumn {
                        source_column, input_column: symbol_name(&column.name),
                        variable: cq_syntax::rust::variable(argument).map(symbol_name)
                            .filter(|name| !fixed.contains(name)),
                    }).collect(),
                partial: false,
            })
        } else { None };
        // Read order, unlike the sorted provides set, determines bound-before order.
        let binding_order = match item {
            cq::BodyItem::Positive { atom } => atom.variables().map(symbol_name).collect(),
            cq::BodyItem::Block(_) => {
                let step: vo::Step = syn::parse_str(&facts.ir)?;
                let vo::Step::Block(block, _) = step else {
                    return Err(ExportError("block reference has no checked interface".into()));
                };
                block.source.variables().map(|(_, name)| symbol_name(name)).collect()
            }
            _ => facts.introduces.iter().chain(&facts.provides).cloned().collect(),
        };
        let mut required = facts.required.clone();
        for name in &facts.domain_requires {
            if !required.contains(name) { required.push(name.clone()); }
        }
        specs.push(Spec {
            kind: hole.candidate_reference.clauses[clause].kind.clone(),
            required, provides: facts.provides.clone(), binding_order,
            atom: facts.atom,
            structural_atom: facts.atom && facts.block_placement.as_deref() != Some("correlated"),
            prefix: read.as_ref().is_some_and(|read| prefix.contains(&read.source_occurrence)),
            read,
        });
    }
    let mut unbounded = Vec::new();
    if hole.candidate_reference.steps.iter().any(|step| !step.branches.is_empty()) {
        unbounded.push("nested scope work is not finitely costed; retain the checked nested interfaces and incumbent".into());
    }
    // A single-read key comprehension has no lookup or bind to place.
    let space = (!matches!(hole.origin, Origin::NegativeKey { .. })).then_some(space);
    // A variable of one clause only joins nothing: binding it ahead of its
    // expansion adds frames and enables no other read.
    let mut occurrences = BTreeMap::<&String, usize>::new();
    for spec in &specs {
        for name in spec.binding_order.iter().chain(&spec.required).collect::<BTreeSet<_>>() {
            *occurrences.entry(name).or_default() += 1;
        }
    }
    let joins = occurrences.into_iter().filter(|(_, count)| *count > 1).map(|(name, _)| name.clone()).collect();
    let problem = Problem { specs, datalog: !matches!(hole.origin, Origin::Cq {}), unbounded, space, joins };

    let mut incumbent = incumbent(&normal, hole, &current, &problem)?;
    // The incumbent is a generated valuation when its reads and
    // introductions are those of one; that valuation then stands for it.
    // Search only the moves that agree with it so far.
    let twin = {
        let agrees = |steps: &[CostStep]| prefix_of(&introductions(steps), &introductions(&incumbent.steps))
            && prefix_of(&reads(steps), &reads(&incumbent.steps));
        let mut found = Vec::new();
        problem.search(&mut State::new(problem.specs.len()), Context::ROOT, &agrees, &mut found);
        found.into_iter().find(|candidate| introductions(&candidate.steps) == introductions(&incumbent.steps)
            && reads(&candidate.steps) == reads(&incumbent.steps))
    };
    let generated = twin.is_some();
    match twin {
        Some(twin) => incumbent = Candidate { current: true, ..twin },
        None => {
            // A checked valuation with lookup or bind phases outside the generated
            // moves is still costed by the same read certificates.
            if incumbent.unbounded.is_empty() && incumbent.is_atom_order() {
                incumbent.unbounded.push("checked incumbent has an introduction/atom skeleton outside the reorderable source prefix; retain its physical schedule".into());
            }
        }
    }
    let recorded = if matches!(hole.origin, Origin::NegativeKey { .. }) {
        // Auxiliary projections scan their recorded canonical provider and
        // apply filters afterwards. They cannot demand a new constant-key
        // index merely because a logical output bound fixes that column.
        let mut recorded = BTreeSet::new();
        for occurrence in &hole.occurrences {
            let layout = input.layouts.iter().find(|layout| layout.input_relation == occurrence.input_relation)
                .ok_or_else(|| ExportError("negative-key source lacks its recorded canonical layout".into()))?;
            let mut key = layout.columns[..layout.key].to_vec();
            key.sort_unstable();
            recorded.insert(IndexDemand {
                logical_relation: occurrence.logical_relation.clone(),
                version: if layout.version == Version::Full { IndexVersion::Full } else { IndexVersion::Delta },
                key,
            });
        }
        incumbent.demands = recorded.clone();
        Some(recorded)
    } else { None };
    Ok(Prepared { problem, incumbent, generated, recorded })
}

/// The clauses of a valuation's introducing steps, in order.
fn introductions(steps: &[CostStep]) -> Vec<usize> {
    steps.iter().filter(|step| step.introduces).map(|step| step.clause).collect()
}
/// The reads of a valuation: occurrence, retained columns, and whether the
/// read introduces.
fn reads(steps: &[CostStep]) -> Vec<(usize, Vec<usize>, bool)> {
    steps.iter().filter_map(|step| step.read.as_ref().map(|read|
        (step.clause, read.columns.iter().map(|column| column.source_column).collect::<Vec<_>>(), step.introduces)))
        .collect()
}
fn prefix_of<T: PartialEq>(prefix: &[T], whole: &[T]) -> bool {
    prefix.len() <= whole.len() && whole[..prefix.len()] == *prefix
}

/// The index a read demands after earlier reads (`read_before`): none for a
/// delta root, which scans the supplied frontier, nor for an unkeyed CQ read.
pub(crate) fn demand(datalog: bool, read_before: bool, step: &CostStep) -> Option<IndexDemand> {
    let read = step.read.as_ref()?;
    let key: BTreeSet<_> = read.columns.iter().filter(|column| column.variable.as_ref()
        .is_none_or(|variable| step.bound_before.contains(variable)))
        .map(|column| column.source_column).collect();
    let delta_root = !read_before && step.bound_before.is_empty() && read.version == Version::Delta;
    (!delta_root && (datalog || !key.is_empty())).then(|| IndexDemand {
        logical_relation: read.logical_relation.clone(),
        version: if read.version == Version::Full { IndexVersion::Full } else { IndexVersion::Delta },
        key: key.into_iter().collect(),
    })
}

/// The work a search minimizes.
pub(crate) trait Work {
    /// The work of `steps[from..]` after `steps[..from]`, adding the indexes
    /// they demand to `demanded`; `None` where a step has no estimate or
    /// demands an excluded index.
    fn work(&mut self, steps: &[CostStep], from: usize, demanded: &mut u64) -> Option<f64>;
}

/// The least-work valuation a search found.
pub(crate) struct Searched {
    pub(crate) candidate: Candidate,
    pub(crate) work: f64,
    /// Whether the search was exhaustive; otherwise greedy past the budget.
    pub(crate) exact: bool,
}

/// Above this many states the search turns greedy, as DuckDB leaves DPhyp
/// after 10,000 pairs (PLANNING-REDESIGN, Search).
const BUDGET: usize = 10_000;

impl Prepared {
    /// Every valuation of the declared space, with the incumbent marked or
    /// appended: the test oracle of [`Prepared::best`].
    pub(crate) fn all(&self) -> Vec<Candidate> {
        let mut candidates = Vec::new();
        self.problem.search(&mut State::new(self.problem.specs.len()), Context::ROOT, &|_| true, &mut candidates);
        let matching = self.generated.then(|| candidates.iter().position(|candidate|
            introductions(&candidate.steps) == introductions(&self.incumbent.steps)
                && reads(&candidate.steps) == reads(&self.incumbent.steps))).flatten();
        match matching {
            Some(index) => candidates[index].current = true,
            None => candidates.push(self.incumbent.clone()),
        }
        if let Some(recorded) = &self.recorded {
            for candidate in &mut candidates { candidate.demands = recorded.clone(); }
        }
        candidates
    }
    /// Every read of the hole as a full read: the occurrences an estimate
    /// needs facts of.
    pub(crate) fn reads(&self) -> impl Iterator<Item = &Read> {
        self.problem.specs.iter().filter_map(|spec| spec.read.as_ref())
    }
    /// The valuation of least work in the declared space, or among the atom
    /// orders alone.
    pub(crate) fn best(&self, work: &mut dyn Work, atom_orders: bool) -> Option<Searched> {
        let mut memo = HashMap::new();
        let narrowed;
        let problem = if atom_orders {
            narrowed = Problem { space: self.problem.space.map(|space| Space { phased: 0, ..space }), ..self.problem.clone() };
            &narrowed
        } else { &self.problem };
        let found = match problem.memoized(&mut State::new(problem.specs.len()), Context::ROOT, 0, work, &mut memo) {
            Ok(Some(_)) => problem.replay(work, &memo).map(|(steps, total)| (steps, total, true)),
            Ok(None) => None,
            Err(Exceeded) => problem.greedy(work).map(|(steps, total)| (steps, total, false)),
        };
        found.map(|(steps, total, exact)| {
            let mut candidate = problem.candidate(steps, false);
            if let Some(recorded) = &self.recorded { candidate.demands = recorded.clone(); }
            Searched { candidate, work: total, exact }
        })
    }
    /// The reorderable occurrences, each with its clause and full read: what
    /// a variable order plans. `None` where an occurrence outside the prefix
    /// still introduces, which no variable order of the prefix reaches.
    pub(crate) fn occurrences(&self) -> Option<Vec<(usize, &Read)>> {
        let planned: BTreeSet<&String> = self.problem.specs.iter().filter(|spec| spec.prefix)
            .flat_map(|spec| spec.binding_order.iter()).collect();
        let outside = self.problem.specs.iter().filter(|spec| !spec.prefix && spec.read.is_some())
            .any(|spec| spec.binding_order.iter().any(|name| !planned.contains(name)));
        (!outside).then(|| self.problem.specs.iter().enumerate()
            .filter_map(|(clause, spec)| spec.read.as_ref().filter(|_| spec.prefix).map(|read| (clause, read))).collect())
    }
    /// VP-Lower: the valuation that reads the given occurrences in full, in
    /// order; ready checks and completed occurrences drain after every
    /// read. `None` where an occurrence is not ready at its turn or the
    /// reads leave the body incomplete.
    pub(crate) fn ordered(&self, clauses: &[usize]) -> Option<Candidate> {
        let problem = &self.problem;
        let mut state = State::new(problem.specs.len());
        problem.drain(&mut state);
        for &clause in clauses {
            // Drained already: every variable of it was bound before its turn.
            if !state.pending[clause] { continue; }
            if !problem.specs[clause].ready(&state.bound) { return None; }
            problem.place(&mut state, clause);
            problem.drain(&mut state);
        }
        if !problem.complete(&state) { return None; }
        let mut candidate = problem.candidate(state.steps, false);
        if let Some(recorded) = &self.recorded { candidate.demands = recorded.clone(); }
        Some(candidate)
    }
}

/// Whether two valuations take the same reads and introductions.
pub(crate) fn same(left: &Candidate, right: &Candidate) -> bool {
    introductions(&left.steps) == introductions(&right.steps) && reads(&left.steps) == reads(&right.steps)
}

fn source_prefix(input: &PlannerInput, hole: &Hole, source: &cq::Program) -> export::Result<BTreeSet<usize>> {
    fn prefix(body: &cq::CommaList<cq::BodyItem>) -> BTreeSet<usize> {
        body.iter().enumerate().skip_while(|(_, item)| !matches!(item, cq::BodyItem::Positive { .. }))
            .take_while(|(_, item)| matches!(item, cq::BodyItem::Positive { .. }))
            .map(|(index, _)| index).collect()
    }
    if let Origin::Rule { source_rule, permutation, .. } = &hole.origin {
        let original: cq_lang_datalog_round::incremental::source::Program = syn::parse_str(&input.source)?;
        let rule = original.rules.get(*source_rule)
            .ok_or_else(|| ExportError("hole has no original source rule".into()))?;
        let result = prefix(&rule.body);
        // The exported source_path already undoes this permutation. Check that
        // correspondence explicitly instead of assuming generated alias names.
        let source_atoms: Vec<_> = source.query.body.iter().enumerate()
            .filter_map(|(index, item)| matches!(item, cq::BodyItem::Positive { .. }).then_some(index)).collect();
        let mapped: Vec<_> = hole.occurrences.iter().filter(|record| record.normalized_path.len() == 1).collect();
        if source_atoms.len() != mapped.len() || source_atoms.iter().zip(mapped).any(|(&position, record)| {
            permutation.get(position) != record.source_path.first()
        }) {
            return Err(ExportError("source occurrence/permutation correspondence is incomplete".into()));
        }
        Ok(result)
    } else { Ok(prefix(&source.query.body)) }
}

/// Only the relational IR supplies fixed-column facts. Disjunctions do not
/// imply either disjunct, and a nested scope cannot fix an outer variable.
fn fixed_columns(body: &cq::CommaList<cq::BodyItem>) -> BTreeSet<String> {
    use cq_syntax::constraint::{Comparison, Constraint, Term};
    use cq_syntax::equality::EqualityValue;
    fn constraint(value: &Constraint, fixed: &mut BTreeSet<String>) {
        match value {
            Constraint::Compare { left: Term::Variable(name), comparison: Comparison::Eq(_), right: Term::Closed(_) | Term::Literal(_) }
            | Constraint::Compare { right: Term::Variable(name), comparison: Comparison::Eq(_), left: Term::Closed(_) | Term::Literal(_) } => { fixed.insert(symbol_name(name)); }
            Constraint::Or(branches) if branches.len() == 1 => {
                for value in &branches[0] { constraint(value, fixed); }
            }
            _ => {}
        }
    }
    let mut fixed = BTreeSet::new();
    for item in body {
        match item {
            cq::BodyItem::Equality(value) if matches!(value.right, EqualityValue::Literal(_)) => { fixed.insert(symbol_name(&value.left)); }
            cq::BodyItem::Constraint(value) => constraint(value, &mut fixed),
            _ => {}
        }
    }
    fixed
}

#[derive(Clone)]
struct Spec {
    kind: ClauseKind,
    required: Vec<String>,
    provides: Vec<String>,
    binding_order: Vec<String>,
    atom: bool,
    structural_atom: bool,
    prefix: bool,
    read: Option<Read>,
}
impl Spec {
    fn ready(&self, bound: &BTreeSet<String>) -> bool { self.required.iter().all(|name| bound.contains(name)) }
    fn introduces(&self, bound: &BTreeSet<String>) -> bool { self.provides.iter().any(|name| !bound.contains(name)) }
    fn connected(&self, bound: &BTreeSet<String>) -> bool { self.atom && self.provides.iter().any(|name| bound.contains(name)) }
    fn rank(&self, clause: usize) -> (bool, bool, usize) {
        (self.structural_atom, !matches!(self.kind, ClauseKind::Constraint | ClauseKind::Negation),
            self.read.as_ref().map_or(clause, |read| read.source_occurrence))
    }
    /// The occurrence's variables not yet bound, in read order.
    fn unbound<'a>(&'a self, bound: &BTreeSet<String>) -> Vec<&'a String> {
        let mut seen = BTreeSet::new();
        self.binding_order.iter().filter(|name| !bound.contains(*name) && seen.insert(*name)).collect()
    }
}
/// What has been read of an occurrence that is not yet expanded.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Phase { Untouched, Looked, Binding }
/// One move from a state: expand, look up, or bind the `at`th variable of
/// the occurrence's binding order.
#[derive(Clone, Copy)]
enum Move { Place(usize), Lookup(usize), Bind(usize, usize) }
/// What the move rules look back on. `last` is the occurrence the
/// immediately preceding move read in part: reading it again at once is
/// dominated by its expansion alone. `fresh` is the binding count before the
/// last introducing move, and `floor` the last occurrence looked up since,
/// which orders the lookups of one choice point.
#[derive(Clone, Copy)]
struct Context { last: Option<usize>, fresh: usize, floor: Option<usize> }
impl Context {
    const ROOT: Self = Self { last: None, fresh: 0, floor: None };
}
/// Everything that decides a state's estimate and the moves it allows.
#[derive(PartialEq, Eq, Hash)]
struct Key {
    pending: Vec<bool>,
    phase: Vec<Phase>,
    bound: Vec<String>,
    /// Retained columns of each occurrence read in part and not yet expanded.
    retained: Vec<(usize, Vec<usize>)>,
    last: Option<usize>,
    floor: Option<usize>,
    fresh: Vec<String>,
    demanded: u64,
}
/// The search left its budget.
struct Exceeded;
/// Memoized least work from a state, and the index of its move.
type Memo = HashMap<Key, Option<(f64, Option<usize>)>>;
#[derive(Clone)]
struct Problem {
    specs: Vec<Spec>,
    datalog: bool,
    unbounded: Vec<String>,
    /// The declared space; `None` generates expansions only.
    space: Option<Space>,
    /// Variables of more than one clause.
    joins: BTreeSet<String>,
}
struct State {
    /// Occurrences and other clauses not yet placed in full.
    pending: Vec<bool>,
    phase: Vec<Phase>,
    bound: BTreeSet<String>,
    binding_order: Vec<String>,
    /// One entry per step: the clause, its phase before, and whether the
    /// step placed it in full.
    sequence: Vec<(usize, Phase, bool)>,
    /// Occurrences read in phases so far.
    phased: usize,
    steps: Vec<CostStep>,
}
impl State {
    fn new(count: usize) -> Self {
        Self { pending: vec![true; count], phase: vec![Phase::Untouched; count], bound: BTreeSet::new(),
            binding_order: Vec::new(), sequence: Vec::with_capacity(count), phased: 0, steps: Vec::with_capacity(count) }
    }
}
impl Problem {
    /// Read or evaluate the clause in full.
    fn place(&self, state: &mut State, clause: usize) {
        let spec = &self.specs[clause];
        state.steps.push(CostStep {
            clause, kind: spec.kind.clone(), read: spec.read.clone(), required: spec.required.clone(),
            provides: spec.provides.clone(), bound_before: state.binding_order.clone(),
            introduces: spec.introduces(&state.bound),
        });
        for name in &spec.binding_order {
            if state.bound.insert(name.clone()) { state.binding_order.push(name.clone()); }
        }
        state.sequence.push((clause, state.phase[clause], true));
        state.pending[clause] = false;
    }
    /// A sub-atom read retaining the occurrence's bound variables and, for a
    /// bind, `variable`: a lookup introduces nothing.
    fn partial(&self, state: &mut State, clause: usize, variable: Option<&str>) {
        let spec = &self.specs[clause];
        let full = spec.read.as_ref().expect("positive occurrence");
        let columns: Vec<_> = full.columns.iter().filter(|column| column.variable.as_ref()
            .is_none_or(|name| state.bound.contains(name) || Some(name.as_str()) == variable)).cloned().collect();
        let provides: BTreeSet<_> = columns.iter().filter_map(|column| column.variable.clone()).collect();
        state.steps.push(CostStep {
            clause, kind: spec.kind.clone(), read: Some(Read { columns, partial: true, ..full.clone() }),
            required: spec.required.clone(), provides: provides.into_iter().collect(),
            bound_before: state.binding_order.clone(), introduces: variable.is_some(),
        });
        state.sequence.push((clause, state.phase[clause], false));
        if state.phase[clause] == Phase::Untouched { state.phased += 1; }
        state.phase[clause] = if variable.is_some() { Phase::Binding } else { Phase::Looked };
        if let Some(name) = variable {
            if state.bound.insert(name.to_owned()) { state.binding_order.push(name.to_owned()); }
        }
    }
    fn restore(&self, state: &mut State, length: usize, bindings: usize) {
        for (clause, phase, full) in state.sequence.drain(length..).rev() {
            if full { state.pending[clause] = true; }
            else if phase == Phase::Untouched { state.phased -= 1; }
            state.phase[clause] = phase;
        }
        for name in state.binding_order.drain(bindings..) { state.bound.remove(&name); }
        state.steps.truncate(length);
    }
    /// Place ready checks before every introduction (I2), and structural
    /// non-atoms in order. Whether anything was placed.
    fn drain(&self, state: &mut State) -> bool {
        let mut placed = false;
        loop {
            // I2 is global, including nonintroducing atoms beyond a barrier.
            // This deterministic drainage removes factorial filter equivalents.
            let filter = self.specs.iter().enumerate().filter(|(clause, spec)| {
                state.pending[*clause] && spec.ready(&state.bound) && !spec.introduces(&state.bound)
            }).min_by_key(|(clause, spec)| spec.rank(*clause)).map(|(clause, _)| clause);
            // A check between a bind and its expansion prunes bound values
            // first, so the expansion no longer follows the bind at once.
            if let Some(clause) = filter { self.place(state, clause); placed = true; continue; }
            // Preserve structural priority of introducing Rust bindings/blocks.
            let opaque = self.specs.iter().enumerate().filter(|(clause, spec)| {
                state.pending[*clause] && !spec.structural_atom && spec.ready(&state.bound)
            }).min_by_key(|(clause, spec)| spec.rank(*clause)).map(|(clause, _)| clause);
            if let Some(clause) = opaque { self.place(state, clause); placed = true; continue; }
            return placed;
        }
    }
    fn complete(&self, state: &State) -> bool { state.pending.iter().all(|pending| !pending) }
    /// The moves of a drained, incomplete state, each with the context it
    /// leaves.
    fn moves(&self, state: &State, context: Context) -> Vec<(Move, Context)> {
        let mut moves = Vec::new();
        let prefix_pending = self.specs.iter().enumerate().any(|(clause, spec)| state.pending[clause] && spec.prefix);
        let connected = self.specs.iter().enumerate().any(|(clause, spec)| {
            state.pending[clause] && spec.ready(&state.bound) && spec.connected(&state.bound)
        });
        let eligible = |clause: usize, spec: &Spec| {
            state.pending[clause] && spec.ready(&state.bound)
                && (!connected || spec.connected(&state.bound))
                && (!prefix_pending || spec.prefix)
        };
        let choices: Vec<_> = self.specs.iter().enumerate()
            .filter(|(clause, spec)| eligible(*clause, spec)).map(|(clause, _)| clause).collect();
        let bound = state.binding_order.len();
        // Only source-prefix atoms are choices. Outside the prefix retain
        // the structural choice, rather than enumerating opaque crossings.
        for clause in choices.into_iter().take(if prefix_pending { usize::MAX } else { 1 }) {
            if context.last == Some(clause) { continue; }
            moves.push((Move::Place(clause), Context { last: None, fresh: bound, floor: None }));
        }
        if let (true, Some(space)) = (prefix_pending, self.space) {
            for (clause, spec) in self.specs.iter().enumerate() {
                if !spec.prefix || spec.read.is_none() || !state.pending[clause] || !spec.ready(&state.bound) { continue; }
                // An occurrence enters its phases only within the bound.
                if state.phase[clause] == Phase::Untouched && state.phased >= space.phased { continue; }
                let unbound = spec.unbound(&state.bound);
                // lookup(o): pushed down to the choice point where its key
                // first holds a variable of the last introduction.
                let keyed = spec.binding_order.iter().any(|name| {
                    state.binding_order.iter().position(|placed| placed == name).is_some_and(|at| at >= context.fresh)
                });
                if state.phase[clause] == Phase::Untouched && !unbound.is_empty() && keyed
                    && context.floor.is_none_or(|floor| clause > floor)
                {
                    moves.push((Move::Lookup(clause), Context { last: Some(clause), fresh: context.fresh, floor: Some(clause) }));
                }
                // bind(o, x): under `rows` it visits the rows expand(o)
                // visits and adds a read, so only `distinct` generates it.
                if space.partial_reads == PartialReads::Distinct && state.phase[clause] != Phase::Looked
                    && unbound.len() >= 2 && context.last != Some(clause)
                    && (!connected || spec.connected(&state.bound))
                {
                    for name in unbound.iter().filter(|name| self.joins.contains(**name)) {
                        let at = spec.binding_order.iter().position(|variable| variable == *name).expect("an unbound variable of the occurrence");
                        moves.push((Move::Bind(clause, at), Context { last: Some(clause), fresh: bound, floor: None }));
                    }
                }
            }
        }
        moves
    }
    fn apply(&self, state: &mut State, step: Move) {
        match step {
            Move::Place(clause) => self.place(state, clause),
            Move::Lookup(clause) => self.partial(state, clause, None),
            Move::Bind(clause, at) => self.partial(state, clause, Some(&self.specs[clause].binding_order[at])),
        }
    }
    /// Every valuation whose every prefix `keep` accepts.
    fn search(&self, state: &mut State, mut context: Context, keep: &dyn Fn(&[CostStep]) -> bool, result: &mut Vec<Candidate>) {
        let start = state.sequence.len();
        let bindings = state.binding_order.len();
        if self.drain(state) { context.last = None; }
        if keep(&state.steps) {
            if self.complete(state) {
                result.push(self.candidate(state.steps.clone(), false));
            } else {
                for (step, next) in self.moves(state, context) {
                    let length = state.sequence.len();
                    let bound = state.binding_order.len();
                    self.apply(state, step);
                    if keep(&state.steps) { self.search(state, next, keep, result); }
                    self.restore(state, length, bound);
                }
            }
        }
        self.restore(state, start, bindings);
    }
    fn key(&self, state: &State, context: Context, demanded: u64) -> Key {
        let retained = (0..self.specs.len()).filter(|&clause| state.pending[clause] && state.phase[clause] != Phase::Untouched)
            .map(|clause| (clause, state.steps.iter().rev().find_map(|step| step.read.as_ref().filter(|read| read.normalized_clause == clause))
                .map(|read| read.columns.iter().map(|column| column.source_column).collect()).unwrap_or_default()))
            .collect();
        let mut bound: Vec<String> = state.bound.iter().cloned().collect();
        bound.sort_unstable();
        let mut fresh = state.binding_order[context.fresh.min(state.binding_order.len())..].to_vec();
        fresh.sort_unstable();
        Key { pending: state.pending.clone(), phase: state.phase.clone(), bound, retained, last: context.last, floor: context.floor, fresh, demanded }
    }
    /// The least work from `state` to a complete valuation, memoized per
    /// state: Selinger's recursion over left-deep move sequences.
    fn memoized(&self, state: &mut State, mut context: Context, demanded: u64, work: &mut dyn Work, memo: &mut Memo) -> Result<Option<f64>, Exceeded> {
        let key = self.key(state, context, demanded);
        if let Some(known) = memo.get(&key) { return Ok(known.map(|(total, _)| total)); }
        if memo.len() >= BUDGET { return Err(Exceeded); }
        let start = state.sequence.len();
        let bindings = state.binding_order.len();
        let before = state.steps.len();
        if self.drain(state) { context.last = None; }
        let mut demanded = demanded;
        let result = match work.work(&state.steps, before, &mut demanded) {
            None => Ok(None),
            Some(drained) if self.complete(state) => Ok(Some((drained, None))),
            Some(drained) => {
                let mut best: Option<(f64, usize)> = None;
                let mut outcome = Ok(());
                for (at, (step, next)) in self.moves(state, context).into_iter().enumerate() {
                    let length = state.sequence.len();
                    let bound = state.binding_order.len();
                    let from = state.steps.len();
                    self.apply(state, step);
                    let mut after = demanded;
                    if let Some(cost) = work.work(&state.steps, from, &mut after) {
                        match self.memoized(state, next, after, work, memo) {
                            Ok(Some(rest)) if best.is_none_or(|(least, _)| cost + rest < least) => best = Some((cost + rest, at)),
                            Ok(_) => {}
                            Err(exceeded) => outcome = Err(exceeded),
                        }
                    }
                    self.restore(state, length, bound);
                    if outcome.is_err() { break; }
                }
                outcome.map(|()| best.map(|(total, at)| (drained + total, Some(at))))
            }
        };
        self.restore(state, start, bindings);
        let result = result?;
        memo.insert(key, result);
        Ok(result.map(|(total, _)| total))
    }
    /// The steps of the memoized least-work valuation, and its work.
    fn replay(&self, work: &mut dyn Work, memo: &Memo) -> Option<(Vec<CostStep>, f64)> {
        let mut state = State::new(self.specs.len());
        let mut context = Context::ROOT;
        let mut demanded = 0;
        let mut total = 0.0;
        loop {
            let key = self.key(&state, context, demanded);
            let before = state.steps.len();
            if self.drain(&mut state) { context.last = None; }
            total += work.work(&state.steps, before, &mut demanded)?;
            let (_, choice) = (*memo.get(&key)?)?;
            let Some(at) = choice else { return Some((state.steps, total)); };
            let (step, next) = self.moves(&state, context).into_iter().nth(at)?;
            let from = state.steps.len();
            self.apply(&mut state, step);
            total += work.work(&state.steps, from, &mut demanded)?;
            context = next;
        }
    }
    /// Past the budget: the move of least work from each state (GOO).
    fn greedy(&self, work: &mut dyn Work) -> Option<(Vec<CostStep>, f64)> {
        let mut state = State::new(self.specs.len());
        let mut context = Context::ROOT;
        let mut demanded = 0;
        let mut total = 0.0;
        loop {
            let before = state.steps.len();
            if self.drain(&mut state) { context.last = None; }
            total += work.work(&state.steps, before, &mut demanded)?;
            if self.complete(&state) { return Some((state.steps, total)); }
            let mut best: Option<(f64, Move, Context)> = None;
            for (step, next) in self.moves(&state, context) {
                let length = state.sequence.len();
                let bound = state.binding_order.len();
                let from = state.steps.len();
                self.apply(&mut state, step);
                let mut after = demanded;
                if let Some(cost) = work.work(&state.steps, from, &mut after) {
                    if best.as_ref().is_none_or(|(least, _, _)| cost < *least) { best = Some((cost, step, next)); }
                }
                self.restore(&mut state, length, bound);
            }
            let (_, step, next) = best?;
            let from = state.steps.len();
            self.apply(&mut state, step);
            total += work.work(&state.steps, from, &mut demanded)?;
            context = next;
        }
    }
    fn candidate(&self, steps: Vec<CostStep>, current: bool) -> Candidate {
        let mut order = Vec::new();
        let mut demands = BTreeSet::new();
        let mut read_before = false;
        for step in &steps {
            if let Some(read) = &step.read {
                if self.specs[step.clause].prefix && !order.contains(&read.source_occurrence) {
                    order.push(read.source_occurrence);
                }
                demands.extend(demand(self.datalog, read_before, step));
                read_before = true;
            }
        }
        Candidate { order, steps, demands, current, unbounded: self.unbounded.clone() }
    }
}

/// The actual checked schedule, with its lookup and bind reads as recorded.
fn incumbent(cq: &cq::Program, hole: &Hole, plan: &vo::Program, problem: &Problem) -> export::Result<Candidate> {
    let mut steps = Vec::new();
    let clauses = recorded_clause_indices(hole, plan)?;
    for ((physical, facts), clause) in plan.valuation.steps.iter().zip(&hole.candidate.steps).zip(clauses) {
        // Restrictions and reductions are preprocessing, certified apart.
        if facts.preprocessing { continue; }
        let clause = clause.ok_or_else(|| ExportError("stream step lacks its checked source clause".into()))?;
        let spec = &problem.specs[clause];
        let mut read = spec.read.clone();
        if let vo::Step::Atom(atom, _) = physical {
            if atom.columns.iter().any(|column| matches!(column, vo::Column::Dropped(_))) {
                // Literals are fixed; a dropped variable is not a fixed column.
                // Keep just the actual projected variable columns plus literals.
                let source = match &cq.query.body[clause] {
                    cq::BodyItem::Positive { atom } => atom,
                    _ => return Err(ExportError("partial read is not an atom".into())),
                };
                let mut columns = atom.columns.iter();
                if let Some(read) = &mut read {
                    read.columns = read.columns.iter().zip(&source.args).filter_map(|(column, argument)| {
                        let keep = cq_syntax::rust::variable(argument).is_none()
                            || matches!(columns.next(), Some(vo::Column::Variable(_)));
                        keep.then(|| column.clone())
                    }).collect();
                    read.partial = true;
                }
            }
        }
        steps.push(CostStep {
            clause, kind: spec.kind.clone(), read, required: facts.required.clone(), provides: facts.provides.clone(),
            bound_before: facts.bound_before.clone(),
            introduces: facts.provides.iter().any(|name| !facts.provided_before.contains(name)),
        });
    }
    // Restrict may have absorbed predicates. Keep their reference meaning in
    // coarse candidates; the actual physical record is still owned by Hole.
    Ok(problem.candidate(steps, true))
}

/// Positional correspondence for the actual recorded VO, shared by incumbent
/// costing and physical calibration. This compares typed VO obligations, never
/// classifies an opaque Rust expression.
pub(crate) fn recorded_clause_indices(hole: &Hole, plan: &vo::Program) -> export::Result<Vec<Option<usize>>> {
    let reference: Vec<vo::Step> = hole.candidate_reference.steps.iter()
        .map(|step| syn::parse_str(&step.ir)).collect::<syn::Result<_>>()?;
    let mut used = BTreeSet::new();
    plan.valuation.steps.iter().zip(&hole.candidate.steps).map(|(physical, facts)| {
        if facts.preprocessing { return Ok(None); }
        if let Some(path) = &facts.occurrence {
            return path.first().copied().map(Some).ok_or_else(|| ExportError("empty checked occurrence path".into()));
        }
        let clause = reference.iter().enumerate()
            .find(|(clause, step)| !used.contains(clause) && *step == physical)
            .map(|(clause, _)| clause)
            .ok_or_else(|| ExportError("checked incumbent step has no reference clause".into()))?;
        used.insert(clause);
        Ok(Some(clause))
    }).collect()
}
