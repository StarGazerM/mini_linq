//! Diagnostic-only observations, scoped to one caller and its joined tasks.
//! Callbacks borrow declaration-order rows and cannot certify a failed query.
//! Expected observer failures remain the observer's responsibility; unexpected
//! panics unwind without completing the program. Scopes never reset counters.
use std::cell::RefCell;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarValue<'a> {
    Signed(i64),
    Unsigned(u64),
    Decimal { unscaled: i64, scale: u32 },
    Date(i64),
    Utf8(&'a str),
    Symbol(u64),
    Unsupported(&'static str),
}
pub trait CalibrationCell {
    fn calibration_value(&self) -> ScalarValue<'_>;
}

/// Borrow one concrete input column without requiring a codec on its query type.
pub struct Cell<'a, T: ?Sized>(pub &'a T);

/// Method-call dispatch uses the owned receiver when a codec exists, otherwise
/// the autoref receiver reports a missing codec without narrowing query types.
pub trait ScalarCell<'a> {
    fn scalar(self) -> ScalarValue<'a>;
}
impl<'a, T: CalibrationCell + ?Sized> ScalarCell<'a> for Cell<'a, T> {
    fn scalar(self) -> ScalarValue<'a> { self.0.calibration_value() }
}
impl<'a, T: ?Sized> ScalarCell<'a> for &Cell<'a, T> {
    fn scalar(self) -> ScalarValue<'a> {
        ScalarValue::Unsupported("no evidence scalar codec")
    }
}
pub trait CalibrationRow {
    fn len(&self) -> usize;
    fn cell(&self, index: usize) -> ScalarValue<'_>;
    fn is_empty(&self) -> bool { self.len() == 0 }
}
impl<const N: usize> CalibrationRow for [ScalarValue<'_>; N] {
    fn len(&self) -> usize { N }
    fn cell(&self, index: usize) -> ScalarValue<'_> { self[index] }
}
impl<T: CalibrationCell + ?Sized> CalibrationCell for &T {
    fn calibration_value(&self) -> ScalarValue<'_> { (**self).calibration_value() }
}
impl<T: CalibrationCell + ?Sized> CalibrationCell for &mut T {
    fn calibration_value(&self) -> ScalarValue<'_> { (**self).calibration_value() }
}
impl<T: CalibrationRow + ?Sized> CalibrationRow for &T {
    fn len(&self) -> usize { (**self).len() }
    fn cell(&self, index: usize) -> ScalarValue<'_> { (**self).cell(index) }
}
macro_rules! integers {
    ($variant:ident, $target:ty; $($ty:ty),*) => { $(
        impl CalibrationCell for $ty {
            fn calibration_value(&self) -> ScalarValue<'_> {
                ScalarValue::$variant(*self as $target)
            }
        }
    )* };
}
integers!(Signed, i64; i8, i16, i32, i64, isize);
integers!(Unsigned, u64; u8, u16, u32, u64, usize, bool, char);
impl CalibrationCell for i128 {
    fn calibration_value(&self) -> ScalarValue<'_> {
        i64::try_from(*self).map_or(
            ScalarValue::Unsupported("signed integer exceeds the evidence scalar's i64 range"),
            ScalarValue::Signed,
        )
    }
}
impl CalibrationCell for u128 {
    fn calibration_value(&self) -> ScalarValue<'_> {
        u64::try_from(*self).map_or(
            ScalarValue::Unsupported("unsigned integer exceeds the evidence scalar's u64 range"),
            ScalarValue::Unsigned,
        )
    }
}
impl<T> CalibrationCell for Vec<T> {
    fn calibration_value(&self) -> ScalarValue<'_> {
        ScalarValue::Unsupported("vector-valued columns are not representable in planner evidence")
    }
}
impl CalibrationCell for str {
    fn calibration_value(&self) -> ScalarValue<'_> { ScalarValue::Utf8(self) }
}
impl CalibrationCell for String {
    fn calibration_value(&self) -> ScalarValue<'_> { ScalarValue::Utf8(self) }
}
impl CalibrationRow for () {
    fn len(&self) -> usize { 0 }
    fn cell(&self, _index: usize) -> ScalarValue<'_> { panic!("empty calibration row") }
}
macro_rules! tuple_rows {
    ($($ty:ident:$index:tt),+) => {
        impl<$($ty: CalibrationCell),+> CalibrationRow for ($($ty,)+) {
            fn len(&self) -> usize { [$(stringify!($ty)),+].len() }
            fn cell(&self, index: usize) -> ScalarValue<'_> {
                match index {
                    $($index => self.$index.calibration_value(),)+
                    _ => panic!("calibration column index out of bounds"),
                }
            }
        }
    };
}
tuple_rows!(A:0);
tuple_rows!(A:0,B:1);
tuple_rows!(A:0,B:1,C:2);
tuple_rows!(A:0,B:1,C:2,D:3);
tuple_rows!(A:0,B:1,C:2,D:3,E:4);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10,L:11);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10,L:11,M:12);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10,L:11,M:12,N:13);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10,L:11,M:12,N:13,O:14);
tuple_rows!(A:0,B:1,C:2,D:3,E:4,F:5,G:6,H:7,I:8,J:9,K:10,L:11,M:12,N:13,O:14,P:15);

pub trait Observer: Send + Sync {
    fn program_begin(&self, _round_ir: &'static str) {}
    fn program_end(&self) {}
    fn metadata(&self, _path: &str, _bytes: &[u8]) {}
    fn input_begin(&self, _relation: usize) {}
    fn input_raw(&self, _relation: usize, _row: &dyn CalibrationRow) {}
    fn input_admitted(&self, _relation: usize, _row: &dyn CalibrationRow) {}
    fn input_end(&self, _relation: usize, _canonical_rows: usize) {}
    fn stratum_begin(&self, _stratum: usize) {}
    fn seed_end(&self, _stratum: usize, _rows: &[(usize, usize)]) {}
    fn round_begin(&self, _stratum: usize, _round: usize, _deltas: &[(usize, usize)]) {}
    fn stratum_end(&self, _stratum: usize, _rounds: usize, _relations: &[(usize, usize, usize)]) {}
}
struct Session {
    observer: Arc<dyn Observer>,
    failures: Mutex<Vec<String>>,
}
impl Session {
    fn notify(&self, callback: impl FnOnce(&dyn Observer)) {
        callback(&*self.observer);
    }
}

type HostAliases = Option<Arc<Vec<(&'static str, &'static str)>>>;

/// Immutable invocation identity; the terminal fills the shared plan slot once.
#[derive(Clone, Debug)]
pub struct CalibrationFrame {
    pub hole: usize,
    pub stratum: Option<usize>,
    /// `None` denotes seed/key evaluation; steady rounds are zero-based.
    pub round: Option<usize>,
    trace: Arc<OnceLock<&'static str>>,
    host_aliases: HostAliases,
}
impl CalibrationFrame {
    pub fn trace(&self) -> Option<&'static str> { self.trace.get().copied() }
    /// Original host and generated parameter identifiers in inherited order.
    /// The printer replays each pair as `host -> (*parameter)`.
    pub fn host_aliases(&self) -> &[(&'static str, &'static str)] {
        self.host_aliases.as_deref().map_or(&[], Vec::as_slice)
    }
}
impl PartialEq for CalibrationFrame {
    fn eq(&self, other: &Self) -> bool {
        self.hole == other.hole && self.stratum == other.stratum
            && self.round == other.round && self.trace() == other.trace()
            && self.host_aliases() == other.host_aliases()
    }
}
impl Eq for CalibrationFrame {}
#[derive(Clone, Default)]
pub(super) struct Context {
    session: Option<Arc<Session>>,
    stratum: Option<usize>,
    round: Option<usize>,
    frame: Option<CalibrationFrame>,
    host_aliases: HostAliases,
}
impl Context {
    pub(super) fn frame(&self) -> Option<CalibrationFrame> { self.frame.clone() }
}
// Runtime state consists only of restored identities and immutable handles;
// observer-owned mutable state is not inspected during restoration.
impl std::panic::UnwindSafe for Context {}
impl std::panic::RefUnwindSafe for Context {}
thread_local! {
    static CONTEXT: RefCell<Context> = RefCell::new(Context::default());
}
pub(super) fn capture() -> Context { CONTEXT.with(|context| context.borrow().clone()) }
pub(super) fn replace(context: Context) -> Context { CONTEXT.with(|current| current.replace(context)) }
pub fn active_frame() -> Option<CalibrationFrame> { CONTEXT.with(|context| context.borrow().frame.clone()) }
/// Skip input codec evaluation entirely when no calibration session is installed.
pub fn observing() -> bool { CONTEXT.with(|context| context.borrow().session.is_some()) }
fn notify(callback: impl FnOnce(&dyn Observer)) {
    let session = CONTEXT.with(|context| context.borrow().session.clone());
    if let Some(session) = session { session.notify(callback); }
}
/// Metadata registration failures in this session, including joined tasks.
pub fn observer_failures() -> Vec<String> {
    CONTEXT.with(|context| context.borrow().session.clone()).map_or_else(Vec::new, |session| {
        session.failures.lock().unwrap_or_else(|error| error.into_inner()).clone()
    })
}
struct Restore(Context);
impl Drop for Restore {
    fn drop(&mut self) { replace(std::mem::take(&mut self.0)); }
}
pub fn with_observer<T>(observer: Arc<dyn Observer>, run: impl FnOnce() -> T) -> T {
    let previous = replace(Context {
        session: Some(Arc::new(Session { observer, failures: Mutex::new(Vec::new()) })),
        ..Context::default()
    });
    let _restore = Restore(previous);
    run()
}
#[must_use = "complete only after the whole query succeeds"]
pub struct ProgramGuard {
    previous: Option<Context>,
    session: Option<Arc<Session>>,
}
impl ProgramGuard {
    pub fn complete(self) {
        if let Some(session) = &self.session { session.notify(|observer| observer.program_end()); }
    }
}
impl Drop for ProgramGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() { replace(previous); }
    }
}
pub fn program_begin(round_ir: &'static str) -> ProgramGuard {
    let previous = capture();
    let session = previous.session.clone();
    replace(Context { session: session.clone(), ..Context::default() });
    let guard = ProgramGuard { previous: Some(previous), session };
    notify(|observer| observer.program_begin(round_ir));
    guard
}
/// Compiler-owned alpha substitutions for one partition, inherited by tasks.
/// Replacing the active frame leaves previously captured frames immutable.
#[must_use = "keep the alias scope through partition evaluation"]
pub struct HostAliasGuard {
    previous_aliases: HostAliases,
    previous_frame: Option<CalibrationFrame>,
}
impl Drop for HostAliasGuard {
    fn drop(&mut self) {
        CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            context.host_aliases = self.previous_aliases.take();
            context.frame = self.previous_frame.take();
        });
    }
}
pub fn host_alias_scope(aliases: &'static [(&'static str, &'static str)]) -> HostAliasGuard {
    CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        let guard = HostAliasGuard {
            previous_aliases: context.host_aliases.clone(),
            previous_frame: context.frame.clone(),
        };
        if !aliases.is_empty() {
            let outer = context.host_aliases.as_deref().map_or(&[][..], Vec::as_slice);
            let mut merged = Vec::with_capacity(outer.len() + aliases.len());
            merged.extend_from_slice(outer);
            for &(host, replacement) in aliases {
                if let Some(entry) = merged.iter_mut().find(|entry| entry.0 == host) {
                    entry.1 = replacement;
                } else {
                    merged.push((host, replacement));
                }
            }
            context.host_aliases = Some(Arc::new(merged));
        }
        if let Some(mut frame) = context.frame.clone() {
            frame.host_aliases = context.host_aliases.clone();
            context.frame = Some(frame);
        }
        guard
    })
}

#[must_use = "keep the hole scope through iterator drain and task joins"]
pub struct HoleGuard(Option<CalibrationFrame>);
impl Drop for HoleGuard {
    fn drop(&mut self) { CONTEXT.with(|context| context.borrow_mut().frame = self.0.take()); }
}
pub fn hole_scope(hole: usize) -> HoleGuard {
    CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        let frame = CalibrationFrame {
            hole, stratum: context.stratum, round: context.round,
            trace: Arc::new(OnceLock::new()),
            host_aliases: context.host_aliases.clone(),
        };
        HoleGuard(context.frame.replace(frame))
    })
}
pub fn register_plan(trace: &'static str) {
    if let Some(frame) = active_frame() {
        if let Err(other) = frame.trace.set(trace) {
            if frame.trace() != Some(other) {
                if let Some(session) = capture().session {
                    session.failures.lock().unwrap_or_else(|error| error.into_inner())
                        .push("conflicting calibration plans in one hole invocation".to_owned());
                }
            }
        }
    }
}
/// Observe bytes already read by the caller; this hook performs no I/O.
pub fn metadata(path: &str, bytes: &[u8]) { notify(|observer| observer.metadata(path, bytes)); }
pub fn input_begin(relation: usize) { notify(|observer| observer.input_begin(relation)); }
pub fn input_raw(relation: usize, row: &dyn CalibrationRow) { notify(|observer| observer.input_raw(relation, row)); }
pub fn input_admitted(relation: usize, row: &dyn CalibrationRow) { notify(|observer| observer.input_admitted(relation, row)); }
pub fn input_end(relation: usize, canonical_rows: usize) { notify(|observer| observer.input_end(relation, canonical_rows)); }
pub fn stratum_begin(stratum: usize) {
    CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        context.stratum = Some(stratum);
        context.round = None;
    });
    notify(|observer| observer.stratum_begin(stratum));
}
pub fn seed_end(stratum: usize, rows: &[(usize, usize)]) { notify(|observer| observer.seed_end(stratum, rows)); }
pub fn round_begin(stratum: usize, round: usize, deltas: &[(usize, usize)]) {
    CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        context.stratum = Some(stratum);
        context.round = Some(round);
    });
    notify(|observer| observer.round_begin(stratum, round, deltas));
}
pub fn stratum_end(stratum: usize, rounds: usize, relations: &[(usize, usize, usize)]) {
    notify(|observer| observer.stratum_end(stratum, rounds, relations));
    CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        context.stratum = None;
        context.round = None;
    });
}
