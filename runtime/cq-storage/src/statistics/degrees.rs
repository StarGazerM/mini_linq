//! Degree statistics of a run, kept as its rows are admitted (PS-Admit).
//!
//! A request is a count of distinct values, a share of visible rows, or a
//! matched degree: per row of a binder (or per pair of a changing row and
//! the binder row it reaches), the distinct values of one column among the
//! read's rows that agree with it on a key. Nothing here keeps a relation.
//! Distinct counts are sketches. A matched degree is taken over a sample of
//! *keys*, chosen by hash before any row is seen: a row of the binder and a
//! row of the read fall in the sample together, so the sample's degrees are
//! the true ones, with the round each value first appeared. A table that
//! outgrows its capacity halves its share of the keys; requests that read
//! the same rows by the same key share one table.
use std::collections::BTreeMap;
use std::hash::{BuildHasherDefault, Hasher};

use hashbrown::HashMap;

use super::{Sketch, hash};

/// The hash of a key: its cells' hashes in the key's column order, so that
/// a binder's key and a read's key of the same values hash alike.
#[inline]
fn keyed(cells: &[u64], columns: &[usize]) -> u64 {
    columns.iter().fold(0x9e37_79b9_7f4a_7c15, |state, &column| (state.rotate_left(23) ^ cells[column]).wrapping_mul(0xd6e8_feb8_6659_fd93))
}
/// The hash of a row's values in a set of its columns.
#[inline]
fn of_set(cells: &[u64], columns: &[usize]) -> u64 {
    columns.iter().fold(0, |state, &column| state ^ cells[column].rotate_left(column as u32 * 13 + 1))
}

/// Sampled keys or key pairs a table holds before it halves its share of
/// the keys.
const CAPACITY: usize = 1 << 13;
/// Values of one key kept exactly before a sketch takes over.
const EXACT: usize = 24;

/// The rows of one relation a request reads: the relation's position and
/// the columns it fixes to a value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rows { pub relation: usize, pub fixed: Vec<(usize, i64)> }

/// The changing rows a binder row is reached from: they agree on
/// `columns` of the changing rows and `binder` columns of the binder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reach { pub changing: Rows, pub columns: Vec<usize>, pub binder: Vec<usize> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// The distinct values of a column list.
    Distinct { rows: Rows, columns: Vec<usize> },
    /// Per binder row, or per pair of a changing row and the binder row it
    /// reaches: the distinct values of `column` among the read's rows whose
    /// `bound` columns equal the binder row's `from` columns; `staged`
    /// counts only values first admitted no later than the changing row
    /// (the binder row, without a reach).
    Joined { reach: Option<Reach>, binder: Rows, from: Vec<usize>, read: Rows, bound: Vec<usize>, column: usize, staged: bool },
    /// Of the pairs of a changing row and a row of `relation`, those whose
    /// row was admitted no later than the changing row.
    Visible { changing: usize, relation: usize },
    /// The publications that published a row of `relation`.
    Rounds { relation: usize },
}

/// A key is identified by its 64-bit hash, which is its own table hash.
#[derive(Default)]
struct Identity(u64);
impl Hasher for Identity {
    fn finish(&self) -> u64 { self.0 }
    fn write(&mut self, _bytes: &[u8]) { unreachable!("keys are 64-bit hashes") }
    fn write_u64(&mut self, value: u64) { self.0 = value; }
}
type Keyed<V> = HashMap<u64, V, BuildHasherDefault<Identity>>;
type Paired<K, V> = HashMap<K, V, ahash::RandomState>;

/// The values one key holds in one column, each with the round it was
/// first admitted: exactly while few, then as a sketch with the round of
/// every register change, which answers for any round as well.
enum Values {
    Exact(Vec<(i64, u32)>),
    Sketch { registers: Box<Sketch<256>>, changes: Vec<(u32, u64)> },
}
impl Values {
    fn insert(&mut self, value: i64, round: u32) {
        match self {
            Self::Exact(values) => {
                if let Some(held) = values.iter_mut().find(|held| held.0 == value) {
                    held.1 = held.1.min(round);
                    return;
                }
                values.push((value, round));
                if values.len() > EXACT {
                    values.sort_unstable_by_key(|held| held.1);
                    let (mut registers, mut changes) = (Box::<Sketch<256>>::default(), Vec::new());
                    for &(value, round) in values.iter() {
                        if registers.insert(hash(value)) { changes.push((round, hash(value))); }
                    }
                    *self = Self::Sketch { registers, changes };
                }
            }
            Self::Sketch { registers, changes } => {
                if registers.insert(hash(value)) { changes.push((round, hash(value))); }
            }
        }
    }
    /// The distinct values by round: for each round a value was first
    /// admitted in, the values admitted up to it, in round order.
    fn by_round(&self) -> Vec<(u32, u64)> {
        let mut counts: Vec<(u32, u64)> = Vec::new();
        let mut push = |round: u32, count: u64| match counts.last_mut() {
            Some(last) if last.0 == round => last.1 = count,
            _ => counts.push((round, count)),
        };
        match self {
            Self::Exact(values) => {
                let mut rounds: Vec<u32> = values.iter().map(|held| held.1).collect();
                rounds.sort_unstable();
                for (seen, round) in rounds.into_iter().enumerate() { push(round, seen as u64 + 1); }
            }
            Self::Sketch { changes, .. } => {
                // Rows arrive in round order, so the changes are in round order.
                let mut registers = Sketch::<256>::default();
                let mut at = 0;
                while at < changes.len() {
                    let round = changes[at].0;
                    while at < changes.len() && changes[at].0 == round { registers.insert(changes[at].1); at += 1; }
                    push(round, registers.count());
                }
            }
        }
        counts
    }
}
/// The values admitted up to `round`; all of them without a round.
fn up_to(counts: &[(u32, u64)], round: Option<u32>) -> u64 {
    match round {
        None => counts.last().map_or(0, |last| last.1),
        Some(round) => match counts.partition_point(|held| held.0 <= round) { 0 => 0, held => counts[held - 1].1 },
    }
}

/// A read's rows by key: for every sampled key, the values of each asked
/// column. It halves its share of the keys when it holds more than the
/// capacity, and also after every `READ` rows it took in while it still
/// holds more than `FLOOR` keys: a table of few keys and many rows.
struct Read { columns: Vec<usize>, limit: u64, taken: u64, keys: Keyed<Vec<Values>> }
const READ: u64 = 1 << 20;
const FLOOR: usize = 1 << 10;
/// The sampled keys some rows hold, each with the rows holding it by
/// round: the keys of a binder, or the reach keys of changing rows. Past
/// `STRIDE` counted rows it counts every other row for two, and so on.
struct Counted { limit: u64, stride: Stride, keys: Keyed<Vec<(u32, u64)>> }
/// The binder rows of a sampled reach key and a sampled read key.
struct Reached { reach_limit: u64, key_limit: u64, stride: Stride, rows: Paired<(u64, u64), u64> }

/// Rows a table counts one by one before it counts every second row for
/// two rows, then every fourth for four: its sums stay unbiased and its
/// work stays bounded.
const STRIDE: u64 = 1 << 16;
#[derive(Default)]
struct Stride { seen: u64, counted: u64, shift: u32 }
impl Stride {
    /// The weight of this row, if it is counted.
    #[inline]
    fn weight(&mut self) -> Option<u64> {
        self.seen += 1;
        if self.seen & ((1 << self.shift) - 1) != 0 { return None; }
        let weight = 1 << self.shift;
        self.counted += 1;
        if self.counted == STRIDE { self.counted = 0; self.shift += 1; }
        Some(weight)
    }
}

#[derive(Clone, Copy)]
enum Role {
    Sketch { rows: usize, set: usize, sketch: usize },
    Read { rows: usize, key: usize, read: usize },
    Counted { rows: usize, key: usize, counted: usize },
    Reached { rows: usize, reach: usize, key: usize, reached: usize },
}

/// What a relation's admitted rows update.
#[derive(Default)]
struct Admitted {
    arity: usize,
    /// Its rows by the publication that made them visible, and by the last
    /// publication each could see as a delta row.
    published: Vec<u64>,
    horizon: Vec<u64>,
    /// The selections of its rows requests read, each with its rows.
    selections: Vec<(Vec<(usize, i64)>, u64)>,
    /// The column lists hashed as keys for each row, and the column sets
    /// hashed for its sketches.
    keys: Vec<Vec<usize>>,
    sets: Vec<Vec<usize>>,
    roles: Vec<Role>,
    passed: Vec<bool>,
    cells: Vec<u64>,
    hashes: Vec<u64>,
}

enum Work {
    Any { relation: usize, rows: usize },
    All { relation: usize },
    Sketched { relation: usize, rows: usize, sketch: usize },
    Visible { changing: usize, relation: usize },
    Rounds { relation: usize },
    Direct { binder: usize, read: usize, slot: usize, staged: bool },
    Reach { changing: usize, reached: usize, read: usize, slot: usize, staged: bool },
}

/// PS-Admit: the degree statistics of a run, updated by every admitted row.
pub struct Degrees {
    relations: Vec<Admitted>,
    sketches: Vec<Sketch<1024>>,
    reads: Vec<Read>,
    counted: Vec<Counted>,
    reached: Vec<Reached>,
    work: Vec<Work>,
}

impl Degrees {
    /// The accumulator of `requests` over relations of these arities.
    pub fn new(requests: &[Request], arities: &[usize]) -> Result<Self, String> {
        let mut plan = Plan {
            relations: arities.iter().map(|&arity| Admitted { arity, ..Admitted::default() }).collect(),
            sketches: Vec::new(), reads: Vec::new(), counted: Vec::new(), reached: Vec::new(),
            sketch_ids: BTreeMap::new(), read_ids: BTreeMap::new(), counted_ids: BTreeMap::new(), reached_ids: BTreeMap::new(),
        };
        let mut work = Vec::with_capacity(requests.len());
        for request in requests {
            work.push(match request {
                Request::Distinct { rows, columns } => {
                    plan.columns(rows.relation, columns)?;
                    let mut wanted = columns.clone();
                    wanted.sort_unstable();
                    wanted.dedup();
                    let relation = rows.relation;
                    if wanted.is_empty() { Work::Any { relation, rows: plan.select(rows)? } }
                    else if rows.fixed.is_empty() && wanted.len() == plan.relations[relation].arity { Work::All { relation } }
                    else {
                        let selected = plan.select(rows)?;
                        let sets = &mut plan.relations[relation].sets;
                        let set = sets.iter().position(|held| *held == wanted).unwrap_or_else(|| { sets.push(wanted.clone()); sets.len() - 1 });
                        let next = plan.sketches.len();
                        let sketch = *plan.sketch_ids.entry((relation, selected, set)).or_insert(next);
                        if sketch == next {
                            plan.sketches.push(Sketch::default());
                            plan.relations[relation].roles.push(Role::Sketch { rows: selected, set, sketch });
                        }
                        Work::Sketched { relation, rows: selected, sketch }
                    }
                }
                Request::Visible { changing, relation } => {
                    plan.columns(*changing, &[])?;
                    plan.columns(*relation, &[])?;
                    Work::Visible { changing: *changing, relation: *relation }
                }
                Request::Rounds { relation } => {
                    plan.columns(*relation, &[])?;
                    Work::Rounds { relation: *relation }
                }
                Request::Joined { reach, binder, from, read, bound, column, staged } => {
                    if from.len() != bound.len() { return Err("joined degree key lengths differ".into()); }
                    plan.columns(binder.relation, from)?;
                    plan.columns(read.relation, bound)?;
                    plan.columns(read.relation, std::slice::from_ref(column))?;
                    let (read, slot) = plan.read(read, bound, *column)?;
                    match reach {
                        None => Work::Direct { binder: plan.counted(binder, from)?, read, slot, staged: *staged },
                        Some(reach) => {
                            if reach.columns.len() != reach.binder.len() { return Err("degree reach key lengths differ".into()); }
                            plan.columns(reach.changing.relation, &reach.columns)?;
                            plan.columns(binder.relation, &reach.binder)?;
                            let changing = plan.counted(&reach.changing, &reach.columns)?;
                            let reached = plan.reached(binder, &reach.binder, from)?;
                            Work::Reach { changing, reached, read, slot, staged: *staged }
                        }
                    }
                }
            });
        }
        for admitted in &mut plan.relations {
            admitted.passed = vec![false; admitted.selections.len()];
            admitted.cells = vec![0; admitted.arity];
            admitted.hashes = vec![0; admitted.keys.len()];
        }
        Ok(Self { relations: plan.relations, sketches: plan.sketches, reads: plan.reads, counted: plan.counted, reached: plan.reached, work })
    }

    /// Whether a request reads this relation.
    pub fn wants(&self, relation: usize) -> bool {
        self.relations.get(relation).is_some_and(|admitted| !admitted.roles.is_empty() || !admitted.selections.is_empty())
            || self.work.iter().any(|work| matches!(work, Work::All { relation: all } | Work::Rounds { relation: all } if *all == relation)
                || matches!(work, Work::Visible { changing, relation: seen } if *changing == relation || *seen == relation))
    }

    /// The rows admitted of a relation.
    pub fn rows(&self, relation: usize) -> u64 { self.relations[relation].published.iter().sum() }

    /// One row of `relation`: visible from publication `published`, and
    /// as a delta row meeting what was published up to `horizon`. Under a
    /// synchronous schedule both are its round: 0 for an input, then each
    /// round's delta. A relation's rows come in ascending order of both.
    pub fn admit(&mut self, relation: usize, published: u32, horizon: u32, row: &[i64]) {
        let Self { relations, sketches, reads, counted, reached, .. } = self;
        let admitted = &mut relations[relation];
        debug_assert_eq!(row.len(), admitted.arity);
        for (rows, at) in [(&mut admitted.published, published), (&mut admitted.horizon, horizon)] {
            if rows.len() <= at as usize { rows.resize(at as usize + 1, 0); }
            rows[at as usize] += 1;
        }
        for (at, (fixed, rows)) in admitted.selections.iter_mut().enumerate() {
            let passes = fixed.iter().all(|&(column, value)| row[column] == value);
            admitted.passed[at] = passes;
            *rows += u64::from(passes);
        }
        if admitted.roles.is_empty() { return; }
        for (cell, &value) in admitted.cells.iter_mut().zip(row) { *cell = hash(value); }
        for (at, columns) in admitted.keys.iter().enumerate() { admitted.hashes[at] = keyed(&admitted.cells, columns); }
        let (passed, hashes, cells, sets) = (&admitted.passed, &admitted.hashes, &admitted.cells, &admitted.sets);
        for role in &admitted.roles {
            match *role {
                Role::Sketch { rows, set, sketch } => if passed[rows] { sketches[sketch].insert(of_set(cells, &sets[set])); },
                Role::Read { rows, key, read } => {
                    let read = &mut reads[read];
                    if !passed[rows] || hashes[key] >= read.limit { continue; }
                    let width = read.columns.len();
                    let values = read.keys.entry(hashes[key]).or_insert_with(|| (0..width).map(|_| Values::Exact(Vec::new())).collect());
                    for (slot, &column) in read.columns.iter().enumerate() { values[slot].insert(row[column], published); }
                    read.taken += 1;
                    if read.keys.len() > CAPACITY || (read.taken >= READ && read.keys.len() > FLOOR) {
                        read.taken = 0;
                        read.limit >>= 1;
                        let limit = read.limit;
                        read.keys.retain(|key, _| *key < limit);
                    }
                }
                Role::Counted { rows, key, counted: at } => {
                    let counted = &mut counted[at];
                    if !passed[rows] || hashes[key] >= counted.limit { continue; }
                    let Some(weight) = counted.stride.weight() else { continue };
                    let rounds = counted.keys.entry(hashes[key]).or_default();
                    match rounds.last_mut() {
                        Some(last) if last.0 == horizon => last.1 += weight,
                        _ => rounds.push((horizon, weight)),
                    }
                    if counted.keys.len() > CAPACITY {
                        counted.limit >>= 1;
                        let limit = counted.limit;
                        counted.keys.retain(|key, _| *key < limit);
                    }
                }
                Role::Reached { rows, reach, key, reached: at } => {
                    let reached = &mut reached[at];
                    if !passed[rows] || hashes[reach] >= reached.reach_limit || hashes[key] >= reached.key_limit { continue; }
                    let Some(weight) = reached.stride.weight() else { continue };
                    *reached.rows.entry((hashes[reach], hashes[key])).or_default() += weight;
                    if reached.rows.len() > CAPACITY {
                        // Halve the share of the side with more distinct keys.
                        let distinct = |side: fn(&(u64, u64)) -> u64| reached.rows.keys().map(side).collect::<hashbrown::HashSet<u64, BuildHasherDefault<Identity>>>().len();
                        if distinct(|held| held.0) >= distinct(|held| held.1) {
                            reached.reach_limit >>= 1;
                            let limit = reached.reach_limit;
                            reached.rows.retain(|held, _| held.0 < limit);
                        } else {
                            reached.key_limit >>= 1;
                            let limit = reached.key_limit;
                            reached.rows.retain(|held, _| held.1 < limit);
                        }
                    }
                }
            }
        }
    }

    /// The value of every request, in request order, as a numerator and a
    /// denominator; `0/1` where the denominator would be zero.
    pub fn finish(self) -> Vec<(u64, u64)> {
        let Self { relations, sketches, reads, counted, reached, work } = self;
        // Per read table: its share of the keys, and per key and column the values by round.
        type ByRound = Keyed<Vec<Vec<(u32, u64)>>>;
        let reads: Vec<(u64, ByRound)> = reads.into_iter().map(|read| {
            (read.limit, read.keys.into_iter().map(|(key, values)| (key, values.iter().map(Values::by_round).collect())).collect())
        }).collect();
        let met = |read: usize, key: u64, slot: usize, round: Option<u32>| reads[read].1.get(&key).map_or(0, |values| up_to(&values[slot], round));
        let ratio = |sum: u128, over: u128| -> (u64, u64) {
            if over == 0 { return (0, 1); }
            // Both fit 64 bits unless the pairs exceed 2^64; then the ratio is kept.
            let shift = (128 - sum.max(over).leading_zeros()).saturating_sub(64);
            ((sum >> shift) as u64, ((over >> shift) as u64).max(1))
        };
        work.iter().map(|work| match *work {
            Work::Any { relation, rows } => (u64::from(relations[relation].selections[rows].1 > 0), 1),
            Work::All { relation } => (relations[relation].published.iter().sum(), 1),
            Work::Rounds { relation } => (relations[relation].published.iter().filter(|&&rows| rows > 0).count() as u64, 1),
            Work::Sketched { relation, rows, sketch } => (sketches[sketch].count().min(relations[relation].selections[rows].1), 1),
            Work::Visible { changing, relation } => {
                // A changing row sees what was published up to its horizon.
                let (changing, seen) = (&relations[changing].horizon, &relations[relation].published);
                let (mut pairs, mut so_far) = (0u128, 0u128);
                for (round, &rows) in changing.iter().enumerate() {
                    if let Some(&gained) = seen.get(round) { so_far += u128::from(gained); }
                    pairs += u128::from(rows) * so_far;
                }
                let total = |rounds: &[u64]| rounds.iter().map(|&rows| u128::from(rows)).sum::<u128>();
                ratio(pairs, total(changing) * total(seen))
            }
            Work::Direct { binder, read, slot, staged } => {
                let binder = &counted[binder];
                let limit = binder.limit.min(reads[read].0);
                let (mut sum, mut over) = (0u128, 0u128);
                for (&key, rounds) in &binder.keys {
                    if key >= limit { continue; }
                    for &(round, rows) in rounds {
                        sum += u128::from(rows) * u128::from(met(read, key, slot, staged.then_some(round)));
                        over += u128::from(rows);
                    }
                }
                ratio(sum, over)
            }
            Work::Reach { changing, reached: at, read, slot, staged } => {
                let (changing, reached) = (&counted[changing], &reached[at]);
                let reach_limit = changing.limit.min(reached.reach_limit);
                let key_limit = reached.key_limit.min(reads[read].0);
                let by_reach = &changing.keys;
                let (mut sum, mut over) = (0u128, 0u128);
                for (&(reach, key), &rows) in &reached.rows {
                    if reach >= reach_limit || key >= key_limit { continue; }
                    let Some(changes) = by_reach.get(&reach) else { continue };
                    for &(round, changes) in changes {
                        let pairs = u128::from(rows) * u128::from(changes);
                        sum += pairs * u128::from(met(read, key, slot, staged.then_some(round)));
                        over += pairs;
                    }
                }
                ratio(sum, over)
            }
        }).collect()
    }
}

struct Plan {
    relations: Vec<Admitted>,
    sketches: Vec<Sketch<1024>>,
    reads: Vec<Read>,
    counted: Vec<Counted>,
    reached: Vec<Reached>,
    sketch_ids: BTreeMap<(usize, usize, usize), usize>,
    read_ids: BTreeMap<(usize, usize, usize), usize>,
    counted_ids: BTreeMap<(usize, usize, usize), usize>,
    reached_ids: BTreeMap<(usize, usize, usize, usize), usize>,
}
impl Plan {
    fn columns(&self, relation: usize, columns: &[usize]) -> Result<(), String> {
        let admitted = self.relations.get(relation).ok_or("a degree request names a relation the run lacks")?;
        if columns.iter().any(|&column| column >= admitted.arity) { return Err("a degree request names a column its relation lacks".into()); }
        Ok(())
    }
    /// The selection of a relation's rows, by position.
    fn select(&mut self, rows: &Rows) -> Result<usize, String> {
        self.columns(rows.relation, &rows.fixed.iter().map(|&(column, _)| column).collect::<Vec<_>>())?;
        let mut fixed = rows.fixed.clone();
        fixed.sort_unstable();
        let selections = &mut self.relations[rows.relation].selections;
        Ok(selections.iter().position(|held| held.0 == fixed).unwrap_or_else(|| { selections.push((fixed, 0)); selections.len() - 1 }))
    }
    /// The column list a relation hashes for each row, by position.
    fn key(&mut self, relation: usize, columns: &[usize]) -> usize {
        let keys = &mut self.relations[relation].keys;
        keys.iter().position(|held| held == columns).unwrap_or_else(|| { keys.push(columns.to_vec()); keys.len() - 1 })
    }
    /// The table of a read's rows by `bound`, and the slot of `column` in it.
    fn read(&mut self, rows: &Rows, bound: &[usize], column: usize) -> Result<(usize, usize), String> {
        let selected = self.select(rows)?;
        let key = self.key(rows.relation, bound);
        let next = self.reads.len();
        let id = *self.read_ids.entry((rows.relation, selected, key)).or_insert(next);
        if id == next {
            self.reads.push(Read { columns: Vec::new(), limit: u64::MAX, taken: 0, keys: Keyed::default() });
            self.relations[rows.relation].roles.push(Role::Read { rows: selected, key, read: id });
        }
        let columns = &mut self.reads[id].columns;
        let slot = columns.iter().position(|&held| held == column).unwrap_or_else(|| { columns.push(column); columns.len() - 1 });
        Ok((id, slot))
    }
    /// The table of the keys `rows` hold in `columns`.
    fn counted(&mut self, rows: &Rows, columns: &[usize]) -> Result<usize, String> {
        let selected = self.select(rows)?;
        let key = self.key(rows.relation, columns);
        let next = self.counted.len();
        let id = *self.counted_ids.entry((rows.relation, selected, key)).or_insert(next);
        if id == next {
            self.counted.push(Counted { limit: u64::MAX, stride: Stride::default(), keys: Keyed::default() });
            self.relations[rows.relation].roles.push(Role::Counted { rows: selected, key, counted: id });
        }
        Ok(id)
    }
    /// The table of the binder `rows` by their reach key and their read key.
    fn reached(&mut self, rows: &Rows, reach: &[usize], from: &[usize]) -> Result<usize, String> {
        let selected = self.select(rows)?;
        let (reach, key) = (self.key(rows.relation, reach), self.key(rows.relation, from));
        let next = self.reached.len();
        let id = *self.reached_ids.entry((rows.relation, selected, reach, key)).or_insert(next);
        if id == next {
            self.reached.push(Reached { reach_limit: u64::MAX, key_limit: u64::MAX, stride: Stride::default(), rows: Paired::default() });
            self.relations[rows.relation].roles.push(Role::Reached { rows: selected, reach, key, reached: id });
        }
        Ok(id)
    }
}
