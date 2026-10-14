//! The same authored scenarios with a Physical/Trace round trip.
#![recursion_limit = "1024"]
macro_rules! compile_case {
    (pipeline { $(plan = $plan:path;)? physical = [$($pass:path),*]; $($rest:tt)* } $($body:tt)*) => {
        mini_linq::compile! { pipeline { $(plan = $plan;)? physical = [$($pass,)* ::stacked_ir::round_trip]; $($rest)* } $($body)* }
    };
    (pipeline { $($config:tt)* } $($body:tt)*) => {
        mini_linq::compile! { pipeline { $($config)* physical = [::stacked_ir::round_trip]; } $($body)* }
    };
}
macro_rules! select_case {
    (pipeline { $($passes:tt)* }; $($physical:tt)*) => {
        stacked_ir::enter! { pipeline { ::stacked_ir::leave, ::mini_linq::passes::select, $($passes)* }; $($physical)* }
    };
}
include!("cases/mod.rs");
