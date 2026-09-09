mod materialized {
    use mini_linq::planned_query as query;
    include!("support/rust_expressions.rs");
}

mod lazy {
    use mini_linq::mini_linq as query;
    include!("support/rust_expressions.rs");
}
