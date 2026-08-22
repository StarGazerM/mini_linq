use std::collections::{BTreeMap, BTreeSet};

use crate::{Column, CommaList, SemicolonList, relational_plan, symbol_name};
use syn::{Ident, token};

/// A relation and a possibly empty, optionally trailing-comma index key.
///
/// ```text
/// IndexRequirement ::= Ident "[" [Column ("," Column)* [","]] "]"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct IndexRequirement {
    pub relation: Ident,
    #[syn(bracketed)]
    pub bracket_token: token::Bracket,
    #[syn(in = bracket_token)]
    #[parse(CommaList::parse_terminated)]
    pub key_columns: CommaList<Column>,
}

/// A complete relational plan followed by a possibly empty index overlay.
///
/// ```text
/// struct TriangleProgram;
/// relation R(c0: i32, c1: i32);
/// relation S(c0: i32, c1: i32);
/// relation T(c0: i32, c1: i32);
/// triangle(x, y, z) :- R(x, y), S(y, z), T(z, x).
/// relational {
///     r0 = rename R {c0 -> x, c1 -> y};
///     r1 = rename S {c0 -> y, c1 -> z};
///     r2 = natural_join r0 with r1;
///     r3 = rename T {c0 -> z, c1 -> x};
///     r4 = natural_join r2 with r3;
///     r5 = project r4 keep {x, y, z};
///     output r5 as triangle(x, y, z).
/// }
/// indexes {
///     S[0];
///     T[0, 1];
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Module {
    pub relational_plan: relational_plan::Module,
    pub indexes_token: crate::kw::indexes,
    #[syn(braced)]
    pub indexes_brace: syn::token::Brace,
    #[syn(in = indexes_brace)]
    #[parse(SemicolonList::parse_terminated)]
    pub indexes: SemicolonList<IndexRequirement>,
}

/// The IndexRequirements contract in diagnostic and predicate form.
pub mod contract {
    /// Check the IndexRequirements language contract with precise diagnostics.
    pub fn check(module: &super::Module) -> syn::Result<()> {
        super::relational_plan::contract::check(&module.relational_plan)?;
        super::check_indexes(module)
    }

    /// Whether a parsed module satisfies the IndexRequirements contract.
    pub fn well_formed(module: &super::Module) -> bool {
        check(module).is_ok()
    }
}

fn check_indexes(module: &Module) -> syn::Result<()> {
    let schema = module
        .relational_plan
        .program
        .inputs
        .iter()
        .map(|input| (symbol_name(&input.name), input.arity()))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();

    for index in &module.indexes {
        let Some(arity) = schema.get(&symbol_name(&index.relation)) else {
            return Err(syn::Error::new_spanned(
                index,
                format!(
                    "index requirement names undeclared relation `{}`",
                    index.relation
                ),
            ));
        };
        if index.key_columns.is_empty() {
            return Err(syn::Error::new_spanned(
                index,
                format!(
                    "index requirement `{}` has an empty key; an empty bound key requires no index entry",
                    index.relation
                ),
            ));
        }
        let columns = index
            .key_columns
            .iter()
            .map(|column| column.index as usize)
            .collect::<Vec<_>>();
        if let Some(column) = columns.iter().copied().find(|column| column >= arity) {
            return Err(syn::Error::new_spanned(
                index,
                format!(
                    "index requirement `{}[{column}]` exceeds relation arity {arity}",
                    index.relation
                ),
            ));
        }
        if columns.windows(2).any(|columns| columns[0] >= columns[1]) {
            return Err(syn::Error::new_spanned(
                index,
                format!(
                    "index key columns for `{}` must be strictly increasing",
                    index.relation
                ),
            ));
        }
        let key = (symbol_name(&index.relation), columns);
        if !seen.insert(key) {
            return Err(syn::Error::new_spanned(
                index,
                format!(
                    "index requirement for `{}` on columns {:?} is duplicated",
                    index.relation, index.key_columns
                ),
            ));
        }
    }

    Ok(())
}
