//! CPU route names shared by TPC-H dispatch and generated differential tests.
macro_rules! cpu_route_names {
    ($($extra:expr),* $(,)?) => {
        &["default", "par", "radix", "radix_retained", "no-reduce", "transfer", "le", "le_runs" $(, $extra)*]
    };
}
pub(crate) use cpu_route_names;
pub const QUICK: &[&str] = &["default", "par", "radix", "advised"];
pub const FULL: &[&str] = self::cpu_route_names!("built_hash", "built_radix", "advised", "push");
