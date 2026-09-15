//! Independent storage/Access correspondence, owned by each physical strategy.
//! These relations read actual generated syntax; they never invoke a lowering.
use cq_ir::{
    cq, index_requirements as ir, relational_plan as ra, rust_access_plan as ac, symbol_name,
};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, Ident, Stmt};

mod context;
use context::*;
mod input_indexed;
pub(crate) mod lazy;
pub(crate) mod materialized;
mod syntax;
use syntax::*;
#[cfg(test)]
mod tests;

pub(crate) use input_indexed::check as input_indexed;
pub(crate) use lazy::check as lazy;
pub(crate) use materialized::{
    callable as materialized_callable, check as materialized_with, query as materialized_query,
};
