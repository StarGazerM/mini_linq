//! Parallel Ascent evaluators for the shared oracle corpus.
macro_rules! parallel_input {
    ($rows:expr) => {
        $rows.iter().cloned().collect()
    };
}
pub(crate) use parallel_input;

oracle_shared::oracle_programs!(
    programs,
    ascent,
    ascent::ascent_par,
    crate::parallel_input,
    pipeline {}
);
pub use programs::*;
