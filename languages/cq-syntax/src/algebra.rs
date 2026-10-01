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
/// Rename ::= "rename" RelationName "{"
///              [AttributeRename ("," AttributeRename)* [","]]
///            "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Rename {
    pub rename_token: kw::rename,
    pub relation: Ident,
    #[syn(braced)]
    pub mapping_brace: token::Brace,
    #[syn(in = mapping_brace)]
    #[parse(CommaList::parse_terminated)]
    pub mapping: CommaList<AttributeRename>,
}

/// A conjunction of relational equalities. Rust guards use [`Guard`].
/// Commas mean conjunction: `select r if (x = 5, y = z)`.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Theta {
    #[parse(CommaList::parse_terminated)]
    pub equalities: CommaList<crate::Equality>,
}

impl Theta {
    /// All columns read by this relational predicate.
    pub fn columns(&self) -> std::collections::BTreeSet<String> {
        self.equalities
            .iter()
            .flat_map(crate::Equality::variables)
            .map(crate::symbol_name)
            .collect()
    }

    /// Compile-time point key, when every conjunct equates a column to a literal.
    /// Duplicate constraints retain one probe value; selection still checks all
    /// conjuncts, including inconsistent literals on the same column.
    pub fn constant_keys(&self) -> Option<std::collections::BTreeMap<String, syn::Expr>> {
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

/// Diagnose syntactically known non-setoid columns. Aliases and user types are
/// checked by generated Rust `Eq + Hash`/`Ord` bounds.
pub fn check_setoid_type(ty: &syn::Type, column: &Ident) -> syn::Result<()> {
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
                "column `{column}` requires Eq + Hash or Ord for relational equality, keys or deduplication; floating-point types such as f64 provide only PartialEq"
            ),
        ))
    } else {
        Ok(())
    }
}
