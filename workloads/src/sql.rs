//! Names shared by authored SQL files and the isolated DuckDB loader.

/// The private DuckDB table populated from the Nth declared input.
/// The slash separates these quoted names from MiniLinq relation identifiers.
pub fn raw_table_name(input_index: usize) -> String {
    format!("mini-linq/raw/{input_index}")
}
