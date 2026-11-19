//! PS-After: what a program hands a statistics collector once it has its
//! result. Nothing here runs while a program evaluates. The rows are
//! borrowed in declaration order and visited only if the observer asks.
use std::cell::RefCell;
use std::sync::Arc;

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

/// Borrow one concrete column without requiring a codec on its query type.
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

/// When a row was visible, on the program's publication clock: the clock
/// ticks once per publication (a round's, under a synchronous schedule;
/// a commit's, under a relational one), whatever the schedule. A row is
/// visible from the publication that published its delta; it is consumed
/// as a delta row until the publication that replaces its delta, and
/// `horizon` is the last publication before that one. A row `r` could be
/// met by a delta row `c` exactly when `r.published <= c.horizon`. Under
/// a synchronous schedule both are the row's round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Age {
    pub published: usize,
    pub horizon: usize,
}
/// A pass over a relation's rows: call it with the visitor of each row and
/// its age, ascending along the pass. The deltas are kept, not copied,
/// and only for a derived relation whose rounds the observer needs: the
/// program moves each of its deltas aside where it would drop it. Every
/// other row has the default age.
pub type RelationRows<'a> = &'a mut dyn FnMut(&mut dyn FnMut(Age, &dyn CalibrationRow));

pub trait Observer: Send + Sync {
    /// A program is about to run; `round_ir` is its recorded Round text.
    fn program_begin(&self, _round_ir: &'static str) {}
    /// Asked once per derived relation as the program begins: whether the
    /// round of each of its rows is needed. Only then are its deltas kept.
    fn needs_rounds(&self, _relation: usize) -> bool { false }
    /// PS-Heavy: once per relation of a program that has its result,
    /// before any relation is offered by `relation`: its position and a
    /// pass over its rows (their ages are not meant). An observer that
    /// never calls `pass` costs none.
    fn survey(&self, _relation: usize, _pass: RelationRows<'_>) {}
    /// Once per relation of a program that has its result: its position
    /// among the declarations, the rows it holds where that is known
    /// without a pass, and a pass over them. An observer that never calls
    /// `pass` costs none.
    fn relation(&self, _relation: usize, _rows: Option<usize>, _pass: RelationRows<'_>) {}
    /// The program returned its result.
    fn program_end(&self) {}
}

thread_local! {
    static OBSERVER: RefCell<Option<Arc<dyn Observer>>> = const { RefCell::new(None) };
}
fn notify(callback: impl FnOnce(&dyn Observer)) {
    let observer = OBSERVER.with(|observer| observer.borrow().clone());
    if let Some(observer) = observer { callback(&*observer); }
}
/// Whether an observer is installed on this thread.
pub fn observing() -> bool { OBSERVER.with(|observer| observer.borrow().is_some()) }
/// Run with `observer` installed on this thread; the previous one is
/// restored when `run` returns or unwinds.
pub fn with_observer<T>(observer: Arc<dyn Observer>, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Arc<dyn Observer>>);
    impl Drop for Restore {
        fn drop(&mut self) { OBSERVER.with(|observer| *observer.borrow_mut() = self.0.take()); }
    }
    let _restore = Restore(OBSERVER.with(|current| current.replace(Some(observer))));
    run()
}
pub fn program_begin(round_ir: &'static str) { notify(|observer| observer.program_begin(round_ir)); }
/// Whether the installed observer needs a derived relation's rounds; `false` without one.
pub fn needs_rounds(relation: usize) -> bool {
    OBSERVER.with(|observer| observer.borrow().clone()).is_some_and(|observer| observer.needs_rounds(relation))
}
pub fn survey(relation: usize, pass: RelationRows<'_>) {
    notify(|observer| observer.survey(relation, pass));
}
pub fn relation(relation: usize, rows: Option<usize>, pass: RelationRows<'_>) {
    notify(|observer| observer.relation(relation, rows, pass));
}
pub fn program_end() { notify(|observer| observer.program_end()); }

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Seen(Mutex<Vec<String>>);
    impl Observer for Seen {
        fn program_begin(&self, round_ir: &'static str) { self.0.lock().unwrap().push(format!("begin {round_ir}")); }
        fn relation(&self, relation: usize, rows: Option<usize>, pass: RelationRows<'_>) {
            let mut cells = Vec::new();
            if relation == 1 { pass(&mut |age, row| cells.push(format!("{}: {}", age.published, (0..row.len()).map(|index| format!("{:?}", row.cell(index))).collect::<Vec<_>>().join(" ")))); }
            self.0.lock().unwrap().push(format!("relation {relation} of {rows:?}: {cells:?}"));
        }
        fn program_end(&self) { self.0.lock().unwrap().push("end".into()); }
    }

    fn program(passes: &std::cell::Cell<usize>) {
        program_begin("p");
        for relation in 0..2 {
            relation_of(relation, passes);
        }
        program_end();
    }
    fn relation_of(index: usize, passes: &std::cell::Cell<usize>) {
        let stored = [(7i32, "a"), (8, "b")];
        relation(index, (index == 0).then_some(stored.len()), &mut |visit| {
            passes.set(passes.get() + 1);
            for (round, row) in stored.iter().enumerate() {
                let row = [Cell(&row.0).scalar(), Cell(row.1).scalar()];
                visit(Age { published: round, horizon: round }, &row);
            }
        });
    }

    #[test]
    fn a_relation_is_passed_over_only_when_the_observer_asks() {
        let passes = std::cell::Cell::new(0);
        program(&passes);
        assert_eq!(passes.get(), 0, "no observer, no pass");
        let seen = Arc::new(Seen::default());
        with_observer(seen.clone(), || program(&passes));
        assert!(!observing());
        assert_eq!(passes.get(), 1, "the observer asked for one relation's rows");
        assert_eq!(*seen.0.lock().unwrap(), [
            "begin p",
            "relation 0 of Some(2): []",
            "relation 1 of None: [\"0: Signed(7) Utf8(\\\"a\\\")\", \"1: Signed(8) Utf8(\\\"b\\\")\"]",
            "end",
        ]);
    }

    #[test]
    fn a_column_without_a_codec_is_reported_unsupported() {
        struct Opaque;
        let value = Opaque;
        assert_eq!((&Cell(&value)).scalar(), ScalarValue::Unsupported("no evidence scalar codec"));
        assert_eq!(Cell(&5u8).scalar(), ScalarValue::Unsigned(5));
    }
}
