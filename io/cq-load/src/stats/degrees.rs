//! Degree facts between the evidence schema and the runtime.
//!
//! `DegreeCollector` is the exact reference (PS-Collect): requested degrees
//! over a copy of the rows, with shared, bounded-lifetime sorted runs. Tests
//! and oracles use it. A run keeps its degrees in the runtime as it admits
//! rows (PS-Admit): `admitted_degrees` hands it the requests and
//! `bind_degrees` turns either's values into facts.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock, atomic::{AtomicUsize, Ordering}};

use cq_evidence::{Claim, DegreeEvidence, DegreeRequest, Fact, Provenance, Rational, Selection, Source};

use super::error;
use crate::Result;

/// A relation named by this collector's requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelationId(usize);

struct Relation {
    columns: Vec<(usize, Vec<i64>)>,
    rounds: Vec<u32>,
    short: bool,
}

/// Retains only columns needed to answer a fixed set of degree requests.
pub struct DegreeCollector {
    requests: Vec<DegreeRequest>,
    names: BTreeMap<String, RelationId>,
    relations: Vec<Relation>,
}

fn add(a: u64, b: u64) -> u64 { a.checked_add(b).expect("degree count overflow") }
fn mul(a: u64, b: u64) -> u64 { a.checked_mul(b).expect("degree count overflow") }
fn count(n: usize) -> u64 { u64::try_from(n).expect("degree count overflow") }
fn normalized(n: u64, d: u64) -> (u64, u64) { if d == 0 { (0, 1) } else { (n, d) } }

impl DegreeCollector {
    /// The requests fix which relations and which of their columns are kept.
    pub fn new(requests: Vec<DegreeRequest>) -> Self {
        let mut needed: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut keep = |selection: &Selection, columns: &[usize]| {
            let set = needed.entry(selection.relation.clone()).or_default();
            set.extend(columns.iter().copied());
            set.extend(selection.fixed.iter().map(|&(column, _)| column));
        };
        for request in &requests {
            match request {
                DegreeRequest::Distinct { rows, columns } => keep(rows, columns),
                DegreeRequest::Joined { reach, binder, from, read, bound, column, .. } => {
                    keep(binder, from);
                    keep(read, bound);
                    keep(read, &[*column]);
                    if let Some(reach) = reach {
                        keep(&reach.changing, &reach.columns);
                        keep(binder, &reach.binder);
                    }
                }
                DegreeRequest::Visible { changing, relation } => {
                    keep(&Selection { relation: changing.clone(), fixed: Vec::new() }, &[]);
                    keep(&Selection { relation: relation.clone(), fixed: Vec::new() }, &[]);
                }
                DegreeRequest::Rounds { relation } => keep(&Selection { relation: relation.clone(), fixed: Vec::new() }, &[]),
            }
        }
        let mut names = BTreeMap::new();
        let mut relations = Vec::with_capacity(needed.len());
        for (name, columns) in needed {
            names.insert(name, RelationId(relations.len()));
            relations.push(Relation { columns: columns.into_iter().map(|column| (column, Vec::new())).collect(), rounds: Vec::new(), short: false });
        }
        Self { requests, names, relations }
    }

    /// Returns `None` when no request needs this relation.
    pub fn relation(&self, relation: &str) -> Option<RelationId> { self.names.get(relation).copied() }

    /// Retains one row and its appearance round; short rows are rejected at finish.
    pub fn push(&mut self, relation: RelationId, round: u32, row: &[i64]) {
        let relation = &mut self.relations[relation.0];
        relation.rounds.push(round);
        for (column, values) in &mut relation.columns {
            match row.get(*column) {
                Some(&value) => values.push(value),
                None => { relation.short = true; values.push(0); }
            }
        }
    }

    /// Returns the number of rows delivered for this relation.
    pub fn rows(&self, relation: RelationId) -> usize { self.relations[relation.0].rounds.len() }

    /// Computes exact values in request order on up to `threads` scoped threads.
    pub fn finish(self, threads: usize) -> Result<Vec<(DegreeRequest, u64, u64)>> {
        for (name, id) in &self.names {
            if self.relations[id.0].short { return Err(error(format!("short row in degree relation {name}"))); }
        }
        let mut plan = Plan::default();
        // Join tables first, then distinct counts from the widest: a distinct
        // count reads any table that starts with its columns.
        let width = |request: &DegreeRequest| match request { DegreeRequest::Distinct { columns, .. } => Some(std::cmp::Reverse(columns.len())), _ => None };
        let mut planning: Vec<_> = (0..self.requests.len()).collect();
        planning.sort_by_key(|&i| width(&self.requests[i]));
        let mut work: Vec<Option<Work>> = self.requests.iter().map(|_| None).collect();
        for i in planning { work[i] = Some(plan.request(&self.requests[i])?); }
        let work: Vec<Work> = work.into_iter().map(|work| work.expect("every request planned")).collect();
        plan.narrow();
        let slots: Vec<_> = plan.kinds.into_iter().zip(plan.users).map(|(kind, users)| Slot {
            kind, users: AtomicUsize::new(users), value: Mutex::new(None),
        }).collect();
        let cache = Cache { slots, names: &self.names, relations: &self.relations };
        let mut order: Vec<_> = (0..work.len()).collect();
        order.sort_unstable_by_key(|&i| work[i].locality());
        let next = AtomicUsize::new(0);
        let results: Vec<_> = (0..work.len()).map(|_| OnceLock::new()).collect();
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1).min(work.len()) {
                let (cache, work, order, next, results) = (&cache, &work, &order, &next, &results);
                scope.spawn(move || while let Some(&index) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let value = work[index].compute(cache);
                    results[index].set(value).expect("degree request computed twice");
                    work[index].dependencies(|id| cache.release(id));
                });
            }
        });
        Ok(self.requests.into_iter().zip(results).map(|(request, value)| {
            let (numerator, denominator) = value.into_inner().expect("degree request not computed");
            (request, numerator, denominator)
        }).collect())
    }
}

/// PS-Admit: the runtime's accumulator of evidence requests. The runtime
/// names a relation by its position; `relations` are the program's
/// relations in that order, each with its arity.
pub fn admitted_degrees(requests: &[DegreeRequest], relations: &[(&str, usize)]) -> Result<cq_storage::statistics::Degrees> {
    use cq_storage::statistics::{Reach, Request, Rows};
    let at = |name: &str| relations.iter().position(|(held, _)| *held == name).ok_or_else(|| error(format!("no relation {name} for a degree request")));
    let rows = |selection: &Selection| Ok::<_, crate::Error>(Rows { relation: at(&selection.relation)?, fixed: selection.fixed.clone() });
    let requests = requests.iter().map(|request| Ok(match request {
        DegreeRequest::Distinct { rows: selection, columns } => Request::Distinct { rows: rows(selection)?, columns: columns.clone() },
        DegreeRequest::Visible { changing, relation } => Request::Visible { changing: at(changing)?, relation: at(relation)? },
        DegreeRequest::Rounds { relation } => Request::Rounds { relation: at(relation)? },
        DegreeRequest::Joined { reach, binder, from, read, bound, column, staged } => Request::Joined {
            reach: reach.as_ref().map(|reach| Ok::<_, crate::Error>(Reach { changing: rows(&reach.changing)?, columns: reach.columns.clone(), binder: reach.binder.clone() })).transpose()?,
            binder: rows(binder)?, from: from.clone(), read: rows(read)?, bound: bound.clone(), column: *column, staged: *staged,
        },
    })).collect::<Result<Vec<_>>>()?;
    let arities: Vec<usize> = relations.iter().map(|(_, arity)| *arity).collect();
    cq_storage::statistics::Degrees::new(&requests, &arities).map_err(error)
}

/// Binds exact collected values to their request identities and supplied origin.
pub fn bind_degrees(values: Vec<(DegreeRequest, u64, u64)>, dataset_hash: &str, source: Source, claim: Claim, provenance: Provenance) -> Vec<DegreeEvidence> {
    values.into_iter().map(|(request, numerator, denominator)| {
        let (numerator, denominator) = normalized(numerator, denominator);
        let value = Fact {
            id: request.id(), source, claim,
            value: Some(Rational { numerator: numerator.to_string(), denominator: denominator.to_string() }),
            lower: None, upper: None, dataset_hash: dataset_hash.to_owned(), provenance: provenance.clone(), unit: request.unit(),
        };
        DegreeEvidence { request, value }
    }).collect()
}

// These maps describe requests, never data keys. All data keys live in flat runs.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Projection(Selection, Vec<usize>),
    Sequence(Selection, Vec<usize>, usize),
}
enum Kind {
    Projection(Selection, Vec<usize>),
    /// The first `keep` columns of a wider projection of the same rows.
    Narrowed { source: usize, keep: usize },
    Sequence { projection: usize, bound: usize },
}
#[derive(Default)]
struct Plan {
    ids: BTreeMap<Key, usize>,
    kinds: Vec<Kind>,
    users: Vec<usize>,
}
impl Plan {
    fn projection(&mut self, selection: &Selection, columns: &[usize]) -> usize {
        let key = Key::Projection(selection.clone(), columns.to_vec());
        if let Some(&id) = self.ids.get(&key) { return id; }
        self.insert(key, Kind::Projection(selection.clone(), columns.to_vec()))
    }
    /// The table and leading columns a distinct count reads: the narrowest
    /// projection of these rows that starts with the columns in any order.
    fn distinct(&mut self, selection: &Selection, columns: &[usize]) -> (usize, usize) {
        let mut wanted = columns.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        let found = self.ids.iter().filter_map(|(key, &id)| match key {
            Key::Projection(rows, columns) if rows == selection && columns.len() >= wanted.len() => {
                let mut start = columns[..wanted.len()].to_vec();
                start.sort_unstable();
                (start == wanted).then_some((columns.len(), id))
            }
            _ => None,
        }).min();
        let id = found.map_or_else(|| self.projection(selection, &wanted), |(_, id)| id);
        (id, wanted.len())
    }
    /// A projection whose columns start a wider projection of the same rows
    /// is read off the narrowest such table, not sorted from the relation.
    fn narrow(&mut self) {
        let projections: Vec<_> = self.ids.iter().filter_map(|(key, &id)| match key {
            Key::Projection(selection, columns) => Some((selection, columns, id)),
            Key::Sequence(..) => None,
        }).collect();
        for &(selection, columns, id) in &projections {
            let wider = projections.iter()
                .filter(|(rows, wide, _)| *rows == selection && wide.len() > columns.len() && wide.starts_with(columns))
                .min_by_key(|(_, wide, _)| wide.len());
            if let Some(&(_, _, source)) = wider {
                self.kinds[id] = Kind::Narrowed { source, keep: columns.len() };
                self.users[source] += 1;
            }
        }
    }
    fn insert(&mut self, key: Key, kind: Kind) -> usize {
        let id = self.kinds.len();
        self.ids.insert(key, id);
        self.kinds.push(kind);
        self.users.push(0);
        id
    }
    fn sequence(&mut self, selection: &Selection, bound: &[usize], column: usize) -> usize {
        let key = Key::Sequence(selection.clone(), bound.to_vec(), column);
        if let Some(&id) = self.ids.get(&key) { return id; }
        let mut columns = bound.to_vec();
        columns.push(column);
        let projection = self.projection(selection, &columns);
        self.users[projection] += 1; // The sequence builder is a single user.
        self.insert(key, Kind::Sequence { projection, bound: bound.len() })
    }
    fn request(&mut self, request: &DegreeRequest) -> Result<Work> {
        let work = match request {
            DegreeRequest::Distinct { rows, columns } => { let (id, keep) = self.distinct(rows, columns); Work::Distinct(id, keep) }
            DegreeRequest::Visible { changing, relation } => Work::Visible(
                self.projection(&Selection { relation: changing.clone(), fixed: Vec::new() }, &[]),
                self.projection(&Selection { relation: relation.clone(), fixed: Vec::new() }, &[]),
            ),
            DegreeRequest::Rounds { relation } => Work::Rounds(self.projection(&Selection { relation: relation.clone(), fixed: Vec::new() }, &[])),
            DegreeRequest::Joined { reach, binder, from, read, bound, column, staged } => {
                if from.len() != bound.len() { return Err(error("joined degree key lengths differ")); }
                let sequence = self.sequence(read, bound, *column);
                let (binder, changing, reach_width) = if let Some(reach) = reach {
                    if reach.columns.len() != reach.binder.len() { return Err(error("degree reach key lengths differ")); }
                    let mut columns = reach.binder.clone();
                    columns.extend_from_slice(from);
                    (self.projection(binder, &columns), Some(self.projection(&reach.changing, &reach.columns)), reach.columns.len())
                } else { (self.projection(binder, from), None, 0) };
                Work::Joined { binder, sequence, changing, reach_width, staged: *staged }
            }
        };
        work.dependencies(|id| self.users[id] += 1);
        Ok(work)
    }
}

struct Slot {
    kind: Kind,
    users: AtomicUsize,
    value: Mutex<Option<Arc<Table>>>,
}
struct Cache<'a> {
    slots: Vec<Slot>,
    names: &'a BTreeMap<String, RelationId>,
    relations: &'a [Relation],
}
impl Cache<'_> {
    /// The table, built here under its lock unless it exists.
    fn get(&self, id: usize) -> Arc<Table> {
        let mut value = self.slots[id].value.lock().unwrap();
        if let Some(value) = value.as_ref() { return Arc::clone(value); }
        let table = match &self.slots[id].kind {
            Kind::Projection(selection, columns) => {
                let relation = &self.relations[self.names[&selection.relation].0];
                Table::projection(relation, selection, columns)
            }
            Kind::Narrowed { source, keep } => {
                let wide = self.get(*source);
                let result = Table::narrowed(&wide, *keep);
                drop(wide);
                self.release(*source);
                result
            }
            Kind::Sequence { projection, bound } => {
                let source = self.get(*projection);
                let result = Table::sequence(&source, *bound);
                drop(source);
                self.release(*projection);
                result
            }
        };
        let table = Arc::new(table);
        *value = Some(Arc::clone(&table));
        table
    }
    fn release(&self, id: usize) {
        if self.slots[id].users.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.slots[id].value.lock().unwrap().take();
        }
    }
}

// A row is [key..., round]. Counts collapse identical rows without allocating
// anything per key. Sequence counts instead count values first seen that round.
// Counts are cumulative, so arbitrary key-group cardinalities take constant time.
struct Table {
    width: usize,
    data: Vec<i64>,
    counts: Vec<u64>,
    total: u64,
}
impl Table {
    fn len(&self) -> usize { self.counts.len() }
    fn row(&self, at: usize) -> &[i64] { &self.data[at * self.width..(at + 1) * self.width] }
    fn round(&self, at: usize) -> i64 { self.data[(at + 1) * self.width - 1] }
    fn end(&self, start: usize, columns: usize) -> usize {
        let key = &self.row(start)[..columns];
        let mut end = start + 1;
        while end < self.len() && &self.row(end)[..columns] == key { end += 1; }
        end
    }
    fn lookup(&self, key: &[i64]) -> (usize, usize) {
        let (mut lo, mut hi) = (0, self.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if &self.row(mid)[..self.width - 1] < key { lo = mid + 1; } else { hi = mid; }
        }
        if lo == self.len() || &self.row(lo)[..self.width - 1] != key { (lo, lo) }
        else { (lo, self.end(lo, self.width - 1)) }
    }
    fn prefix(&self, end: usize) -> u64 { if end == 0 { 0 } else { self.counts[end - 1] } }
    fn sum(&self, start: usize, end: usize) -> u64 { self.prefix(end) - self.prefix(start) }
    fn projection(relation: &Relation, selection: &Selection, columns: &[usize]) -> Self {
        let column = |c| relation.columns.binary_search_by_key(&c, |&(c, _)| c).expect("unretained degree column");
        let fixed: Vec<_> = selection.fixed.iter().map(|&(c, v)| (&relation.columns[column(c)].1, v)).collect();
        let columns: Vec<_> = columns.iter().map(|&c| &relation.columns[column(c)].1).collect();
        let width = columns.len().checked_add(1).expect("degree width overflow");
        let rows = if fixed.is_empty() { relation.rounds.len() } else { 0 };
        let selected = |at: usize| fixed.iter().all(|&(values, value)| values[at] == value);
        // Records are built as arrays and sorted where they are built.
        macro_rules! records {
            ($($cells:literal),*) => { match width {
                $($cells => {
                    let mut records = Vec::<[i64; $cells]>::with_capacity(rows);
                    for (at, &round) in relation.rounds.iter().enumerate() {
                        if selected(at) { records.push(std::array::from_fn(|cell| columns.get(cell).map_or(i64::from(round), |values| values[at]))); }
                    }
                    records.sort_unstable();
                    records.into_flattened()
                })*
                _ => {
                    let mut data = Vec::with_capacity(rows * width);
                    for (at, &round) in relation.rounds.iter().enumerate() {
                        if !selected(at) { continue; }
                        for values in &columns { data.push(values[at]); }
                        data.push(i64::from(round));
                    }
                    if data.len() / width >= 2 { merge_records(&mut data, width); }
                    data
                }
            } };
        }
        Self::counted(width, records!(1, 2, 3, 4, 5, 6, 7, 8))
    }
    fn narrowed(source: &Self, keep: usize) -> Self {
        let (mut data, mut counts) = (Vec::new(), Vec::new());
        let mut rounds: Vec<(i64, u64)> = Vec::new();
        let mut at = 0;
        while at < source.len() {
            let end = source.end(at, keep);
            rounds.clear();
            rounds.extend((at..end).map(|row| (source.round(row), source.sum(row, row + 1))));
            rounds.sort_unstable();
            let mut next = 0;
            while next < rounds.len() {
                let (round, mut rows) = (rounds[next].0, 0);
                while next < rounds.len() && rounds[next].0 == round { rows = add(rows, rounds[next].1); next += 1; }
                data.extend_from_slice(&source.row(at)[..keep]);
                data.push(round);
                counts.push(add(counts.last().copied().unwrap_or(0), rows));
            }
            at = end;
        }
        Self { width: keep + 1, data, counts, total: source.total }
    }
    fn sequence(source: &Self, bound: usize) -> Self {
        let width = bound + 1;
        let mut data = Vec::new();
        let mut at = 0;
        while at < source.len() {
            // Source order is bound, value, round: the first row is the earliest.
            data.extend_from_slice(&source.row(at)[..bound]);
            data.push(source.round(at));
            at = source.end(at, bound + 1);
        }
        sort_records(&mut data, width);
        Self::counted(width, data)
    }
    /// Sorted records with equal ones collapsed into cumulative counts.
    fn counted(width: usize, mut data: Vec<i64>) -> Self {
        let rows = data.len() / width;
        let mut counts = Vec::new();
        let mut written = 0;
        let mut at = 0;
        while at < rows {
            let mut end = at + 1;
            while end < rows && data[at * width..(at + 1) * width] == data[end * width..(end + 1) * width] { end += 1; }
            data.copy_within(at * width..(at + 1) * width, written * width);
            counts.push(add(counts.last().copied().unwrap_or(0), count(end - at)));
            written += 1;
            at = end;
        }
        data.truncate(written * width);
        // Release excess capacity after duplicate-heavy runs, before caching.
        data.shrink_to_fit();
        counts.shrink_to_fit();
        Self { width, data, counts, total: count(rows) }
    }
}

// Records up to eight cells wide sort as arrays with the standard unstable
// sort: sequential, in place but for one copy, nothing allocated per key.
fn sort_records(data: &mut Vec<i64>, width: usize) {
    macro_rules! fixed {
        ($($cells:literal),*) => { match width {
            1 => data.sort_unstable(),
            $($cells => {
                let mut records: Vec<[i64; $cells]> = data.chunks_exact($cells).map(|record| record.try_into().expect("a whole record")).collect();
                records.sort_unstable();
                data.copy_from_slice(records.as_flattened());
            })*
            _ => merge_records(data, width),
        } };
    }
    if data.len() / width >= 2 { fixed!(2, 3, 4, 5, 6, 7, 8); }
}

// Wider records: bottom-up merging over the flat buffer with one scratch
// buffer. Both merge inputs are read sequentially.
fn merge_records(data: &mut Vec<i64>, width: usize) {
    let rows = data.len() / width;
    let mut scratch = vec![0; data.len()];
    let mut run = 1;
    while run < rows {
        let mut start = 0;
        while start < rows {
            let mid = start.saturating_add(run).min(rows);
            let end = mid.saturating_add(run).min(rows);
            let (mut left, mut right, mut out) = (start, mid, start);
            while left < mid && right < end {
                let chosen = if data[left * width..(left + 1) * width] <= data[right * width..(right + 1) * width] {
                    let chosen = left; left += 1; chosen
                } else { let chosen = right; right += 1; chosen };
                scratch[out * width..(out + 1) * width].copy_from_slice(&data[chosen * width..(chosen + 1) * width]);
                out += 1;
            }
            let (tail, tail_end) = if left < mid { (left, mid) } else { (right, end) };
            scratch[out * width..end * width].copy_from_slice(&data[tail * width..tail_end * width]);
            start = end;
        }
        std::mem::swap(data, &mut scratch);
        run = run.saturating_mul(2);
    }
}

enum Work {
    /// The distinct values of a table's leading columns.
    Distinct(usize, usize),
    Visible(usize, usize),
    /// The rounds of a table without columns: a row a round.
    Rounds(usize),
    Joined { binder: usize, sequence: usize, changing: Option<usize>, reach_width: usize, staged: bool },
}
impl Work {
    // Deduplicate direct uses (e.g. Visible of a relation against itself).
    fn dependencies(&self, mut visit: impl FnMut(usize)) {
        match *self {
            Self::Distinct(id, _) | Self::Rounds(id) => visit(id),
            Self::Visible(a, b) => { visit(a); if a != b { visit(b); } }
            Self::Joined { binder, sequence, changing, .. } => {
                visit(sequence);
                if binder != sequence { visit(binder); }
                if let Some(id) = changing { if id != sequence && id != binder { visit(id); } }
            }
        }
    }
    fn locality(&self) -> (usize, usize, usize) {
        match *self {
            Self::Distinct(id, _) | Self::Rounds(id) => (id, id, id),
            Self::Visible(a, b) => (a, b, a),
            Self::Joined { binder, sequence, changing, .. } => (sequence, changing.unwrap_or(binder), binder),
        }
    }
    fn compute(&self, cache: &Cache<'_>) -> (u64, u64) {
        match *self {
            Self::Distinct(id, keep) => {
                let table = cache.get(id);
                let (mut at, mut distinct) = (0, 0);
                while at < table.len() { distinct = add(distinct, 1); at = table.end(at, keep); }
                (distinct, 1)
            }
            Self::Rounds(id) => (cache.get(id).len() as u64, 1),
            Self::Visible(a, b) => {
                let (changing, relation) = (cache.get(a), cache.get(b));
                normalized(weighted(&changing, 0, changing.len(), &relation, 0, relation.len()), mul(changing.total, relation.total))
            }
            Self::Joined { binder, sequence, changing, reach_width, staged } => {
                let (binder, sequence) = (cache.get(binder), cache.get(sequence));
                if let Some(changing) = changing {
                    reached(&binder, &sequence, &cache.get(changing), reach_width, staged)
                } else {
                    let (mut at, mut total) = (0, 0);
                    while at < binder.len() {
                        let end = binder.end(at, binder.width - 1);
                        let (lo, hi) = sequence.lookup(&binder.row(at)[..binder.width - 1]);
                        let met = if staged { weighted(&binder, at, end, &sequence, lo, hi) }
                            else { mul(binder.sum(at, end), sequence.sum(lo, hi)) };
                        total = add(total, met);
                        at = end;
                    }
                    normalized(total, binder.total)
                }
            }
        }
    }
}

// Merge distinct rounds and their multiplicities against first-round counts.
fn weighted(changing: &Table, start: usize, end: usize, sequence: &Table, mut lo: usize, hi: usize) -> u64 {
    let (mut visible, mut total) = (0, 0);
    for at in start..end {
        while lo < hi && sequence.round(lo) <= changing.round(at) {
            visible = add(visible, sequence.sum(lo, lo + 1));
            lo += 1;
        }
        total = add(total, mul(changing.sum(at, at + 1), visible));
    }
    total
}

fn reached(binder: &Table, sequence: &Table, changing: &Table, reach_width: usize, staged: bool) -> (u64, u64) {
    let (mut b, mut x, mut numerator, mut denominator) = (0, 0, 0, 0);
    while b < binder.len() && x < changing.len() {
        match binder.row(b)[..reach_width].cmp(&changing.row(x)[..reach_width]) {
            std::cmp::Ordering::Less => b = binder.end(b, reach_width),
            std::cmp::Ordering::Greater => x = changing.end(x, reach_width),
            std::cmp::Ordering::Equal => {
                let be = binder.end(b, reach_width);
                let xe = changing.end(x, reach_width);
                let changes = changing.sum(x, xe);
                denominator = add(denominator, mul(binder.sum(b, be), changes));
                while b < be {
                    let end = binder.end(b, binder.width - 1);
                    let (lo, hi) = sequence.lookup(&binder.row(b)[reach_width..binder.width - 1]);
                    let met = if staged { weighted(changing, x, xe, sequence, lo, hi) }
                        else { mul(changes, sequence.sum(lo, hi)) };
                    numerator = add(numerator, mul(binder.sum(b, end), met));
                    b = end;
                }
                x = xe;
            }
        }
    }
    normalized(numerator, denominator)
}
