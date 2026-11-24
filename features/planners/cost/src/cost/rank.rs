//! The estimated work of valuations, which ranks them (PLANNING-REDESIGN,
//! Ranking): the containment estimate of System R and DuckDB over the reads a
//! prefix made, charged per step as certificates charge it. An estimate is a
//! prediction; the certificates beside it bound.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_cq as cq;
use cq_syntax::constraint::{Comparison, Constraint, Term};
use cq_syntax::symbol_name;

use super::bounds::{BoundModel, OccurrenceFacts};
use super::enumerate::{self, Work};
use super::{CostStep, IndexDemand, Read};
use crate::export::{self, ClauseKind, Hole, Origin, Version};

/// A source predicate as the estimate reads it: a variable against a known
/// value, a conjunction, a disjunction, or something it cannot judge.
enum Filter {
    Compare { variable: String, op: Op, value: i64 },
    All(Vec<Filter>),
    Any(Vec<Filter>),
    Opaque,
}
#[derive(Clone, Copy)]
enum Op { Eq, Ne, Lt, Le, Gt, Ge }

impl Filter {
    fn new(constraint: &Constraint, closed: Closed<'_>) -> Self {
        let side = |term: &Term| match term {
            Term::Variable(name) => Err(symbol_name(name)),
            Term::Closed(expression) => Ok(value(expression, closed)),
            Term::Literal(expression) => Ok(value(expression, closed)),
        };
        let compare = |variable: String, op: Op, value: Option<i64>| value.map_or(Self::Opaque, |value| Self::Compare { variable, op, value });
        match constraint {
            Constraint::Compare { left, comparison, right } => {
                let op = match comparison {
                    Comparison::Eq(_) => Op::Eq, Comparison::Ne(_) => Op::Ne, Comparison::Lt(_) => Op::Lt,
                    Comparison::Le(_) => Op::Le, Comparison::Gt(_) => Op::Gt, Comparison::Ge(_) => Op::Ge,
                };
                let flipped = match op { Op::Lt => Op::Gt, Op::Le => Op::Ge, Op::Gt => Op::Lt, Op::Ge => Op::Le, other => other };
                match (side(left), side(right)) {
                    (Err(variable), Ok(value)) => compare(variable, op, value),
                    (Ok(value), Err(variable)) => compare(variable, flipped, value),
                    _ => Self::Opaque,
                }
            }
            Constraint::Between { term, low, high } => match (side(term), side(low), side(high)) {
                (Err(variable), Ok(low), Ok(high)) => Self::All(vec![compare(variable.clone(), Op::Ge, low), compare(variable, Op::Le, high)]),
                _ => Self::Opaque,
            },
            Constraint::In { term, literals } => match side(term) {
                Err(variable) => Self::Any(literals.iter().map(|literal| compare(variable.clone(), Op::Eq, value(literal, closed))).collect()),
                Ok(_) => Self::Opaque,
            },
            Constraint::Or(branches) => Self::Any(branches.iter().map(|branch| Self::All(branch.iter().map(|part| Self::new(part, closed)).collect())).collect()),
        }
    }
    fn variables<'s>(&'s self, into: &mut BTreeSet<&'s str>) {
        match self {
            Self::Compare { variable, .. } => { into.insert(variable); }
            Self::All(parts) | Self::Any(parts) => for part in parts { part.variables(into); },
            Self::Opaque => {}
        }
    }
}

/// The value of a closed constant by field name, where the evidence binds it.
pub(crate) type Closed<'a> = &'a dyn Fn(&str) -> Option<i64>;

/// An integer literal, or a closed constant the evidence binds by field name.
fn value(expression: &syn::Expr, closed: Closed<'_>) -> Option<i64> {
    match expression {
        syn::Expr::Paren(inner) => value(&inner.expr, closed),
        syn::Expr::Group(inner) => value(&inner.expr, closed),
        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(int), .. }) => int.base10_parse().ok(),
        syn::Expr::Unary(syn::ExprUnary { op: syn::UnOp::Neg(_), expr, .. }) => value(expr, closed).map(|value| -value),
        syn::Expr::Field(field) => match &field.member { syn::Member::Named(name) => closed(&name.to_string()), _ => None },
        _ => None,
    }
}

/// The source predicates by clause, with their variables.
type Predicates = Vec<(usize, BTreeSet<String>, Filter)>;

/// The variables an equality fixes to a value, and the source predicates.
fn fixed(normal: &cq::Program, closed: Closed<'_>) -> (BTreeMap<String, i64>, Predicates) {
    let mut fixed: BTreeMap<String, i64> = BTreeMap::new();
    let mut filters = Vec::new();
    for (clause, item) in normal.query.body.iter().enumerate() {
        match item {
            cq::BodyItem::Constraint(constraint) => {
                let filter = Filter::new(constraint, closed);
                if let Filter::Compare { variable, op: Op::Eq, value } = &filter { fixed.insert(variable.clone(), *value); }
                let mut variables = BTreeSet::new();
                filter.variables(&mut variables);
                let variables = variables.into_iter().map(str::to_string).collect();
                filters.push((clause, variables, filter));
            }
            cq::BodyItem::Equality(equality) => {
                if let cq_syntax::equality::EqualityValue::Literal(literal) = &equality.right {
                    if let Some(value) = value(literal, closed) { fixed.insert(symbol_name(&equality.left), value); }
                }
            }
            _ => {}
        }
    }
    (fixed, filters)
}

/// The value of each constant column of `reads`, by clause and source
/// column: a literal or closed argument, or a variable an equality fixes.
fn columns<'r>(normal: &cq::Program, reads: impl Iterator<Item = &'r Read>, fixed: &BTreeMap<String, i64>, closed: Closed<'_>) -> BTreeMap<(usize, usize), i64> {
    let mut constants = BTreeMap::new();
    for read in reads {
        let clause = read.normalized_clause;
        let Some(cq::BodyItem::Positive { atom }) = normal.query.body.iter().nth(clause) else { continue };
        for (column, argument) in read.columns.iter().zip(atom.args.iter()) {
            if column.variable.is_some() { continue; }
            let known = match cq_syntax::rust::variable(argument) {
                Some(name) => fixed.get(&symbol_name(name)).copied(),
                None => value(argument, closed),
            };
            if let Some(known) = known { constants.insert((clause, column.source_column), known); }
        }
    }
    constants
}

/// `columns` of a hole's reads, for a caller that reads no predicate.
pub(crate) fn constant_columns<'r>(normal: &cq::Program, reads: impl Iterator<Item = &'r Read>, closed: Closed<'_>) -> BTreeMap<(usize, usize), i64> {
    columns(normal, reads, &fixed(normal, closed).0, closed)
}

/// The share of an occurrence's rows whose `column` satisfies `op value`:
/// a heavy value's own rows, else the average of the other values; a
/// comparison from the ranges, a straddling range in proportion to its
/// width; 1 without the facts. At least one row's share.
fn share(facts: &OccurrenceFacts, column: usize, op: Op, value: i64) -> f64 {
    let rows = facts.rows.max(1.0);
    let values = facts.values.get(&column);
    let heavy = values.map_or(&[][..], |values| &values.heavy[..]);
    let equal = || match heavy.iter().find(|(heavy, _)| *heavy == value) {
        Some((_, count)) => count / rows,
        None => {
            let rest = (facts.rows - heavy.iter().map(|(_, count)| count).sum::<f64>()).max(0.0);
            rest / (facts.distinct[&column] - heavy.len() as f64).max(1.0) / rows
        }
    };
    // The share below `limit` (exclusive).
    let below = |limit: i128| -> Option<f64> {
        let ranges = &values?.ranges;
        if ranges.is_empty() { return None; }
        let held: f64 = ranges.iter().map(|&(lo, hi, count)| {
            let (lo, hi) = (i128::from(lo), i128::from(hi));
            if hi < limit { count } else if lo < limit { count * (limit - lo) as f64 / (hi - lo + 1) as f64 } else { 0.0 }
        }).sum();
        Some(held / rows)
    };
    let value_at = i128::from(value);
    let share = match op {
        Op::Eq => equal(),
        Op::Ne => 1.0 - equal(),
        Op::Lt => below(value_at).unwrap_or(1.0),
        Op::Le => below(value_at + 1).unwrap_or(1.0),
        Op::Gt => below(value_at + 1).map_or(1.0, |below| 1.0 - below),
        Op::Ge => below(value_at).map_or(1.0, |below| 1.0 - below),
    };
    share.clamp(1.0 / rows, 1.0)
}

/// The estimate of one hole's valuations, with the indexes earlier holes
/// already demand (`shared`) and those production does not provision
/// (`forbidden`).
pub(crate) struct Estimator<'a> {
    model: &'a BoundModel<'a>,
    hole: &'a Hole,
    /// Per clause, its full read and its relation's facts.
    facts: BTreeMap<usize, (Read, OccurrenceFacts)>,
    /// The value of each constant column, by clause and source column.
    constants: BTreeMap<(usize, usize), i64>,
    /// The source predicates by clause, with their variables.
    filters: Vec<(usize, BTreeSet<String>, Filter)>,
    steady: bool,
    cq: bool,
    datalog: bool,
    /// The changing occurrence of a steady variant: its clause and full read.
    delta: Option<(usize, Read)>,
    /// Final rows and rounds of the changing relation.
    f: f64,
    n: f64,
    shared: &'a BTreeSet<IndexDemand>,
    forbidden: &'a BTreeSet<IndexDemand>,
    /// Demands by bit, for the search's state.
    interned: Vec<IndexDemand>,
    maintenance: BTreeMap<IndexDemand, Option<f64>>,
    failure: Option<export::ExportError>,
}

impl<'a> Estimator<'a> {
    /// `None` where a read has no row fact, or a steady variant's changing
    /// relation no final rows and rounds: the hole is unestimated.
    pub(crate) fn new(model: &'a BoundModel<'a>, hole: &'a Hole, reads: impl Iterator<Item = &'a Read>,
        shared: &'a BTreeSet<IndexDemand>, forbidden: &'a BTreeSet<IndexDemand>) -> export::Result<Option<Self>> {
        let steady = matches!(hole.origin, Origin::Rule { seed: false, .. });
        let mut facts = BTreeMap::new();
        let mut delta = None;
        let (mut f, mut n) = (1.0, 1.0);
        for read in reads {
            let Some(occurrence) = model.occurrence_facts(hole, read)? else { return Ok(None); };
            if steady && read.version == Version::Delta {
                let Some((rows, rounds)) = occurrence.recursive else { return Ok(None); };
                (f, n) = (rows, rounds);
                delta = Some((read.normalized_clause, read.clone()));
            }
            facts.insert(read.normalized_clause, (read.clone(), occurrence));
        }
        if steady && delta.is_none() { return Ok(None); }
        let normal: cq::Program = syn::parse_str(&hole.normalized_cq)?;
        let closed = |name: &str| model.constant(name);
        let (fixed, filters) = fixed(&normal, &closed);
        let constants = columns(&normal, facts.values().map(|(read, _)| read), &fixed, &closed);
        Ok(Some(Self {
            model, hole, facts, constants, filters, steady, cq: matches!(hole.origin, Origin::Cq {}), datalog: !matches!(hole.origin, Origin::Cq {}),
            delta, f, n, shared, forbidden, interned: Vec::new(), maintenance: BTreeMap::new(), failure: None,
        }))
    }

    /// The whole-run estimate of a complete valuation.
    pub(crate) fn total(&mut self, steps: &[CostStep]) -> export::Result<Option<f64>> {
        let total = self.work(steps, 0, &mut 0);
        self.take_failure()?;
        Ok(total)
    }

    /// The estimate of each step of a complete valuation.
    pub(crate) fn steps(&mut self, steps: &[CostStep]) -> export::Result<Vec<Option<f64>>> {
        let mut demanded = 0;
        let estimates = (0..steps.len()).map(|at| self.work(&steps[..=at], at, &mut demanded)).collect();
        self.take_failure()?;
        Ok(estimates)
    }

    /// An error met while estimating, which the search could only report as
    /// a missing estimate.
    pub(crate) fn take_failure(&mut self) -> export::Result<()> {
        self.failure.take().map_or(Ok(()), Err)
    }

    /// Distinct values of a set of columns: a key-set fact naming exactly
    /// them, else the product of the column counts, at most the rows.
    fn support(facts: &OccurrenceFacts, kept: &BTreeSet<usize>) -> f64 {
        if kept.is_empty() { return 1.0; }
        facts.sets.iter().find(|(columns, _)| columns == kept).map(|(_, distinct)| *distinct)
            .unwrap_or_else(|| kept.iter().map(|column| facts.distinct[column]).product::<f64>())
            .min(facts.rows)
    }

    /// The estimated frames after `prefix`: `(k, true)` is `k * delta`,
    /// `(k, false)` is `k`. Each occurrence contributes `R` once expanded,
    /// else its projection on the columns its partial reads retained. The
    /// estimate is the product of their rows divided, per variable, by the
    /// distinct counts of every contribution holding it but the one holding
    /// fewest values. Where one contribution meets the same partner on
    /// several variables, their composite count lies between the largest
    /// single count and the product; the estimate takes the largest, the end
    /// that predicts more frames, since correlated columns (a method's name
    /// and descriptor) make the product far too selective. Different
    /// partners still multiply. Then every source predicate whose variables
    /// the prefix binds, but `skip`, keeps its share of the frames.
    fn frames(&self, prefix: &[CostStep], skip: Option<usize>) -> (f64, bool) {
        let mut parts: BTreeMap<usize, Option<BTreeSet<usize>>> = BTreeMap::new();
        for read in prefix.iter().filter_map(|step| step.read.as_ref()) {
            let part = parts.entry(read.normalized_clause).or_insert_with(|| Some(BTreeSet::new()));
            match part {
                Some(kept) if read.partial => kept.extend(read.columns.iter().map(|column| column.source_column)),
                _ => *part = None,
            }
        }
        let mut rows = 1.0;
        let mut delta = false;
        let mut distinct: BTreeMap<&str, Vec<(usize, f64)>> = BTreeMap::new();
        for (clause, part) in &parts {
            let (read, facts) = &self.facts[clause];
            let changing = self.delta.as_ref().is_some_and(|(at, _)| at == clause);
            // A constant column keeps its value's share of the rows.
            let narrowed: f64 = read.columns.iter().filter(|column| column.variable.is_none())
                .map(|column| match self.constants.get(&(*clause, column.source_column)) {
                    Some(&value) => share(facts, column.source_column, Op::Eq, value),
                    None => 1.0 / facts.distinct[&column.source_column].max(1.0),
                }).product();
            let selected = facts.rows * narrowed;
            let selected = if facts.rows >= 1.0 { selected.max(1.0) } else { selected };
            let columns: Vec<_> = read.columns.iter().filter(|column| column.variable.is_some()
                && part.as_ref().is_none_or(|kept| kept.contains(&column.source_column))).collect();
            let size = match part {
                None if changing => narrowed,
                None => selected,
                Some(_) => {
                    let kept: BTreeSet<usize> = columns.iter().map(|column| column.source_column).collect();
                    let support = Self::support(facts, &kept);
                    if changing { support / self.f.max(1.0) } else { support.min(selected) }
                }
            };
            rows *= size;
            delta |= changing;
            // The changing occurrence's values range over the final relation's.
            let cap = if changing { f64::INFINITY } else { size };
            let mut own: BTreeMap<&str, f64> = BTreeMap::new();
            for column in columns {
                let count = facts.distinct[&column.source_column].min(cap);
                let held = own.entry(column.variable.as_deref().unwrap()).or_insert(count);
                *held = held.min(count);
            }
            for (variable, count) in own { distinct.entry(variable).or_default().push((*clause, count.max(1.0))); }
        }
        // Per contribution and the partner holding fewest values of a shared
        // variable: the largest count that contribution divides by.
        let mut divisors: BTreeMap<(usize, usize), f64> = BTreeMap::new();
        for holders in distinct.values().filter(|holders| holders.len() > 1) {
            let least = holders.iter().copied().min_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0))).unwrap();
            for &(clause, count) in holders.iter().filter(|holder| **holder != least) {
                let divisor = divisors.entry((clause, least.0)).or_insert(1.0);
                *divisor = divisor.max(count);
            }
        }
        for divisor in divisors.values() { rows /= divisor; }
        // A variable takes the column of the first read that binds it.
        let mut binding: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for read in prefix.iter().filter_map(|step| step.read.as_ref()) {
            for column in &read.columns {
                if let Some(variable) = &column.variable { binding.entry(variable).or_insert((read.normalized_clause, column.source_column)); }
            }
        }
        for (clause, variables, filter) in &self.filters {
            if Some(*clause) == skip || !variables.iter().all(|variable| binding.contains_key(variable.as_str())) { continue; }
            rows *= self.selectivity(filter, &binding);
        }
        (rows, delta)
    }

    fn selectivity(&self, filter: &Filter, binding: &BTreeMap<&str, (usize, usize)>) -> f64 {
        match filter {
            Filter::Compare { variable, op, value } => {
                let (clause, column) = binding[variable.as_str()];
                share(&self.facts[&clause].1, column, *op, *value)
            }
            Filter::All(parts) => parts.iter().map(|part| self.selectivity(part, binding)).product(),
            Filter::Any(parts) => parts.iter().map(|part| self.selectivity(part, binding)).sum::<f64>().min(1.0),
            Filter::Opaque => 1.0,
        }
    }

    fn bit(&mut self, demand: &IndexDemand) -> Option<u64> {
        let at = match self.interned.iter().position(|known| known == demand) {
            Some(at) => at,
            None => { self.interned.push(demand.clone()); self.interned.len() - 1 }
        };
        (at < 64).then(|| 1 << at)
    }

    fn maintained(&mut self, read: &Read, demand: &IndexDemand) -> Option<f64> {
        if let Some(known) = self.maintenance.get(demand) { return *known; }
        let delta = self.delta.as_ref().map(|(_, read)| read);
        let value = match self.model.maintenance_estimate(self.hole, read, delta, demand) {
            Ok(value) => value,
            Err(error) => { self.failure.get_or_insert(error); None }
        };
        self.maintenance.insert(demand.clone(), value);
        value
    }
}

impl Work for Estimator<'_> {
    /// An introducing read is charged its output frames, every other step
    /// its input frames; a step at `k * delta` costs `k * F` over the run
    /// and one at `k` costs `k * N`; a seed or CQ step costs `k` once and the
    /// first read of a CQ nothing. A newly demanded index adds its
    /// maintenance unless an earlier hole demands it.
    fn work(&mut self, steps: &[CostStep], from: usize, demanded: &mut u64) -> Option<f64> {
        let mut total = 0.0;
        for at in from..steps.len() {
            let step = &steps[at];
            if matches!(step.kind, ClauseKind::OpaqueFor | ClauseKind::Block) { return None; }
            let introducing = step.read.is_some() && step.introduces;
            let skip = step.read.is_none().then_some(step.clause);
            let (k, delta) = self.frames(if introducing { &steps[..=at] } else { &steps[..at] }, skip);
            let leaf = self.cq && introducing && !steps[..at].iter().any(|before| before.read.is_some() && before.introduces);
            let k = if leaf { 0.0 } else { k };
            total += if self.steady { k * if delta { self.f } else { self.n } } else { k };
            let read_before = steps[..at].iter().any(|before| before.read.is_some());
            if let Some(demand) = enumerate::demand(self.datalog, read_before, step) {
                if self.forbidden.contains(&demand) { return None; }
                let bit = self.bit(&demand)?;
                if *demanded & bit == 0 {
                    *demanded |= bit;
                    if !self.shared.contains(&demand) {
                        total += self.maintained(step.read.as_ref().expect("a demand comes from a read"), &demand)?;
                    }
                }
            }
        }
        Some(total)
    }
}
