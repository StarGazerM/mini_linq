//! Existing RA definitions under only their free physical binding assumptions.
//!
//! Administrative macro configuration is not part of this syntax. A projection
//! constructs a key resource; a semijoin consumes that projected relation. The
//! whole-source edge establishes the projection identity and resource lifetime.
use crate::{cq, relational_plan as ra, symbol_name};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Path, Token};

/// A row provider is explicit; an omitted provider binds a resource interpreted
/// by the selected recipe. Both forms denote the named relation, never its source.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Binding {
    pub atom: cq::Atom,
    #[syn(optional, peek = Token![=>])]
    pub provider: Option<Provider>,
    #[syn(braced)]
    pub value_brace: syn::token::Brace,
    #[syn(in = value_brace)]
    pub value: syn::Expr,
    pub semi_token: Token![;],
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Provider {
    pub fat_arrow_token: Token![=>],
    pub path: Path,
}

/// `Context ::= Definition "{" Binding* "}"`.
/// `Binding ::= Atom ["=>" Path] "{" RustExpr "}" ";"`.
/// The domain is exactly the source's distinct operands. No phase, dependency
/// path, instrumentation, copied key ordinals or resource-origin assertion occurs.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Context {
    pub source: ra::Definition,
    #[syn(braced)]
    pub bindings_brace: syn::token::Brace,
    #[syn(in = bindings_brace, repeat)]
    pub bindings: Vec<Binding>,
}

#[derive(Debug)]
pub struct Analysis<'a> {
    pub left: &'a Binding,
    pub right: &'a Binding,
    pub left_layout: Vec<String>,
    pub right_layout: Vec<String>,
    pub separator: Vec<String>,
    pub left_columns: Vec<usize>,
    pub right_columns: Vec<usize>,
    pub output_layout: Vec<String>,
}
#[derive(Debug)]
pub struct Projection<'a> {
    pub input: &'a Binding,
    pub columns: Vec<usize>,
    pub output_layout: Vec<String>,
}

fn bindings<'a>(
    context: &'a Context,
    operands: &[&syn::Ident],
) -> syn::Result<BTreeMap<String, (&'a Binding, Vec<String>)>> {
    let operands: BTreeSet<_> = operands.iter().map(|id| symbol_name(id)).collect();
    if operands.contains(&symbol_name(&context.source.result)) {
        return Err(syn::Error::new_spanned(
            &context.source.result,
            "physical context result must not capture an operand name",
        ));
    }
    let mut bindings = BTreeMap::new();
    for binding in &context.bindings {
        let name = symbol_name(&binding.atom.relation);
        if !operands.contains(&name) {
            return Err(syn::Error::new_spanned(
                &binding.atom,
                "physical context contains an unreferenced operand binding",
            ));
        }
        let layout = layout(&binding.atom)?;
        if bindings.insert(name, (binding, layout)).is_some() {
            return Err(syn::Error::new_spanned(
                &binding.atom,
                "physical context repeats an operand binding",
            ));
        }
    }
    if bindings.keys().cloned().collect::<BTreeSet<_>>() != operands {
        return Err(syn::Error::new_spanned(
            context,
            "physical context is missing an operand binding",
        ));
    }
    Ok(bindings)
}

/// Derive paired key positions and result layout from existing binary RA.
pub fn analyze(context: &Context) -> syn::Result<Analysis<'_>> {
    let (left, right, join) = match &context.source.operator {
        ra::Operator::NaturalJoin(op) => (&op.left, &op.right, true),
        ra::Operator::Semijoin(op) => (&op.left, &op.right, false),
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "binary physical context requires NaturalJoin or Semijoin",
            ));
        }
    };
    let bindings = bindings(context, &[left, right])?;
    let (left, left_layout) = &bindings[&symbol_name(left)];
    let (right, right_layout) = &bindings[&symbol_name(right)];
    let left_heading: BTreeSet<_> = left_layout.iter().cloned().collect();
    let right_heading: BTreeSet<_> = right_layout.iter().cloned().collect();
    let separator: Vec<_> = left_heading.intersection(&right_heading).cloned().collect();
    let positions = |layout: &[String]| {
        separator
            .iter()
            .map(|name| {
                layout
                    .iter()
                    .position(|attribute| attribute == name)
                    .unwrap()
            })
            .collect()
    };
    Ok(Analysis {
        left,
        right,
        left_layout: left_layout.clone(),
        right_layout: right_layout.clone(),
        left_columns: positions(left_layout),
        right_columns: positions(right_layout),
        output_layout: if join {
            left_heading.union(&right_heading).cloned().collect()
        } else {
            left_layout.clone()
        },
        separator,
    })
}

/// Projection is already an RA operation; only its source value is free here.
pub fn analyze_project(context: &Context) -> syn::Result<Projection<'_>> {
    let ra::Operator::Project(op) = &context.source.operator else {
        return Err(syn::Error::new_spanned(
            &context.source.operator,
            "key construction requires a Project definition",
        ));
    };
    let bindings = bindings(context, &[&op.input])?;
    let (input, layout) = &bindings[&symbol_name(&op.input)];
    let mut keep = BTreeSet::new();
    for attribute in &op.attributes {
        let name = symbol_name(attribute);
        if !layout.contains(&name) || !keep.insert(name) {
            return Err(syn::Error::new_spanned(
                attribute,
                "projection requires distinct attributes of its input",
            ));
        }
    }
    let output_layout: Vec<_> = keep.into_iter().collect();
    let columns = output_layout
        .iter()
        .map(|name| layout.iter().position(|column| column == name).unwrap())
        .collect();
    Ok(Projection {
        input,
        columns,
        output_layout,
    })
}

fn layout(atom: &cq::Atom) -> syn::Result<Vec<String>> {
    let mut names = Vec::new();
    let mut seen = BTreeSet::new();
    for argument in &atom.args {
        let variable = cq::rust::variable(argument).ok_or_else(|| {
            syn::Error::new_spanned(
                argument,
                "physical context layouts require bare variable arguments",
            )
        })?;
        let name = symbol_name(variable);
        if !seen.insert(name.clone()) {
            return Err(syn::Error::new_spanned(
                argument,
                "physical context layouts require distinct attributes",
            ));
        }
        names.push(name);
    }
    Ok(names)
}

/// Local representation obligations; whole-source correspondence establishes
/// actual values, projected resource origins, selected implementations and scope.
pub mod contract {
    use super::*;
    fn rows(binding: &Binding) -> syn::Result<()> {
        if binding.provider.is_none() {
            return Err(syn::Error::new_spanned(
                binding,
                "row access requires a configured provider",
            ));
        }
        Ok(())
    }
    pub fn check_join(context: &Context) -> syn::Result<()> {
        if !matches!(context.source.operator, ra::Operator::NaturalJoin(_)) {
            return Err(syn::Error::new_spanned(
                &context.source.operator,
                "join recipe requires a NaturalJoin definition",
            ));
        }
        let analysis = analyze(context)?;
        rows(analysis.left)?;
        rows(analysis.right)
    }
    pub fn check_project(context: &Context) -> syn::Result<()> {
        rows(analyze_project(context)?.input)
    }
    pub fn check_semijoin(context: &Context) -> syn::Result<()> {
        if !matches!(context.source.operator, ra::Operator::Semijoin(_)) {
            return Err(syn::Error::new_spanned(
                &context.source.operator,
                "reduction requires a Semijoin definition",
            ));
        }
        let analysis = analyze(context)?;
        rows(analysis.left)?;
        if analysis.right.provider.is_some() || analysis.right_layout != analysis.separator {
            return Err(syn::Error::new_spanned(
                analysis.right,
                "semijoin requires a projected key resource in canonical shared-heading order",
            ));
        }
        Ok(())
    }
    pub fn well_formed_join(context: &Context) -> bool {
        check_join(context).is_ok()
    }
    pub fn well_formed_semijoin(context: &Context) -> bool {
        check_semijoin(context).is_ok()
    }
}
