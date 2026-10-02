//! Shared syntax, declarations and Rust free-variable/pattern analysis.
//!
//! Compiler programs denote `bottom + Set`: one panic aborts evaluation.
//! Guard comparisons recognized under the premises below are relational. Other
//! Rust expressions are opaque partial functions of their free variables;
//! only relational steps commute or move earlier across the written Rust chain.
//! Equality in the relational centre is identity in classical Set.
//!
//! 1. Leibniz admissibility of a column type: a type may be compared, joined,
//!    grouped, keyed or deduplicated by the relational centre only if its `==`
//!    decides Leibniz equality (Rust code cannot distinguish `==`-equal values).
//!    Holds for integers, bool, char, String/&str, the cq-load Date/Decimal,
//!    tuples and derived-Eq structs of admissible types; fails for f64/f32
//!    (0.0 == -0.0) and hand-written lenient Eq.
//! 2. Functional extensionality of embedded Rust: an embedded expression receives
//!    query variables as shared references (`&T`, never `&mut`), so its value is
//!    a function of the values of its free variables; equal (Leibniz) inputs give
//!    equal results. Interior mutability, globals and I/O are outside the meaning,
//!    as already documented (evaluation count is not preserved).
//!
//! Any surviving representative of `==`-equal values IS the value. Dedup,
//! canonical projection, and projecting away variables before an opaque Rust step
//! are sound whenever the Rust step's free variables are retained. `Eq` and the
//! selected representation's `Hash`/`Ord` bounds are necessary mechanical checks;
//! rustc cannot prove Leibniz admissibility of a hand-written implementation.

use syn::ext::IdentExt;
use syn::{Ident, Index, Token, Type, token};

pub mod kw {
    syn::custom_keyword!(agg);
    syn::custom_keyword!(indexes);
    syn::custom_keyword!(relation);
}

/// A list that retains the separators written between its items.
///
/// It supports the familiar list operations used by the homework: iteration,
/// `len`, `is_empty`, `push`, and construction with `collect`.
pub type SyntaxList<Item, Separator> = syn::punctuated::Punctuated<Item, Separator>;

/// A syntax-preserving comma-separated list.
pub type CommaList<Item> = SyntaxList<Item, Token![,]>;

/// A syntax-preserving semicolon-separated list.
pub type SemicolonList<Item> = SyntaxList<Item, Token![;]>;

/// A zero-based relation-column number written as a Rust integer literal.
pub type Column = Index;

/// Return the MiniLinq symbol represented by a Rust identifier.
///
/// In particular, `x` and the Rust raw spelling `r#x` return the same name.
pub fn symbol_name(identifier: &Ident) -> String {
    identifier.unraw().to_string()
}

pub mod keyed;
pub use keyed::SupplyKind;
pub mod constraint;
pub mod guard;
pub mod predicate;
pub use constraint::Constraint;
pub mod rust;

/// One named, typed column in a declared input relation.
///
/// ```text
/// ColumnDecl ::= Ident ":" Type
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ColumnDecl {
    pub name: Ident,
    pub colon_token: Token![:],
    pub ty: Type,
}

/// One typed relation declaration supplied by the host program.
///
/// CQ atom arguments remain positional against this declaration order.
///
/// Declaration order is the host layout. A grouped host stores a leading
/// prefix as its key; Rust traits decide which operations its type supports.
///
/// ```text
/// RelationDecl ::= "relation" Ident "(" [ColumnDecl ("," ColumnDecl)* [","]] ")"
///                  ["=" "(" Expr ")"] ";"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct RelationDecl {
    pub relation_token: kw::relation,
    pub name: Ident,
    #[syn(parenthesized)]
    pub columns_paren: token::Paren,
    #[syn(in = columns_paren)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<ColumnDecl>,
    #[syn(optional, peek = Token![=])]
    pub value: Option<LeafValue>,
    pub semi_token: Token![;],
}

impl RelationDecl {
    /// The relation arity derived from its declared columns.
    pub fn arity(&self) -> usize {
        self.columns.len()
    }

    /// A result schema describes types, not a runtime input value.
    pub fn check_result(&self) -> syn::Result<()> {
        if self.value.is_some() {
            return Err(syn::Error::new_spanned(
                self,
                "result declarations cannot bind a host value",
            ));
        }
        Ok(())
    }
}

/// A leaf expression is evaluated once in the enclosing Rust environment.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct LeafValue {
    pub eq_token: Token![=],
    #[syn(parenthesized)]
    pub paren_token: token::Paren,
    #[syn(in = paren_token)]
    pub expression: syn::Expr,
}

use std::collections::{BTreeMap, BTreeSet};
/// A relation name applied to a possibly empty, optionally trailing-comma list.
///
/// ```text
/// Atom ::= Ident "(" [RustExpr ("," RustExpr)* [","]] ")"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Atom {
    pub relation: Ident,
    #[syn(parenthesized)]
    pub paren_token: token::Paren,
    #[syn(in = paren_token)]
    #[parse(CommaList::parse_terminated)]
    pub args: CommaList<syn::Expr>,
}

impl Atom {
    /// Bare variable arguments only. Expression dependencies are a separate
    /// question; callers must never use this iterator as an expression analysis.
    pub fn variables(&self) -> impl Iterator<Item = &Ident> {
        self.args.iter().filter_map(rust::variable)
    }
    pub fn is_normalized(&self) -> bool {
        self.variables().count() == self.args.len()
            && self
                .variables()
                .map(symbol_name)
                .collect::<BTreeSet<_>>()
                .len()
                == self.args.len()
    }
}

/// An embedded Rust condition. Parsing does not establish reorderability.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Filter {
    pub if_token: Token![if],
    pub condition: syn::Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Let {
    pub let_token: Token![let],
    #[parse(syn::Pat::parse_single)]
    pub pattern: syn::Pat,
    #[syn(optional, peek = Token![:])]
    pub annotation: Option<TypeAnnotation>,
    pub eq_token: Token![=],
    pub expression: syn::Expr,
}

impl Let {
    /// Written binding types, including structurally annotated tuple fields.
    /// Unannotated or nonstructural patterns retain Rust type inference.
    pub fn binding_types(&self) -> Vec<(Ident, syn::Type)> {
        fn assign(pattern: &syn::Pat, ty: &syn::Type, output: &mut [(Ident, syn::Type)]) {
            let ty = match ty {
                syn::Type::Paren(group) => &*group.elem,
                syn::Type::Group(group) => &*group.elem,
                ty => ty,
            };
            if matches!(ty, syn::Type::Paren(_) | syn::Type::Group(_)) {
                assign(pattern, ty, output);
                return;
            }
            match pattern {
                syn::Pat::Paren(group) => assign(&group.pat, ty, output),
                syn::Pat::Ident(binding) => {
                    if let Some((_, inferred)) = output
                        .iter_mut()
                        .find(|(name, _)| symbol_name(name) == symbol_name(&binding.ident))
                    {
                        *inferred =
                            if binding.by_ref.is_some() && !matches!(ty, syn::Type::Infer(_)) {
                                if binding.mutability.is_some() {
                                    syn::parse_quote!(&mut #ty)
                                } else {
                                    syn::parse_quote!(& #ty)
                                }
                            } else {
                                ty.clone()
                            };
                    }
                    if let Some((_, inner)) = &binding.subpat {
                        assign(inner, ty, output);
                    }
                }
                syn::Pat::Tuple(pattern) => {
                    let syn::Type::Tuple(tuple) = ty else {
                        // Named types and reference match ergonomics require
                        // Rust type resolution; do not invent field types.
                        return;
                    };
                    let rest = pattern
                        .elems
                        .iter()
                        .position(|p| matches!(p, syn::Pat::Rest(_)));
                    if let Some(rest) = rest {
                        if tuple.elems.len() + 1 < pattern.elems.len() {
                            return;
                        }
                        for (pattern, ty) in pattern.elems.iter().take(rest).zip(&tuple.elems) {
                            assign(pattern, ty, output);
                        }
                        let trailing = pattern.elems.len() - rest - 1;
                        for (pattern, ty) in pattern
                            .elems
                            .iter()
                            .rev()
                            .take(trailing)
                            .zip(tuple.elems.iter().rev())
                        {
                            assign(pattern, ty, output);
                        }
                    } else if pattern.elems.len() == tuple.elems.len() {
                        for (pattern, ty) in pattern.elems.iter().zip(&tuple.elems) {
                            assign(pattern, ty, output);
                        }
                    }
                }
                // Reference and constructor patterns are left to Rust; tuple
                // propagation never guesses a default binding mode.
                _ => {}
            }
        }
        let mut vars: Vec<_> = rust::bindings(&self.pattern)
            .into_iter()
            .map(|id| (id, syn::parse_quote!(_)))
            .collect();
        if let Some(annotation) = &self.annotation {
            assign(&self.pattern, &annotation.ty, &mut vars);
        }
        vars
    }
}

/// The optional Rust type ascription on a CQ let binding.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct TypeAnnotation {
    pub colon_token: Token![:],
    pub ty: Type,
}

/// Formal rule IF-Input: Check declaration names and arity; preserve layout order.
pub fn check_input_declarations<'a>(
    inputs: impl IntoIterator<Item = &'a RelationDecl>,
) -> syn::Result<BTreeMap<String, &'a RelationDecl>> {
    let mut schema = BTreeMap::new();
    for input in inputs {
        if input.columns.is_empty() {
            return Err(syn::Error::new_spanned(
                input,
                format!("input relation `{}` must have positive arity", input.name),
            ));
        }

        let mut column_names = BTreeSet::new();
        for column in &input.columns {
            let name = symbol_name(&column.name);
            if !column_names.insert(name.clone()) {
                return Err(syn::Error::new_spanned(
                    &column.name,
                    format!(
                        "input relation `{}` declares column `{name}` more than once",
                        input.name
                    ),
                ));
            }
        }

        if schema.insert(symbol_name(&input.name), input).is_some() {
            return Err(syn::Error::new_spanned(
                &input.name,
                format!("input relation `{}` is declared more than once", input.name),
            ));
        }
    }
    Ok(schema)
}

pub mod algebra;

/// Parse declarations while the next token starts a relation declaration.
pub fn parse_declarations(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<RelationDecl>> {
    let mut declarations = Vec::new();
    while input.peek(kw::relation) {
        declarations.push(input.parse()?);
    }
    Ok(declarations)
}

pub mod equality;
pub use equality::{Equality, EqualityValue};
