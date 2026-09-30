#![allow(unused_imports, unused_macros)]
macro_rules! strategy {
    ($($ix:tt)*) => { ::mini_linq::lower_physical! { $($ix)* policy {} } };
}
pub(crate) use strategy as physical_policy;
pub(crate) use ::mini_linq::{
    index_demands as choose_index_policy, select_structures, structures_to_rust,
};
pub(crate) use cq_state_providers::{
    default_indexes as index_provider, hash_distinct as distinct_state,
};
