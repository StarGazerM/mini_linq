//! IX records exact index demands over RA results.
//!
//! The complete relational plan is retained unchanged. Index descriptors use
//! sorted logical headings of named RA results. Physical derives demand uses
//! from its operands and checks them against these descriptors.

#[cfg(feature = "reference-prefix")]
pub use cq_lang_ra as relational_plan;

use std::collections::{BTreeMap, BTreeSet};

pub use cq_syntax::{Column, CommaList, SemicolonList, kw, symbol_name};
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
pub struct Section {
    pub indexes_token: crate::kw::indexes,
    #[syn(braced)]
    pub indexes_brace: token::Brace,
    #[syn(in = indexes_brace)]
    #[parse(SemicolonList::parse_terminated)]
    pub indexes: SemicolonList<IndexDefinition>,
}

/// A binary logical operation whose right operand can demand an index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub result: String,
    pub left: String,
    pub right: String,
    pub semi: bool,
}

/// The RA facts needed to validate an indexes section.
pub trait RaEnv {
    fn heading(&self, result: &str) -> Option<&[String]>;
    fn joins(&self) -> Vec<Join>;
}

pub mod contract {
    pub fn check(section: &super::Section, env: &impl super::RaEnv) -> syn::Result<()> {
        super::check_indexes(section, env)
    }
    pub fn well_formed(section: &super::Section, env: &impl super::RaEnv) -> bool {
        check(section, env).is_ok()
    }
    #[cfg(feature = "reference-prefix")]
    pub fn check_module(module: &super::Module) -> syn::Result<()> {
        check(
            &module.section,
            &super::reference::Env::new(&module.relational_plan)?,
        )
    }
    #[cfg(feature = "reference-prefix")]
    pub fn well_formed_module(module: &super::Module) -> bool {
        check_module(module).is_ok()
    }
}

fn check_indexes(module: &Section, env: &impl RaEnv) -> syn::Result<()> {
    if !module.indexes.is_empty() && !module.indexes.trailing_punct() {
        return Err(syn::Error::new_spanned(
            module.indexes.last().unwrap(),
            "index definitions must end with a semicolon",
        ));
    }
    let mut indexes = BTreeMap::new();
    let mut descriptors = BTreeSet::new();
    for index in &module.indexes {
        let requirement = &index.requirement;
        let (relation, columns) = signature(requirement);
        let Some(info) = env.heading(&relation) else {
            return Err(syn::Error::new_spanned(
                requirement,
                format!("index source `@{relation}` must name an exact RA result"),
            ));
        };
        if let Some(column) = columns.iter().find(|&&column| column >= info.len()) {
            return Err(syn::Error::new_spanned(
                requirement,
                format!(
                    "index column {column} exceeds heading arity {} of `@{relation}`",
                    info.len()
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
    for join in env.joins() {
        let Some(left) = env.heading(&join.left) else {
            return Err(syn::Error::new_spanned(
                module.indexes_token,
                "join's left result has no heading",
            ));
        };
        let Some(right) = env.heading(&join.right) else {
            return Err(syn::Error::new_spanned(
                module.indexes_token,
                "join's right result has no heading",
            ));
        };
        expected.insert((join.right, result_join_columns(left, right)));
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

/// Shared syntax used by this language.
pub use cq_syntax as syntax;

#[cfg(feature = "reference-prefix")]
pub mod reference;
#[cfg(feature = "reference-prefix")]
pub use reference::Module;

mod env;
pub use env::{Env, Operation};
pub use env::{RowFacts, rows};
