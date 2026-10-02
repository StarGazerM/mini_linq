use cq_testkit::harness::{
    Outcome, PassFailure, RefinementFailure, check_refinement, check_refinement_on,
};
use cq_testkit::{CASES, Case, PassCase, Rows};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn authored_case() -> &'static Case {
    CASES.iter().find(|case| case.name == "scan").unwrap()
}

fn correct_pass() -> PassCase<'static, Rows, Rows, &'static str> {
    let source = authored_case().expected_rows();
    PassCase {
        expected_next: Some(source.clone()),
        source,
        compile: Box::new(|source| Ok(source.clone())),
        check: Box::new(|source, target| {
            if source == target {
                Ok(())
            } else {
                Err("identity violated")
            }
        }),
        mutations: vec![Box::new(|_| Rows::new())],
        answers: Some(Box::new(|target, _| target.clone())),
    }
}

#[test]
fn correct_pass_checks_fixture_mutations_and_authored_answers() {
    assert_eq!(
        correct_pass().run(Some(authored_case())).unwrap(),
        authored_case().expected_rows()
    );
}

#[test]
fn checker_accepting_an_invalid_mutation_fails_conformance() {
    let mut pass = correct_pass();
    pass.check = Box::new(|_, _| Ok(()));
    assert_eq!(
        pass.run(Some(authored_case())),
        Err(PassFailure::MutationAccepted { index: 0 })
    );
}

#[test]
fn wrong_answers_fail_even_with_a_matching_fixture_and_valid_check() {
    let mut pass = correct_pass();
    pass.answers = Some(Box::new(|_, _| Rows::new()));
    assert!(matches!(
        pass.run(Some(authored_case())),
        Err(PassFailure::AnswersMismatch { case: "scan", .. })
    ));
}

#[test]
fn compilation_fixture_and_check_failures_are_distinguishable() {
    let mut pass = correct_pass();
    pass.compile = Box::new(|_| Err("compile error"));
    assert_eq!(
        pass.run(Some(authored_case())),
        Err(PassFailure::Compile("compile error"))
    );

    let mut pass = correct_pass();
    pass.expected_next = Some(Rows::new());
    assert!(matches!(
        pass.run(Some(authored_case())),
        Err(PassFailure::FixtureMismatch { .. })
    ));

    let mut pass = correct_pass();
    pass.check = Box::new(|_, _| Err("check error"));
    assert_eq!(
        pass.run(Some(authored_case())),
        Err(PassFailure::Check("check error"))
    );
}

#[test]
fn optional_fixture_and_answers_work_but_answers_require_authored_data() {
    let mut pass = correct_pass();
    assert_eq!(pass.run(None), Err(PassFailure::MissingAnswerCase));
    pass.expected_next = None;
    pass.answers = None;
    assert_eq!(pass.run(None).unwrap(), authored_case().expected_rows());
}

#[test]
fn every_mutation_is_checked_before_running_answers_and_closures_can_borrow() {
    let events = RefCell::new(Vec::new());
    let pass = PassCase {
        source: 1,
        expected_next: Some(1),
        compile: Box::new(|source: &i32| {
            events.borrow_mut().push("compile");
            Ok::<_, &str>(*source)
        }),
        check: Box::new(|source: &i32, target: &i32| {
            events.borrow_mut().push("check");
            if source == target { Ok(()) } else { Err(7_u8) }
        }),
        mutations: vec![
            Box::new(|target: &i32| target + 1),
            Box::new(|target: &i32| target - 1),
        ],
        answers: Some(Box::new(|_, database| {
            events.borrow_mut().push("answers");
            database["R"].clone()
        })),
    };
    pass.run(Some(authored_case())).unwrap();
    assert_eq!(
        *events.borrow(),
        ["compile", "check", "check", "check", "answers"]
    );
}

#[test]
fn checker_panic_cannot_count_as_rejection() {
    let mut pass = correct_pass();
    pass.check = Box::new(|source, target| {
        assert_eq!(source, target, "checker crashed on a mutation");
        Ok(())
    });
    assert!(catch_unwind(AssertUnwindSafe(|| pass.run(Some(authored_case())))).is_err());
}

#[test]
fn returning_reference_requires_equal_answers_without_target_panic() {
    let expected = authored_case().expected_rows();
    let success = check_refinement(|| expected.clone(), || expected.clone()).unwrap();
    assert_eq!(success.reference, Outcome::Returned(expected.clone()));
    assert!(matches!(
        check_refinement(|| expected.clone(), Rows::new),
        Err(RefinementFailure::AnswersDiffer { .. })
    ));
    assert!(matches!(
        check_refinement(|| expected.clone(), || panic!("target")),
        Err(RefinementFailure::TargetPanicked { .. })
    ));
}

#[test]
fn panicking_reference_permits_either_target_outcome() {
    let returned = check_refinement(|| -> Rows { panic!("reference") }, Rows::new).unwrap();
    assert!(matches!(returned.reference, Outcome::Panicked(_)));
    assert_eq!(returned.target, Outcome::Returned(Rows::new()));
    let panicked = check_refinement(
        || -> Rows { panic!("reference") },
        || panic!("different target panic"),
    )
    .unwrap();
    assert!(matches!(panicked.target, Outcome::Panicked(_)));
}

#[test]
fn refinement_on_a_database_compares_set_answers_and_executes_both_once() {
    let calls = RefCell::new(Vec::new());
    check_refinement_on(
        &authored_case().database(),
        |database| {
            calls.borrow_mut().push("reference");
            database["R"].clone()
        },
        |database| {
            calls.borrow_mut().push("target");
            database["R"].iter().rev().cloned().collect::<Rows>()
        },
    )
    .unwrap();
    assert_eq!(*calls.borrow(), ["reference", "target"]);
}
