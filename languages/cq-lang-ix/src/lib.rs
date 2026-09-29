//! IX records exact index demands over RA results.
//!
//! The complete relational plan is retained unchanged. Index descriptors use
//! sorted logical headings of named RA results. Physical lowering checks its probe and semijoin reads against these demands.

pub use cq_lang_ra as ra;

use std::collections::{BTreeMap, BTreeSet};

pub use cq_syntax::{Column, CommaList, SemicolonList, kw, symbol_name};
use syn::{Ident, Token, token};

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
/// Program ::= ra::Program "indexes" "{" (IndexDefinition ";")* "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Program {
    pub ra: ra::Program,
    pub indexes_token: crate::kw::indexes,
    #[syn(braced)]
    pub indexes_brace: token::Brace,
    #[syn(in = indexes_brace)]
    #[parse(SemicolonList::parse_terminated)]
    pub indexes: SemicolonList<IndexDefinition>,
}

pub mod contract {
    pub fn check(program: &super::Program) -> syn::Result<()> {
        super::check_indexes(program)
    }
}

fn check_indexes(module: &Program) -> syn::Result<()> {
    let info = ra::contract::check(&module.ra)?;
    if !module.indexes.is_empty() && !module.indexes.trailing_punct() {
        return Err(syn::Error::new_spanned(
            module.indexes.last().unwrap(),
            format!(
                "index `{}` must end with a semicolon",
                module.indexes.last().unwrap().name
            ),
        ));
    }
    let mut indexes = BTreeMap::new();
    let mut descriptors = BTreeSet::new();
    for index in &module.indexes {
        let requirement = &index.requirement;
        let (relation, columns) = signature(requirement);
        let Some(info) = info.get(&relation).map(|i| &i.heading) else {
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
                format!(
                    "index `{}` on `{relation}` columns must be strictly increasing logical heading ordinals",
                    index.name
                ),
            ));
        }
        if indexes.insert(symbol_name(&index.name), index).is_some() {
            return Err(syn::Error::new_spanned(
                &index.name,
                format!("duplicate index identity `{}`", index.name),
            ));
        }
        if !descriptors.insert((relation, columns)) {
            return Err(syn::Error::new_spanned(
                requirement,
                format!(
                    "duplicate index descriptor `{}` for `{}`",
                    index.name, index.requirement.relation
                ),
            ));
        }
    }

    let mut expected = BTreeSet::new();
    for definition in &module.ra.plan.definitions {
        let (left, right) = match &definition.operator {
            ra::Operator::NaturalJoin(j) => (&j.left, &j.right),
            ra::Operator::Semijoin(j) => (&j.left, &j.right),
            _ => continue,
        };
        let lh = &info[&symbol_name(left)].heading;
        let rh = &info[&symbol_name(right)].heading;
        let columns = rh
            .iter()
            .enumerate()
            .filter_map(|(i, c)| lh.contains(c).then_some(i))
            .collect();
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

/// Shared syntax used by this language.
pub use cq_syntax as syntax;
