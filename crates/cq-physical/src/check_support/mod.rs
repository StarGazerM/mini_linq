//! Syntax observations and environments used only by independent checkers.
//! This module contains no lowering, route selection or implementation dispatch.
use cq_ir::rust_access_plan as ac;
#[cfg(any(feature = "lazy", feature = "materialized"))]
use cq_ir::{cq, index_requirements as ir, relational_plan as ra, symbol_name};
#[cfg(any(feature = "lazy", feature = "materialized"))]
use quote::format_ident;
#[cfg(any(feature = "lazy", feature = "materialized"))]
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, Ident, Stmt};
#[cfg(any(feature = "lazy", feature = "materialized"))]
mod context;
#[cfg(any(feature = "lazy", feature = "materialized"))]
mod hash_collections;
mod syntax;
#[cfg(feature = "materialized")]
pub use context::Information;
#[cfg(any(feature = "lazy", feature = "materialized"))]
pub use context::{Names, RuleContext, join_key};
#[cfg(any(feature = "lazy", feature = "materialized"))]
pub use hash_collections::{collection_key_type, tuple_set_type};
#[cfg(any(feature = "input", feature = "lazy"))]
pub use syntax::access_expression;
#[cfg(any(feature = "input", feature = "materialized"))]
pub use syntax::cloned_names;
pub use syntax::{
    access_macro, check_for, check_if, equal, expect_statement, initializer, method, next_local,
};
#[cfg(any(feature = "lazy", feature = "materialized"))]
pub use syntax::{
    alias_expression, check_head, check_scan, cloned_columns, extend_value, guard_value,
    tuple_names,
};
#[cfg(feature = "materialized")]
pub use syntax::{last_expression, locals};
