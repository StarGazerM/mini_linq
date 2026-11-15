//! Variable-order planning (PLANNING-REDESIGN, Variable-order planning): the
//! plan of a rule variant is an order of its variables, as in worst-case
//! optimal joins, chosen from the degrees of each variable given the bound
//! variables of each occurrence, and lowered to a valuation.
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use super::enumerate;
use super::{Candidate, Read, Space};
use crate::export::{self, Hole, Origin, PlannerInput, Version};

/// How the changing occurrence reaches a binder: its columns and the
/// binder's columns they equal, one for one.
pub struct Reach<'a> { pub changing: &'a Read, pub columns: &'a [usize], pub binder: &'a [usize] }

/// VP-Degree: the degree sequence of a column given bound columns, over the
/// rows an occurrence reads (its constant columns fixed). Columns are source
/// columns of the read's relation.
pub trait Degrees {
    /// The distinct values of `columns`; 1 for no column.
    fn distinct(&self, read: &Read, columns: &[usize]) -> Option<f64>;
    /// The degree a row of `binder` meets: the degree of `column` given
    /// `bound` in `read`, at the row's values of `from` (the binder's
    /// columns, one per bound column), averaged over the binder's rows; a
    /// row whose values the read lacks counts zero. The two degree
    /// sequences are matched by value, so a miss and a heavy value met often
    /// both count.
    ///
    /// Where `binder` is the changing occurrence its rows are those of the
    /// whole run, each meeting only the rows of `read` no younger than
    /// itself. With `reach`, the average is over the pairs of a row of the
    /// changing occurrence and a binder row it reaches, each meeting the
    /// rows no younger than the changing row.
    fn joined(&self, reach: Option<Reach<'_>>, binder: &Read, from: &[usize], read: &Read, bound: &[usize], column: usize) -> Option<f64>;
    /// The share of the relation's rows a round of the variant sees: below 1
    /// only for a relation of the variant's own recursive component.
    fn visible(&self, _read: &Read) -> f64 { 1.0 }
}

/// A planned variable order with its valuation.
#[derive(Clone, Debug)]
pub struct Planned {
    pub order: Vec<String>,
    /// VP-Cost: the bindings of every prefix of the order, summed, in units
    /// of the delta for a steady variant.
    pub estimate: f64,
    pub candidate: Candidate,
}

struct Occurrence<'a> {
    clause: usize,
    read: &'a Read,
    changing: bool,
    /// Variable index and source column, one per distinct variable.
    columns: Vec<(usize, usize)>,
    /// `Degrees::visible` of the read.
    visible: f64,
}

/// An occurrence, its binder, the binder's key, a variable, and the
/// binder's variables the changing occurrence bound.
type Probe = (usize, usize, u64, usize, u64);

struct Model<'a> {
    occurrences: Vec<Occurrence<'a>>,
    /// The changing occurrence of a steady variant.
    changing: Option<usize>,
    /// Variable names, sorted: a plan depends on no written order.
    names: Vec<String>,
    holders: Vec<Vec<usize>>,
    degrees: &'a dyn Degrees,
    expected: RefCell<HashMap<Probe, Option<f64>>>,
}

/// One binding step: the values expected per earlier binding, and the
/// occurrence proposing them.
#[derive(Clone, Copy)]
struct Step { expected: f64, proposer: usize }

/// No occurrence has bound the variable yet.
const UNBOUND: usize = usize::MAX;

impl Model<'_> {
    /// The source columns of an occurrence's variables in `mask`, in
    /// variable order.
    fn key(&self, at: usize, mask: u64) -> Vec<usize> {
        let mut key: Vec<(usize, usize)> = self.occurrences[at].columns.iter().filter(|(held, _)| mask & (1 << held) != 0).copied().collect();
        key.sort_unstable();
        key.into_iter().map(|(_, column)| column).collect()
    }
    fn mask(&self, at: usize, bound: u64) -> u64 {
        self.occurrences[at].columns.iter().filter(|(held, _)| bound & (1 << held) != 0).fold(0, |mask, (held, _)| mask | 1 << held)
    }
    fn column(&self, at: usize, variable: usize) -> Option<usize> {
        self.occurrences[at].columns.iter().find(|(held, _)| *held == variable).map(|(_, column)| *column)
    }
    /// The values of `variable` an occurrence is expected to hold for a
    /// binding whose variables in `mask` the occurrence `binder` bound.
    /// Its own keys it meets once each: the average degree. Another
    /// occurrence's keys it meets as that occurrence's rows hold them;
    /// `reach` are the binder's variables the changing occurrence bound.
    fn expected(&self, at: usize, binder: usize, mask: u64, variable: usize, reach: u64) -> Option<f64> {
        if let Some(known) = self.expected.borrow().get(&(at, binder, mask, variable, reach)) { return *known; }
        let occurrence = &self.occurrences[at];
        let result = (|| {
            let column = self.column(at, variable)?;
            let key = self.key(at, mask);
            if binder == at || mask == 0 {
                let mut extended = key.clone();
                extended.push(column);
                let (groups, values) = (self.degrees.distinct(occurrence.read, &key)?, self.degrees.distinct(occurrence.read, &extended)?);
                return Some(if groups > 0.0 { values / groups } else { 0.0 });
            }
            let changing = self.changing.filter(|_| reach != 0);
            let (columns, reached) = changing.map_or((Vec::new(), Vec::new()), |changing| (self.key(changing, reach), self.key(binder, reach)));
            let reach = changing.map(|changing| Reach { changing: self.occurrences[changing].read, columns: &columns, binder: &reached });
            self.degrees.joined(reach, self.occurrences[binder].read, &self.key(binder, mask), occurrence.read, &key, column)
        })();
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
            if owners[*held] == UNBOUND || !self.occurrences[owners[*held]].changing { return 0; }
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
            let expected = {
                let mask = self.mask(at, bound);
                let mut binders: Vec<usize> = occurrence.columns.iter().filter(|(held, _)| mask & (1 << held) != 0).map(|(held, _)| owners[*held]).collect();
                binders.sort_unstable();
                binders.dedup();
                let visible = occurrence.visible;
                let mut expected = f64::INFINITY;
                if binders.is_empty() { expected = self.expected(at, at, 0, variable, 0)? * visible; }
                for binder in binders {
                    let part = occurrence.columns.iter().filter(|(held, _)| mask & (1 << held) != 0 && owners[*held] == binder).fold(0u64, |part, (held, _)| part | 1 << held);
                    let reach = if binder == at || self.occurrences[binder].changing { 0 } else { self.reach(binder, bound, owners) };
                    // Through the changing occurrence the degree already counts what its rows see.
                    let seen = if self.occurrences[binder].changing || reach != 0 { 1.0 } else { visible };
                    expected = expected.min(self.expected(at, binder, part, variable, reach)? * seen);
                }
                expected
            };
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

fn model<'a>(hole: &Hole, occurrences: &[(usize, &'a Read)], degrees: &'a dyn Degrees) -> Option<Model<'a>> {
    let names: Vec<String> = occurrences.iter().flat_map(|(_, read)| read.columns.iter().filter_map(|column| column.variable.clone()))
        .collect::<BTreeSet<_>>().into_iter().collect();
    if names.len() > 60 { return None; }
    // Only a steady variant has a changing occurrence.
    let steady = matches!(hole.origin, Origin::Rule { seed: false, .. });
    let mut occurrences: Vec<Occurrence> = occurrences.iter().map(|&(clause, read)| {
        let mut columns: Vec<(usize, usize)> = Vec::new();
        for column in &read.columns {
            let Some(name) = &column.variable else { continue };
            let variable = names.binary_search(name).expect("a collected name");
            if !columns.iter().any(|(held, _)| *held == variable) { columns.push((variable, column.source_column)); }
        }
        Occurrence { clause, read, changing: steady && read.version == Version::Delta, columns, visible: degrees.visible(read) }
    }).collect();
    // Ties between occurrences break by what they read, not where they are written.
    occurrences.sort_by(|left, right| (&left.read.logical_relation, left.read.version == Version::Delta, &left.columns)
        .cmp(&(&right.read.logical_relation, right.read.version == Version::Delta, &right.columns)));
    let holders = (0..names.len()).map(|variable| (0..occurrences.len())
        .filter(|&at| occurrences[at].columns.iter().any(|(held, _)| *held == variable)).collect()).collect();
    let changing = occurrences.iter().position(|occurrence| occurrence.changing);
    Some(Model { occurrences, changing, names, holders, degrees, expected: RefCell::new(HashMap::new()) })
}

/// VP-Delta: the variables of the changing occurrence, bound before any other.
fn root(model: &Model<'_>) -> Vec<usize> {
    let mut root: Vec<usize> = model.changing.map_or(Vec::new(), |at| model.occurrences[at].columns.iter().map(|(variable, _)| *variable).collect());
    root.sort_unstable();
    root
}

/// VP-Lower: the clauses of the occurrences in the order they first propose
/// a variable; each is read in full there.
fn lowering(model: &Model<'_>, order: &[usize]) -> Option<Vec<usize>> {
    let steps = model.steps(order)?;
    let mut clauses = Vec::new();
    let mut bound = 0u64;
    for (place, &variable) in order.iter().enumerate() {
        if bound & (1 << variable) != 0 { continue; }
        let occurrence = &model.occurrences[steps[place].proposer];
        clauses.push(occurrence.clause);
        bound |= occurrence.columns.iter().fold(0u64, |mask, (held, _)| mask | 1 << held);
    }
    Some(clauses)
}

fn planned(model: &Model<'_>, prepared: &enumerate::Prepared, order: Vec<usize>) -> Option<Planned> {
    let estimate = model.cost(&order)?;
    let candidate = prepared.ordered(&lowering(model, &order)?)?;
    Some(Planned { order: order.iter().map(|&variable| model.names[variable].clone()).collect(), estimate, candidate })
}

/// Plan the variable order of one rule hole: VP-Delta and VP-Cost, lowered
/// by VP-Lower. `None` where a degree is missing or the hole has an
/// introducing occurrence outside its reorderable prefix.
pub fn plan(input: &PlannerInput, hole: &Hole, degrees: &dyn Degrees) -> export::Result<Option<Planned>> {
    let prepared = enumerate::prepare(input, hole, Space::default())?;
    let Some(occurrences) = prepared.occurrences() else { return Ok(None) };
    let Some(model) = model(hole, &occurrences, degrees) else { return Ok(None) };
    let root = root(&model);
    let rest: Vec<usize> = (0..model.names.len()).filter(|variable| !root.contains(variable)).collect();
    let mut search = Search { model: &model, rest: &rest, best: None, tried: 0 };
    // The search starts where `Model::steps` arrives after the root.
    let mut owners = vec![UNBOUND; model.names.len()];
    for &variable in &root { owners[variable] = model.changing.expect("root variables have a changing occurrence"); }
    let bound = root.iter().fold(0u64, |bound, variable| bound | 1 << variable);
    if search.extend(&mut Vec::new(), &mut owners, bound, 1.0, root.len() as f64).is_none() { return Ok(None); }
    let Some((_, order)) = search.best else { return Ok(None) };
    Ok(planned(&model, &prepared, root.into_iter().chain(order).collect()))
}

/// The valuation of a given variable order under the same lowering, with
/// its VP-Cost: a hand order, for comparison. The changing occurrence's
/// variables still come first; variables the order does not name follow.
pub fn lower(input: &PlannerInput, hole: &Hole, degrees: &dyn Degrees, order: &[String]) -> export::Result<Option<Planned>> {
    let prepared = enumerate::prepare(input, hole, Space::default())?;
    let Some(occurrences) = prepared.occurrences() else { return Ok(None) };
    let Some(model) = model(hole, &occurrences, degrees) else { return Ok(None) };
    let root = root(&model);
    let mut complete: Vec<usize> = root.clone();
    for name in order {
        if let Ok(variable) = model.names.binary_search(name) { if !complete.contains(&variable) { complete.push(variable); } }
    }
    complete.extend((0..model.names.len()).filter(|variable| !complete.contains(variable)).collect::<Vec<_>>());
    Ok(planned(&model, &prepared, complete))
}
