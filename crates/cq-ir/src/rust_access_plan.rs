use std::collections::BTreeSet;

use syn::{Ident, Pat, Token};

use crate::{CommaList, cq, symbol_name};

/// A complete source-ordered query whose logical occurrences are paired with
/// their selected Rust access or output expression.
///
/// This is the residual query region embedded in the complete staged Rust
/// file. The surrounding file already declares concrete collections,
/// builders, and the public lazy-query API; expressions in this region refer
/// to those ordinary Rust bindings. The pull pass exposes the corresponding
/// binary operators and intermediate bindings as an `IteratorPipeline`; a
/// separate lowering composes that plan into a concrete Rust expression.
///
/// ```text
/// triangle(x, y, z) => ((x.clone(), y.clone(), z.clone(),)) :-
///     R(x, y) => for (x, y,) in (rows0.iter()),
///     S(y, z) => for (z,) in
///         (index0.get(&(y.clone(),)).into_iter().flatten()),
///     T(z, x) => if (index1.contains(&(z.clone(), x.clone(),))).
/// ```
///
/// The logical query shape plus one Rust annotation on every occurrence. The
/// generated R1 physical sources yield references, so positive variables in
/// those plans are borrowed; keys and the owned output row explicitly
/// dereference those bindings. The Plan language itself still accepts an
/// arbitrary Rust source expression.
///
/// ```text
/// Plan ::= Output ":-" Clause ("," Clause)* "."
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Plan {
    pub head: Output,
    pub colon_token: Token![:],
    pub minus_token: Token![-],
    #[parse(CommaList::parse_separated_nonempty)]
    pub body: CommaList<Clause>,
    pub dot_token: Token![.],
}

/// The result occurrence paired directly with the Rust value to yield.
///
/// ```text
/// Output ::= Atom "=>" "(" RustExpr ")"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Output {
    pub atom: cq::Atom,
    pub fat_arrow_token: Token![=>],
    #[syn(parenthesized)]
    pub output_paren: syn::token::Paren,
    #[syn(in = output_paren)]
    pub output: syn::Expr,
}

/// One logical body occurrence paired directly with its actual Rust access.
///
/// ```text
/// Clause ::= BodyItem "=>" RustAccess
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Clause {
    pub item: cq::BodyItem,
    pub fat_arrow_token: Token![=>],
    pub access: RustAccess,
}

/// The already-selected Rust control shape and its Rust leaves.
///
/// No `scan`, `lookup`, `contains`, storage kind, or logical-operator
/// descriptor is repeated here. Collection types and method calls are the
/// concrete Rust syntax in `source` and `condition`.
///
/// ```text
/// RustAccess ::= "for" RustPat "in" "(" RustExpr ")"
///              | "if" "(" RustExpr ")"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum RustAccess {
    #[parse(peek = Token![for])]
    For {
        for_token: Token![for],
        #[parse(Pat::parse_multi_with_leading_vert)]
        pattern: Pat,
        in_token: Token![in],
        #[syn(parenthesized)]
        source_paren: syn::token::Paren,
        #[syn(in = source_paren)]
        source: syn::Expr,
    },
    #[parse(peek = Token![if])]
    If {
        if_token: Token![if],
        #[syn(parenthesized)]
        condition_paren: syn::token::Paren,
        #[syn(in = condition_paren)]
        condition: syn::Expr,
    },
}

// OPTIONAL HW4 BOUNDARY: do not add negation or aggregation variants here.
// The physical pass lowers negation to an ordinary Rust predicate in `If` and
// aggregation to an ordinary Rust iterator expression in `For`. The supplied
// pull pass exposes those Rust leaves as iterator operators without needing to
// rediscover the logical clauses they implement.

/// The occurrence-to-access contract in diagnostic and predicate form.
pub mod contract {
    /// Check the occurrence-to-access relation without scanning Rust expressions.
    ///
    /// The checker owns source-order binding and the required Rust control shape.
    /// Rust itself checks the types and names inside the already-concrete leaves.
    pub fn check(plan: &super::Plan) -> syn::Result<()> {
        super::check_plan(plan)
    }

    /// Whether a parsed plan satisfies the occurrence-to-access contract.
    pub fn well_formed(plan: &super::Plan) -> bool {
        check(plan).is_ok()
    }
}

fn check_plan(query: &Plan) -> syn::Result<()> {
    let mut bound = BTreeSet::new();

    for (position, clause) in query.body.iter().enumerate() {
        // OPTIONAL HW4 — STUDENT 5/6 (ANNOTATION): extend this relation when
        // BodyItem gains Negation and Aggregate. Safe negation requires `If`
        // and binds nothing; aggregation requires `For` with its simple result
        // identifier pattern and binds only that result.
        match &clause.item {
            cq::BodyItem::Positive { atom } => {
                check_positive_clause(position, atom, &clause.access, &mut bound)?;
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    &clause.item,
                    "this clause requires the extended access contract",
                ));
            }
        }
    }

    check_distinct_variables("result atom", &query.head.atom.args)?;
    for variable in query.head.atom.variables() {
        if !bound.contains(&symbol_name(variable)) {
            return Err(syn::Error::new_spanned(
                variable,
                format!("result variable `{variable}` is not bound by the annotated query body"),
            ));
        }
    }

    Ok(())
}

fn check_positive_clause(
    position: usize,
    atom: &cq::Atom,
    access: &RustAccess,
    bound: &mut BTreeSet<String>,
) -> syn::Result<()> {
    check_distinct_variables(&format!("body atom `{}`", atom.relation), &atom.args)?;

    let fresh = atom
        .variables()
        .filter(|variable| !bound.contains(&symbol_name(variable)))
        .collect::<Vec<_>>();

    match (fresh.is_empty(), access) {
        (false, RustAccess::For { pattern, .. }) => {
            check_fresh_pattern(position, pattern, &fresh)?;
        }
        (false, RustAccess::If { .. }) => {
            return Err(syn::Error::new_spanned(
                access,
                format!(
                    "body clause {} has fresh variables and therefore requires a Rust `for` access",
                    position + 1
                ),
            ));
        }
        (true, RustAccess::For { .. }) => {
            return Err(syn::Error::new_spanned(
                access,
                format!(
                    "body clause {} is fully bound and therefore requires a Rust `if` access",
                    position + 1
                ),
            ));
        }
        (true, RustAccess::If { .. }) => {}
    }

    bound.extend(atom.variables().map(symbol_name));
    Ok(())
}

fn check_fresh_pattern(position: usize, pattern: &Pat, expected: &[&Ident]) -> syn::Result<()> {
    let elements = match pattern {
        Pat::Tuple(tuple) if tuple.attrs.is_empty() => &tuple.elems,
        _ => return Err(fresh_pattern_error(position, pattern, expected)),
    };

    let matches = elements.len() == expected.len()
        && elements.iter().zip(expected).all(|(actual, expected)| {
            simple_pattern_ident(actual)
                .is_some_and(|actual| symbol_name(actual) == symbol_name(expected))
        });

    if !matches {
        return Err(fresh_pattern_error(position, pattern, expected));
    }
    Ok(())
}

fn fresh_pattern_error(position: usize, pattern: &Pat, expected: &[&Ident]) -> syn::Error {
    let expected = expected
        .iter()
        .map(|variable| symbol_name(variable))
        .collect::<Vec<_>>()
        .join(", ");
    syn::Error::new_spanned(
        pattern,
        format!(
            "Rust `for` pattern at body clause {} must be the simple tuple pattern `({expected},)` containing exactly its fresh variables in relation-column order",
            position + 1
        ),
    )
}

fn simple_pattern_ident(pattern: &Pat) -> Option<&Ident> {
    let Pat::Ident(pattern) = pattern else {
        return None;
    };
    (pattern.attrs.is_empty()
        && pattern.by_ref.is_none()
        && pattern.mutability.is_none()
        && pattern.subpat.is_none())
    .then_some(&pattern.ident)
}

fn check_distinct_variables(context: &str, variables: &CommaList<syn::Expr>) -> syn::Result<()> {
    if variables.is_empty() {
        return Err(syn::Error::new_spanned(
            variables,
            format!("{context} must have positive arity"),
        ));
    }

    let mut seen = BTreeSet::new();
    for argument in variables {
        let variable = cq::rust::variable(argument).ok_or_else(|| {
            syn::Error::new_spanned(
                argument,
                "computed arguments require the extended access contract",
            )
        })?;
        if !seen.insert(symbol_name(variable)) {
            return Err(syn::Error::new_spanned(
                variable,
                format!(
                    "{context} repeats variable `{variable}`; repeated terms are a stretch extension"
                ),
            ));
        }
    }
    Ok(())
}
