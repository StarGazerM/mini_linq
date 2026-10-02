//! Plain equality-state representations for recorded Structures types.
/// Hash-based canonicalization state.
pub type HashDistinct<T> = std::collections::HashSet<T>;
/// Ordered canonicalization state.
pub type OrderedDistinct<T> = std::collections::BTreeSet<T>;
