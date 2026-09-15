//! One concrete indexed-set implementation and its independent checker.
pub(crate) mod materialized;
mod rust;
pub(crate) mod check {
    use cq_ir::{
        cq, index_requirements as ir, relational_plan as ra, rust_access_plan as ac, symbol_name,
    };
    use quote::{format_ident, quote};
    use std::collections::{BTreeMap, BTreeSet};
    use syn::{Expr, Ident, Stmt};
    mod context;
    use context::*;
    // This profile checks set holes; the same observations also handle pull holes.
    #[allow(dead_code)]
    mod syntax;
    use syntax::*;
    pub(crate) mod materialized;
    pub(crate) use materialized::callable as materialized_callable;
    pub(crate) use materialized::check as materialized_with;
    pub(crate) use materialized::query as materialized_query;
}
