//! Closure-based pass conformance and executable written-order refinement.
//!
//! Pass cases test only the supplied fixtures, mutations, and authored data.
//! Checkers are supplied independently of compilation. Panics propagate in
//! pass testing, so a crashing checker cannot count as mutation rejection.
//! Refinement catches unwinding panics and implements `bottom + Set`: a target
//! may panic only when the reference panics, and must match a returned answer.

use crate::{Case, Database, Rows};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// A compiler under test; any caller-defined result error is supported.
pub type Compile<'a, S, T, E> = Box<dyn Fn(&S) -> std::result::Result<T, E> + 'a>;
/// An independently supplied source/target relation checker.
pub type Check<'a, S, T, E> = Box<dyn Fn(&S, &T) -> std::result::Result<(), E> + 'a>;
/// A deliberately invalid variant of a compiled target.
pub type Mutation<'a, T> = Box<dyn Fn(&T) -> T + 'a>;
/// An executable target interpretation under Set semantics.
pub type Answers<'a, T> = Box<dyn Fn(&T, &Database) -> Rows + 'a>;

/// One pass test, independent of any particular pass crate or pipeline protocol.
///
/// Closures can capture borrowed configuration. Compile and check errors may
/// have different types. `run` returns the target for further caller checks.
pub struct PassCase<'a, S, T, CompileError, CheckError = CompileError> {
    pub source: S,
    pub expected_next: Option<T>,
    pub compile: Compile<'a, S, T, CompileError>,
    pub check: Check<'a, S, T, CheckError>,
    pub mutations: Vec<Mutation<'a, T>>,
    pub answers: Option<Answers<'a, T>>,
}

impl<S, T: PartialEq + fmt::Debug, CE, KE> PassCase<'_, S, T, CE, KE> {
    /// Compile, compare an optional fixture, check, reject mutations, and
    /// optionally execute on the authored case, in that order.
    ///
    /// An answers closure requires `Some(case)`; its omission is an error.
    pub fn run(&self, case: Option<&Case>) -> std::result::Result<T, PassFailure<CE, KE>> {
        let target = (self.compile)(&self.source).map_err(PassFailure::Compile)?;
        if let Some(expected) = &self.expected_next {
            if &target != expected {
                return Err(PassFailure::FixtureMismatch {
                    expected: format!("{expected:?}"),
                    actual: format!("{target:?}"),
                });
            }
        }
        (self.check)(&self.source, &target).map_err(PassFailure::Check)?;
        for (index, mutate) in self.mutations.iter().enumerate() {
            let mutated = mutate(&target);
            if (self.check)(&self.source, &mutated).is_ok() {
                return Err(PassFailure::MutationAccepted { index });
            }
        }
        if let Some(answers) = &self.answers {
            let case = case.ok_or(PassFailure::MissingAnswerCase)?;
            let actual = answers(&target, &case.database());
            let expected = case.expected_rows();
            if actual != expected {
                return Err(PassFailure::AnswersMismatch {
                    case: case.name,
                    expected,
                    actual,
                });
            }
        }
        Ok(target)
    }
}

/// The precise failed conformance obligation, preserving caller errors.
#[derive(Debug, PartialEq, Eq)]
pub enum PassFailure<CompileError, CheckError = CompileError> {
    Compile(CompileError),
    FixtureMismatch {
        expected: String,
        actual: String,
    },
    Check(CheckError),
    MutationAccepted {
        index: usize,
    },
    MissingAnswerCase,
    AnswersMismatch {
        case: &'static str,
        expected: Rows,
        actual: Rows,
    },
}

impl<CE: fmt::Display, KE: fmt::Display> fmt::Display for PassFailure<CE, KE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => write!(f, "compilation failed: {error}"),
            Self::FixtureMismatch { expected, actual } => {
                write!(
                    f,
                    "compiled target differs: expected {expected}; got {actual}"
                )
            }
            Self::Check(error) => write!(f, "source/target check failed: {error}"),
            Self::MutationAccepted { index } => write!(f, "checker accepted mutation {index}"),
            Self::MissingAnswerCase => write!(f, "answer execution requires an authored case"),
            Self::AnswersMismatch {
                case,
                expected,
                actual,
            } => write!(f, "{case}: expected answers {expected:?}; got {actual:?}"),
        }
    }
}

impl<CE: fmt::Debug + fmt::Display, KE: fmt::Debug + fmt::Display> std::error::Error
    for PassFailure<CE, KE>
{
}

/// The observable result of an executable closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<A> {
    Returned(A),
    /// Panic messages are diagnostic only; all panics denote the same bottom.
    Panicked(String),
}

/// Both observations from a successful refinement comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refinement<A> {
    pub reference: Outcome<A>,
    pub target: Outcome<A>,
}

/// A target violated a returning written-order reference's denotation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefinementFailure<A> {
    AnswersDiffer { reference: A, target: A },
    TargetPanicked { reference: A, message: String },
}

impl<A: fmt::Debug> fmt::Display for RefinementFailure<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AnswersDiffer { reference, target } => {
                write!(
                    f,
                    "reference returned {reference:?}; target returned {target:?}"
                )
            }
            Self::TargetPanicked { reference, message } => {
                write!(
                    f,
                    "reference returned {reference:?}; target panicked: {message}"
                )
            }
        }
    }
}

impl<A: fmt::Debug> std::error::Error for RefinementFailure<A> {}

/// Run both executables once and check target refinement of the reference.
///
/// For query conformance, return `Rows` to compare answers as sets. A panicking
/// reference permits either target outcome. Effects and panic message equality
/// are not checked. Only unwinding panics can be caught; no global panic hook
/// is installed or modified.
pub fn check_refinement<A: PartialEq, F: FnOnce() -> A, G: FnOnce() -> A>(
    reference: F,
    target: G,
) -> std::result::Result<Refinement<A>, RefinementFailure<A>> {
    let reference = observe(reference);
    let target = observe(target);
    match (reference, target) {
        (Outcome::Returned(reference), Outcome::Returned(target)) if reference != target => {
            Err(RefinementFailure::AnswersDiffer { reference, target })
        }
        (Outcome::Returned(reference), Outcome::Panicked(message)) => {
            Err(RefinementFailure::TargetPanicked { reference, message })
        }
        (reference, target) => Ok(Refinement { reference, target }),
    }
}

/// Check executable refinement on one shared borrowed input, such as a database.
pub fn check_refinement_on<I: ?Sized, A: PartialEq>(
    input: &I,
    reference: impl FnOnce(&I) -> A,
    target: impl FnOnce(&I) -> A,
) -> std::result::Result<Refinement<A>, RefinementFailure<A>> {
    check_refinement(|| reference(input), || target(input))
}

fn observe<A>(execute: impl FnOnce() -> A) -> Outcome<A> {
    match catch_unwind(AssertUnwindSafe(execute)) {
        Ok(answer) => Outcome::Returned(answer),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "non-string panic payload".to_owned());
            Outcome::Panicked(message)
        }
    }
}
