//! Exact introducing-atom choices over the source's first positive prefix.
//! Binding semantics come from the checked proof reference, never Rust syntax.
use std::collections::BTreeSet;

use cq_lang_cq as cq;
use cq_lang_vo as vo;
use cq_syntax::symbol_name;

use super::{Candidate, CostStep, IndexDemand, IndexVersion, Read, ReadColumn};
use crate::export::{self, ClauseKind, ExportError, Hole, Origin, PlannerInput, Version};

/// Enumerate the complete source-prefix problem in a checked compiler record.
/// Production VO-Order remains the final check when the caller adopts advice.
pub fn enumerate(input: &PlannerInput, hole: &Hole) -> export::Result<Vec<Candidate>> {
    input.check()?;
    if !input.holes.iter().any(|record| record == hole) {
        return Err(ExportError("enumeration hole is not part of the checked input".into()));
    }
    enumerate_checked(input, hole)
}

/// The cost entry point has already checked the complete input once.
pub(crate) fn enumerate_checked(input: &PlannerInput, hole: &Hole) -> export::Result<Vec<Candidate>> {
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
    if hole.candidate.positive_clauses.len() > 1
        && current.valuation.steps.iter().any(|step| matches!(step, vo::Step::Restrict(..)))
    {
        unbounded.push(super::RESTRICTED_VIEW_GAP.into());
    }
    let problem = Problem { specs, datalog: !matches!(hole.origin, Origin::Cq {}), unbounded };
    let mut candidates = Vec::new();
    problem.search(&mut State::new(problem.specs.len()), &mut candidates);

    let (mut incumbent, physical) = incumbent(&normal, hole, &current, &problem)?;
    let introductions = |steps: &[CostStep]| steps.iter().filter(|step| step.introduces)
        .map(|step| step.clause).collect::<Vec<_>>();
    let atoms = |steps: &[CostStep]| steps.iter().filter(|step| step.read.is_some())
        .map(|step| step.clause).collect::<Vec<_>>();
    let matching = if physical.is_empty() {
        candidates.iter().position(|candidate| introductions(&candidate.steps) == introductions(&incumbent.steps)
            && atoms(&candidate.steps) == atoms(&incumbent.steps))
    } else { None };
    if let Some(index) = matching {
        candidates[index].current = true;
    } else {
        incumbent.unbounded.extend(physical);
        if incumbent.unbounded.is_empty() {
            incumbent.unbounded.push("checked incumbent has an introduction/atom skeleton outside the reorderable source prefix; retain its physical schedule".into());
        }
        candidates.push(incumbent);
    }
    if matches!(hole.origin, Origin::NegativeKey { .. }) {
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
        for candidate in &mut candidates { candidate.demands = recorded.clone(); }
    }
    Ok(candidates)
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
}
struct Problem { specs: Vec<Spec>, datalog: bool, unbounded: Vec<String> }
struct State {
    pending: Vec<bool>,
    bound: BTreeSet<String>,
    binding_order: Vec<String>,
    sequence: Vec<usize>,
    steps: Vec<CostStep>,
}
impl State {
    fn new(count: usize) -> Self {
        Self { pending: vec![true; count], bound: BTreeSet::new(), binding_order: Vec::new(),
            sequence: Vec::with_capacity(count), steps: Vec::with_capacity(count) }
    }
}
impl Problem {
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
        state.pending[clause] = false;
        state.sequence.push(clause);
    }
    fn restore(&self, state: &mut State, length: usize, bindings: usize) {
        for clause in state.sequence.drain(length..) { state.pending[clause] = true; }
        for name in state.binding_order.drain(bindings..) { state.bound.remove(&name); }
        state.steps.truncate(length);
    }
    fn search(&self, state: &mut State, result: &mut Vec<Candidate>) {
        let start = state.sequence.len();
        let bindings = state.binding_order.len();
        loop {
            // I2 is global, including nonintroducing atoms beyond a barrier.
            // This deterministic drainage removes factorial filter equivalents.
            let filter = self.specs.iter().enumerate().filter(|(clause, spec)| {
                state.pending[*clause] && spec.ready(&state.bound) && !spec.introduces(&state.bound)
            }).min_by_key(|(clause, spec)| spec.rank(*clause)).map(|(clause, _)| clause);
            if let Some(clause) = filter { self.place(state, clause); continue; }
            // Preserve structural priority of introducing Rust bindings/blocks.
            let opaque = self.specs.iter().enumerate().filter(|(clause, spec)| {
                state.pending[*clause] && !spec.structural_atom && spec.ready(&state.bound)
            }).min_by_key(|(clause, spec)| spec.rank(*clause)).map(|(clause, _)| clause);
            if let Some(clause) = opaque { self.place(state, clause); continue; }
            break;
        }
        if state.sequence.len() == self.specs.len() {
            result.push(self.candidate(state.steps.clone(), false));
        } else {
            let prefix_pending = self.specs.iter().enumerate().any(|(clause, spec)| state.pending[clause] && spec.prefix);
            let connected = self.specs.iter().enumerate().any(|(clause, spec)| {
                state.pending[clause] && spec.ready(&state.bound) && spec.connected(&state.bound)
            });
            let choices: Vec<_> = self.specs.iter().enumerate().filter(|(clause, spec)| {
                state.pending[*clause] && spec.ready(&state.bound)
                    && (!connected || spec.connected(&state.bound))
                    && (!prefix_pending || spec.prefix)
            }).map(|(clause, _)| clause).collect();
            // Only source-prefix atoms are choices. Outside the prefix retain
            // the structural choice, rather than enumerating opaque crossings.
            for clause in choices.into_iter().take(if prefix_pending { usize::MAX } else { 1 }) {
                let length = state.sequence.len();
                let bound = state.binding_order.len();
                self.place(state, clause);
                self.search(state, result);
                self.restore(state, length, bound);
            }
        }
        self.restore(state, start, bindings);
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
                let key: BTreeSet<_> = read.columns.iter().filter(|column| column.variable.as_ref()
                    .is_none_or(|variable| step.bound_before.contains(variable)))
                    .map(|column| column.source_column).collect();
                // Delta roots scan the supplied frontier, not an ephemeral index.
                let delta_root = !read_before && step.bound_before.is_empty() && read.version == Version::Delta;
                if !delta_root && (self.datalog || !key.is_empty()) {
                    demands.insert(IndexDemand {
                        logical_relation: read.logical_relation.clone(),
                        version: if read.version == Version::Full { IndexVersion::Full } else { IndexVersion::Delta },
                        key: key.into_iter().collect(),
                    });
                }
                read_before = true;
            }
        }
        Candidate { order, steps, demands, current, unbounded: self.unbounded.clone() }
    }
}

/// Keep the actual checked phase schedule when coarse atom choices cannot
/// represent it. Missing finite phase work is explicit, not a zero-cost rewrite.
fn incumbent(cq: &cq::Program, hole: &Hole, plan: &vo::Program, problem: &Problem) -> export::Result<(Candidate, Vec<String>)> {
    let mut reasons = BTreeSet::new();
    let mut steps = Vec::new();
    let clauses = recorded_clause_indices(hole, plan)?;
    for ((physical, facts), clause) in plan.valuation.steps.iter().zip(&hole.candidate.steps).zip(clauses) {
        if facts.preprocessing {
            if matches!(physical, vo::Step::Reduce(..)) {
                reasons.insert("checked incumbent reduction work is not finitely costed; preserve its preprocessing".into());
            }
            continue;
        }
        let clause = clause.ok_or_else(|| ExportError("stream step lacks its checked source clause".into()))?;
        let spec = &problem.specs[clause];
        let mut read = spec.read.clone();
        if let vo::Step::Atom(atom, _) = physical {
            if atom.columns.iter().any(|column| matches!(column, vo::Column::Dropped(_)))
                || hole.occurrences.iter().any(|record| record.normalized_path == [clause] && record.vo_steps.len() != 1)
            {
                reasons.insert(format!("checked incumbent clause {clause} has partial-read phases not finitely costed; preserve its physical reads"));
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
    let candidate = problem.candidate(steps, true);
    Ok((candidate, reasons.into_iter().collect()))
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
