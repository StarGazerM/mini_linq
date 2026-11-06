#![cfg(feature = "planner-calibration")]
use cq_storage::counters::{self, TaskContext};
use cq_storage::counters::calibration::{self, CalibrationRow, Observer, ScalarValue};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Default, Debug)]
struct Metrics {
    begins: usize,
    ends: usize,
    raw: Vec<(i64, String)>,
    admitted: Vec<(i64, String)>,
    canonical: Vec<(usize, usize)>,
    seeds: Vec<(usize, Vec<(usize, usize)>)>,
    rounds: Vec<(usize, usize, Vec<(usize, usize)>)>,
    finals: Vec<(usize, usize, Vec<(usize, usize, usize)>)>,
}
#[derive(Default)]
struct Recorder(Mutex<Metrics>);
fn row(row: &dyn CalibrationRow) -> (i64, String) {
    assert_eq!(row.len(), 2);
    let (ScalarValue::Signed(number), ScalarValue::Utf8(name)) = (row.cell(0), row.cell(1)) else {
        panic!("callbacks preserve borrowed declaration-order columns")
    };
    (number, name.to_owned())
}
impl Observer for Recorder {
    fn program_begin(&self, round_ir: &'static str) {
        assert!(!round_ir.is_empty());
        self.0.lock().unwrap().begins += 1;
    }
    fn program_end(&self) { self.0.lock().unwrap().ends += 1; }
    fn input_raw(&self, relation: usize, input: &dyn CalibrationRow) {
        assert_eq!(relation, 0);
        self.0.lock().unwrap().raw.push(row(input));
    }
    fn input_admitted(&self, relation: usize, input: &dyn CalibrationRow) {
        assert_eq!(relation, 0);
        self.0.lock().unwrap().admitted.push(row(input));
    }
    fn input_end(&self, relation: usize, canonical: usize) {
        self.0.lock().unwrap().canonical.push((relation, canonical));
    }
    fn seed_end(&self, stratum: usize, rows: &[(usize, usize)]) {
        self.0.lock().unwrap().seeds.push((stratum, rows.to_vec()));
    }
    fn round_begin(&self, stratum: usize, round: usize, deltas: &[(usize, usize)]) {
        self.0.lock().unwrap().rounds.push((stratum, round, deltas.to_vec()));
    }
    fn stratum_end(&self, stratum: usize, rounds: usize, relations: &[(usize, usize, usize)]) {
        self.0.lock().unwrap().finals.push((stratum, rounds, relations.to_vec()));
    }
}

#[test]
fn duplicate_admission_seed_consumption_and_completion_match_pull_and_push() {
    macro_rules! run {
        ($terminal:path, $input:ident) => {{
            let recorder = Arc::new(Recorder::default());
            TaskContext::default().run(|| calibration::with_observer(recorder.clone(), || {
                let rows = [(0_i32, String::from("seed")), (0, String::from("seed"))];
                let result: BTreeSet<_> = cq_datalog::datalog! {
                    pipeline { cq = pipeline { print = $terminal; print_input = $input; }; }
                    struct ObservedRounds;
                    relation E(x:i32,label:String) = (&rows);
                    relation R(x:i32);
                    output R;
                    R(*x) :- E(x,label);
                    R(*x + 1) :- R(x), where x < 2;
                }.collect();
                assert_eq!(result, BTreeSet::from([(0,), (1,), (2,)]));
                let records = counters::operators();
                let observed: Vec<_> = records.iter().filter(|record| record.rows_out != 0).collect();
                assert!(!observed.is_empty());
                assert!(observed.iter().all(|record| record.calibration.is_some()));
                assert!(observed.iter().any(|record| record.calibration.as_ref().unwrap().round.is_none()));
                assert!(observed.iter().any(|record| record.calibration.as_ref().unwrap().round == Some(1)));
                assert!(calibration::observer_failures().is_empty());
            }));
            let metrics = recorder.0.lock().unwrap();
            assert_eq!((metrics.begins, metrics.ends), (1, 1));
            assert_eq!(metrics.raw, [(0, "seed".to_owned()), (0, "seed".to_owned())]);
            assert_eq!(metrics.admitted, [(0, "seed".to_owned())]);
            assert_eq!(metrics.canonical, [(0, 1)]);
            assert_eq!(metrics.seeds, [(0, vec![(1, 1)])]);
            assert_eq!(metrics.rounds, [
                (0, 0, vec![(1, 1)]),
                (0, 1, vec![(1, 1)]),
                (0, 2, vec![(1, 1)]),
            ]);
            assert_eq!(metrics.finals, [(0, 3, vec![(1, 3, 0)])]);
        }};
    }
    run!(::mini_linq::passes::translate_pull, sequence);
    run!(::cq_datalog::passes::print_push, structures);
}

#[test]
fn seed_only_stratum_records_its_executed_empty_body_round() {
    let recorder = Arc::new(Recorder::default());
    TaskContext::default().run(|| calibration::with_observer(recorder.clone(), || {
        let rows = [(7_i32, String::from("seed")), (7, String::from("seed"))];
        let result: Vec<_> = cq_datalog::datalog! {
            struct SeedOnly;
            relation E(x:i32,label:String) = (&rows);
            relation R(x:i32);
            output R;
            R(*x) :- E(x,label);
        }.collect();
        assert_eq!(result, [(7,)]);
    }));
    let metrics = recorder.0.lock().unwrap();
    assert_eq!(metrics.seeds, [(0, vec![(1, 1)])]);
    // The recorded default loop consumes the nonempty seed frontier once,
    // despite having no steady CQ holes. This is not a terminal-empty round.
    assert_eq!(metrics.rounds, [(0, 0, vec![(1, 1)])]);
    assert_eq!(metrics.finals, [(0, 1, vec![(1, 1, 0)])]);
    assert_eq!(metrics.ends, 1);
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct NoCodec(u32);

fn generic_identity<T: Clone + Eq + Ord + std::hash::Hash>(rows: &[(T,)]) -> Vec<(T,)> {
    cq_datalog::datalog! {
        struct GenericIdentity;
        relation E(x:T) = (rows);
        relation R(x:T);
        output R;
        R(x) :- E(x);
    }.collect()
}

#[test]
fn custom_and_generic_columns_without_codecs_preserve_queries_and_report_unsupported() {
    #[derive(Default)]
    struct Missing {
        counts: Mutex<(usize, usize)>,
        raw_address: Mutex<usize>,
    }
    impl Observer for Missing {
        fn input_raw(&self, _: usize, row: &dyn CalibrationRow) {
            assert_eq!(row.len(), 1);
            assert_eq!(row.cell(0), ScalarValue::Unsupported("no evidence scalar codec"));
            *self.raw_address.lock().unwrap() = row as *const dyn CalibrationRow as *const () as usize;
            self.counts.lock().unwrap().0 += 1;
        }
        fn input_admitted(&self, _: usize, row: &dyn CalibrationRow) {
            assert_eq!(row.cell(0), ScalarValue::Unsupported("no evidence scalar codec"));
            assert_eq!(
                row as *const dyn CalibrationRow as *const () as usize,
                *self.raw_address.lock().unwrap(),
                "raw and admitted callbacks reuse the same borrowed stack row",
            );
            self.counts.lock().unwrap().1 += 1;
        }
    }
    let rows = [(NoCodec(7),), (NoCodec(7),)];
    assert_eq!(generic_identity(&rows), [(NoCodec(7),)]);
    let observer = Arc::new(Missing::default());
    TaskContext::default().run(|| calibration::with_observer(observer.clone(), || {
        let actual: Vec<_> = cq_datalog::datalog! {
            struct ConcreteMissingCodec;
            relation E(x:NoCodec) = (&rows);
            relation R(x:NoCodec);
            output R;
            R(x) :- E(x);
        }.collect();
        assert_eq!(actual, [(NoCodec(7),)]);
        assert_eq!(generic_identity(&rows), [(NoCodec(7),)]);
    }));
    assert_eq!(*observer.counts.lock().unwrap(), (4, 2));
}

#[test]
fn installed_codec_is_not_evaluated_without_an_observer() {
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct Encoded(u32);
    impl calibration::CalibrationCell for Encoded {
        fn calibration_value(&self) -> ScalarValue<'_> {
            panic!("no session may evaluate an input codec")
        }
    }
    let rows = [(Encoded(3),)];
    let result: Vec<_> = cq_datalog::datalog! {
        struct UnobservedCodec;
        relation E(x:Encoded) = (&rows);
        relation R(x:Encoded);
        output R;
        R(x) :- E(x);
    }.collect();
    assert_eq!(result, [(Encoded(3),)]);
}
