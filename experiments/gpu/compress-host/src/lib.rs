//! Compression schemas shared by host build scripts and resident catalogs.

pub mod tpch;

/// Stored kind of one column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Dense u32 ID of the named domain.
    Id(String),
    /// Declared 32-bit integer, stored at that type.
    I32,
}

/// Which source columns share a domain, and every stored table's columns.
pub struct Schema {
    /// Domain name -> member source columns (`table.column`).
    pub domains: Vec<(String, Vec<String>)>,
    /// Table -> (column, kind), in stored column order.
    pub tables: Vec<(String, Vec<(String, Kind)>)>,
}
