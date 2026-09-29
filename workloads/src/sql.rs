//! Structural translation of a positive CQ to a visible DuckDB oracle query.
//!
//! The SQL deliberately remains independent of all three basic homework passes. Every raw
//! input is deduplicated in a CTE, every body occurrence receives its own alias,
//! and the result is explicitly distinct and ordered in head-column order.

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;

use cq_lang_cq as cq;
use cq_syntax::symbol_name;

/// The private DuckDB table populated from the Nth declared input.
///
/// The slash makes this quoted SQL name impossible to confuse with a MiniLinq
/// relation identifier.
pub fn raw_table_name(input_index: usize) -> String {
    format!("mini-linq/raw/{input_index}")
}

fn normalized_table_name(input_index: usize) -> String {
    format!("mini-linq/input/{input_index}")
}

/// Translate one well-formed positive CQ without consulting either compiler pass.
pub fn translate(module: &cq::Program) -> Result<String, SqlError> {
    cq::contract::check(module).map_err(SqlError::InvalidQuery)?;
    if !module.query.is_basic() {
        return Err(SqlError::UnsupportedBodyItem);
    }

    let atoms = positive_atoms(&module.query)?;
    let mut output = String::from("WITH\n");
    let input_positions = module
        .inputs()
        .enumerate()
        .map(|(input_index, input)| (symbol_name(&input.name), input_index))
        .collect::<BTreeMap<_, _>>();
    for (input_index, input) in module.inputs().enumerate() {
        if input_index != 0 {
            output.push_str(",\n");
        }
        output.push_str("  ");
        output.push_str(&quoted_identifier(&normalized_table_name(input_index)));
        output.push_str(" AS (\n    SELECT DISTINCT ");
        push_column_list(&mut output, input.columns.len());
        output.push_str("\n    FROM ");
        output.push_str(&quoted_identifier(&raw_table_name(input_index)));
        output.push_str("\n  )");
    }

    let mut bindings = BTreeMap::<String, Binding>::new();
    let mut equalities = Vec::new();
    for (atom_index, atom) in atoms.iter().enumerate() {
        for (column, variable) in atom.variables().enumerate() {
            let name = symbol_name(variable);
            let current = Binding {
                atom: atom_index,
                column,
            };
            if let Some(previous) = bindings.get(&name) {
                equalities.push(format!(
                    "{} = {}",
                    binding_sql(current),
                    binding_sql(*previous)
                ));
            } else {
                bindings.insert(name, current);
            }
        }
    }

    output.push_str("\nSELECT DISTINCT\n");
    for (position, variable) in module.query.head.variables().enumerate() {
        if position != 0 {
            output.push_str(",\n");
        }
        let name = symbol_name(variable);
        let binding = bindings
            .get(&name)
            .copied()
            .ok_or_else(|| SqlError::MissingHeadBinding(name.clone()))?;
        output.push_str("  ");
        output.push_str(&binding_sql(binding));
        output.push_str(" AS ");
        output.push_str(&quoted_identifier(&name));
    }

    output.push_str("\nFROM ");
    for (atom_index, atom) in atoms.iter().enumerate() {
        if atom_index != 0 {
            output.push_str("\nCROSS JOIN ");
        }
        let relation = symbol_name(&atom.relation);
        let input_index = input_positions
            .get(&relation)
            .copied()
            .ok_or_else(|| SqlError::UnknownRelation(relation.clone()))?;
        output.push_str(&quoted_identifier(&normalized_table_name(input_index)));
        output.push_str(" AS ");
        output.push_str(&alias_sql(atom_index));
    }
    if !equalities.is_empty() {
        output.push_str("\nWHERE ");
        for (position, equality) in equalities.iter().enumerate() {
            if position != 0 {
                output.push_str("\n  AND ");
            }
            output.push_str(equality);
        }
    }

    output.push_str("\nORDER BY ");
    for position in 1..=module.query.head.args.len() {
        if position != 1 {
            output.push_str(", ");
        }
        output.push_str(&position.to_string());
    }
    output.push_str(";\n");
    Ok(output)
}

fn positive_atoms(query: &cq::Query) -> Result<Vec<&cq::Atom>, SqlError> {
    query
        .body
        .iter()
        .map(|item| {
            #[allow(unreachable_patterns)]
            match item {
                cq::BodyItem::Positive { atom } => Ok(atom),
                _ => Err(SqlError::UnsupportedBodyItem),
            }
        })
        .collect()
}

fn push_column_list(output: &mut String, arity: usize) {
    for column in 0..arity {
        if column != 0 {
            output.push_str(", ");
        }
        output.push_str(&quoted_identifier(&format!("c{column}")));
    }
}

#[derive(Clone, Copy)]
struct Binding {
    atom: usize,
    column: usize,
}

fn binding_sql(binding: Binding) -> String {
    format!(
        "{}.{}",
        alias_sql(binding.atom),
        quoted_identifier(&format!("c{}", binding.column))
    )
}

fn alias_sql(atom: usize) -> String {
    quoted_identifier(&format!("a{atom}"))
}

fn quoted_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[derive(Debug)]
pub enum SqlError {
    InvalidQuery(syn::Error),
    UnsupportedBodyItem,
    MissingHeadBinding(String),
    UnknownRelation(String),
}

impl fmt::Display for SqlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuery(error) => write!(formatter, "invalid workload query: {error}"),
            Self::UnsupportedBodyItem => {
                formatter.write_str("the baseline DuckDB translator accepts only positive atoms")
            }
            Self::MissingHeadBinding(variable) => {
                write!(formatter, "result variable `{variable}` has no SQL binding")
            }
            Self::UnknownRelation(relation) => {
                write!(formatter, "body relation `{relation}` has no SQL input")
            }
        }
    }
}

impl StdError for SqlError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::InvalidQuery(error) => Some(error),
            Self::UnsupportedBodyItem | Self::MissingHeadBinding(_) | Self::UnknownRelation(_) => {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> cq::Program {
        syn::parse_str(source).unwrap()
    }

    #[test]
    fn triangle_sql_deduplicates_inputs_and_orders_the_distinct_head() {
        let sql = translate(&parse(
            "struct Q; relation R(c0: i32, c1: i32); relation S(c0: i32, c1: i32); relation T(c0: i32, c1: i32); q(x, y, z) :- R(x, y), S(y, z), T(z, x);",
        ))
        .unwrap();

        assert_eq!(sql.matches("SELECT DISTINCT").count(), 4);
        assert!(sql.contains("FROM \"mini-linq/raw/0\""));
        assert!(sql.contains("\"a1\".\"c0\" = \"a0\".\"c1\""));
        assert!(sql.contains("\"a2\".\"c1\" = \"a0\".\"c0\""));
        assert!(sql.ends_with("ORDER BY 1, 2, 3;\n"));
    }

    #[test]
    fn self_joins_use_occurrence_aliases_and_normalize_raw_identifiers() {
        let sql = translate(&parse(
            "struct Q; relation r#type(c0: i32, c1: i32); q(x, z) :- r#type(x, y), r#type(y, z);",
        ))
        .unwrap();

        assert_eq!(sql.matches("\"mini-linq/input/0\" AS \"a").count(), 2);
        assert!(!sql.contains("r#type"));
        assert!(sql.contains("\"a1\".\"c0\" = \"a0\".\"c1\""));
    }

    #[test]
    fn disconnected_atoms_remain_an_explicit_cross_product() {
        let sql = translate(&parse(
            "struct Q; relation A(c0: i32); relation B(c0: i32, c1: i32); q(x, y, z) :- A(x), B(y, z);",
        ))
        .unwrap();

        assert!(sql.contains(
            "FROM \"mini-linq/input/0\" AS \"a0\"\nCROSS JOIN \"mini-linq/input/1\" AS \"a1\""
        ));
        assert!(!sql.contains("\nWHERE "));
    }

    #[test]
    fn generated_cte_names_do_not_case_fold_caller_relation_names() {
        let sql = translate(&parse(
            "struct Q; relation R(c0: i32); relation r(c0: i32); q(x) :- R(x), r(x);",
        ))
        .unwrap();

        assert!(sql.contains("\"mini-linq/input/0\" AS ("));
        assert!(sql.contains("\"mini-linq/input/1\" AS ("));
        assert!(!sql.contains("\"R\" AS ("));
        assert!(!sql.contains("\"r\" AS ("));
    }
}
