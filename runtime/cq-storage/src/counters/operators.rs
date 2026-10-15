//! Stream times are inclusive pipeline spans from first demand to exhaustion or
//! drop, including upstream and downstream work between demands. They are not
//! exclusive CPU times and must not be summed. No read-ahead or per-row clock
//! reads: two clock reads per stream lifetime/build, local increments per row.
use std::cell::{Cell, RefCell};
use std::fmt::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Static identity of a Physical result or concrete build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperatorLabel {
    pub result: &'static str,
    pub kind: &'static str,
    pub key: &'static str,
    pub inputs: &'static [&'static str],
}
/// Snapshot of one execution; program IDs distinguish composed invocations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorRecord {
    pub id: usize,
    pub program: usize,
    pub label: OperatorLabel,
    pub rows_in: u64,
    pub rows_out: u64,
    /// Build input rows, including duplicate insertion attempts.
    pub rows_inserted: u64,
    pub elapsed: Duration,
    pub calls: u64,
}
struct State {
    id: usize,
    program: usize,
    label: OperatorLabel,
    rows_in: Cell<u64>,
    rows_out: Cell<u64>,
    inserted: Cell<u64>,
    elapsed: Cell<Duration>,
    calls: Cell<u64>,
    alias: Cell<Option<usize>>,
    stored_rows: Cell<Option<u64>>,
    time_source: Cell<Option<usize>>,
}
type BuildContext = Option<(Rc<State>, Rc<Cell<bool>>)>;
thread_local! {
static ACTIVE_BUILD: RefCell<BuildContext> = const { RefCell::new(None) };
static OPERATORS: RefCell<Vec<Rc<State>>> = const { RefCell::new(Vec::new()) }; }
pub(super) fn reset_operators() {
    OPERATORS.with(|ops| ops.borrow_mut().clear());
}
/// Snapshot in registration order, including unvisited results. Exhaust/drop
/// iterators first to close their spans. Reset isolates subsequent executions.
pub fn operators() -> Vec<OperatorRecord> {
    OPERATORS.with(|ops| {
        let ops = ops.borrow();
        ops.iter()
            .map(|s| {
                let mut source = s;
                while let Some(i) = source.alias.get() {
                    source = &ops[i];
                }
                let time = source.time_source.get().map_or(source, |i| &ops[i]);
                OperatorRecord {
                    id: s.id,
                    program: s.program,
                    label: s.label,
                    rows_in: if s.alias.get().is_some() {
                        source.rows_out.get()
                    } else {
                        s.rows_in.get()
                    },
                    rows_out: source.rows_out.get(),
                    rows_inserted: s.inserted.get(),
                    elapsed: time.elapsed.get(),
                    calls: time.calls.get(),
                }
            })
            .collect()
    })
}
/// Handles cloned per iterator, never per row. Registration alone accesses TLS.
#[derive(Clone)]
pub struct OperatorProgram(Rc<Vec<Rc<State>>>);
impl OperatorProgram {
    pub fn new(labels: &[OperatorLabel]) -> Self {
        OPERATORS.with(|ops| {
            let mut ops = ops.borrow_mut();
            let program = ops.len();
            let states = labels
                .iter()
                .map(|&label| {
                    let state = Rc::new(State {
                        id: ops.len(),
                        program,
                        label,
                        rows_in: Cell::new(0),
                        rows_out: Cell::new(0),
                        inserted: Cell::new(0),
                        elapsed: Cell::new(Duration::ZERO),
                        calls: Cell::new(0),
                        alias: Cell::new(None),
                        stored_rows: Cell::new(None),
                        time_source: Cell::new(None),
                    });
                    ops.push(state.clone());
                    state
                })
                .collect();
            Self(Rc::new(states))
        })
    }
    /// A Physical alias shares its source's produced cardinality and span.
    pub fn alias(&self, index: usize, source: usize) {
        let mut candidate = source;
        loop {
            assert_ne!(index, candidate, "operator aliases must be acyclic");
            match self.0[candidate].alias.get() {
                Some(id) => candidate = id - self.0[0].program,
                None => break,
            }
        }
        self.0[index].alias.set(Some(self.0[source].id));
    }
    /// Fused boundaries share one measured inclusive pipeline span.
    pub fn share_time(&self, index: usize, source: usize) {
        self.0[index].time_source.set(Some(self.0[source].id));
    }
    pub fn stream<I: Iterator>(&self, index: usize, source: I) -> OperatorStream<I> {
        OperatorStream {
            source: Some(source),
            state: self.0[index].clone(),
            started: None,
            count: 0,
        }
    }
    #[cfg(not(feature = "counters"))]
    pub fn output<I: Iterator>(&self, _index: usize, source: I) -> I {
        source
    }
    #[cfg(feature = "counters")]
    pub fn output<I: Iterator>(&self, index: usize, source: I) -> OperatorOutput<I> {
        OperatorOutput {
            source: Some(source),
            state: self.0[index].clone(),
            count: 0,
        }
    }
    #[cfg(not(feature = "counters"))]
    pub fn input<I: Iterator>(&self, _index: usize, source: I) -> I {
        source
    }
    #[cfg(feature = "counters")]
    pub fn input<I: Iterator>(&self, index: usize, source: I) -> OperatorInput<I> {
        OperatorInput {
            source: Some(source),
            state: self.0[index].clone(),
            insert: false,
            count: 0,
        }
    }
    #[cfg(not(feature = "counters"))]
    pub fn inserting<I: Iterator>(&self, _index: usize, source: I) -> I {
        source
    }
    #[cfg(feature = "counters")]
    pub fn inserting<I: Iterator>(&self, index: usize, source: I) -> OperatorInput<I> {
        OperatorInput {
            source: Some(source),
            state: self.0[index].clone(),
            insert: true,
            count: 0,
        }
    }
    /// Time an execution boundary without treating nested lazy builds as its own.
    pub fn scope<T>(&self, index: usize, run: impl FnOnce() -> T) -> T {
        let clock = DeferredClock(Some(OperatorClock {
            state: self.0[index].clone(),
            started: Instant::now(),
        }));
        let result = run();
        drop(clock);
        result
    }
    #[cfg(not(feature = "counters"))]
    pub fn predicate<P>(&self, _index: usize, _source: Option<usize>, predicate: P) -> P {
        predicate
    }
    #[cfg(feature = "counters")]
    pub fn predicate<P>(
        &self,
        index: usize,
        source: Option<usize>,
        predicate: P,
    ) -> OperatorPredicate<P> {
        OperatorPredicate {
            predicate,
            count: Cell::new(0),
            state: self.0[index].clone(),
            source: source.map(|i| self.0[i].clone()),
        }
    }
    /// One clock pair around synchronous initialization, including upstream work.
    pub fn build<T>(&self, index: usize, build: impl FnOnce() -> T) -> T {
        let deferred = Rc::new(Cell::new(false));
        let previous = ACTIVE_BUILD
            .with(|active| active.replace(Some((self.0[index].clone(), deferred.clone()))));
        let guard = ActiveBuild {
            previous,
            deferred,
            clock: Some(OperatorClock {
                state: self.0[index].clone(),
                started: Instant::now(),
            }),
        };
        let value = build();
        drop(guard);
        value
    }
    /// Cardinality supplied by an independent storage boundary, e.g. selection.
    #[cfg(not(feature = "counters"))]
    #[inline]
    pub fn rows(&self, _index: usize, _input: u64, _output: u64, _inserted: u64) {}
    #[cfg(feature = "counters")]
    pub fn rows(&self, index: usize, input: u64, output: u64, inserted: u64) {
        let s = &self.0[index];
        s.rows_in.set(s.rows_in.get() + input);
        s.rows_out.set(s.rows_out.get() + output);
        s.inserted.set(s.inserted.get() + inserted);
    }
}
struct ActiveBuild {
    previous: BuildContext,
    deferred: Rc<Cell<bool>>,
    clock: Option<OperatorClock>,
}
impl Drop for ActiveBuild {
    fn drop(&mut self) {
        ACTIVE_BUILD.with(|active| active.replace(self.previous.take()));
        if let Some(clock) = self.clock.take().filter(|_| !self.deferred.get()) {
            clock.finish();
        }
    }
}
struct OperatorClock {
    state: Rc<State>,
    started: Instant,
}
impl OperatorClock {
    fn finish(self) {
        self.state
            .elapsed
            .set(self.state.elapsed.get() + self.started.elapsed());
        self.state.calls.set(self.state.calls.get() + 1);
        if self.state.label.kind.ends_with("_BUILD") {
            self.state.rows_out.set(
                self.state
                    .stored_rows
                    .get()
                    .unwrap_or(self.state.inserted.get()),
            );
        }
    }
}
/// Demand-preserving counter for an input or insertion stream.
pub struct OperatorInput<I> {
    source: Option<I>,
    state: Rc<State>,
    insert: bool,
    count: u64,
}
impl<I> OperatorInput<I> {
    fn flush(&mut self) {
        self.state
            .rows_in
            .set(self.state.rows_in.get() + self.count);
        if self.insert {
            self.state
                .inserted
                .set(self.state.inserted.get() + self.count);
        }
        self.count = 0;
    }
}
impl<I> Drop for OperatorInput<I> {
    fn drop(&mut self) {
        self.flush();
    }
}
impl<I: Iterator> Iterator for OperatorInput<I> {
    type Item = I::Item;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let row = self.source.as_mut().expect("live iterator").next();
        if row.is_some() {
            self.count += 1;
        } else {
            self.flush();
        }
        row
    }
    fn fold<B, F>(mut self, init: B, mut f: F) -> B
    where
        F: FnMut(B, Self::Item) -> B,
    {
        let source = self.source.take().expect("live iterator");
        source.fold(init, |acc, row| {
            self.count += 1;
            f(acc, row)
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.as_ref().expect("live iterator").size_hint()
    }
}
/// Cumulative inclusive span; does not force a row the consumer did not request.
pub struct OperatorStream<I> {
    source: Option<I>,
    state: Rc<State>,
    started: Option<Instant>,
    count: u64,
}
impl<I> OperatorStream<I> {
    fn finish(&mut self) {
        if let Some(started) = self.started.take() {
            self.state
                .elapsed
                .set(self.state.elapsed.get() + started.elapsed());
            self.state.calls.set(self.state.calls.get() + 1);
        }
        self.state
            .rows_out
            .set(self.state.rows_out.get() + self.count);
        self.count = 0;
    }
}
impl<I: Iterator> Iterator for OperatorStream<I> {
    type Item = I::Item;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.started.is_none() {
            self.started = Some(Instant::now());
        }
        let row = self.source.as_mut().expect("live iterator").next();
        if row.is_some() {
            #[cfg(feature = "counters")]
            {
                self.count += 1;
            }
        } else {
            self.finish();
        }
        row
    }
    fn fold<B, F>(mut self, init: B, mut f: F) -> B
    where
        F: FnMut(B, Self::Item) -> B,
    {
        self.started.get_or_insert_with(Instant::now);
        let source = self.source.take().expect("live iterator");
        source.fold(init, |acc, row| {
            #[cfg(feature = "counters")]
            {
                self.count += 1;
            }
            f(acc, row)
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.as_ref().expect("live iterator").size_hint()
    }
}
impl<I> Drop for OperatorStream<I> {
    fn drop(&mut self) {
        self.finish();
    }
}
/// Render dependency trees with references for shared materializations.
/// Nested times are inclusive spans and cannot be added together.
pub fn render_operators(records: &[OperatorRecord]) -> String {
    fn walk(
        out: &mut String,
        records: &[OperatorRecord],
        index: usize,
        depth: usize,
        seen: &mut std::collections::BTreeSet<usize>,
    ) {
        let r = &records[index];
        let repeat = !seen.insert(r.id);
        writeln!(
            out,
            "{}{:<16} {} [{}] inputs={} rows={} in={} inserted={} t={:.3}ms{}",
            "  ".repeat(depth),
            r.label.kind,
            r.label.result,
            r.label.key,
            r.label.inputs.join(","),
            r.rows_out,
            r.rows_in,
            r.rows_inserted,
            r.elapsed.as_secs_f64() * 1000.,
            if repeat { " (shared)" } else { "" }
        )
        .unwrap();
        if repeat {
            return;
        }
        for input in r.label.inputs {
            if let Some(i) = records
                .iter()
                .position(|c| c.program == r.program && c.label.result == *input)
            {
                walk(out, records, i, depth + 1, seen);
            }
        }
    }
    let mut out = String::new();
    let mut seen = std::collections::BTreeSet::new();
    let query = records.iter().position(|r| r.label.kind == "QUERY");
    if let Some(i) = query {
        walk(&mut out, records, i, 0, &mut seen);
    }
    let depth = usize::from(query.is_some());
    for (i, r) in records.iter().enumerate().rev() {
        if query == Some(i) {
            continue;
        }
        if !records
            .iter()
            .any(|p| p.program == r.program && p.label.inputs.contains(&r.label.result))
        {
            walk(&mut out, records, i, depth, &mut seen);
        }
    }
    for (i, r) in records.iter().enumerate() {
        if !seen.contains(&r.id) {
            walk(&mut out, records, i, depth, &mut seen);
        }
    }
    out
}

/// A portable identity: a storage handle keeps its existing auto traits.
/// Moving a deferred build to another thread excludes it from the old epoch.
#[derive(Clone)]
pub(crate) struct DeferredBuild {
    id: usize,
    epoch: u64,
    thread: std::thread::ThreadId,
}
pub(crate) fn defer_build() -> Option<DeferredBuild> {
    ACTIVE_BUILD.with(|active| {
        active.borrow().as_ref().map(|(state, deferred)| {
            deferred.set(true);
            DeferredBuild {
                id: state.id,
                epoch: super::epoch(),
                thread: std::thread::current().id(),
            }
        })
    })
}
pub(crate) struct DeferredClock(Option<OperatorClock>);
impl DeferredBuild {
    fn state(&self) -> Option<Rc<State>> {
        if self.thread != std::thread::current().id() || self.epoch != super::epoch() {
            return None;
        }
        OPERATORS.with(|ops| ops.borrow().get(self.id).cloned())
    }
    pub(crate) fn cardinality(&self, rows: usize) {
        if let Some(state) = self.state() {
            state.stored_rows.set(Some(rows as u64));
        }
    }
    pub(crate) fn start(&self) -> DeferredClock {
        DeferredClock(self.state().map(|state| OperatorClock {
            state,
            started: Instant::now(),
        }))
    }
}
impl Drop for DeferredClock {
    fn drop(&mut self) {
        if let Some(clock) = self.0.take() {
            clock.finish();
        }
    }
}

/// Counts tested positions while preserving block pruning and predicate order.
pub struct OperatorPredicate<P> {
    predicate: P,
    count: Cell<u64>,
    state: Rc<State>,
    source: Option<Rc<State>>,
}
impl<P> Drop for OperatorPredicate<P> {
    fn drop(&mut self) {
        self.state
            .rows_in
            .set(self.state.rows_in.get() + self.count.get());
        if let Some(source) = &self.source {
            source
                .rows_out
                .set(source.rows_out.get() + self.count.get());
        }
    }
}
impl<K, P: crate::ReadPredicate<K>> crate::ReadPredicate<K> for OperatorPredicate<P> {
    #[inline]
    fn test(&self, key: &K) -> bool {
        self.count.set(self.count.get() + 1);
        self.predicate.test(key)
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        self.predicate.any_in(lo, hi)
    }
}

/// Row-only cursor boundary, suitable for repeated keyed reads without clocks.
pub struct OperatorOutput<I> {
    source: Option<I>,
    state: Rc<State>,
    count: u64,
}
impl<I> OperatorOutput<I> {
    fn flush(&mut self) {
        self.state
            .rows_out
            .set(self.state.rows_out.get() + self.count);
        self.count = 0;
    }
}
impl<I> Drop for OperatorOutput<I> {
    fn drop(&mut self) {
        self.flush();
    }
}
impl<I: Iterator> Iterator for OperatorOutput<I> {
    type Item = I::Item;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let row = self.source.as_mut().expect("live iterator").next();
        if row.is_some() {
            self.count += 1;
        } else {
            self.flush();
        }
        row
    }
    fn fold<B, F>(mut self, init: B, mut f: F) -> B
    where
        F: FnMut(B, Self::Item) -> B,
    {
        let source = self.source.take().expect("live iterator");
        source.fold(init, |acc, row| {
            self.count += 1;
            f(acc, row)
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.as_ref().expect("live iterator").size_hint()
    }
}
impl<I: crate::OrderedCursor> crate::OrderedCursor for OperatorOutput<I> {
    type Key = I::Key;
    type Value = I::Value;
    fn seek(&mut self, bound: &Self::Key) {
        self.source.as_mut().expect("live iterator").seek(bound);
    }
}
impl<I: crate::OrderedCursor> crate::OrderedCursor for OperatorInput<I> {
    type Key = I::Key;
    type Value = I::Value;
    fn seek(&mut self, bound: &Self::Key) {
        self.source.as_mut().expect("live iterator").seek(bound);
    }
}

#[cfg(feature = "counters")]
pub(crate) fn build_cardinality(rows: usize) {
    ACTIVE_BUILD.with(|active| {
        if let Some((state, _)) = &*active.borrow() {
            state.stored_rows.set(Some(rows as u64));
        }
    });
}
