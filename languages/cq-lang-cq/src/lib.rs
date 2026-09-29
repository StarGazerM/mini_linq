//! Complete CQ programs, local contracts, normalization and hypergraph analysis.

pub use cq_syntax::predicate::{Comparison, Predicate, Term};
pub mod hypergraph;
pub mod normal;
pub use cq_syntax::rust;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, Visibility, token};

pub use cq_syntax::{CommaList, kw, symbol_name};

/// A logical query paired with its program declaration and input schema.
///
/// ```text
/// Program ::= RustVisibility? "struct" Ident ";"
///             RelationDecl*
///             Query
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Program {
    pub visibility: Visibility,
    pub struct_token: Token![struct],
    pub name: Ident,
    pub declaration_semi: Token![;],
    #[parse(parse_declarations)]
    #[to_tokens(|tokens, declarations| quote::TokenStreamExt::append_all(tokens, declarations))]
    pub declarations: Vec<RelationDecl>,
    pub query: Query,
}

fn parse_declarations(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<RelationDecl>> {
    let mut declarations = Vec::new();
    while input.peek(Token![#]) || input.peek(kw::relation) {
        declarations.push(input.parse()?);
    }
    Ok(declarations)
}

impl Program {
    /// Input schemas in declaration order. Classifying the head is analysis,
    /// so parsing and token emission preserve every declaration where written.
    pub fn inputs(&self) -> impl Iterator<Item = &RelationDecl> + Clone {
        self.declarations.iter().filter(|declaration| {
            symbol_name(&declaration.name) != symbol_name(&self.query.head.relation)
        })
    }

    pub fn output(&self) -> Option<&RelationDecl> {
        self.declarations.iter().find(|declaration| {
            symbol_name(&declaration.name) == symbol_name(&self.query.head.relation)
        })
    }
}

pub use cq_syntax::{ColumnDecl, RelationDecl};

pub use cq_syntax::Atom;

/// One query, with a nonempty syntactic body.
///
/// ```text
/// Query ::= Atom ":-" BodyItem ("," BodyItem)* ";"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Query {
    pub head: Atom,
    pub colon_token: Token![:],
    pub minus_token: Token![-],
    #[parse(CommaList::parse_separated_nonempty)]
    pub body: CommaList<BodyItem>,
    pub semi_token: Token![;],
}

impl Query {
    /// The positive, distinct-variable fragment accepted by the basic route.
    pub fn is_basic(&self) -> bool {
        let bound: BTreeSet<_> = self
            .body
            .iter()
            .filter_map(BodyItem::atom)
            .flat_map(Atom::variables)
            .map(symbol_name)
            .collect();
        self.head.is_normalized()
            && self
                .head
                .variables()
                .all(|id| bound.contains(&symbol_name(id)))
            && self
                .body
                .iter()
                .all(|item| matches!(item, BodyItem::Positive { atom } if atom.is_normalized()))
    }
}

/// One source-ordered logical body clause.
///
/// Expressions retain Rust syntax. Whether a condition can move is a planner
/// premise, not a parser restriction.
///
/// ```text
/// BodyItem ::= Atom | "if" RustExpr | "let" RustPat [":" RustType] "=" RustExpr
///            | "!" Atom | "agg" RustPat "=" Aggregator "(" [Ident ("," Ident)* [","]] ")" "in" Atom
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum BodyItem {
    #[parse(peek = Token![if])]
    Filter(Filter),
    #[parse(peek = Token![let])]
    Let(Box<Let>),
    #[parse(peek = Token![!])]
    Negation(Negation),
    #[parse(peek = kw::agg)]
    Aggregate(Box<Aggregate>),
    #[parse(peek = Ident)]
    Positive { atom: Atom },
}

pub use cq_syntax::{Filter, Let, TypeAnnotation};

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Negation {
    pub not_token: Token![!],
    pub atom: Atom,
}

/// The names in `arguments` are aggregate-local binders, as in Ascent.
/// Rust expressions occur in the aggregator and the input atom's arguments.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Aggregate {
    pub agg_token: kw::agg,
    #[parse(syn::Pat::parse_multi)]
    pub pattern: syn::Pat,
    pub eq_token: Token![=],
    pub aggregator: Aggregator,
    #[syn(parenthesized)]
    pub arguments_paren: token::Paren,
    #[syn(in = arguments_paren)]
    #[parse(CommaList::parse_terminated)]
    pub arguments: CommaList<Ident>,
    pub in_token: Token![in],
    pub atom: Atom,
}

/// `sum(value)` or `(make_aggregator(config))(value)`.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Aggregator {
    #[parse(peek = token::Paren)]
    Expression {
        #[syn(parenthesized)]
        paren_token: token::Paren,
        #[syn(in = paren_token)]
        expression: syn::Expr,
    },
    Path {
        path: syn::Path,
    },
}

impl BodyItem {
    pub fn atom(&self) -> Option<&Atom> {
        match self {
            Self::Positive { atom } => Some(atom),
            _ => None,
        }
    }
}

/// The CQ contract checks declarations, scope, and output types.
pub mod contract {
    pub fn check(program: &super::Program) -> syn::Result<()> {
        super::check_program(program)
    }
}

/// Formal rule WF-CQ: Check CQ declarations, positive scope, output arity and symbolic type availability.
fn check_program(program: &Program) -> syn::Result<()> {
    let mut schema = check_input_declarations(&program.declarations)?;
    if let Some(output) = schema.remove(&symbol_name(&program.query.head.relation)) {
        if output.arity() != program.query.head.args.len() {
            return Err(syn::Error::new_spanned(
                output,
                "result declaration and head have different arities",
            ));
        }
    }
    check_query(&schema, &program.query, program.output().is_some())?;
    if program.output().is_none() {
        let mut types = BTreeMap::new();
        for item in &program.query.body {
            match item {
                BodyItem::Positive { atom } => {
                    let declaration = schema[&symbol_name(&atom.relation)];
                    for (arg, col) in atom.args.iter().zip(&declaration.columns) {
                        if let Some(id) = rust::variable(arg) {
                            types
                                .entry(symbol_name(id))
                                .or_insert_with(|| col.ty.clone());
                        }
                    }
                }
                BodyItem::Let(binding) => types.extend(
                    binding
                        .binding_types()
                        .into_iter()
                        .map(|(id, ty)| (symbol_name(&id), ty)),
                ),
                BodyItem::Filter(_) | BodyItem::Negation(_) | BodyItem::Aggregate(_) => (),
            }
        }
        for arg in &program.query.head.args {
            if !rust::variable(arg)
                .and_then(|id| types.get(&symbol_name(id)))
                .is_some_and(|ty| !matches!(ty, syn::Type::Infer(_)))
            {
                return Err(syn::Error::new_spanned(
                    arg,
                    "declare the result relation's Rust column types for a computed head (or annotate the let binding's type)",
                ));
            }
        }
    }
    Ok(())
}

pub use cq_syntax::check_input_declarations;

fn check_query(
    schema: &BTreeMap<String, &RelationDecl>,
    query: &Query,
    declared_output: bool,
) -> syn::Result<()> {
    check_result_atom(schema, &query.head)?;
    let logical: BTreeSet<_> = query
        .body
        .iter()
        .flat_map(|item| match item {
            BodyItem::Positive { atom } => atom.variables().cloned().collect(),
            BodyItem::Let(binding) => rust::bindings(&binding.pattern),
            BodyItem::Filter(_) | BodyItem::Negation(_) | BodyItem::Aggregate(_) => Vec::new(),
        })
        .map(|id| symbol_name(&id))
        .collect();
    let check_uses = |expr: &syn::Expr, bound: &BTreeSet<String>| -> syn::Result<()> {
        if let Some(name) = rust::known_uses(expr, &logical).difference(bound).next() {
            return Err(syn::Error::new_spanned(
                expr,
                format!("logical variable `{name}` is used before it is bound"),
            ));
        }
        Ok(())
    };
    let mut body_variables = BTreeSet::new();
    for item in &query.body {
        // OPTIONAL HW4 — STUDENT 2/6 (WF): add Negation and Aggregate cases.
        // Negation requires named terms already bound and binds nothing. Sum
        // keeps its value local, requires correlated terms already bound, and
        // then binds one fresh result.
        match item {
            BodyItem::Positive { atom } => {
                check_input_atom(schema, atom)?;
                for arg in &atom.args {
                    if let Some(id) = rust::variable(arg) {
                        body_variables.insert(symbol_name(id));
                    } else {
                        check_uses(arg, &body_variables)?;
                    }
                }
            }
            BodyItem::Negation(_) | BodyItem::Aggregate(_) => {
                return Err(syn::Error::new_spanned(
                    item,
                    "negation and aggregation require the extended CQ semantic contract",
                ));
            }
            BodyItem::Filter(filter) => check_uses(&filter.condition, &body_variables)?,
            BodyItem::Let(binding) => {
                check_uses(&binding.expression, &body_variables)?;
                for variable in rust::bindings(&binding.pattern) {
                    if !body_variables.insert(symbol_name(&variable)) {
                        return Err(syn::Error::new_spanned(
                            variable,
                            "let binding shadows an existing logical variable",
                        ));
                    }
                }
            }
        }
    }

    for variable in query.head.variables() {
        if !declared_output && !body_variables.contains(&symbol_name(variable)) {
            return Err(syn::Error::new_spanned(
                variable,
                format!("result variable `{variable}` is not bound by the CQ body"),
            ));
        }
    }
    if !query.body.iter().any(|item| item.atom().is_some()) {
        return Err(syn::Error::new_spanned(
            query,
            "CQ requires a relational occurrence",
        ));
    }
    Ok(())
}

fn check_result_atom(schema: &BTreeMap<String, &RelationDecl>, head: &Atom) -> syn::Result<()> {
    if schema.contains_key(&symbol_name(&head.relation)) {
        return Err(syn::Error::new_spanned(
            &head.relation,
            format!(
                "result relation `{}` must not also be declared as an input",
                head.relation
            ),
        ));
    }
    if head.args.is_empty() {
        return Err(syn::Error::new_spanned(
            &head.args,
            "result atom must have positive arity",
        ));
    }
    Ok(())
}

fn check_input_atom<'a>(
    schema: &'a BTreeMap<String, &'a RelationDecl>,
    atom: &Atom,
) -> syn::Result<&'a RelationDecl> {
    let Some(input) = schema.get(&symbol_name(&atom.relation)).copied() else {
        return Err(syn::Error::new_spanned(
            &atom.relation,
            format!(
                "body relation `{}` is not declared as an input relation",
                atom.relation
            ),
        ));
    };
    if atom.args.len() != input.arity() {
        return Err(syn::Error::new_spanned(
            atom,
            format!(
                "body relation `{}` has arity {}; declared arity is {}",
                atom.relation,
                atom.args.len(),
                input.arity()
            ),
        ));
    }
    Ok(input)
}

/// Shared syntax used by this language.
pub use cq_syntax as syntax;
