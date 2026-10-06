//! Operator details shared by relational and physical programs.
use crate::CommaList;
use syn::{Ident, Token, token};
pub mod kw {
    syn::custom_keyword!(keep);
    syn::custom_keyword!(project);
    syn::custom_keyword!(rename);
    syn::custom_keyword!(unit);
    syn::custom_keyword!(select);
    syn::custom_keyword!(with);
    syn::custom_keyword!(bind);
    syn::custom_keyword!(guard);
}
/// The singleton zero-column relation used to seed a global logical clause.
///
/// ```text
/// Unit ::= "unit"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Unit {
    pub unit_token: kw::unit,
}

/// One source-attribute to query-variable rename.
///
/// ```text
/// AttributeRename ::= Ident "->" Ident
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct AttributeRename {
    pub source: Ident,
    pub minus_token: Token![-],
    pub gt_token: Token![>],
    pub target: Ident,
}

/// A complete one-to-one rename of one declared relation occurrence.
///
/// ```text
/// Rename ::= "rename" RelationName "@" Int "{"
///              [AttributeRename ("," AttributeRename)* [","]]
///            "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Rename {
    pub rename_token: kw::rename,
    pub relation: Ident,
    pub at: Token![@],
    pub occurrence: syn::LitInt,
    #[syn(braced)]
    pub mapping_brace: token::Brace,
    #[syn(in = mapping_brace)]
    #[parse(CommaList::parse_terminated)]
    pub mapping: CommaList<AttributeRename>,
}

/// A conjunction of central relational constraints, including boolean
/// disjunctions. Rust guards use [`Guard`]. Commas mean conjunction:
/// `select r if (x >= @lower, x < @upper)`. Captures are explicit in IR.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Theta {
    pub equalities: CommaList<crate::Equality>,
    pub constraints: CommaList<crate::Constraint>,
}

impl syn::parse::Parse for Theta {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut theta = Self::default();
        while !input.is_empty() {
            let c: crate::Constraint = input.parse()?;
            if let Some(e) = c.equality() {
                theta.equalities.push(e);
            } else {
                theta.constraints.push(c);
            }
            if input.is_empty() {
                break;
            }
            if !input.peek(Token![,]) {
                return Err(crate::constraint::unexpected(
                    input,
                    "`,` between relational constraints",
                ));
            }
            input.parse::<Token![,]>()?;
        }
        Ok(theta)
    }
}

impl quote::ToTokens for Theta {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        use quote::quote;
        for (i, c) in self
            .equalities
            .iter()
            .map(quote::ToTokens::to_token_stream)
            .chain(
                self.constraints
                    .iter()
                    .map(quote::ToTokens::to_token_stream),
            )
            .enumerate()
        {
            if i != 0 {
                tokens.extend(quote!(,));
            }
            tokens.extend(c);
        }
    }
}

impl Theta {
    /// All columns read by this relational predicate.
    pub fn columns(&self) -> std::collections::BTreeSet<String> {
        self.equalities
            .iter()
            .flat_map(crate::Equality::variables)
            .map(crate::symbol_name)
            .chain(
                self.constraints
                    .iter()
                    .flat_map(crate::Constraint::variables)
                    .map(crate::symbol_name),
            )
            .collect()
    }

    /// Compile-time point key, when every conjunct equates a column to a literal.
    /// Duplicate constraints retain one probe value; selection still checks all
    /// conjuncts, including inconsistent literals on the same column.
    pub fn constant_keys(&self) -> Option<std::collections::BTreeMap<String, syn::Expr>> {
        if !self.constraints.is_empty() {
            return None;
        }
        if self.equalities.is_empty() {
            return None;
        }
        self.equalities
            .iter()
            .map(|e| match &e.right {
                crate::EqualityValue::Literal(value) => {
                    Some((crate::symbol_name(&e.left), value.clone()))
                }
                crate::EqualityValue::Variable(_) => None,
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.equalities.is_empty() && self.constraints.is_empty()
    }

    pub fn check(&self) -> syn::Result<()> {
        for e in &self.equalities {
            e.check()?;
        }
        for c in &self.constraints {
            c.check()?;
        }
        if self.is_empty() {
            return Err(syn::Error::new_spanned(
                self,
                "selection requires a relational constraint",
            ));
        }
        Ok(())
    }

    /// Point demands include literal membership and captured constant keys.
    /// Multi-point membership can still be realized by a scan and filter.
    pub fn point_columns(&self) -> Option<std::collections::BTreeSet<String>> {
        use crate::constraint::{Comparison, Constraint, Term};
        if self.is_empty() {
            return None;
        }
        let mut columns = std::collections::BTreeSet::new();
        for e in &self.equalities {
            if !matches!(e.right, crate::EqualityValue::Literal(_)) {
                return None;
            }
            columns.insert(crate::symbol_name(&e.left));
        }
        for c in &self.constraints {
            let variable = match c {
                Constraint::Compare {
                    left: Term::Variable(v),
                    comparison: Comparison::Eq(_),
                    right,
                } if right.constant() => v,
                Constraint::Compare {
                    left,
                    comparison: Comparison::Eq(_),
                    right: Term::Variable(v),
                } if left.constant() => v,
                Constraint::In {
                    term: Term::Variable(v),
                    ..
                } => v,
                _ => return None,
            };
            columns.insert(crate::symbol_name(variable));
        }
        (!columns.is_empty()).then_some(columns)
    }

    /// A conjunction of constant order bounds on exactly one logical column.
    /// Bounds are retained separately; the printer intersects them with Ord.
    pub fn range_bounds(&self) -> Option<(String, Vec<RangeBound>)> {
        use crate::constraint::{Comparison, Constraint, Term};
        if !self.equalities.is_empty() || self.constraints.is_empty() {
            return None;
        }
        let mut column = None;
        let mut bounds = Vec::new();
        for c in &self.constraints {
            let (v, entries) = match c {
                Constraint::Between {
                    term: Term::Variable(v),
                    low,
                    high,
                } if low.constant() && high.constant() => (
                    v,
                    vec![
                        RangeBound {
                            value: low.clone(),
                            lower: true,
                            inclusive: true,
                        },
                        RangeBound {
                            value: high.clone(),
                            lower: false,
                            inclusive: true,
                        },
                    ],
                ),
                Constraint::Compare {
                    left: Term::Variable(v),
                    comparison,
                    right,
                } if comparison.ordered() && right.constant() => (
                    v,
                    vec![RangeBound {
                        value: right.clone(),
                        lower: matches!(comparison, Comparison::Gt(_) | Comparison::Ge(_)),
                        inclusive: matches!(comparison, Comparison::Le(_) | Comparison::Ge(_)),
                    }],
                ),
                Constraint::Compare {
                    left,
                    comparison,
                    right: Term::Variable(v),
                } if comparison.ordered() && left.constant() => (
                    v,
                    vec![RangeBound {
                        value: left.clone(),
                        lower: matches!(comparison, Comparison::Lt(_) | Comparison::Le(_)),
                        inclusive: matches!(comparison, Comparison::Le(_) | Comparison::Ge(_)),
                    }],
                ),
                _ => return None,
            };
            let name = crate::symbol_name(v);
            if column.as_ref().is_some_and(|old| old != &name) {
                return None;
            }
            column = Some(name);
            bounds.extend(entries);
        }
        Some((column?, bounds))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeBound {
    pub value: crate::constraint::Term,
    pub lower: bool,
    pub inclusive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Select {
    pub select_token: kw::select,
    pub input: Ident,
    pub if_token: Token![if],
    #[syn(parenthesized)]
    pub condition_paren: token::Paren,
    #[syn(in = condition_paren)]
    pub condition: Theta,
}

/// Opaque partial Rust computation at its position in the ancestry chain.
/// Its meaning is a function of free variables to a value or whole-query abort.
/// I/O, mutation and evaluation counts are outside this meaning.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Bind {
    pub bind_token: kw::bind,
    pub input: Ident,
    pub with_token: kw::with,
    #[parse(syn::Pat::parse_single)]
    pub pattern: syn::Pat,
    #[syn(optional, peek = Token![:])]
    pub annotation: Option<crate::TypeAnnotation>,
    pub eq_token: Token![=],
    #[syn(parenthesized)]
    pub expression_paren: token::Paren,
    #[syn(in = expression_paren)]
    pub expression: syn::Expr,
}

impl Bind {
    pub fn binding(&self) -> crate::Let {
        crate::Let {
            let_token: Default::default(),
            pattern: self.pattern.clone(),
            annotation: self.annotation.clone(),
            eq_token: self.eq_token,
            expression: self.expression.clone(),
        }
    }

    pub fn binding_types(&self) -> Vec<(Ident, syn::Type)> {
        self.binding().binding_types()
    }
}

/// Opaque partial Rust predicate at its position in the ancestry chain.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Guard {
    pub guard_token: kw::guard,
    pub input: Ident,
    pub if_token: Token![if],
    #[syn(parenthesized)]
    pub condition_paren: token::Paren,
    #[syn(in = condition_paren)]
    pub condition: syn::Expr,
}

/// Set projection of an earlier result to a distinct subset of attributes.
///
/// The attribute braces denote a set. Ordered output tuple layout belongs to
/// the separate output metadata.
///
/// ```text
/// Project ::= "project" RelationId "keep" "{"
///               [Ident ("," Ident)* [","]]
///             "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Project {
    pub project_token: kw::project,
    pub input: Ident,
    pub keep_token: kw::keep,
    #[syn(braced)]
    pub attributes_brace: token::Brace,
    #[syn(in = attributes_brace)]
    #[parse(CommaList::parse_terminated)]
    pub attributes: CommaList<Ident>,
}

/// Diagnose syntactically known failures of Leibniz admissibility. Generated
/// Rust checks necessary `Eq + Hash`/`Ord` bounds for aliases and user types;
/// their semantic admissibility remains a caller premise.
pub fn check_leibniz_type(ty: &syn::Type, column: &Ident) -> syn::Result<()> {
    fn contains_float(ty: &syn::Type) -> bool {
        match ty {
            syn::Type::Path(path) => {
                let segments = &path.path.segments;
                path.qself.is_none()
                    && ((segments.len() == 1
                        && matches!(segments[0].ident.to_string().as_str(), "f32" | "f64"))
                        || (segments.len() == 3
                            && (segments[0].ident == "core" || segments[0].ident == "std")
                            && segments[1].ident == "primitive"
                            && matches!(segments[2].ident.to_string().as_str(), "f32" | "f64")))
            }
            syn::Type::Reference(ty) => contains_float(&ty.elem),
            syn::Type::Tuple(ty) => ty.elems.iter().any(contains_float),
            syn::Type::Array(ty) => contains_float(&ty.elem),
            syn::Type::Slice(ty) => contains_float(&ty.elem),
            syn::Type::Paren(ty) => contains_float(&ty.elem),
            syn::Type::Group(ty) => contains_float(&ty.elem),
            _ => false,
        }
    }
    if contains_float(ty) {
        Err(syn::Error::new_spanned(
            ty,
            format!(
                "column `{column}` requires Eq + Hash or Ord for relational storage and must satisfy this premise. Leibniz admissibility of a column type: a type may be compared, joined, grouped, keyed or deduplicated by the relational centre only if its `==` decides Leibniz equality (Rust code cannot distinguish `==`-equal values). Holds for integers, bool, char, String/&str, the cq-load Date/Decimal, tuples and derived-Eq structs of admissible types; fails for f64/f32 (0.0 == -0.0) and hand-written lenient Eq."
            ),
        ))
    } else {
        Ok(())
    }
}

/// Necessary syntactic checks; rustc checks Ord for aliases and user types.
/// Formal rule ORD-Admissible: Order is total and consistent with Leibniz identity.
pub fn check_ord_type(ty: &syn::Type, column: &Ident) -> syn::Result<()> {
    check_leibniz_type(ty, column)?;
    if matches!(ty, syn::Type::BareFn(_) | syn::Type::TraitObject(_)) {
        return Err(syn::Error::new_spanned(
            ty,
            format!(
                "column `{column}` requires Ord: a total order consistent with Leibniz equality"
            ),
        ));
    }
    Ok(())
}
