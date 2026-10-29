//! IX records exact index demands over RA results.
//!
//! The complete relational plan is retained unchanged. Index descriptors use
//! sorted logical headings of named RA results and explicitly record whether
//! key values come from a stream or a closed host expression. Rust expressions
//! are opaque and total; index analysis preserves their recorded placement
//! without deriving relational demands from their syntax.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! IX preserves union/anti and block paths.
//! Anti requests membership on its exact F heading, while union streams each operand without adding an index demand.
//! Group retains the complete RA valuation domain and introduces no index demand;
//! its canonical input and key-unique output are realized by Physical.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Union siblings are unordered; fallback retains its relational anti steps.
//! Only differing literals on one interface column prove union arms disjoint.
//!
pub use cq_lang_ra as ra;

use std::collections::{BTreeMap, BTreeSet};

pub use cq_syntax::{Column, CommaList, SemicolonList, SupplyKind, symbol_name};
use syn::{Ident, Token, token};

pub mod kw {
    pub use cq_syntax::kw::indexes;
    syn::custom_keyword!(stream);
    syn::custom_keyword!(range);
}

/// Key/window supply: another result, fixed point values, or constant order bounds.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Supply {
    #[parse(peek = kw::stream)]
    Stream(kw::stream),
    #[parse(peek = Token![const])]
    Const(Token![const]),
    #[parse(peek = kw::range)]
    Range(kw::range),
}

impl Supply {
    pub fn kind(&self) -> SupplyKind {
        match self {
            Self::Stream(_) => SupplyKind::Stream,
            Self::Const(_) => SupplyKind::Const,
            Self::Range(_) => SupplyKind::Range,
        }
    }
}

impl From<SupplyKind> for Supply {
    fn from(kind: SupplyKind) -> Self {
        match kind {
            SupplyKind::Stream => Self::Stream(Default::default()),
            SupplyKind::Const => Self::Const(Default::default()),
            SupplyKind::Range => Self::Range(Default::default()),
        }
    }
}

/// An exact RA result version and its ordered logical column ordinals.
///
/// ```text
/// IndexRequirement ::= "@" Ident "[" [Column ("," Column)* [","]] "]" ("stream" | "const" | "range")
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
    pub supply: Supply,
}

/// `i = @r[C] stream` or `i = @r[C] const`, followed by a semicolon.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct IndexDefinition {
    pub name: Ident,
    pub eq_token: Token![=],
    pub requirement: IndexRequirement,
}

/// Semantic identity of an index demand; index names form a separate namespace.
pub type Signature = (String, Vec<usize>, SupplyKind);

/// One first-use demand of a checked RA program, in definition order.
#[derive(Clone, Debug)]
pub struct Demand<'a> {
    pub definition: &'a Ident,
    pub relation: &'a Ident,
    pub signature: Signature,
}

/// Derive exact demands from the analysis returned by the RA contract.
/// Repeated descriptors share the first occurrence, including its source span.
pub fn demands<'a>(program: &'a ra::Program, info: &ra::Analysis) -> Vec<Demand<'a>> {
    let mut seen = BTreeSet::new();
    let mut demands = Vec::new();
    for definition in &program.plan.definitions {
        let (right, keys, supply) = match &definition.operator {
            // IX-Join: increasing logical ordinals in the right heading.
            ra::Operator::NaturalJoin(join) => (
                &join.right,
                info[&symbol_name(&join.left)].heading.clone(),
                SupplyKind::Stream,
            ),
            ra::Operator::Semijoin(join) => (
                &join.right,
                info[&symbol_name(&join.left)].heading.clone(),
                SupplyKind::Stream,
            ),
            ra::Operator::Anti(anti) => (
                &anti.right,
                anti.columns.iter().map(symbol_name).collect(),
                SupplyKind::Stream,
            ),
            ra::Operator::Select(selection) => {
                // IX-Const: column equalities and membership supply constant keys.
                let (keys, supply) = if let Some(keys) = selection.condition.point_columns() {
                    (keys, SupplyKind::Const)
                // IX-Range: one column with constant order bounds supplies a window.
                } else if let Some((column, _)) = selection.condition.range_bounds() {
                    (BTreeSet::from([column]), SupplyKind::Range)
                } else {
                    continue;
                };
                (&selection.input, keys, supply)
            }
            // IX-Other: no demand is introduced by other operators.
            _ => continue,
        };
        let columns = info[&symbol_name(right)]
            .heading
            .iter()
            .enumerate()
            .filter_map(|(i, c)| keys.contains(c).then_some(i))
            .collect();
        let signature = (symbol_name(right), columns, supply);
        // IX-Reuse: only the first occurrence contributes a demand.
        if seen.insert(signature.clone()) {
            demands.push(Demand {
                definition: &definition.result,
                relation: right,
                signature,
            });
        }
    }
    demands
}

pub fn signature(index: &IndexRequirement) -> Signature {
    (
        symbol_name(&index.relation),
        index
            .columns
            .iter()
            .map(|column| column.index as usize)
            .collect(),
        index.supply.kind(),
    )
}

/// A complete RA program followed by named index demands.
///
/// ```text
/// Program ::= ra::Program "indexes" "{" [IndexDefinition (";" IndexDefinition)* [";"]] "}"
/// ```
///
/// This is the parser grammar. The contract requires every definition,
/// including the last, to end with a semicolon.
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

/// Formal rule WF-IX: Check valid unique used descriptors; complete coverage is a pass obligation.
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
        let (relation, columns, supply) = signature(requirement);
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
        if !descriptors.insert((relation, columns, supply)) {
            return Err(syn::Error::new_spanned(
                requirement,
                format!(
                    "duplicate index descriptor `{}` for `{}`",
                    index.name, index.requirement.relation
                ),
            ));
        }
    }

    let expected: BTreeSet<_> = demands(&module.ra, &info)
        .into_iter()
        .map(|d| d.signature)
        .collect();
    for (name, index) in indexes {
        if !expected.contains(&signature(&index.requirement)) {
            return Err(syn::Error::new_spanned(
                &index.name,
                format!(
                    "index `{name}` is demanded by no join or literal selection with this supply"
                ),
            ));
        }
    }
    Ok(())
}

/// Shared syntax used by this language.
pub use cq_syntax as syntax;

pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    if source.ra != target.ra {
        return Err(syn::Error::new_spanned(&target.ra, "IX rewrite changes RA"));
    }
    Ok(())
}
impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
