//! The two workload catalogs.

mod classic;
mod retail;

use crate::QueryCase;

pub use classic::CLASSIC;
pub use retail::RETAIL;

/// Iterate over both catalogs in their documented order.
pub fn all() -> impl Iterator<Item = &'static QueryCase> {
    CLASSIC.iter().chain(RETAIL.iter())
}

/// Return one complete suite.
pub fn named(name: &str) -> Option<&'static [QueryCase]> {
    match name {
        "classic" => Some(CLASSIC),
        "retail" => Some(RETAIL),
        _ => None,
    }
}
