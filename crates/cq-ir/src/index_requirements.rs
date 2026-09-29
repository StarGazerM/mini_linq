//! IX records exact index demands over RA results.
//!
//! The complete relational plan is retained unchanged. Index descriptors use
//! sorted logical headings of named RA results. Physical derives demand uses
//! from its operands and checks them against these descriptors.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Column, CommaList, SemicolonList, relational_plan, symbol_name};
use syn::{Ident, Token, token};

mod joins;
pub use joins::result_join_columns;

/// An exact RA result version and its ordered logical column ordinals.
///
/// ```text
/// IndexRequirement ::= "@" Ident "[" [Column ("," Column)* [","]] "]"
/// ```
///
/// The empty list denotes indexing by the unit tuple, including a Cartesian
/// join or a semijoin that tests whether its right relation is nonempty.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct IndexRequirement {
    pub at_token: Token![@],
    pub relation: Ident,
    #[syn(bracketed)]
    pub bracket_token: token::Bracket,
    #[syn(in = bracket_token)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Column>,
}

/// `i = @r[C]`, separated from other definitions by a semicolon.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct IndexDefinition {
    pub name: Ident,
    pub eq_token: Token![=],
    pub requirement: IndexRequirement,
}

/// Semantic identity of an index demand; index names form a separate namespace.
pub type Signature = (String, Vec<usize>);

pub fn signature(index: &IndexRequirement) -> Signature {
    (
        symbol_name(&index.relation),
        index
            .columns
            .iter()
            .map(|column| column.index as usize)
            .collect(),
    )
}

/// A complete RA program followed by named index demands.
///
/// ```text
/// Module ::= RA "indexes" "{" (IndexDefinition ";")* "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Module {
    pub relational_plan: relational_plan::Module,
    pub indexes_token: crate::kw::indexes,
    #[syn(braced)]
    pub indexes_brace: token::Brace,
    #[syn(in = indexes_brace)]
    #[parse(SemicolonList::parse_terminated)]
    pub indexes: SemicolonList<IndexDefinition>,
}

pub mod contract {
    /// Check RA, unique descriptors, valid ordinals, and no unused demand.
    pub fn check(module: &super::Module) -> syn::Result<()> {
        super::relational_plan::contract::check(&module.relational_plan)?;
        super::check_indexes(module)
    }

    pub fn well_formed(module: &super::Module) -> bool {
        check(module).is_ok()
    }
}

fn check_indexes(module: &Module) -> syn::Result<()> {
    if !module.indexes.is_empty() && !module.indexes.trailing_punct() {
        return Err(syn::Error::new_spanned(
            module.indexes.last().unwrap(),
            "index definitions must end with a semicolon",
        ));
    }
    let plan = &module.relational_plan;
    let results = relational_plan::check_in(plan.program.inputs(), &plan.plan)?;
    let mut indexes = BTreeMap::new();
    let mut descriptors = BTreeSet::new();
    for index in &module.indexes {
        let requirement = &index.requirement;
        let (relation, columns) = signature(requirement);
        let Some(info) = results.get(&relation) else {
            return Err(syn::Error::new_spanned(
                requirement,
                format!("index source `@{relation}` must name an exact RA result"),
            ));
        };
        if let Some(column) = columns.iter().find(|&&column| column >= info.heading.len()) {
            return Err(syn::Error::new_spanned(
                requirement,
                format!(
                    "index column {column} exceeds heading arity {} of `@{relation}`",
                    info.heading.len()
                ),
            ));
        }
        if columns.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(syn::Error::new_spanned(
                requirement,
                "index columns must be strictly increasing logical heading ordinals",
            ));
        }
        if indexes.insert(symbol_name(&index.name), index).is_some() {
            return Err(syn::Error::new_spanned(
                &index.name,
                "duplicate index identity",
            ));
        }
        if !descriptors.insert((relation, columns)) {
            return Err(syn::Error::new_spanned(
                requirement,
                "duplicate index descriptor: uses of the same version and columns must share one index identity",
            ));
        }
    }

    let mut expected = BTreeSet::new();
    for definition in &plan.plan.definitions {
        let (left, right) = match &definition.operator {
            relational_plan::Operator::NaturalJoin(join) => (&join.left, &join.right),
            relational_plan::Operator::Semijoin(join) => (&join.left, &join.right),
            relational_plan::Operator::Unit(_)
            | relational_plan::Operator::Rename(_)
            | relational_plan::Operator::Project(_)
            | relational_plan::Operator::Select(_)
            | relational_plan::Operator::Extend(_) => continue,
        };
        let columns =
            result_join_columns(&results[&symbol_name(left)], &results[&symbol_name(right)]);
        expected.insert((symbol_name(right), columns));
    }

    for (name, index) in indexes {
        if !expected.contains(&signature(&index.requirement)) {
            return Err(syn::Error::new_spanned(
                &index.name,
                format!("index `{name}` is demanded by no join"),
            ));
        }
    }
    Ok(())
}
