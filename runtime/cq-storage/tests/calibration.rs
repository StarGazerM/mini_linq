#![cfg(feature = "planner-calibration")]
use cq_storage::counters::{self, OperatorLabel, OperatorProgram, TaskContext};
use cq_storage::counters::calibration::{self, CalibrationCell, CalibrationRow, Observer, ScalarValue};
use std::sync::{Arc, Mutex};

const LABELS: &[OperatorLabel] = &[
    OperatorLabel { result: "scan", kind: "SCAN", key: "R", inputs: &[] },
    OperatorLabel { result: "alias", kind: "PROJECT", key: "", inputs: &["scan"] },
    OperatorLabel { result: "filtered", kind: "SELECT", key: "", inputs: &["scan"] },
];

#[test]
fn aliases_are_rows_not_shared_times_and_trace_registration_is_late_visible() {
    TaskContext::default().run(|| {
        calibration::stratum_begin(3);
        let seed;
        {
            let _hole = calibration::hole_scope(7);
            let program = OperatorProgram::new(LABELS);
            seed = calibration::active_frame().unwrap();
            program.alias(1, 0);
            program.share_time(2, 0);
            calibration::register_plan("physical seed trace");
            assert_eq!(program.stream(0, 0..4).count(), 4);
            assert_eq!(program.output(2, 0..2).count(), 2);
        }
        calibration::round_begin(3, 0, &[(8, 4)]);
        {
            let _hole = calibration::hole_scope(7);
            calibration::register_plan("physical step trace");
            let program = OperatorProgram::new(&LABELS[..1]);
            assert_eq!(program.stream(0, 0..1).count(), 1);
        }
        let records = counters::operators();
        assert_eq!(records.len(), 4, "hole entry does not reset the query epoch");
        assert_eq!(records[1].row_alias, Some(records[0].id));
        assert_eq!(records[2].row_alias, None, "time sharing is not a row alias");
        assert_eq!((records[0].rows_out, records[1].rows_out, records[2].rows_out), (4, 4, 2));
        assert_eq!(seed.trace(), Some("physical seed trace"));
        assert_eq!(seed.round, None);
        assert_eq!(records[3].calibration.as_ref().unwrap().round, Some(0));
        assert_eq!(records[0].calibration.as_ref(), Some(&seed));
        assert!(calibration::active_frame().is_none());
    });
}

#[derive(Default)]
struct Events(Mutex<Vec<&'static str>>);
impl Observer for Events {
    fn program_begin(&self, _: &'static str) { self.0.lock().unwrap().push("begin"); }
    fn program_end(&self) { self.0.lock().unwrap().push("end"); }
    fn input_begin(&self, _: usize) { self.0.lock().unwrap().push("input"); }
}

#[test]
fn task_context_carries_observer_frame_and_counter_epoch_and_restores_worker() {
    let events = Arc::new(Events::default());
    TaskContext::default().run(|| calibration::with_observer(events.clone(), || {
        let guard = calibration::program_begin("round");
        calibration::stratum_begin(2);
        calibration::round_begin(2, 4, &[(0, 3)]);
        let _hole = calibration::hole_scope(5);
        calibration::register_plan("plan");
        let task = TaskContext::capture();
        std::thread::spawn(move || {
            assert!(calibration::active_frame().is_none());
            task.run(|| {
                calibration::input_begin(0);
                let program = OperatorProgram::new(&LABELS[..1]);
                assert_eq!(program.stream(0, 0..3).count(), 3);
                let nested = TaskContext::capture();
                nested.run(|| assert_eq!(calibration::active_frame().unwrap().round, Some(4)));
            });
            assert!(calibration::active_frame().is_none());
            calibration::input_begin(0); // The installed observer must not leak.
        }).join().unwrap();
        let records = counters::operators();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].rows_out, 3);
        let frame = records[0].calibration.as_ref().unwrap();
        assert_eq!((frame.hole, frame.stratum, frame.round), (5, Some(2), Some(4)));
        assert_eq!(frame.trace(), Some("plan"));
        guard.complete();
    }));
    assert_eq!(*events.0.lock().unwrap(), ["begin", "input", "end"]);
}

#[test]
fn unwind_restores_scopes_without_certifying_completion_and_push_counts_survive() {
    let events = Arc::new(Events::default());
    TaskContext::default().run(|| calibration::with_observer(events.clone(), || {
        let failed = std::panic::catch_unwind(|| {
            let _program = calibration::program_begin("round");
            let _hole = calibration::hole_scope(1);
            let program = OperatorProgram::new(&LABELS[..1]);
            let mut pushed = program.push(0);
            pushed.input();
            pushed.output();
            panic!("query failure");
        });
        assert!(failed.is_err());
        assert!(calibration::active_frame().is_none());
        let records = counters::operators();
        assert_eq!((records[0].rows_in, records[0].rows_out), (1, 1));
    }));
    assert_eq!(*events.0.lock().unwrap(), ["begin"]);
}

#[test]
fn unexpected_observer_panic_propagates_and_does_not_complete() {
    struct Panicking(Events);
    impl Observer for Panicking {
        fn program_begin(&self, ir: &'static str) { self.0.program_begin(ir); }
        fn program_end(&self) { self.0.program_end(); }
        fn input_begin(&self, _: usize) { panic!("unexpected observer failure"); }
    }
    let observer = Arc::new(Panicking(Events::default()));
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        calibration::with_observer(observer.clone(), || {
            let program = calibration::program_begin("round");
            calibration::input_begin(0);
            program.complete();
        });
    }));
    assert!(failed.is_err());
    assert_eq!(*observer.0.0.lock().unwrap(), ["begin"]);
    calibration::input_begin(0);
}

#[test]
fn rows_borrow_fields_in_declaration_order_without_coercing_unsupported_values() {
    let name = String::from("name");
    let row = (&17_i32, &name, &9_u64);
    let borrowed: &dyn CalibrationRow = &row;
    assert_eq!(borrowed.len(), 3);
    assert_eq!(borrowed.cell(0), ScalarValue::Signed(17));
    assert_eq!(borrowed.cell(1), ScalarValue::Utf8("name"));
    assert_eq!(borrowed.cell(2), ScalarValue::Unsigned(9));
    assert!(matches!(vec![1_i64].calibration_value(), ScalarValue::Unsupported(_)));
    assert!(().is_empty());
}

#[test]
fn metadata_borrows_the_callers_bytes_and_propagates_through_tasks() {
    struct Metadata(Mutex<Vec<(String, Vec<u8>)>>);
    impl Observer for Metadata {
        fn metadata(&self, path: &str, bytes: &[u8]) {
            self.0.lock().unwrap().push((path.to_owned(), bytes.to_vec()));
        }
    }
    let observer = Arc::new(Metadata(Mutex::new(Vec::new())));
    calibration::with_observer(observer.clone(), || {
        let task = TaskContext::capture();
        std::thread::spawn(move || task.run(|| {
            calibration::metadata("fixture/meta.json", b"{\"rows\":3}");
        })).join().unwrap();
    });
    calibration::metadata("not-observed", b"ignored");
    assert_eq!(*observer.0.lock().unwrap(), [("fixture/meta.json".to_owned(), b"{\"rows\":3}".to_vec())]);
}

#[test]
fn failed_program_begin_restores_the_enclosing_location() {
    struct FailsBegin;
    impl Observer for FailsBegin {
        fn program_begin(&self, _: &'static str) { panic!("begin failed"); }
    }
    calibration::with_observer(Arc::new(FailsBegin), || {
        calibration::stratum_begin(8);
        calibration::round_begin(8, 9, &[]);
        let _hole = calibration::hole_scope(10);
        let before = calibration::active_frame().unwrap();
        let failure = std::panic::catch_unwind(|| {
            let _program = calibration::program_begin("nested");
        });
        assert!(failure.is_err());
        assert_eq!(calibration::active_frame(), Some(before));
    });
}

#[test]
fn overflow_is_rejected_as_exact_evidence_and_follows_only_row_aliases() {
    TaskContext::default().run(|| {
        let _hole = calibration::hole_scope(2);
        let program = OperatorProgram::new(LABELS);
        program.alias(1, 0);
        program.share_time(2, 0);
        program.rows(0, u64::MAX, u64::MAX, u64::MAX);
        program.rows(0, 1, 1, 1);
        let records = counters::operators();
        assert!(records[0].row_overflow);
        assert!(records[1].row_overflow);
        assert!(!records[2].row_overflow, "time sharing does not share row overflow");
    });
}

#[test]
fn autoref_scalar_dispatch_prefers_registered_codecs_and_preserves_borrowed_text() {
    use calibration::{Cell, ScalarCell as _};
    struct Registered(u64);
    impl CalibrationCell for Registered {
        fn calibration_value(&self) -> ScalarValue<'_> { ScalarValue::Symbol(self.0) }
    }
    struct Missing;
    fn generic<T>(value: &T) -> ScalarValue<'_> { Cell(value).scalar() }

    let text = String::from("borrowed");
    let integer = 17_i32;
    let registered = Registered(23);
    let missing = Missing;
    let row = (
        &integer,
        &text,
        &registered,
        &missing,
    );
    let cells = [
        Cell(&row.0).scalar(),
        Cell(&row.1).scalar(),
        Cell(&row.2).scalar(),
        Cell(&row.3).scalar(),
    ];
    let observed: &dyn CalibrationRow = &cells;
    assert_eq!(observed.len(), 4);
    assert_eq!(observed.cell(0), ScalarValue::Signed(17));
    assert_eq!(observed.cell(2), ScalarValue::Symbol(23));
    assert_eq!(observed.cell(3), ScalarValue::Unsupported("no evidence scalar codec"));
    let ScalarValue::Utf8(value) = observed.cell(1) else { panic!("borrowed text codec") };
    assert_eq!(value.as_ptr(), text.as_ptr(), "codec does not allocate a string");
    assert_eq!(generic(&missing), ScalarValue::Unsupported("no evidence scalar codec"));
    let empty: [ScalarValue<'_>; 0] = [];
    assert!(CalibrationRow::is_empty(&empty));
}

#[test]
fn nested_host_aliases_override_restore_and_propagate_as_immutable_frames() {
    TaskContext::default().run(|| {
        {
            let _outer = calibration::host_alias_scope(&[
                ("host", "outer_parameter"),
                ("shared", "shared_parameter"),
            ]);
            {
                let _hole = calibration::hole_scope(7);
                let outer_frame = calibration::active_frame().unwrap();
                let _outer_program = OperatorProgram::new(&LABELS[..1]);
                {
                    let _inner = calibration::host_alias_scope(&[
                        ("host", "inner_parameter"),
                        ("new", "new_parameter"),
                    ]);
                    let inner_frame = calibration::active_frame().unwrap();
                    assert_eq!(inner_frame.host_aliases(), &[
                        ("host", "inner_parameter"),
                        ("shared", "shared_parameter"),
                        ("new", "new_parameter"),
                    ]);
                    assert_eq!(outer_frame.host_aliases(), &[
                        ("host", "outer_parameter"),
                        ("shared", "shared_parameter"),
                    ], "previously captured frame is not mutated");
                    calibration::register_plan("shared trace slot");
                    assert_eq!(outer_frame.trace(), Some("shared trace slot"));
                    let task = TaskContext::capture();
                    std::thread::spawn(move || task.run(|| {
                        assert_eq!(calibration::active_frame().unwrap(), inner_frame);
                        let program = OperatorProgram::new(&LABELS[..1]);
                        assert_eq!(program.stream(0, 0..2).count(), 2);
                    })).join().unwrap();
                }
                assert_eq!(calibration::active_frame(), Some(outer_frame.clone()));
                let failed = std::panic::catch_unwind(|| {
                    let _failed = calibration::host_alias_scope(&[("host", "failed_parameter")]);
                    panic!("partition failed");
                });
                assert!(failed.is_err());
                assert_eq!(calibration::active_frame(), Some(outer_frame));
            }
            assert!(calibration::active_frame().is_none());
            let _key_hole = calibration::hole_scope(8);
            assert_eq!(calibration::active_frame().unwrap().host_aliases()[0], ("host", "outer_parameter"));
        }
        let _fresh_hole = calibration::hole_scope(9);
        assert!(calibration::active_frame().unwrap().host_aliases().is_empty());
        let records = counters::operators();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].calibration.as_ref().unwrap().host_aliases()[0], ("host", "outer_parameter"));
        assert_eq!(records[1].calibration.as_ref().unwrap().host_aliases()[0], ("host", "inner_parameter"));
        assert_eq!(records[1].rows_out, 2);
    });
}

#[test]
fn empty_output_observations_are_distinct_from_untouched_input_and_timing_records() {
    TaskContext::default().run(|| {
        let program = OperatorProgram::new(&[LABELS[0]; 9]);
        drop(program.stream(0, std::iter::empty::<i32>()));
        assert_eq!(program.stream(1, std::iter::empty::<i32>()).next(), None);
        drop(program.output(2, std::iter::empty::<i32>()));
        assert_eq!(program.output(3, std::iter::empty::<i32>()).next(), None);
        assert_eq!(program.output(4, std::iter::empty::<i32>()).fold(0, |n, _| n + 1), 0);
        program.rows(5, 0, 0, 0);
        program.scope(6, || {});
        assert_eq!(program.input(7, 0..3).count(), 3);
        assert_eq!(program.stream(8, std::iter::empty::<i32>()).fold(0, |n, _| n + 1), 0);
        let records = counters::operators();
        assert_eq!(
            records.iter().map(|record| record.row_observed).collect::<Vec<_>>(),
            [false, true, false, true, true, true, false, false, true],
        );
        assert!(records.iter().all(|record| record.rows_out == 0));
        assert_eq!(records[5].calls, 0, "explicit zero is observed without a timer");
        assert_eq!(records[6].calls, 1, "a timer alone does not observe rows");
        assert_eq!(records[7].rows_in, 3, "input counts alone do not observe output");
    });
}

#[test]
fn observation_provenance_follows_row_alias_chains_but_not_time_sharing() {
    TaskContext::default().run(|| {
        let program = OperatorProgram::new(&[LABELS[0]; 5]);
        program.alias(1, 0);
        program.alias(2, 1);
        program.share_time(3, 0);
        program.share_time(4, 0);
        assert_eq!(program.stream(0, std::iter::empty::<i32>()).count(), 0);
        program.rows(4, 0, 0, 0);
        let records = counters::operators();
        assert_eq!(
            records.iter().map(|record| record.row_observed).collect::<Vec<_>>(),
            [true, true, true, false, true],
        );
        assert_eq!(records[3].calls, records[0].calls);
        assert!(records.iter().all(|record| record.rows_out == 0));
    });
}

#[test]
fn empty_push_scope_and_tested_predicate_record_observations_without_visiting_others() {
    struct Reject;
    impl cq_storage::ReadPredicate<i32> for Reject {
        fn test(&self, _: &i32) -> bool { false }
    }
    TaskContext::default().run(|| {
        let program = OperatorProgram::new(&[LABELS[0]; 4]);
        drop(program.push(0));
        let predicate = program.predicate(1, Some(2), Reject);
        assert!(!cq_storage::ReadPredicate::test(&predicate, &17));
        drop(predicate);
        drop(program.predicate(3, None, Reject));
        let records = counters::operators();
        assert_eq!(
            records.iter().map(|record| record.row_observed).collect::<Vec<_>>(),
            [true, true, true, false],
        );
        assert_eq!(records[0].rows_out, 0);
        assert_eq!(records[1].rows_out, 0);
        assert_eq!(records[2].rows_out, 1);
    });
}
