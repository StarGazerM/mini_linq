//! Symbolic evaluated-position work and dependency-chain span.
//!
//! Tiles, references and publication are not extra row evaluations. Stores and
//! dispatches are separate tie breakers. Span follows data dependencies (a join
//! of independent branches takes their maximum), not definition order. A tile
//! only creates parallel parts when grading records `Distribute`; strip widths
//! and per-lane item counts never manufacture additional processors.
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

use cq_lang_sequence::structures::physical::Operator;
use cq_lang_sequence::{Analysis, Len, LevelNode, Machine, OutputProtocol, Program, Term, Tiling};
use cq_syntax::symbol_name;

/// A conservative symbolic cost. Incomparable costs retain candidate order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cost {
    work: Sum,
    span: Sum,
    /// Shrinking lookups evaluated after an upstream Expand in the same pipeline.
    pub lookup_after_expand: usize,
    pub stores: usize,
    pub dispatches: usize,
}

/// Check the grading and derive its cost, without supplying input cardinalities.
pub fn cost(program: &Program, machine: &Machine) -> syn::Result<Cost> {
    let analysis = cq_lang_sequence::check_grading(program, machine)?;
    Cost::from_analysis(program, machine, &analysis)
}

impl Cost {
    pub(crate) fn from_analysis(
        program: &Program,
        machine: &Machine,
        analysis: &Analysis,
    ) -> syn::Result<Self> {
        let mut evaluator = Evaluator {
            program,
            analysis,
            facts: BTreeMap::new(),
        };
        let mut work = Sum::default();
        let mut ends = Vec::new();
        let mut stores = 0usize;
        let mut lookup_after_expand = 0usize;
        for definition in &program.sequence.definitions {
            let name = symbol_name(&definition.name);
            let mut level = &machine.root;
            for component in analysis.levels[&name].iter().skip(1) {
                level = level
                    .children
                    .iter()
                    .find(|child| symbol_name(&child.name) == *component)
                    .ok_or_else(|| {
                        syn::Error::new(definition.name.span(), "PL-Cost: unknown execution level")
                    })?;
            }
            let mut tile = 0;
            let mut fact = evaluator.term(&definition.term, &name, level, &mut tile)?;
            if let Some(result) = analysis.results.get(&name) {
                fact.result.clone_from(result);
            }
            stores = stores
                .checked_add(count_stores(&definition.term)?)
                .ok_or_else(overflow)?;
            // Pure named views run at their consumers, not at their declaration.
            if matches!(definition.term, Term::Store { .. })
                || program
                    .sequence
                    .definitions
                    .last()
                    .is_some_and(|last| last.name == definition.name)
            {
                work.add(&fact.work)?;
                lookup_after_expand = lookup_after_expand
                    .checked_add(fact.lookup_after_expand)
                    .ok_or_else(overflow)?;
                ends.push(fact.span.clone());
            }
            evaluator.facts.insert(name, fact);
        }
        // Count interpretation evaluates the final producer again after counting;
        // references to already materialized definitions are dependencies, not
        // duplicated row computations.
        if analysis
            .output
            .as_ref()
            .is_some_and(|(_, protocol)| *protocol == OutputProtocol::Count)
            && let Some(last) = program.sequence.definitions.last()
        {
            let fact = &evaluator.facts[&symbol_name(&last.name)];
            work.add(&fact.work)?;
            let mut replay = fact.span.clone();
            replay.add(&fact.local_span)?;
            ends.push(replay);
        }
        Ok(Self {
            work,
            span: Sum::maximum(ends),
            lookup_after_expand,
            stores,
            dispatches: analysis.dispatches.len(),
        })
    }

    /// True only when symbolic comparison establishes no greater cost.
    pub fn leq(&self, other: &Self) -> bool {
        matches!(
            self.partial_cmp(other),
            Some(Ordering::Less | Ordering::Equal)
        )
    }

    /// Strict improvement; ties and unknown comparisons keep the earlier plan.
    pub fn better_than(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(Ordering::Less)
    }
}

impl PartialOrd for Cost {
    // Formal rule PL-Cost: Compare work, Lookup-before-Expand, dependency span, stores, then dispatches; unresolved work comparisons retain enumeration order.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self.work.compare(&other.work)? {
            Ordering::Equal => match self.lookup_after_expand.cmp(&other.lookup_after_expand) {
                Ordering::Equal => match self.span.compare(&other.span)? {
                    Ordering::Equal => {
                        Some((self.stores, self.dispatches).cmp(&(other.stores, other.dispatches)))
                    }
                    ordering => Some(ordering),
                },
                ordering => Some(ordering),
            },
            ordering => Some(ordering),
        }
    }
}

impl fmt::Display for Cost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(work={}, lookup_after_expand={}, span={}, stores={}, dispatches={})",
            self.work, self.lookup_after_expand, self.span, self.stores, self.dispatches
        )
    }
}

/// Stable type-length identities: scan names denote the operation's result,
/// not the name of an inserted store. Unknown bucket sizes are never capacities.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Size {
    Constant(u128),
    Input(String),
    Scan(String),
    Unknown(String),
    Add(Box<Size>, Box<Size>),
    CeilDiv(Box<Size>, usize),
}

impl Size {
    fn from_len(length: &Len, identity: &str) -> Self {
        match length {
            Len::Lit(n) => Self::Constant(*n as u128),
            Len::Input(name) => Self::Input(name.clone()),
            Len::Scan(_) => Self::Scan(identity.into()),
            Len::Unknown(_) => Self::Unknown(identity.into()),
            Len::Add(a, b) => Self::Add(
                Box::new(Self::from_len(a, identity)),
                Box::new(Self::from_len(b, identity)),
            ),
            Len::CeilDiv(inner, n) => Self::CeilDiv(Box::new(Self::from_len(inner, identity)), *n),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Partition {
    driver: Size,
    rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Atom {
    One,
    Positions(Size),
    /// `part` is the largest actual part, not an average bucket cardinality.
    /// Ordinary depth is ceil(part/lanes). For nonempty inputs scan depth is
    /// ceil(part/lanes) + ceil(log2(min(part,lanes))); empty inputs have depth
    /// zero. Thus one lane is serial and a whole-width scan is logarithmic.
    Depth {
        size: Size,
        partition: Option<Partition>,
        lanes: usize,
        scan: bool,
    },
    Max(Vec<Sum>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Sum(BTreeMap<Atom, u128>);

fn overflow() -> syn::Error {
    syn::Error::new(
        proc_macro2::Span::call_site(),
        "PL-Cost: symbolic coefficient overflow",
    )
}

impl Sum {
    fn atom(atom: Atom, coefficient: u128) -> Self {
        let mut result = Self::default();
        if coefficient != 0 {
            result.0.insert(atom, coefficient);
        }
        result
    }

    fn positions(size: &Size) -> syn::Result<Self> {
        match size {
            Size::Constant(n) => Ok(Self::atom(Atom::One, *n)),
            Size::Add(a, b) => {
                let mut sum = Self::positions(a)?;
                sum.add(&Self::positions(b)?)?;
                Ok(sum)
            }
            _ => Ok(Self::atom(Atom::Positions(size.clone()), 1)),
        }
    }

    fn add(&mut self, other: &Self) -> syn::Result<()> {
        for (atom, coefficient) in &other.0 {
            let entry = self.0.entry(atom.clone()).or_default();
            *entry = entry.checked_add(*coefficient).ok_or_else(overflow)?;
        }
        Ok(())
    }

    fn maximum(mut alternatives: Vec<Self>) -> Self {
        alternatives.sort();
        alternatives.dedup();
        let mut retained: Vec<Self> = Vec::new();
        for alternative in alternatives {
            if retained
                .iter()
                .any(|current| alternative.proven_leq(current, &mut 4096))
            {
                continue;
            }
            retained.retain(|current| !current.proven_leq(&alternative, &mut 4096));
            retained.push(alternative);
        }
        match retained.len() {
            0 => Self::default(),
            1 => retained.pop().expect("one alternative"),
            _ => Self::atom(Atom::Max(retained), 1),
        }
    }

    fn compare(&self, other: &Self) -> Option<Ordering> {
        if self == other {
            return Some(Ordering::Equal);
        }
        match (
            self.proven_leq(other, &mut 4096),
            other.proven_leq(self, &mut 4096),
        ) {
            (true, true) => Some(Ordering::Equal),
            (true, false) => Some(Ordering::Less),
            (false, true) => Some(Ordering::Greater),
            _ => None,
        }
    }

    /// Bounded coefficient matching proves an inequality or gives up. It never
    /// substitutes sample cardinalities and never multiplies coefficients.
    fn proven_leq(&self, other: &Self, budget: &mut usize) -> bool {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        if self == other || self.0.is_empty() {
            return true;
        }
        if let Some((Atom::Max(branches), 1)) = self.single() {
            return branches
                .iter()
                .all(|branch| branch.proven_leq(other, budget));
        }
        if let Some((Atom::Max(branches), 1)) = other.single()
            && branches
                .iter()
                .any(|branch| self.proven_leq(branch, budget))
        {
            return true;
        }
        let mut available: Vec<_> = other.0.iter().map(|(atom, n)| (atom, *n)).collect();
        for (atom, coefficient) in &self.0 {
            let mut needed = *coefficient;
            // Exact terms first; do not spend their coefficients on weaker bounds.
            for exact in [true, false] {
                for (candidate, remaining) in &mut available {
                    if needed == 0 {
                        break;
                    }
                    if *budget == 0 {
                        return false;
                    }
                    *budget -= 1;
                    if (*candidate == atom) != exact || *remaining == 0 {
                        continue;
                    }
                    if atom_leq(atom, candidate, budget) {
                        let taken = needed.min(*remaining);
                        needed -= taken;
                        *remaining -= taken;
                    }
                }
            }
            if needed != 0 {
                return false;
            }
        }
        true
    }

    fn single(&self) -> Option<(&Atom, u128)> {
        if self.0.len() != 1 {
            return None;
        }
        self.0
            .first_key_value()
            .map(|(atom, coefficient)| (atom, *coefficient))
    }
}

fn atom_leq(left: &Atom, right: &Atom, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    if left == right {
        return true;
    }
    match (left, right) {
        (
            Atom::Depth {
                size: a,
                partition: ap,
                lanes: aw,
                scan: ac,
            },
            Atom::Depth {
                size: b,
                partition: bp,
                lanes: bw,
                scan: bc,
            },
        ) => {
            a == b
                && ac == bc
                && (aw == bw || *bw == 1)
                && (ap == bp
                    || bp.is_none()
                    || matches!((ap, bp), (Some(a), Some(b)) if a.driver == b.driver && b.rows % a.rows == 0))
        }
        (Atom::Depth { size, .. }, Atom::Positions(other)) => size == other,
        (Atom::Max(branches), _) => branches
            .iter()
            .all(|branch| branch.proven_leq(&Sum::atom(right.clone(), 1), budget)),
        (_, Atom::Max(branches)) => branches
            .iter()
            .any(|branch| Sum::atom(left.clone(), 1).proven_leq(branch, budget)),
        _ => false,
    }
}

#[derive(Clone)]
enum Layer {
    Tile { parents: Size },
    Bucket { parents: Size },
}

#[derive(Clone)]
struct Fact {
    size: Size,
    result: String,
    layers: Vec<Layer>,
    partition: Option<Partition>,
    work: Sum,
    span: Sum,
    local_span: Sum,
    expands: usize,
    lookup_after_expand: usize,
}

impl Fact {
    fn evaluate(&mut self, size: &Size, level: &LevelNode, scan: bool) -> syn::Result<()> {
        self.work.add(&Sum::positions(size)?)?;
        let lanes = level.width.unwrap_or(1).max(1);
        let depth = if lanes == 1 && self.partition.is_none() {
            Sum::positions(size)?
        } else if matches!(size, Size::Constant(0)) {
            Sum::default()
        } else if let Size::Constant(n) = size
            && self.partition.is_none()
        {
            let lanes = lanes as u128;
            let waves = n / lanes + u128::from(n % lanes != 0);
            let reduction = if scan {
                u128::from(u128::BITS - ((*n).min(lanes) - 1).leading_zeros())
            } else {
                0
            };
            Sum::atom(
                Atom::One,
                waves.checked_add(reduction).ok_or_else(overflow)?,
            )
        } else {
            Sum::atom(
                Atom::Depth {
                    size: size.clone(),
                    partition: self.partition.clone(),
                    lanes,
                    scan,
                },
                1,
            )
        };
        self.span.add(&depth)?;
        self.local_span.add(&depth)
    }
}

struct Evaluator<'a> {
    program: &'a Program,
    analysis: &'a Analysis,
    facts: BTreeMap<String, Fact>,
}

impl Evaluator<'_> {
    fn term(
        &self,
        term: &Term,
        owner: &str,
        level: &LevelNode,
        tile: &mut usize,
    ) -> syn::Result<Fact> {
        match term {
            Term::Rows(name) => {
                let result = symbol_name(name);
                let size = if self
                    .program
                    .structures
                    .physical
                    .program
                    .definitions
                    .iter()
                    .any(|definition| {
                        symbol_name(&definition.name) == result
                            && matches!(definition.operator, Operator::Unit(_))
                    }) {
                    Size::Constant(1)
                } else {
                    Size::Input(result.clone())
                };
                let mut fact = Fact {
                    size: size.clone(),
                    result,
                    layers: Vec::new(),
                    partition: None,
                    work: Sum::default(),
                    span: Sum::default(),
                    local_span: Sum::default(),
                    expands: 0,
                    lookup_after_expand: 0,
                };
                fact.evaluate(&size, level, false)?;
                Ok(fact)
            }
            Term::Ref(name) => {
                let mut fact = self.facts[&symbol_name(name)].clone();
                if self.program.sequence.definitions.iter().any(|definition| {
                    definition.name == *name && matches!(definition.term, Term::Store { .. })
                }) {
                    fact.work = Sum::default();
                    fact.local_span = Sum::default();
                    fact.expands = 0;
                    fact.lookup_after_expand = 0;
                }
                Ok(fact)
            }
            Term::Store { inner, .. } => {
                let separated = symbol_name(&level.name) != "host" && !level.collaborative;
                let producer = if separated && level.children.len() == 1 {
                    &level.children[0]
                } else {
                    level
                };
                let mut fact = self.term(inner, owner, producer, tile)?;
                // A collaborative scratch store does not close the enclosing
                // dispatch's partition; separated publication does.
                if separated {
                    fact.partition = None;
                }
                Ok(fact)
            }
            Term::Tile { n, inner, .. } => {
                let distribute = self
                    .analysis
                    .tilings
                    .get(owner)
                    .and_then(|kinds| kinds.get(*tile))
                    == Some(&Tiling::Distribute);
                *tile += 1;
                let mut fact = self.term(inner, owner, level, tile)?;
                if distribute {
                    fact.partition = Some(Partition {
                        driver: fact.size.clone(),
                        rows: *n,
                    });
                }
                fact.layers.push(Layer::Tile {
                    parents: Size::CeilDiv(Box::new(fact.size.clone()), *n),
                });
                Ok(fact)
            }
            Term::Lookup {
                provider,
                keys,
                inner,
            } => {
                let mut fact = self.term(inner, owner, level, tile)?;
                fact.lookup_after_expand = fact
                    .lookup_after_expand
                    .checked_add(fact.expands)
                    .ok_or_else(overflow)?;
                let size = fact.size.clone();
                fact.evaluate(&size, level, false)?;
                let mut keys: Vec<_> = keys.iter().map(symbol_name).collect();
                keys.sort();
                let provider = symbol_name(provider);
                let result = self.program.structures.physical.program.definitions.iter().find(|definition| {
                    matches!(&definition.operator, Operator::Probe(_, keyed) | Operator::Merge(_, keyed)
                        if symbol_name(&keyed.left) == fact.result && symbol_name(&keyed.right) == provider
                        && { let mut columns: Vec<_> = keyed.columns.iter().map(symbol_name).collect(); columns.sort(); columns == keys })
                }).map(|definition| symbol_name(&definition.name))
                    .unwrap_or_else(|| format!("lookup({provider},{keys:?},{})", fact.result));
                fact.layers.push(Layer::Bucket { parents: size });
                fact.size = Size::Unknown(result.clone());
                fact.result = result;
                Ok(fact)
            }
            Term::Map(name, inner) | Term::Compact(name, inner) => {
                // A membership lookup tests one parent position; it does not
                // enumerate (or guess the cardinality of) the matching bucket.
                let filtering_lookup = matches!(term, Term::Compact(_, inner)
                    if matches!(inner.as_ref(), Term::Lookup { .. }));
                let inner = if matches!(term, Term::Compact(_, _)) {
                    if let Term::Lookup { inner, .. } = inner.as_ref() {
                        inner.as_ref()
                    } else {
                        inner.as_ref()
                    }
                } else {
                    inner.as_ref()
                };
                let mut fact = self.term(inner, owner, level, tile)?;
                if filtering_lookup {
                    fact.lookup_after_expand = fact
                        .lookup_after_expand
                        .checked_add(fact.expands)
                        .ok_or_else(overflow)?;
                }
                let size = fact.size.clone();
                fact.evaluate(&size, level, false)?;
                let result = symbol_name(name);
                let mut current = result.clone();
                let mut changes_length = false;
                let mut nested = false;
                while current != fact.result {
                    let Some(definition) = self
                        .program
                        .structures
                        .physical
                        .program
                        .definitions
                        .iter()
                        .find(|definition| symbol_name(&definition.name) == current)
                    else {
                        break;
                    };
                    changes_length |= !matches!(
                        definition.operator,
                        Operator::Rename(_) | Operator::Project(_) | Operator::Unit(_)
                    );
                    nested |= matches!(
                        definition.operator,
                        Operator::Probe(_, _) | Operator::Merge(_, _)
                    );
                    let Some(input) = definition.operator.input() else {
                        break;
                    };
                    current = symbol_name(input);
                }
                if nested {
                    fact.layers.push(Layer::Bucket { parents: size });
                    fact.size = Size::Unknown(result.clone());
                } else if changes_length || matches!(term, Term::Compact(_, _)) {
                    fact.size = Size::Scan(result.clone());
                }
                fact.result = result;
                Ok(fact)
            }
            Term::Flatten(inner) => {
                let mut fact = self.term(inner, owner, level, tile)?;
                if let Some(Layer::Bucket { parents }) = fact.layers.pop() {
                    fact.expands = fact.expands.checked_add(1).ok_or_else(overflow)?;
                    // Offsets scan one length per parent; enumeration evaluates
                    // the resulting payload positions. Neither is output count.
                    fact.evaluate(&parents, level, true)?;
                    if let Size::Unknown(identity) = &fact.size {
                        fact.size = Size::Scan(identity.clone());
                    }
                    let size = fact.size.clone();
                    fact.evaluate(&size, level, false)?;
                }
                // Flattening a tile is only a view: no new row evaluations.
                Ok(fact)
            }
            Term::Group(name, inner) => {
                let mut fact = self.term(inner, owner, level, tile)?;
                let size = fact.size.clone();
                fact.evaluate(&size, level, true)?;
                fact.result = symbol_name(name);
                fact.size = Size::Scan(format!("group({})", fact.result));
                Ok(fact)
            }
            Term::Fold(_, inner) | Term::Canon(inner) => {
                let mut fact = self.term(inner, owner, level, tile)?;
                let size = fact.size.clone();
                fact.evaluate(&size, level, true)?;
                fact.size = if matches!(term, Term::Fold(_, _)) {
                    match fact.layers.last() {
                        None => Size::Constant(1),
                        Some(Layer::Tile { parents } | Layer::Bucket { parents, .. }) => {
                            parents.clone()
                        }
                    }
                } else {
                    Size::Scan(format!("canon({})", fact.result))
                };
                Ok(fact)
            }
            Term::Union(arms) => {
                let mut facts = arms.iter().map(|arm| self.term(arm, owner, level, tile));
                let mut fact = facts.next().expect("checked nonempty union")?;
                let mut spans = vec![fact.span.clone()];
                let mut local_spans = vec![fact.local_span.clone()];
                for next in facts {
                    let next = next?;
                    fact.expands = fact.expands.max(next.expands);
                    fact.lookup_after_expand = fact
                        .lookup_after_expand
                        .checked_add(next.lookup_after_expand)
                        .ok_or_else(overflow)?;
                    fact.size = Size::Add(Box::new(fact.size), Box::new(next.size));
                    fact.work.add(&next.work)?;
                    spans.push(next.span);
                    local_spans.push(next.local_span);
                }
                fact.span = Sum::maximum(spans);
                fact.local_span = Sum::maximum(local_spans);
                fact.partition = None;
                Ok(fact)
            }
            Term::Choose(arms) => {
                let mut facts = arms
                    .iter()
                    .map(|(_, arm)| self.term(arm, owner, level, tile));
                let mut fact = facts.next().expect("checked exhaustive choice")?;
                let mut works = vec![fact.work.clone()];
                let mut spans = vec![fact.span.clone()];
                let mut local_spans = vec![fact.local_span.clone()];
                for next in facts {
                    let next = next?;
                    fact.expands = fact.expands.max(next.expands);
                    fact.lookup_after_expand =
                        fact.lookup_after_expand.max(next.lookup_after_expand);
                    works.push(next.work);
                    spans.push(next.span);
                    local_spans.push(next.local_span);
                    if fact.size != next.size {
                        fact.size = Size::from_len(&self.analysis.lengths[owner], &fact.result);
                    }
                    if fact.partition != next.partition {
                        fact.partition = None;
                    }
                }
                fact.work = Sum::maximum(works);
                fact.span = Sum::maximum(spans);
                fact.local_span = Sum::maximum(local_spans);
                Ok(fact)
            }
        }
    }
}

fn count_stores(term: &Term) -> syn::Result<usize> {
    match term {
        Term::Store { inner, .. } => count_stores(inner)?.checked_add(1).ok_or_else(overflow),
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::Lookup { inner, .. } => count_stores(inner),
        Term::Union(arms) => arms.iter().try_fold(0usize, |sum, arm| {
            sum.checked_add(count_stores(arm)?).ok_or_else(overflow)
        }),
        Term::Choose(arms) => arms.iter().try_fold(0usize, |sum, (_, arm)| {
            sum.checked_add(count_stores(arm)?).ok_or_else(overflow)
        }),
        Term::Rows(_) | Term::Ref(_) => Ok(0),
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Constant(n) => write!(f, "{n}"),
            Self::Input(name) => write!(f, "len({name})"),
            Self::Scan(name) => write!(f, "scan({name})"),
            Self::Unknown(name) => write!(f, "unknown({name})"),
            Self::Add(a, b) => write!(f, "({a}+{b})"),
            Self::CeilDiv(inner, n) => write!(f, "ceil({inner}/{n})"),
        }
    }
}

impl fmt::Display for Sum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("0");
        }
        for (index, (atom, coefficient)) in self.0.iter().enumerate() {
            if index != 0 {
                f.write_str(" + ")?;
            }
            if *coefficient != 1 || matches!(atom, Atom::One) {
                write!(f, "{coefficient}")?;
            }
            if *coefficient != 1 && !matches!(atom, Atom::One) {
                f.write_str("*")?;
            }
            match atom {
                Atom::One => {}
                Atom::Positions(size) => write!(f, "{size}")?,
                Atom::Depth {
                    size,
                    partition,
                    lanes,
                    scan,
                } => {
                    f.write_str(if *scan { "scan_depth(" } else { "depth(" })?;
                    if let Some(partition) = partition {
                        write!(f, "maxpart({size};{};{})", partition.driver, partition.rows)?;
                    } else {
                        write!(f, "{size}")?;
                    }
                    write!(f, ";lanes={lanes})")?;
                }
                Atom::Max(branches) => {
                    f.write_str("max(")?;
                    for (index, branch) in branches.iter().enumerate() {
                        if index != 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{branch}")?;
                    }
                    f.write_str(")")?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn sequential() -> Machine {
        Machine {
            layout: None,
            name: syn::parse_quote!(target),
            items: 1,
            root: LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(host),
                collaborative: false,
                dynamic: false,
                width: None,
                children: Vec::new(),
            },
        }
    }

    fn program(body: proc_macro2::TokenStream) -> Program {
        syn::parse2(quote! {
            physical {
                relation R(a:i32);
                r = pipe canon rename R@0 {a -> a};
                return r as answer(a);
            }
            structures { r host scan = Reader []; r canon = Canon; }
            sequence { #body }
        })
        .unwrap()
    }

    #[test]
    fn only_growing_flattens_contribute_lookup_inversions() {
        let source = program(quote!(input = rows r;));
        let analysis = cq_lang_sequence::contract::check(&source).unwrap();
        let evaluator = Evaluator {
            program: &source,
            analysis: &analysis,
            facts: BTreeMap::new(),
        };
        let level = sequential().root;
        let delayed: Term = syn::parse_quote!(flatten lookup r [a] flatten lookup r [a] rows r);
        let early: Term = syn::parse_quote!(flatten flatten lookup r [a] lookup r [a] rows r);
        let partition: Term = syn::parse_quote!(flatten lookup r [a] flatten tile 8 rows r);
        assert_eq!(
            evaluator
                .term(&delayed, "test", &level, &mut 0)
                .unwrap()
                .lookup_after_expand,
            1
        );
        assert_eq!(
            evaluator
                .term(&early, "test", &level, &mut 0)
                .unwrap()
                .lookup_after_expand,
            0
        );
        assert_eq!(
            evaluator
                .term(&partition, "test", &level, &mut 0)
                .unwrap()
                .lookup_after_expand,
            0
        );
    }

    #[test]
    fn aliases_and_partition_views_do_not_add_evaluated_positions() {
        let plain = program(quote!(a = store rows r; output reserve;));
        let aliased = program(quote!(source = rows r; a = store source; output reserve;));
        let tiled = program(quote!(a = store flatten tile 8 rows r; output reserve;));
        let expected = cost(&plain, &sequential()).unwrap();
        assert_eq!(cost(&aliased, &sequential()).unwrap(), expected);
        assert_eq!(cost(&tiled, &sequential()).unwrap(), expected);
    }

    #[test]
    fn a_fold_evaluates_its_input_not_its_single_output() {
        let folded = program(quote!(a = store fold count rows r; output reserve;));
        let evaluated = cost(&folded, &sequential()).unwrap();
        let mut expected = Sum::positions(&Size::Input("r".into())).unwrap();
        expected.add(&expected.clone()).unwrap();
        assert_eq!(evaluated.work, expected);
        assert_eq!(evaluated.span, expected);
    }

    #[test]
    fn independent_definitions_take_maximum_span_not_sum() {
        let independent = program(quote!(
            a = store rows r;
            b = store rows r;
            output reserve;
        ));
        let evaluated = cost(&independent, &sequential()).unwrap();
        let one = Sum::positions(&Size::Input("r".into())).unwrap();
        let mut two = one.clone();
        two.add(&one).unwrap();
        assert_eq!(evaluated.work, two);
        assert_eq!(evaluated.span, one);
        assert_eq!(evaluated.stores, 2);
    }

    #[test]
    fn unused_pure_views_are_not_evaluated() {
        let plain = program(quote!(a = store rows r; output reserve;));
        let unused = program(quote!(unused = map r rows r; a = store rows r; output reserve;));
        assert_eq!(
            cost(&plain, &sequential()).unwrap(),
            cost(&unused, &sequential()).unwrap()
        );
    }

    #[test]
    fn count_records_producer_replay_but_not_materialized_dependencies() {
        let reserve =
            program(quote!(source = store rows r; a = store map r source; output reserve;));
        let count = program(quote!(source = store rows r; a = store map r source; output count;));
        let first = cost(&reserve, &sequential()).unwrap();
        let second = cost(&count, &sequential()).unwrap();
        let mut expected = first.work.clone();
        expected
            .add(&Sum::positions(&Size::Input("r".into())).unwrap())
            .unwrap();
        assert_eq!(second.work, expected);
        assert!(first.better_than(&second));
    }

    fn gpu() -> Machine {
        let mut machine = sequential();
        machine.items = 4;
        machine.root.children.push(LevelNode {
            shared_bytes: None,
            name: syn::parse_quote!(device),
            collaborative: false,
            dynamic: true,
            width: None,
            children: vec![LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(block),
                collaborative: true,
                dynamic: false,
                width: Some(8),
                children: vec![LevelNode {
                    shared_bytes: None,
                    name: syn::parse_quote!(lane),
                    collaborative: false,
                    dynamic: false,
                    width: None,
                    children: Vec::new(),
                }],
            }],
        });
        machine
    }

    fn join(body: proc_macro2::TokenStream) -> Program {
        syn::parse2(quote! {
            physical {
                relation L(a:i32); relation R(a:i32,b:i32);
                l = pipe rename L@0 {a -> a};
                r = let rename R@1 {a -> a,b -> b};
                j = pipe canon probe first [a] l,r;
                return j as answer(a,b);
            }
            structures {
                l host scan = Reader [];
                r host scan = Reader [];
                r = Provider [a];
                j canon = Canon;
            }
            sequence {
                left = store rows l;
                right = store rows r;
                result = #body;
                output reserve;
            }
            graded target { left at host; right at host; result at device; }
        })
        .unwrap()
    }

    #[test]
    fn segmented_b_and_r_like_schedules_dominate_single_part_a() {
        let a = join(quote!(store canon flatten lookup r [a] left));
        let b = join(quote!(store canon flatten flatten lookup r [a] tile 8 left));
        let r = join(quote!(
            store canon flatten flatten tile 32 items 4
                store flatten lookup r [a] tile 8 left
        ));
        let a = cost(&a, &gpu()).unwrap();
        for candidate in [b, r] {
            let candidate = cost(&candidate, &gpu()).unwrap();
            assert_eq!(
                candidate.work, a.work,
                "partitioning preserves evaluated positions"
            );
            assert!(candidate.better_than(&a), "{candidate} must dominate {a}");
            assert!(candidate.leq(&a));
        }
    }

    #[test]
    fn named_views_preserve_the_enclosing_publication_partition() {
        let inline = join(quote!(store canon flatten flatten lookup r [a] tile 8 left));
        let mut named = join(quote!(store canon flatten expanded));
        named.sequence.definitions.insert(
            2,
            cq_lang_sequence::Definition {
                name: syn::parse_quote!(partitioned),
                term: syn::parse_quote!(tile 8 left),
            },
        );
        named.sequence.definitions.insert(
            3,
            cq_lang_sequence::Definition {
                name: syn::parse_quote!(expanded),
                term: syn::parse_quote!(flatten lookup r [a] partitioned),
            },
        );
        named.graded.as_mut().unwrap().grades.extend([
            cq_lang_sequence::Grade {
                name: syn::parse_quote!(partitioned),
                level: vec![syn::parse_quote!(device), syn::parse_quote!(block)],
            },
            cq_lang_sequence::Grade {
                name: syn::parse_quote!(expanded),
                level: vec![syn::parse_quote!(device), syn::parse_quote!(block)],
            },
        ]);
        assert_eq!(
            cost(&named, &gpu()).unwrap(),
            cost(&inline, &gpu()).unwrap()
        );
    }

    #[test]
    fn extra_strip_store_without_span_improvement_loses_the_tie() {
        let plain = join(quote!(store canon flatten flatten lookup r [a] tile 8 left));
        let segmented = join(quote!(
            store canon flatten flatten tile 32 items 4
                store flatten lookup r [a] tile 8 left
        ));
        let plain = cost(&plain, &gpu()).unwrap();
        let segmented = cost(&segmented, &gpu()).unwrap();
        assert_eq!(plain.work, segmented.work);
        assert_eq!(plain.span, segmented.span);
        assert!(plain.better_than(&segmented));
    }

    #[test]
    fn item_count_does_not_invent_lanes_and_nested_stores_are_counted() {
        let one = join(quote!(
            store canon flatten flatten tile 32 items 1
                store flatten lookup r [a] tile 8 left
        ));
        let four = join(quote!(
            store canon flatten flatten tile 32 items 4
                store flatten lookup r [a] tile 8 left
        ));
        let one = cost(&one, &gpu()).unwrap();
        let four = cost(&four, &gpu()).unwrap();
        assert_eq!(one, four);
        assert_eq!(one.stores, 4);
        assert!(one.to_string().contains("maxpart("));
        assert!(one.to_string().contains("scan(j)"));
    }

    #[test]
    fn unknown_lengths_remain_distinct_and_do_not_become_capacities() {
        assert_eq!(
            Size::from_len(&Len::Unknown(Some(1024)), "bucket"),
            Size::Unknown("bucket".into())
        );
        let a = Sum::positions(&Size::Unknown("a".into())).unwrap();
        let b = Sum::positions(&Size::Unknown("b".into())).unwrap();
        assert_eq!(a.compare(&b), None);
        let a = Cost {
            work: a,
            span: Sum::default(),
            lookup_after_expand: 0,
            stores: 0,
            dispatches: 0,
        };
        let b = Cost {
            work: b,
            span: Sum::default(),
            lookup_after_expand: 0,
            stores: 0,
            dispatches: 0,
        };
        assert!(!a.leq(&b));
        assert!(!a.better_than(&b));
    }

    #[test]
    fn coefficient_comparison_does_not_cross_multiply_or_overflow() {
        let a = Sum::atom(Atom::Positions(Size::Input("n".into())), u128::MAX);
        let b = Sum::atom(Atom::Positions(Size::Input("n".into())), u128::MAX - 1);
        assert_eq!(b.compare(&a), Some(Ordering::Less));
        let mut full = a;
        assert!(
            full.add(&Sum::atom(Atom::Positions(Size::Input("n".into())), 1))
                .is_err()
        );
    }

    #[test]
    fn incompatible_part_boundaries_are_not_assumed_to_reduce_skew() {
        let depth = |rows| {
            Sum::atom(
                Atom::Depth {
                    size: Size::Scan("payload".into()),
                    partition: Some(Partition {
                        driver: Size::Input("driver".into()),
                        rows,
                    }),
                    lanes: 8,
                    scan: true,
                },
                1,
            )
        };
        assert_eq!(depth(2).compare(&depth(3)), None);
        assert_eq!(depth(2).compare(&depth(4)), Some(Ordering::Less));
    }

    #[test]
    fn equal_work_uses_span_then_stores_then_dispatches_and_stable_ties() {
        let base = Cost {
            work: Sum::default(),
            span: Sum::default(),
            lookup_after_expand: 0,
            stores: 1,
            dispatches: 1,
        };
        let mut later = base.clone();
        later.dispatches += 1;
        assert!(base.better_than(&later));
        later = base.clone();
        later.stores += 1;
        later.dispatches = 0;
        assert!(base.better_than(&later));
        later = base.clone();
        later.span = Sum::atom(Atom::One, 1);
        later.stores = 0;
        assert!(base.better_than(&later));
        later = base.clone();
        later.lookup_after_expand = 1;
        later.stores = 0;
        later.dispatches = 0;
        assert!(base.better_than(&later));
        assert!(base.leq(&base));
        assert!(!base.better_than(&base));
    }
}
