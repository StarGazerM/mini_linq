//! Feature profiles: which generator features a route supports.

/// Generator features enabled for a route. `generate_with` only emits cases
/// whose every feature is enabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Opaque total Rust operations (compiled written-order oracle lane).
    pub opaque_rust: bool,
    /// Finite `for` generators.
    pub generators: bool,
    /// Union and fallback blocks.
    pub blocks: bool,
    /// Negated atoms.
    pub negation: bool,
    /// Grouped `count`/`sum`/`min`/`max` heads.
    pub aggregates: bool,
    /// Declared (possibly multi-column, unsorted) keys.
    pub keys: bool,
    /// i64 and u32 columns, including extremes and low-32-bit collisions.
    pub wide_ints: bool,
    /// Interned string columns. Off until interned `StringCode` columns land.
    pub strings: bool,
    /// Raw keyword identifiers (`r#type`) for relations and variables.
    pub raw_identifiers: bool,
    /// Closed host captures in lets, guards, heads and folds; `()` heads.
    pub closed_captures: bool,
    /// Largest relation; above 1,024 enables the larger-than-one-tile shape.
    pub max_rows: usize,
}

impl Profile {
    /// Every CPU feature, small tables: the fast tier.
    pub const fn quick() -> Self {
        Self {
            opaque_rust: true,
            generators: true,
            blocks: true,
            negation: true,
            aggregates: true,
            keys: true,
            wide_ints: true,
            strings: false,
            raw_identifiers: true,
            closed_captures: true,
            max_rows: 64,
        }
    }

    /// Every CPU feature, including tables larger than one GPU tile.
    pub const fn full() -> Self {
        Self {
            max_rows: 5_000,
            ..Self::quick()
        }
    }

    /// The GPU-supported subset: integer columns, constraints, negation,
    /// grouped folds and membership; no opaque Rust, generators or blocks.
    pub const fn gpu() -> Self {
        Self {
            opaque_rust: false,
            generators: false,
            blocks: false,
            ..Self::full()
        }
    }
}
