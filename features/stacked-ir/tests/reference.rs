//! Reference execution of the shared authored scenarios.
#![recursion_limit = "1024"]
macro_rules! compile_case {
    ($($tokens:tt)*) => { ::mini_linq::compile! { $($tokens)* } };
}
macro_rules! select_case {
    ($($tokens:tt)*) => { ::mini_linq::passes::select! { $($tokens)* } };
}
include!("cases/mod.rs");
