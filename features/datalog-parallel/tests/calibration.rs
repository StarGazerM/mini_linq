#![cfg(feature = "planner-calibration")]
use cq_storage::counters::{self, TaskContext};
use cq_storage::counters::calibration::{self, CalibrationRow, Observer};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Metrics {
    raw: usize,
    admitted: usize,
    canonical: Vec<(usize, usize)>,
    seeds: Vec<(usize, Vec<(usize, usize)>)>,
    rounds: Vec<(usize, usize, Vec<(usize, usize)>)>,
    finals: Vec<(usize, usize, Vec<(usize, usize, usize)>)>,
    completed: usize,
}
#[derive(Default)]
struct Recorder(Mutex<Metrics>);
impl Observer for Recorder {
    fn input_raw(&self, _: usize, _: &dyn CalibrationRow) { self.0.lock().unwrap().raw += 1; }
    fn input_admitted(&self, _: usize, _: &dyn CalibrationRow) { self.0.lock().unwrap().admitted += 1; }
    fn input_end(&self, relation: usize, canonical: usize) { self.0.lock().unwrap().canonical.push((relation, canonical)); }
    fn seed_end(&self, stratum: usize, rows: &[(usize, usize)]) { self.0.lock().unwrap().seeds.push((stratum, rows.to_vec())); }
    fn round_begin(&self, stratum: usize, round: usize, deltas: &[(usize, usize)]) { self.0.lock().unwrap().rounds.push((stratum, round, deltas.to_vec())); }
    fn stratum_end(&self, stratum: usize, rounds: usize, rows: &[(usize, usize, usize)]) { self.0.lock().unwrap().finals.push((stratum, rounds, rows.to_vec())); }
    fn program_end(&self) { self.0.lock().unwrap().completed += 1; }
}

#[test]
fn chunked_push_tasks_keep_hole_round_counter_epoch_through_join() {
    for workers in [1, 4] {
        rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
            let observer = Arc::new(Recorder::default());
            TaskContext::default().run(|| calibration::with_observer(observer.clone(), || {
                let rows: Vec<_> = (0..32_i32).chain(0..32).map(|x| (x,)).collect();
                let actual: BTreeSet<_> = cq_datalog::datalog! {
                    pipeline {
                        storage_rewrites = [cq_datalog_parallel::shard_storage];
                        round = [cq_datalog_parallel::partition(threshold = 0, grain = 2)];
                        print = cq_datalog_parallel::print_rounds;
                    }
                    struct ParallelObserved;
                    relation E(x:i32) = (&rows);
                    relation R(x:i32);
                    output R;
                    R(*x) :- E(x);
                    R(*x + 32) :- R(x), where x < 32;
                }.collect();
                assert_eq!(actual, (0..64).map(|x| (x,)).collect());
                let records = counters::operators();
                assert!(!records.is_empty());
                let scoped: Vec<_> = records.iter().filter(|record| record.calibration.is_some()).collect();
                assert!(!scoped.is_empty());
                assert!(scoped.iter().all(|record| record.calibration.as_ref().unwrap().stratum == Some(0)));
                for phase in [None, Some(0), Some(1)] {
                    assert!(scoped.iter().any(|record| {
                        record.rows_out != 0 && record.calibration.as_ref().unwrap().round == phase
                    }), "missing actual operator rows for {phase:?} with {workers} workers");
                }
                // Every chunk's original physical program is retained, rather than
                // resetting counters at the next hole or substituting storage rows.
                let seed_rows: u64 = scoped.iter().filter(|record| {
                    record.label.kind == "RENAME" && record.calibration.as_ref().unwrap().round.is_none()
                }).map(|record| record.rows_out).sum();
                assert_eq!(seed_rows, 32);
                assert!(calibration::observer_failures().is_empty());
            }));
            let metrics = observer.0.lock().unwrap();
            assert_eq!((metrics.raw, metrics.admitted), (64, 32));
            assert_eq!(metrics.canonical, [(0, 32)]);
            assert_eq!(metrics.seeds, [(0, vec![(1, 32)])]);
            assert_eq!(metrics.rounds, [(0, 0, vec![(1, 32)]), (0, 1, vec![(1, 32)])]);
            assert_eq!(metrics.finals, [(0, 2, vec![(1, 64, 0)])]);
            assert_eq!(metrics.completed, 1);
        });
    }
}
