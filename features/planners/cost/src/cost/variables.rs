//! Variable-order planning (PLANNING-REDESIGN, Variable-order planning and
//! Planner statistics path): the plan of a rule variant is an order of its
//! variables, as in worst-case optimal joins, chosen from the degree facts of
//! the dataset evidence and lowered to a valuation.
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use cq_datalog_incremental::RulePlan;
use cq_evidence::{DatasetEvidence, DegreeRequest, Reach, Scalar, Selection};
use cq_lang_cq as cq;

use super::{advice, enumerate, rank};
use super::{Candidate, Space};
use crate::export::{self, Origin, PlannerInput, Version};

/// A planned variable order with its valuation.
#[derive(Clone, Debug)]
pub struct Planned {
    pub order: Vec<String>,
    /// VP-Cost: the bindings of every prefix of the order, summed, in units
    /// of the delta for a steady variant.
    pub estimate: f64,
    pub candidate: Candidate,
    /// Every statistic read to reach this valuation: by the search for a
    /// planned order, by the one order for a lowered one.
    pub read: Vec<DegreeRequest>,
}

/// PS-Advise: the directives production accepts for a program's planned
/// holes, and the holes whose plan it rejects, each with the reason.
pub struct Advised {
    pub plans: Vec<RulePlan>,
    pub adopted: Vec<usize>,
    pub rejected: Vec<(usize, String)>,
}

struct Occurrence {
    clause: usize,
    /// The rows it reads.
    rows: Selection,
    changing: bool,
    /// Its relation is of the changing relation's recursive component: a
    /// round sees part of it.
    member: bool,
    /// Variable index and source column, one per distinct variable.
    columns: Vec<(usize, usize)>,
}

/// An occurrence, its binder, the binder's key, a variable, and the
/// binder's variables the changing occurrence bound.
type Probe = (usize, usize, u64, usize, u64);

struct Model<'a> {
    occurrences: Vec<Occurrence>,
    /// The changing occurrence of a steady variant.
    changing: Option<usize>,
    /// Variable names, sorted: a plan depends on no written order.
    names: Vec<String>,
    holders: Vec<Vec<usize>>,
    facts: &'a HashMap<DegreeRequest, f64>,
    expected: RefCell<HashMap<Probe, Option<f64>>>,
    read: RefCell<BTreeSet<DegreeRequest>>,
}

/// One binding step: the values expected per earlier binding, and the
/// occurrence proposing them.
#[derive(Clone, Copy)]
struct Step { expected: f64, proposer: usize }

/// No occurrence has bound the variable yet.
const UNBOUND: usize = usize::MAX;

impl Model<'_> {
    fn variables(&self, at: usize) -> u64 {
        self.occurrences[at].columns.iter().fold(0, |mask, (held, _)| mask | 1 << held)
    }
    fn column(&self, at: usize, variable: usize) -> usize {
        self.occurrences[at].columns.iter().find(|(held, _)| *held == variable).expect("a variable of the occurrence").1
    }
    /// The source columns of the variables in `mask` that two occurrences
    /// share, pairwise, ascending by the first's column.
    fn pairs(&self, first: usize, second: usize, mask: u64) -> (Vec<usize>, Vec<usize>) {
        let mut pairs: Vec<(usize, usize)> = self.occurrences[first].columns.iter().filter(|(held, _)| mask & (1 << held) != 0)
            .map(|&(held, column)| (column, self.column(second, held))).collect();
        pairs.sort_unstable();
        pairs.into_iter().unzip()
    }
    /// PS-Request: the distinct values an occurrence's rows hold on its
    /// variables in `mask`.
    fn distinct(&self, at: usize, mask: u64) -> DegreeRequest {
        DegreeRequest::Distinct { rows: self.occurrences[at].rows.clone(), columns: self.pairs(at, at, mask).0 }
    }
    /// PS-Request: the values of `variable` an occurrence holds at the keys
    /// `binder` bound (its variables in `mask`); `reach` are the binder's
    /// variables the changing occurrence bound.
    fn joined(&self, at: usize, binder: usize, mask: u64, variable: usize, reach: u64) -> DegreeRequest {
        let (bound, from) = self.pairs(at, binder, mask);
        let reached = (reach != 0).then(|| {
            let changing = self.changing.expect("a reach has a changing occurrence");
            let (columns, binder_columns) = self.pairs(changing, binder, reach);
            Reach { changing: self.occurrences[changing].rows.clone(), columns, binder: binder_columns }
        });
        DegreeRequest::Joined {
            // A round sees the rows no younger than the changing row.
            staged: self.occurrences[at].member && (reach != 0 || self.occurrences[binder].changing),
            reach: reached, binder: self.occurrences[binder].rows.clone(), from, read: self.occurrences[at].rows.clone(), bound,
            column: self.column(at, variable),
        }
    }
    /// PS-Request: the share of an occurrence's rows a round sees, asked
    /// for a relation of the changing relation's component.
    fn visible(&self, at: usize) -> Option<DegreeRequest> {
        let changing = self.changing.filter(|changing| *changing != at && self.occurrences[at].member)?;
        Some(DegreeRequest::Visible { changing: self.occurrences[changing].rows.relation.clone(), relation: self.occurrences[at].rows.relation.clone() })
    }
    /// PS-Derive: every statistic `step` can ask for.
    fn requests(&self, into: &mut BTreeSet<DegreeRequest>) {
        let pinned = self.changing.map_or(0, |changing| self.variables(changing));
        let subsets = |of: u64| { let mut next = Some(of); std::iter::from_fn(move || { let mask = next?; next = (mask != 0).then(|| (mask - 1) & of); Some(mask) }) };
        for at in (0..self.occurrences.len()).filter(|at| Some(*at) != self.changing) {
            into.extend(self.visible(at));
            let free = self.variables(at) & !pinned;
            // Its own keys: every set of the variables it can propose.
            for mask in subsets(free) { into.insert(self.distinct(at, mask)); }
            for variable in (0..self.names.len()).filter(|variable| free & (1 << variable) != 0) {
                // The changing occurrence binds all of its variables at once.
                if let Some(changing) = self.changing.filter(|_| self.variables(at) & pinned != 0) {
                    into.insert(self.joined(at, changing, self.variables(at) & pinned, variable, 0));
                }
                for binder in (0..self.occurrences.len()).filter(|binder| *binder != at && Some(*binder) != self.changing) {
                    let reach = self.variables(binder) & pinned;
                    for mask in subsets(self.variables(binder) & free & !(1 << variable)).filter(|mask| *mask != 0) {
                        into.insert(self.joined(at, binder, mask, variable, 0));
                        if reach != 0 { into.insert(self.joined(at, binder, mask, variable, reach)); }
                    }
                }
            }
        }
    }
    /// The values of `variable` an occurrence is expected to hold for a
    /// binding whose variables in `mask` the occurrence `binder` bound.
    /// Its own keys it meets once each: the average degree. Another
    /// occurrence's keys it meets as that occurrence's rows hold them;
    /// `reach` are the binder's variables the changing occurrence bound.
    fn expected(&self, at: usize, binder: usize, mask: u64, variable: usize, reach: u64) -> Option<f64> {
        if let Some(known) = self.expected.borrow().get(&(at, binder, mask, variable, reach)) { return *known; }
        let fact = |request: DegreeRequest| {
            let value = self.facts.get(&request).copied();
            self.read.borrow_mut().insert(request);
            value
        };
        let result = if binder == at || mask == 0 {
            fact(self.distinct(at, mask)).zip(fact(self.distinct(at, mask | 1 << variable)))
                .map(|(groups, values)| if groups > 0.0 { values / groups } else { 0.0 })
        } else {
            fact(self.joined(at, binder, mask, variable, reach))
        };
        self.expected.borrow_mut().insert((at, binder, mask, variable, reach), result);
        result
    }
    /// The variables of a binder that the changing occurrence bound, where
    /// it bound every variable the binder did not bind itself: the binder's
    /// rows are then met as the changing rows reach them. 0 otherwise.
    fn reach(&self, binder: usize, bound: u64, owners: &[usize]) -> u64 {
        let mut reach = 0u64;
        for (held, _) in &self.occurrences[binder].columns {
            if bound & (1 << held) == 0 || owners[*held] == binder { continue; }
            if Some(owners[*held]) != self.changing { return 0; }
            reach |= 1 << held;
        }
        reach
    }
    /// VP-Bind: every occurrence holding the variable restricts it; the
    /// least expected degree proposes. An occurrence's degree is the least
    /// over the binders of its bound variables, each taken with the
    /// variables it bound. VP-Delta: the changing occurrence is taken as
    /// far smaller than any relation, so it proposes each of its variables,
    /// one value per binding.
    fn step(&self, bound: u64, owners: &[usize], variable: usize) -> Option<Step> {
        if let Some(changing) = self.changing.filter(|changing| self.holders[variable].contains(changing)) {
            return Some(Step { expected: 1.0, proposer: changing });
        }
        let mut step: Option<Step> = None;
        for &at in &self.holders[variable] {
            let occurrence = &self.occurrences[at];
            let mask = self.variables(at) & bound;
            let mut binders: Vec<usize> = occurrence.columns.iter().filter(|(held, _)| mask & (1 << held) != 0).map(|(held, _)| owners[*held]).collect();
            binders.sort_unstable();
            binders.dedup();
            let visible = match self.visible(at) {
                Some(request) => {
                    let value = self.facts.get(&request).copied();
                    self.read.borrow_mut().insert(request);
                    value?
                }
                None => 1.0,
            };
            let mut expected = f64::INFINITY;
            if binders.is_empty() { expected = self.expected(at, at, 0, variable, 0)? * visible; }
            for binder in binders {
                let part = occurrence.columns.iter().filter(|(held, _)| mask & (1 << held) != 0 && owners[*held] == binder).fold(0u64, |part, (held, _)| part | 1 << held);
                let through = Some(binder) == self.changing;
                let reach = if binder == at || through { 0 } else { self.reach(binder, bound, owners) };
                // Through the changing occurrence the degree already counts what its rows see.
                let seen = if through || reach != 0 { 1.0 } else { visible };
                expected = expected.min(self.expected(at, binder, part, variable, reach)? * seen);
            }
            step = Some(match step {
                Some(known) if known.expected <= expected => known,
                _ => Step { expected, proposer: at },
            });
        }
        step
    }
    /// The steps of an order.
    fn steps(&self, order: &[usize]) -> Option<Vec<Step>> {
        let mut owners = vec![UNBOUND; self.names.len()];
        let mut bound = 0u64;
        order.iter().map(|&variable| {
            let step = self.step(bound, &owners, variable)?;
            owners[variable] = step.proposer;
            bound |= 1 << variable;
            Some(step)
        }).collect()
    }
    /// VP-Cost of an order.
    fn cost(&self, order: &[usize]) -> Option<f64> {
        let (mut bindings, mut total) = (1.0, 0.0);
        for step in self.steps(order)? {
            bindings *= step.expected;
            total += bindings;
        }
        Some(total)
    }
    /// VP-Delta: the variables of the changing occurrence, bound before any other.
    fn root(&self) -> Vec<usize> {
        let mut root: Vec<usize> = self.changing.map_or(Vec::new(), |at| self.occurrences[at].columns.iter().map(|(variable, _)| *variable).collect());
        root.sort_unstable();
        root
    }
    /// VP-Lower: the clauses of the occurrences in the order they first
    /// propose a variable; each is read in full there.
    fn lowering(&self, order: &[usize]) -> Option<Vec<usize>> {
        let steps = self.steps(order)?;
        let mut clauses = Vec::new();
        let mut bound = 0u64;
        for (place, &variable) in order.iter().enumerate() {
            if bound & (1 << variable) != 0 { continue; }
            clauses.push(self.occurrences[steps[place].proposer].clause);
            bound |= self.variables(steps[place].proposer);
        }
        Some(clauses)
    }
}

/// Orders tried before the search keeps the best found so far.
const BUDGET: usize = 200_000;

struct Search<'a, 'b> {
    model: &'a Model<'b>,
    /// The variables to order after those already bound.
    rest: &'a [usize],
    best: Option<(f64, Vec<usize>)>,
    tried: usize,
}
impl Search<'_, '_> {
    fn extend(&mut self, order: &mut Vec<usize>, owners: &mut Vec<usize>, bound: u64, bindings: f64, total: f64) -> Option<()> {
        if self.tried >= BUDGET { return Some(()); }
        if order.len() == self.rest.len() {
            self.tried += 1;
            if self.best.as_ref().is_none_or(|(known, _)| total < *known) { self.best = Some((total, order.clone())); }
            return Some(());
        }
        // Candidates in the order of their step, so good orders come first.
        let mut next: Vec<(usize, Step)> = Vec::new();
        for &variable in self.rest {
            if bound & (1 << variable) != 0 { continue; }
            next.push((variable, self.model.step(bound, owners, variable)?));
        }
        next.sort_by(|left, right| left.1.expected.total_cmp(&right.1.expected).then(left.0.cmp(&right.0)));
        for (variable, step) in next {
            let bindings = bindings * step.expected;
            let total = total + bindings;
            if self.best.as_ref().is_some_and(|(known, _)| total >= *known) { continue; }
            order.push(variable);
            owners[variable] = step.proposer;
            self.extend(order, owners, bound | 1 << variable, bindings, total)?;
            owners[variable] = UNBOUND;
            order.pop();
        }
        Some(())
    }
}

/// The variable-order planner of one program over one dataset's evidence
/// (PS-Read): it reads degree facts and closed constants, never rows.
pub struct Planner<'a> {
    input: &'a PlannerInput,
    /// The search problem of every rule hole, by hole.
    prepared: Vec<Option<enumerate::Prepared>>,
    /// The relations that change in each recursive component, by stratum.
    members: BTreeMap<usize, BTreeSet<String>>,
    facts: HashMap<DegreeRequest, f64>,
    constants: BTreeMap<String, i64>,
}

impl<'a> Planner<'a> {
    pub fn new(input: &'a PlannerInput, evidence: &DatasetEvidence) -> export::Result<Self> {
        let mut members: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
        let mut prepared = Vec::with_capacity(input.holes.len());
        for hole in &input.holes {
            let Origin::Rule { stratum, seed, .. } = &hole.origin else { prepared.push(None); continue };
            let problem = enumerate::prepare(input, hole, Space::default())?;
            if !seed {
                members.entry(*stratum).or_default().extend(problem.reads().filter(|read| read.version == Version::Delta).map(|read| read.logical_relation.clone()));
            }
            prepared.push(Some(problem));
        }
        let facts = evidence.degrees.iter().filter_map(|degree| {
            let value = degree.value.value.as_ref()?;
            Some((degree.request.clone(), value.numerator.parse::<f64>().ok()? / value.denominator.parse::<f64>().ok()?))
        }).collect();
        let constants = evidence.constants.iter().filter_map(|(name, fact)| match fact.value.as_ref()? {
            Scalar::SignedInteger { value } => Some((name.clone(), *value)),
            _ => None,
        }).collect();
        Ok(Self { input, prepared, members, facts, constants })
    }

    /// The model of a rule hole. `None` for another hole, or one with an
    /// introducing occurrence outside its reorderable prefix.
    fn model(&self, hole: usize) -> export::Result<Option<(Model<'_>, &enumerate::Prepared)>> {
        let (Origin::Rule { stratum, seed, .. }, Some(prepared)) = (&self.input.holes[hole].origin, self.prepared[hole].as_ref()) else { return Ok(None) };
        let Some(reads) = prepared.occurrences() else { return Ok(None) };
        let names: Vec<String> = reads.iter().flat_map(|(_, read)| read.columns.iter().filter_map(|column| column.variable.clone()))
            .collect::<BTreeSet<_>>().into_iter().collect();
        if names.len() > 60 { return Ok(None); }
        let normal: cq::Program = syn::parse_str(&self.input.holes[hole].normalized_cq)?;
        let constants = rank::constant_columns(&normal, reads.iter().map(|(_, read)| *read), &|name| self.constants.get(name).copied());
        // Only a steady variant has a changing occurrence.
        let members = self.members.get(stratum).filter(|_| !seed);
        let mut occurrences: Vec<Occurrence> = reads.iter().map(|&(clause, read)| {
            let mut columns: Vec<(usize, usize)> = Vec::new();
            let mut fixed = Vec::new();
            for column in &read.columns {
                let Some(name) = &column.variable else {
                    fixed.extend(constants.get(&(read.normalized_clause, column.source_column)).map(|value| (column.source_column, *value)));
                    continue;
                };
                let variable = names.binary_search(name).expect("a collected name");
                if !columns.iter().any(|(held, _)| *held == variable) { columns.push((variable, column.source_column)); }
            }
            fixed.sort_unstable();
            Occurrence {
                clause, rows: Selection { relation: read.logical_relation.clone(), fixed },
                changing: !seed && read.version == Version::Delta,
                member: members.is_some_and(|members| members.contains(&read.logical_relation)), columns,
            }
        }).collect();
        // Ties between occurrences break by what they read, not where they are written.
        occurrences.sort_by(|left, right| (&left.rows.relation, left.changing, &left.columns, &left.rows.fixed)
            .cmp(&(&right.rows.relation, right.changing, &right.columns, &right.rows.fixed)));
        let holders = (0..names.len()).map(|variable| (0..occurrences.len())
            .filter(|&at| occurrences[at].columns.iter().any(|(held, _)| *held == variable)).collect()).collect();
        let changing = occurrences.iter().position(|occurrence| occurrence.changing);
        Ok(Some((Model { occurrences, changing, names, holders, facts: &self.facts, expected: RefCell::new(HashMap::new()), read: RefCell::new(BTreeSet::new()) }, prepared)))
    }

    fn planned(model: &Model<'_>, prepared: &enumerate::Prepared, order: Vec<usize>) -> Option<Planned> {
        let estimate = model.cost(&order)?;
        let candidate = prepared.ordered(&model.lowering(&order)?)?;
        let read = model.read.borrow().iter().cloned().collect();
        Some(Planned { order: order.iter().map(|&variable| model.names[variable].clone()).collect(), estimate, candidate, read })
    }

    /// PS-Derive: every degree statistic planning this program can ask
    /// for, at the evidence's closed constants.
    pub fn requests(&self) -> export::Result<Vec<DegreeRequest>> {
        let mut requests = BTreeSet::new();
        for hole in 0..self.input.holes.len() {
            if let Some((model, _)) = self.model(hole)? { model.requests(&mut requests); }
        }
        Ok(requests.into_iter().collect())
    }

    /// Plan the variable order of one rule hole: VP-Delta and VP-Cost,
    /// lowered by VP-Lower. `None` where a fact is missing, or the hole is
    /// no rule hole or has an introducing occurrence outside its prefix.
    pub fn plan(&self, hole: usize) -> export::Result<Option<Planned>> {
        let Some((model, prepared)) = self.model(hole)? else { return Ok(None) };
        let root = model.root();
        let rest: Vec<usize> = (0..model.names.len()).filter(|variable| !root.contains(variable)).collect();
        let mut search = Search { model: &model, rest: &rest, best: None, tried: 0 };
        // The search starts where `Model::steps` arrives after the root.
        let mut owners = vec![UNBOUND; model.names.len()];
        for &variable in &root { owners[variable] = model.changing.expect("root variables have a changing occurrence"); }
        let bound = root.iter().fold(0u64, |bound, variable| bound | 1 << variable);
        if search.extend(&mut Vec::new(), &mut owners, bound, 1.0, root.len() as f64).is_none() { return Ok(None); }
        let Some((_, order)) = search.best else { return Ok(None) };
        Ok(Self::planned(&model, prepared, root.into_iter().chain(order).collect()))
    }

    /// The valuation of a given variable order under the same lowering,
    /// with its VP-Cost: a hand order, for comparison. The changing
    /// occurrence's variables still come first; variables the order does
    /// not name follow.
    pub fn lower(&self, hole: usize, order: &[String]) -> export::Result<Option<Planned>> {
        let Some((model, prepared)) = self.model(hole)? else { return Ok(None) };
        let mut complete = model.root();
        for name in order {
            if let Ok(variable) = model.names.binary_search(name) { if !complete.contains(&variable) { complete.push(variable); } }
        }
        complete.extend((0..model.names.len()).filter(|variable| !complete.contains(variable)).collect::<Vec<_>>());
        Ok(Self::planned(&model, prepared, complete))
    }

    /// VP-Cost of the hole's recorded order: its variables in the order
    /// its reads introduce them, under the same lowering.
    pub fn recorded(&self, hole: usize) -> export::Result<Option<f64>> {
        let Some(prepared) = self.prepared[hole].as_ref() else { return Ok(None) };
        let order: Vec<String> = prepared.incumbent.steps.iter().filter_map(|step| step.read.as_ref())
            .flat_map(|read| read.columns.iter().filter_map(|column| column.variable.clone())).collect();
        Ok(self.lower(hole, &order)?.map(|planned| planned.estimate))
    }

    /// PS-Advise: plan every rule hole and adopt the plans.
    pub fn advise(&self) -> export::Result<Advised> {
        let mut planned = BTreeMap::new();
        for hole in 0..self.input.holes.len() {
            if let Some(plan) = self.plan(hole)? { planned.insert(hole, plan.candidate); }
        }
        self.adopt(&planned)
    }

    /// Compile valuations (by hole) as advice through production. Where
    /// production rejects them together, each is compiled alone and the
    /// rejected ones keep their variant's own directives.
    pub fn adopt(&self, valuations: &BTreeMap<usize, Candidate>) -> export::Result<Advised> {
        self.input.check()?;
        let input = self.input;
        let program = advice::recorded(input)?;
        let mut chosen: BTreeMap<usize, &Candidate> = valuations.iter().map(|(hole, candidate)| (*hole, candidate)).collect();
        let mut rejected = Vec::new();
        let plans = match advice::compile_plans_for(input, &program, &chosen) {
            Ok(plans) => plans,
            Err(_) => {
                let holes: Vec<usize> = chosen.keys().copied().collect();
                let alone = advice::each_alone(input, &holes, |program, &hole|
                    Ok(advice::compile_plans_for(input, program, &BTreeMap::from([(hole, chosen[&hole])])).err()))?;
                rejected = holes.iter().zip(alone).filter_map(|(&hole, reason)| Some((hole, reason?.0))).collect();
                for (hole, _) in &rejected { chosen.remove(hole); }
                advice::compile_plans_for(input, &program, &chosen)?
            }
        };
        Ok(Advised { plans, adopted: chosen.keys().copied().collect(), rejected })
    }
}
