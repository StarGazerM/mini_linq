//! Stream times are inclusive pipeline spans from first demand to exhaustion or
//! drop, including upstream and downstream work between demands. They are not
//! exclusive CPU times and must not be summed. Parallel invocations accumulate
//! into the originating query's graph through shared handles. Counts remain
//! local to each cursor and merge at exhaustion/drop, not on every row.
//! `TASKS_BUILD_WALL_TOTAL` is the measured union of setup/publication phases
//! across this query epoch. It includes upstream production, routing and joins;
//! it is not the sum of operator builds or exclusive index-insertion time.
use std::cell::RefCell;
use std::fmt::Write;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
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
    /// Build time spent producing the build's input rows (timing builds only;
    /// the input is drained before insertion). `elapsed - source_elapsed` is
    /// cumulative construction time across invocations. Parallel invocations
    /// can overlap: this difference is not an elapsed wall-time phase.
    pub source_elapsed: Duration,
    /// Immutable hole/round identity captured when the program was registered.
    #[cfg(feature = "planner-calibration")]
    pub calibration: Option<super::calibration::CalibrationFrame>,
    /// Actual row-counter alias, as a global record ID; time sharing is separate.
    #[cfg(feature = "planner-calibration")]
    pub row_alias: Option<usize>,
    /// A row tally overflowed; none of this record's counts are exact evidence.
    #[cfg(feature = "planner-calibration")]
    pub row_overflow: bool,
    /// An actual output boundary was observed, including an observed empty result.
    /// False means unobserved, not exact zero; time sharing never supplies this fact.
    #[cfg(feature = "planner-calibration")]
    pub row_observed: bool,
}
pub(super) struct State {
    id: usize,
    program: usize,
    label: OperatorLabel,
    metrics: Mutex<Metrics>,
    #[cfg(feature = "planner-calibration")]
    calibration: Option<super::calibration::CalibrationFrame>,
    #[cfg(feature = "planner-calibration")]
    row_overflow: AtomicBool,
}
#[derive(Clone, Default)]
struct Metrics {
    rows_in: u64,
    rows_out: u64,
    inserted: u64,
    elapsed: Duration,
    calls: u64,
    alias: Option<usize>,
    stored_rows: Option<u64>,
    time_source: Option<usize>,
    source_elapsed: Duration,
    #[cfg(feature = "planner-calibration")]
    row_observed: bool,
}
impl State {
    #[inline]
    fn add_rows(&self, value: &mut u64, amount: u64) {
        crate::calibration_select! {
            plain { *value += amount; }
            calibration {
                if let Some(sum) = value.checked_add(amount) {
                    *value = sum;
                } else {
                    self.row_overflow.store(true, Ordering::Relaxed);
                }
            }
        }
    }
    fn update(&self, update: impl FnOnce(&mut Metrics)) {
        update(
            &mut self
                .metrics
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
    }
    fn snapshot(&self) -> Metrics {
        self.metrics
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}
pub(super) type BuildContext = Option<(Arc<State>, Arc<AtomicBool>)>;
thread_local! {
    static ACTIVE_BUILD: RefCell<BuildContext> = const { RefCell::new(None) };
}
pub(super) fn capture_build() -> BuildContext {
    ACTIVE_BUILD.with(|active| active.borrow().clone())
}
pub(super) fn replace_build(context: BuildContext) -> BuildContext {
    ACTIVE_BUILD.with(|active| active.replace(context))
}
/// Snapshot in registration order, including unvisited results. Exhaust/drop
/// iterators first to close their spans. Reset isolates subsequent executions.
pub fn operators() -> Vec<OperatorRecord> {
    let context = super::TaskContext::capture();
    let mut records: Vec<_> = {
        let ops = context
            .0
            .operators
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        ops.iter()
            .map(|s| {
                let own = s.snapshot();
                let mut source = own.clone();
                #[cfg(feature = "planner-calibration")]
                let mut row_overflow = s.row_overflow.load(Ordering::Relaxed);
                while let Some(i) = source.alias {
                    source = ops[i].snapshot();
                    #[cfg(feature = "planner-calibration")]
                    { row_overflow |= ops[i].row_overflow.load(Ordering::Relaxed); }
                }
                let time = source
                    .time_source
                    .map_or_else(|| source.clone(), |i| ops[i].snapshot());
                OperatorRecord {
                    id: s.id,
                    program: s.program,
                    label: s.label,
                    rows_in: if own.alias.is_some() {
                        source.rows_out
                    } else {
                        own.rows_in
                    },
                    rows_out: source.rows_out,
                    rows_inserted: own.inserted,
                    elapsed: time.elapsed,
                    calls: time.calls,
                    source_elapsed: time.source_elapsed,
                    #[cfg(feature = "planner-calibration")]
                    calibration: s.calibration.clone(),
                    #[cfg(feature = "planner-calibration")]
                    row_alias: own.alias,
                    #[cfg(feature = "planner-calibration")]
                    row_overflow,
                    #[cfg(feature = "planner-calibration")]
                    row_observed: source.row_observed,
                }
            })
            .collect()
    };
    if let Some(elapsed) = context.build_wall() {
        records.push(OperatorRecord {
            id: records.len(),
            program: 0,
            label: OperatorLabel {
                result: "__cq_tasks_build_wall_total",
                kind: "TASKS_BUILD_WALL_TOTAL",
                key: "",
                inputs: &[],
            },
            rows_in: 0,
            rows_out: 0,
            rows_inserted: 0,
            elapsed,
            calls: 1,
            source_elapsed: Duration::ZERO,
            #[cfg(feature = "planner-calibration")]
            calibration: None,
            #[cfg(feature = "planner-calibration")]
            row_alias: None,
            #[cfg(feature = "planner-calibration")]
            row_overflow: false,
            #[cfg(feature = "planner-calibration")]
            row_observed: false,
        });
    }
    records
}
/// Push-loop cardinalities accumulated locally and merged even on early exit.
#[cfg(feature = "planner-calibration")]
pub struct OperatorPush {
    state: Arc<State>,
    input: u64,
    output: u64,
    started: Instant,
}
#[cfg(feature = "planner-calibration")]
impl OperatorPush {
    pub fn input(&mut self) { self.state.add_rows(&mut self.input, 1); }
    pub fn output(&mut self) { self.state.add_rows(&mut self.output, 1); }
}
#[cfg(feature = "planner-calibration")]
impl Drop for OperatorPush {
    fn drop(&mut self) {
        self.state.update(|metrics| {
            metrics.row_observed = true;
            self.state.add_rows(&mut metrics.rows_in, self.input);
            self.state.add_rows(&mut metrics.rows_out, self.output);
            metrics.calls += 1;
            metrics.elapsed += self.started.elapsed();
        });
    }
}

/// Shared handles cloned per iterator/task, never looked up through TLS per row.
#[derive(Clone)]
pub struct OperatorProgram(Arc<Vec<Arc<State>>>, super::TaskContext);
impl OperatorProgram {
    #[cfg(feature = "planner-calibration")]
    pub fn push(&self, index: usize) -> OperatorPush {
        OperatorPush { state: self.0[index].clone(), input: 0, output: 0, started: Instant::now() }
    }
    pub fn new(labels: &[OperatorLabel]) -> Self {
        let context = super::TaskContext::capture();
        let states = {
            let mut ops = context
                .0
                .operators
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let program = ops.len();
            labels
                .iter()
                .map(|&label| {
                    let state = Arc::new(State {
                        id: ops.len(),
                        program,
                        label,
                        metrics: Mutex::new(Metrics::default()),
                        #[cfg(feature = "planner-calibration")]
                        calibration: context.2.frame(),
                        #[cfg(feature = "planner-calibration")]
                        row_overflow: AtomicBool::new(false),
                    });
                    ops.push(state.clone());
                    state
                })
                .collect()
        };
        Self(Arc::new(states), context)
    }
    /// A Physical alias shares its source's produced cardinality and span.
    pub fn alias(&self, index: usize, source: usize) {
        let mut candidate = source;
        loop {
            assert_ne!(index, candidate, "operator aliases must be acyclic");
            match self.0[candidate].snapshot().alias {
                Some(id) => candidate = id - self.0[0].program,
                None => break,
            }
        }
        self.0[index].update(|metrics| metrics.alias = Some(self.0[source].id));
    }
    /// Fused boundaries share one measured inclusive pipeline span.
    pub fn share_time(&self, index: usize, source: usize) {
        self.0[index].update(|metrics| metrics.time_source = Some(self.0[source].id));
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
            #[cfg(feature = "planner-calibration")]
            polled: false,
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
    /// Timing builds drain their input before insertion, so the input pipeline
    /// is timed apart from construction; the drain happens on first demand,
    /// preserving deferred initialization.
    #[cfg(not(feature = "counters"))]
    pub fn inserting<I: Iterator>(&self, index: usize, source: I) -> OperatorDrained<I> {
        OperatorDrained {
            source: Some(source),
            rows: Vec::new().into_iter(),
            state: self.0[index].clone(),
        }
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
    /// Merge task-local primitive counters into the originating query epoch.
    pub fn task<T>(&self, run: impl FnOnce() -> T) -> T {
        self.1.run(run)
    }
    /// Start a host phase without extending a borrow across generated bindings.
    pub fn start_scope(&self, index: usize) -> OperatorScope {
        let build = (self.0[index].label.kind == "TASKS_BUILD_WALL").then(|| {
            self.1.start_build();
            self.1.clone()
        });
        OperatorScope {
            clock: Some(OperatorClock {
                state: self.0[index].clone(),
                started: Instant::now(),
            }),
            build,
        }
    }
    /// Time an execution boundary without treating nested lazy builds as its own.
    pub fn scope<T>(&self, index: usize, run: impl FnOnce() -> T) -> T {
        let clock = self.start_scope(index);
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
            count: AtomicU64::new(0),
            state: self.0[index].clone(),
            source: source.map(|i| self.0[i].clone()),
        }
    }
    /// One clock pair around synchronous initialization, including upstream work.
    pub fn build<T>(&self, index: usize, build: impl FnOnce() -> T) -> T {
        let deferred = Arc::new(AtomicBool::new(false));
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
        self.0[index].update(|metrics| {
            crate::calibration_select! {
                plain {}
                calibration { metrics.row_observed = true; }
            }
            self.0[index].add_rows(&mut metrics.rows_in, input);
            self.0[index].add_rows(&mut metrics.rows_out, output);
            self.0[index].add_rows(&mut metrics.inserted, inserted);
        });
    }
}
/// An elapsed wall scope. Task build phases additionally feed a session-wide
/// interval union, so nested or concurrent programs never double-count build.
pub struct OperatorScope {
    clock: Option<OperatorClock>,
    build: Option<super::TaskContext>,
}
impl Drop for OperatorScope {
    fn drop(&mut self) {
        if let Some(context) = self.build.take() {
            context.finish_build();
        }
        if let Some(clock) = self.clock.take() {
            clock.finish();
        }
    }
}
struct ActiveBuild {
    previous: BuildContext,
    deferred: Arc<AtomicBool>,
    clock: Option<OperatorClock>,
}
impl Drop for ActiveBuild {
    fn drop(&mut self) {
        ACTIVE_BUILD.with(|active| active.replace(self.previous.take()));
        if let Some(clock) = self
            .clock
            .take()
            .filter(|_| !self.deferred.load(Ordering::Relaxed))
        {
            clock.finish();
        }
    }
}
struct OperatorClock {
    state: Arc<State>,
    started: Instant,
}
impl OperatorClock {
    fn finish(self) {
        let elapsed = self.started.elapsed();
        self.state.update(|metrics| {
            metrics.elapsed += elapsed;
            metrics.calls += 1;
            if self.state.label.kind.ends_with("_BUILD") {
                metrics.rows_out = metrics.stored_rows.unwrap_or(metrics.inserted);
            }
        });
    }
}
/// Demand-preserving counter for an input or insertion stream.
pub struct OperatorInput<I> {
    source: Option<I>,
    state: Arc<State>,
    insert: bool,
    count: u64,
}
impl<I> OperatorInput<I> {
    fn flush(&mut self) {
        if self.count == 0 {
            return;
        }
        self.state.update(|metrics| {
            self.state.add_rows(&mut metrics.rows_in, self.count);
            if self.insert {
                self.state.add_rows(&mut metrics.inserted, self.count);
            }
        });
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
            self.state.add_rows(&mut self.count, 1);
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
            self.state.add_rows(&mut self.count, 1);
            f(acc, row)
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.as_ref().expect("live iterator").size_hint()
    }
}
/// A build input drained on first demand; the drain time is the build's
/// source time, excluded from its exclusive construction time.
pub struct OperatorDrained<I: Iterator> {
    source: Option<I>,
    rows: std::vec::IntoIter<I::Item>,
    state: Arc<State>,
}
impl<I: Iterator> Iterator for OperatorDrained<I> {
    type Item = I::Item;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(source) = self.source.take() {
            let started = Instant::now();
            let rows: Vec<_> = source.collect();
            let elapsed = started.elapsed();
            self.state
                .update(|metrics| metrics.source_elapsed += elapsed);
            self.rows = rows.into_iter();
        }
        self.rows.next()
    }
}
/// Cumulative inclusive span; does not force a row the consumer did not request.
pub struct OperatorStream<I> {
    source: Option<I>,
    state: Arc<State>,
    started: Option<Instant>,
    count: u64,
}
impl<I> OperatorStream<I> {
    fn finish(&mut self) {
        let elapsed = self.started.take().map(|started| started.elapsed());
        if elapsed.is_none() && self.count == 0 {
            return;
        }
        self.state.update(|metrics| {
            crate::calibration_select! {
                plain {}
                calibration { metrics.row_observed = true; }
            }
            if let Some(elapsed) = elapsed {
                metrics.elapsed += elapsed;
                metrics.calls += 1;
            }
            self.state.add_rows(&mut metrics.rows_out, self.count);
        });
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
                self.state.add_rows(&mut self.count, 1);
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
                self.state.add_rows(&mut self.count, 1);
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
        let exclusive = if r.label.kind.ends_with("_BUILD") {
            format!(
                " excl={:.3}ms",
                r.elapsed.saturating_sub(r.source_elapsed).as_secs_f64() * 1000.
            )
        } else {
            String::new()
        };
        writeln!(
            out,
            "{}{:<16} {} [{}] inputs={} rows={} in={} inserted={} t={:.3}ms{exclusive}{}",
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
    let exclusive: Duration = records
        .iter()
        .filter(|r| r.label.kind.ends_with("_BUILD"))
        .map(|r| r.elapsed.saturating_sub(r.source_elapsed))
        .sum();
    writeln!(
        out,
        "BUILD_EXCLUSIVE_TOTAL t={:.3}ms",
        exclusive.as_secs_f64() * 1000.
    )
    .unwrap();
    if let Some(total) = records
        .iter()
        .find(|record| record.label.kind == "TASKS_BUILD_WALL_TOTAL")
    {
        writeln!(
            out,
            "TASKS_BUILD_WALL_TOTAL t={:.9}ms",
            total.elapsed.as_secs_f64() * 1000.
        )
        .unwrap();
    }
    out
}

/// Shared identity of the registered build. A deferred provider can migrate to
/// a worker without losing attribution; resetting drops the old registry, so
/// subsequent work on an old handle cannot contaminate a new query epoch.
#[derive(Clone)]
pub(crate) struct DeferredBuild(Arc<State>);
pub(crate) fn defer_build() -> Option<DeferredBuild> {
    ACTIVE_BUILD.with(|active| {
        active.borrow().as_ref().map(|(state, deferred)| {
            deferred.store(true, Ordering::Relaxed);
            DeferredBuild(state.clone())
        })
    })
}
pub(crate) struct DeferredClock(Option<OperatorClock>);
impl DeferredBuild {
    pub(crate) fn cardinality(&self, rows: usize) {
        self.0.update(|metrics| {
            crate::calibration_select! {
                plain {}
                calibration { metrics.row_observed = true; }
            }
            self.0.add_rows(metrics.stored_rows.get_or_insert(0), rows as u64);
        });
    }
    pub(crate) fn start(&self) -> DeferredClock {
        DeferredClock(Some(OperatorClock {
            state: self.0.clone(),
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
    count: AtomicU64,
    state: Arc<State>,
    source: Option<Arc<State>>,
}
impl<P> Drop for OperatorPredicate<P> {
    fn drop(&mut self) {
        let count = *self.count.get_mut();
        self.state.update(|metrics| {
            self.state.add_rows(&mut metrics.rows_in, count);
            crate::calibration_select! {
                plain {}
                calibration { metrics.row_observed |= count != 0; }
            }
        });
        if let Some(source) = &self.source {
            source.update(|metrics| {
                source.add_rows(&mut metrics.rows_out, count);
                crate::calibration_select! {
                    plain {}
                    calibration { metrics.row_observed |= count != 0; }
                }
            });
            #[cfg(feature = "planner-calibration")]
            if self.state.row_overflow.load(Ordering::Relaxed) {
                source.row_overflow.store(true, Ordering::Relaxed);
            }
        }
    }
}
impl<K, P: crate::ReadPredicate<K>> crate::ReadPredicate<K> for OperatorPredicate<P> {
    #[inline]
    fn test(&self, key: &K) -> bool {
        crate::calibration_select! {
            plain { self.count.fetch_add(1, Ordering::Relaxed); }
            calibration {
                if self.count.fetch_add(1, Ordering::Relaxed) == u64::MAX {
                    self.state.row_overflow.store(true, Ordering::Relaxed);
                }
            }
        }
        self.predicate.test(key)
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        self.predicate.any_in(lo, hi)
    }
}

/// Row-only cursor boundary, suitable for repeated keyed reads without clocks.
pub struct OperatorOutput<I> {
    source: Option<I>,
    state: Arc<State>,
    count: u64,
    #[cfg(feature = "planner-calibration")]
    polled: bool,
}
impl<I> OperatorOutput<I> {
    fn flush(&mut self) {
        if crate::calibration_select! {
            plain { self.count == 0 }
            calibration { self.count == 0 && !self.polled }
        } {
            return;
        }
        self.state.update(|metrics| {
            self.state.add_rows(&mut metrics.rows_out, self.count);
            crate::calibration_select! {
                plain {}
                calibration { metrics.row_observed = true; }
            }
        });
        crate::calibration_select! {
            plain {}
            calibration { self.polled = false; }
        }
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
        crate::calibration_select! {
            plain {}
            calibration { self.polled = true; }
        }
        let row = self.source.as_mut().expect("live iterator").next();
        if row.is_some() {
            self.state.add_rows(&mut self.count, 1);
        } else {
            self.flush();
        }
        row
    }
    fn fold<B, F>(mut self, init: B, mut f: F) -> B
    where
        F: FnMut(B, Self::Item) -> B,
    {
        crate::calibration_select! {
            plain {}
            calibration { self.polled = true; }
        }
        let source = self.source.take().expect("live iterator");
        source.fold(init, |acc, row| {
            self.state.add_rows(&mut self.count, 1);
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
            state.update(|metrics| {
                crate::calibration_select! {
                    plain {}
                    calibration { metrics.row_observed = true; }
                }
                state.add_rows(metrics.stored_rows.get_or_insert(0), rows as u64);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counters::{self, TaskContext};

    const SCAN: OperatorLabel = OperatorLabel {
        result: "scan",
        kind: "SCAN",
        key: "",
        inputs: &[],
    };
    const BUILD: OperatorLabel = OperatorLabel {
        result: "build",
        kind: "TASKS_BUILD_WALL",
        key: "",
        inputs: &[],
    };

    #[test]
    fn profile_handles_preserve_send_sync() {
        fn shareable<T: Send + Sync>() {}
        shareable::<OperatorProgram>();
        shareable::<OperatorInput<std::ops::Range<u64>>>();
        shareable::<OperatorOutput<std::ops::Range<u64>>>();
        shareable::<OperatorStream<std::ops::Range<u64>>>();
        shareable::<OperatorDrained<std::ops::Range<u64>>>();
        shareable::<OperatorPredicate<()>>();
        shareable::<DeferredBuild>();
        shareable::<OperatorScope>();
        shareable::<TaskContext>();
    }

    #[test]
    fn task_registration_and_spans_reach_the_originating_epoch() {
        counters::reset();
        let context = TaskContext::capture();
        std::thread::scope(|threads| {
            threads
                .spawn(|| {
                    context.run(|| {
                        let program = OperatorProgram::new(&[SCAN]);
                        assert_eq!(program.stream(0, 0..4).sum::<u64>(), 6);
                    })
                })
                .join()
                .unwrap();
        });
        let records = operators();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].calls, 1);
        assert!(records[0].elapsed > Duration::ZERO);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn worker_rows_and_primitive_costs_merge_once() {
        counters::reset();
        counters::record(|counts| counts.lookups += 1);
        let program = OperatorProgram::new(&[
            SCAN,
            OperatorLabel {
                result: "index",
                kind: "INDEX_BUILD",
                key: "",
                inputs: &["scan"],
            },
        ]);
        std::thread::scope(|threads| {
            for _ in 0..4 {
                let program = &program;
                threads.spawn(move || {
                    program.task(|| {
                        counters::record(|counts| counts.lookups += 3);
                        program.task(|| counters::record(|counts| counts.lookups += 2));
                        assert_eq!(program.stream(0, 0..3).sum::<u64>(), 3);
                        let rows: Vec<_> =
                            program.build(1, || program.inserting(1, 0..2).collect());
                        assert_eq!(rows, [0, 1]);
                    })
                });
            }
        });
        let records = operators();
        assert_eq!(records[0].rows_out, 12);
        assert_eq!(records[0].calls, 4);
        assert_eq!(records[1].rows_inserted, 8);
        assert_eq!(records[1].rows_out, 8);
        assert_eq!(records[1].calls, 4);
        assert_eq!(counters::snapshot().lookups, 21);
        assert_eq!(counters::snapshot().lookups, 21);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn task_store_and_sort_canon_match_independent_primitives() {
        counters::reset();
        let program = OperatorProgram::new(&[
            OperatorLabel {
                result: "rows",
                kind: "ROWS_BUILD",
                key: "",
                inputs: &[],
            },
            OperatorLabel {
                result: "parts",
                kind: "PARTITION_BUILD",
                key: "",
                inputs: &[],
            },
            OperatorLabel {
                result: "canon",
                kind: "DEDUP",
                key: "",
                inputs: &["parts"],
            },
        ]);
        std::thread::scope(|threads| {
            for _ in 0..4 {
                let program = &program;
                threads.spawn(move || {
                    program.task(|| {
                        let rows: Vec<_> = program.build(0, || {
                            program.inserting(0, [1u64, 1, 2].into_iter()).collect()
                        });
                        let _rows = crate::retain_rows(&rows);
                        let part: Vec<_> = program
                            .build(1, || program.inserting(1, rows.iter().copied()).collect());
                        let _part = crate::retain_rows(&part);
                        let canonical =
                            crate::dedup::sort(program.inserting(2, part.into_iter()), Ord::cmp);
                        assert_eq!(program.stream(2, canonical).collect::<Vec<_>>(), [1, 2]);
                    })
                });
            }
        });
        let records = operators();
        let counts = counters::snapshot();
        assert_eq!(records[0].rows_inserted, 12);
        assert_eq!(records[1].rows_inserted, 12);
        assert_eq!(records[2].rows_inserted, 12);
        assert_eq!(records[2].rows_out, 8);
        assert_eq!(
            counts.rows_copied,
            records[0].rows_inserted + records[1].rows_inserted
        );
        assert_eq!(counts.distinct_insertions, 0);
        assert_eq!(counts.hash_computations, 0);
        assert_eq!(counts.bytes_retained_current, 0);
        assert!(counts.bytes_retained_peak > 0);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn transferred_allocations_keep_their_epoch_until_drop() {
        counters::reset();
        let context = TaskContext::capture();
        let allocation = std::thread::scope(|threads| {
            threads
                .spawn(|| context.run(|| crate::counters_memory::Allocation::new(128)))
                .join()
                .unwrap()
        });
        assert_eq!(counters::snapshot().bytes_retained_current, 128);
        assert_eq!(counters::snapshot().bytes_retained_peak, 128);
        drop(allocation);
        assert_eq!(counters::snapshot().bytes_retained_current, 0);
        let old = crate::counters_memory::Allocation::new(64);
        counters::reset();
        drop(old);
        assert_eq!(counters::snapshot().bytes_retained_current, 0);
        assert_eq!(counters::snapshot().bytes_retained_peak, 0);
    }

    #[test]
    fn build_wall_is_one_union_across_nested_and_concurrent_programs() {
        counters::reset();
        let context = TaskContext::capture();
        let outer = OperatorProgram::new(&[BUILD]);
        let clock = outer.start_scope(0);
        let first_start = context.0.build.lock().unwrap().started;
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|threads| {
            for _ in 0..2 {
                let context = &context;
                let barrier = &barrier;
                threads.spawn(move || {
                    context.run(|| {
                        let nested = OperatorProgram::new(&[BUILD]);
                        let clock = nested.start_scope(0);
                        barrier.wait();
                        barrier.wait();
                        drop(clock);
                    })
                });
            }
            barrier.wait();
            {
                let build = context.0.build.lock().unwrap();
                assert_eq!(build.active, 3);
                assert_eq!(build.started, first_start);
                assert_eq!(build.elapsed, Duration::ZERO);
            }
            barrier.wait();
        });
        drop(clock);
        let records = operators();
        let totals: Vec<_> = records
            .iter()
            .filter(|record| record.label.kind == "TASKS_BUILD_WALL_TOTAL")
            .collect();
        assert_eq!(totals.len(), 1);
        assert!(totals[0].elapsed > Duration::ZERO);
        assert_eq!(Some(totals[0].elapsed), context.build_wall());
        assert!(
            render_operators(&records)
                .lines()
                .any(|line| line.starts_with("TASKS_BUILD_WALL_TOTAL t="))
        );
        counters::reset();
        assert!(operators().is_empty());
    }
}

#[cfg(all(test, feature = "planner-calibration"))]
mod calibration_overflow_tests {
    use super::*;

    #[test]
    fn local_cursor_push_and_atomic_predicate_overflows_are_observable() {
        const LABEL: OperatorLabel = OperatorLabel {
            result: "rows", kind: "SCAN", key: "", inputs: &[],
        };
        super::super::TaskContext::default().run(|| {
            let program = OperatorProgram::new(&[LABEL; 5]);
            let mut stream = program.stream(0, std::iter::once(1));
            stream.count = u64::MAX;
            assert_eq!(stream.next(), Some(1));
            drop(stream);
            let mut input = program.input(1, std::iter::once(1));
            input.count = u64::MAX;
            assert_eq!(input.next(), Some(1));
            drop(input);
            let mut output = program.output(2, std::iter::once(1));
            output.count = u64::MAX;
            assert_eq!(output.next(), Some(1));
            drop(output);
            let mut push = program.push(3);
            push.output = u64::MAX;
            push.output();
            drop(push);
            struct All;
            impl crate::ReadPredicate<i32> for All {
                fn test(&self, _: &i32) -> bool { true }
                fn any_in(&self, _: &i32, _: &i32) -> bool { true }
            }
            let predicate = program.predicate(4, None, All);
            predicate.count.store(u64::MAX, Ordering::Relaxed);
            assert!(crate::ReadPredicate::test(&predicate, &1));
            drop(predicate);
            assert!(operators().iter().all(|record| record.row_overflow));
        });
    }
}

#[cfg(all(test, feature = "planner-calibration"))]
mod calibration_observation_tests {
    use super::*;

    #[test]
    fn concrete_zero_build_cardinality_is_observed_without_a_row_or_clock_event() {
        const LABEL: OperatorLabel = OperatorLabel {
            result: "index", kind: "INDEX_BUILD", key: "", inputs: &[],
        };
        super::super::TaskContext::default().run(|| {
            let program = OperatorProgram::new(&[LABEL; 3]);
            program.build(0, || build_cardinality(0));
            let deferred = program.build(1, || defer_build().unwrap());
            assert!(!operators()[1].row_observed, "a deferred handle does not build rows");
            deferred.cardinality(0);
            let records = operators();
            assert_eq!(records.iter().map(|record| record.row_observed).collect::<Vec<_>>(), [true, true, false]);
            assert_eq!(records[1].calls, 0);
            assert!(records.iter().all(|record| record.rows_out == 0));
        });
    }
}
