//! The origin of values supplied to a relational keyed read.

/// Stream probes may amortize a reusable index; a const read supplies one
/// compile-time key and must use direct host access or scan and filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SupplyKind {
    Stream,
    Const,
}
