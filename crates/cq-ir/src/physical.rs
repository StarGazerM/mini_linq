//! IX with every relation provider, index provider, and lowering rule recorded.
//!
//! This language records physical choices; it does not expand those choices or
//! infer their meaning from a Rust path. Its structural check establishes the
//! exact assignment domains. Provider admission and representation laws require
//! the external definitions and are a separate validation obligation.
//!
//! The one relation-name table requires host inputs and RA results to have
//! distinct names. RA itself permits a collision; this physical domain rejects
//! it rather than conflating two different representation assignments.

pub mod lookup;

use std::collections::BTreeSet;

use syn::{Ident, Path, Token, token};

use crate::{SemicolonList, index_requirements, relational_plan, symbol_name};

/// Keywords owned by the Physical language.
pub mod kw {
    syn::custom_keyword!(physical);
    syn::custom_keyword!(relations);
    syn::custom_keyword!(lowerings);
}

/// One fixed external implementation instance, named through Rust resolution.
///
/// ```text
/// Binding ::= Ident "=" RustPath
/// ```
///
/// The containing list owns the semicolon. The path is opaque to the common
/// compiler; a provider's name neither declares nor proves its semantics.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Binding {
    pub name: Ident,
    pub eq_token: Token![=],
    pub implementation: Path,
}

/// Complete explicit choices for one IX program.
///
/// ```text
/// Assignments ::= "physical" "{"
///     "relations" "{" (Binding ";")* "}"
///     "indexes"   "{" (Binding ";")* "}"
///     "lowerings" "{" (Binding ";")* "}"
/// "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Assignments {
    pub physical_token: kw::physical,
    #[syn(braced)]
    pub physical_brace: token::Brace,
    #[syn(in = physical_brace)]
    pub relations_token: kw::relations,
    #[syn(in = physical_brace, braced)]
    pub relations_brace: token::Brace,
    #[syn(in = relations_brace)]
    #[parse(SemicolonList::parse_terminated)]
    pub relations: SemicolonList<Binding>,
    #[syn(in = physical_brace)]
    pub indexes_token: crate::kw::indexes,
    #[syn(in = physical_brace, braced)]
    pub indexes_brace: token::Brace,
    #[syn(in = indexes_brace)]
    #[parse(SemicolonList::parse_terminated)]
    pub indexes: SemicolonList<Binding>,
    #[syn(in = physical_brace)]
    pub lowerings_token: kw::lowerings,
    #[syn(in = physical_brace, braced)]
    pub lowerings_brace: token::Brace,
    #[syn(in = lowerings_brace)]
    #[parse(SemicolonList::parse_terminated)]
    pub lowerings: SemicolonList<Binding>,
}

/// A complete IX payload followed by its resolved physical choices.
///
/// ```text
/// Module ::= IX Assignments
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Module {
    pub index_requirements: index_requirements::Module,
    pub assignments: Assignments,
}

/// Structural validity is necessary but does not establish provider admission.
pub mod contract {
    /// Check IX and the exact domains of its physical assignment tables.
    ///
    /// This does not resolve implementation paths, check external provider
    /// premises, prove representation laws, or type-check generated Rust.
    pub fn check_structure(module: &super::Module) -> syn::Result<()> {
        super::index_requirements::contract::check(&module.index_requirements)?;
        super::check_assignments(module)
    }
}

fn check_assignments(module: &Module) -> syn::Result<()> {
    let plan = &module.index_requirements.relational_plan.plan;
    let results: BTreeSet<_> = plan
        .definitions
        .iter()
        .map(|definition| symbol_name(&definition.result))
        .collect();
    let inputs: BTreeSet<_> = plan
        .definitions
        .iter()
        .filter_map(|definition| match &definition.operator {
            relational_plan::Operator::Rename(rename) => Some(symbol_name(&rename.relation)),
            relational_plan::Operator::Unit(_)
            | relational_plan::Operator::NaturalJoin(_)
            | relational_plan::Operator::Project(_)
            | relational_plan::Operator::Semijoin(_)
            | relational_plan::Operator::Select(_)
            | relational_plan::Operator::Extend(_) => None,
        })
        .collect();

    if let Some(name) = inputs.intersection(&results).next() {
        return Err(syn::Error::new_spanned(
            &module.assignments,
            format!(
                "physical relation name `{name}` denotes both a host input and an RA result; use distinct names"
            ),
        ));
    }

    let relations = inputs.union(&results).cloned().collect();
    let indexes = module
        .index_requirements
        .indexes
        .iter()
        .map(|index| symbol_name(&index.name))
        .collect();
    check_domain(
        &module.assignments.relations,
        &relations,
        "relation provider",
        &module.assignments.relations_token,
    )?;
    check_domain(
        &module.assignments.indexes,
        &indexes,
        "index provider",
        &module.assignments.indexes_token,
    )?;
    check_domain(
        &module.assignments.lowerings,
        &results,
        "lowering",
        &module.assignments.lowerings_token,
    )
}

fn check_domain(
    bindings: &SemicolonList<Binding>,
    expected: &BTreeSet<String>,
    description: &str,
    table: &impl quote::ToTokens,
) -> syn::Result<()> {
    if !bindings.is_empty() && !bindings.trailing_punct() {
        return Err(syn::Error::new_spanned(
            table,
            format!("each {description} assignment must end with `;`"),
        ));
    }
    let mut seen = BTreeSet::new();
    for binding in bindings {
        let name = symbol_name(&binding.name);
        if !seen.insert(name.clone()) {
            return Err(syn::Error::new_spanned(
                &binding.name,
                format!("duplicate {description} assignment for `{name}`"),
            ));
        }
        if !expected.contains(&name) {
            return Err(syn::Error::new_spanned(
                &binding.name,
                format!("unexpected {description} assignment for `{name}`"),
            ));
        }
    }
    if let Some(name) = expected.difference(&seen).next() {
        return Err(syn::Error::new_spanned(
            table,
            format!("missing {description} assignment for `{name}`"),
        ));
    }
    Ok(())
}
