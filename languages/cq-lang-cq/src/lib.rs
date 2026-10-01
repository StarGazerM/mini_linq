//! Complete CQ programs with denotation `T X = bottom + Set X`.
//! The reference evaluates the desugared body in written order; bottom is one
//! undistinguished panic aborting the evaluation. Rust lets and guards denote
//! opaque partial functions of their free variables and host captures. I/O,
//! mutation, counters and evaluation counts are outside this meaning.
//! Relational steps commute and may move earlier across a Rust step. Rust
//! steps keep their relative order and follow every free-variable binder.
//!
//! Relational constraints use `t = t`, `t != t`, `t < t`, `t <= t`, `t > t`,
//! `t >= t`, inclusive `t between t and t`, or `t in { literal, ... }`.
//! Conjunction is separate body items; `(C and C) or (C and C)` is a
//! disjunction of parenthesized atomic conjunctions. Terms are a query
//! variable, a Syn literal (or negative numeric literal), or a bare enclosing
//! identifier not among the query's binders. Other Rust expressions are rejected.
//! Captured parameters are borrowed once per evaluation outside query loops.
//! `if e` is always an opaque Rust guard, even when e looks like a comparison.
//! Equality is Leibniz identity, decided by admissible column `==`.
//! Order requires a total `Ord` consistent with that identity. Constraints are
//! total and central: they commute with relational steps and may move earlier
//! across Rust steps. Rust checks Eq/Ord for aliases and user types; semantic
//! admissibility is a trusted caller premise. f32/f64 are inadmissible.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

pub use cq_syntax::predicate::{Comparison, Predicate, Term};
pub mod hypergraph;
pub mod normal;
pub use cq_syntax::Constraint;
pub use cq_syntax::rust;
pub use cq_syntax::{Equality, EqualityValue};

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
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Query {
    pub head: Atom,
    pub colon_token: Token![:],
    pub minus_token: Token![-],
    pub body: CommaList<BodyItem>,
    pub semi_token: Token![;],
}

impl syn::parse::Parse for Query {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut query = Self {
            head: input.parse()?,
            colon_token: input.parse()?,
            minus_token: input.parse()?,
            body: CommaList::parse_separated_nonempty(input)?,
            semi_token: input.parse()?,
        };
        // Scope is determined by binders, never by identifiers in predicates.
        // A later query binder is still a query variable and must pass the
        // written-scope check; an enclosing value is a captured parameter.
        let logical = query
            .body
            .iter()
            .flat_map(|item| match item {
                BodyItem::Positive { atom } => atom.variables().cloned().collect(),
                BodyItem::Let(binding) => rust::bindings(&binding.pattern),
                _ => Vec::new(),
            })
            .map(|v| symbol_name(&v))
            .collect();
        for item in &mut query.body {
            let mut constraint = match item {
                BodyItem::Equality(e) => Constraint::from_equality(e),
                BodyItem::Constraint(c) => c.clone(),
                _ => continue,
            };
            constraint.resolve(&logical);
            *item = match constraint.equality() {
                Some(e) => BodyItem::Equality(e),
                None => BodyItem::Constraint(constraint),
            };
        }
        Ok(query)
    }
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
/// Rust expressions retain their syntax as opaque partial functions. Lets
/// and guards share one written chain; relational steps alone may move earlier
/// across that chain under the sufficient refinement check.
///
/// ```text
/// BodyItem ::= Atom | Constraint
///            | "if" RustExpr | "let" RustPat [":" RustType] "=" RustExpr
///            | Ident "!" "{" TokenTree* "}"
///            | "!" Atom | "agg" RustPat "=" Aggregator "(" [Ident ("," Ident)* [","]] ")" "in" Atom
/// ```
#[derive(Clone, Debug)]
pub enum BodyItem {
    Equality(Equality),
    Constraint(Constraint),
    Filter(Filter),
    Let(Box<Let>),
    Negation(Negation),
    Aggregate(Box<Aggregate>),
    /// Opaque extension text; an enclosing extension must expand it before CQ checking.
    Extension {
        name: Ident,
        tokens: proc_macro2::TokenStream,
    },
    Positive {
        atom: Atom,
    },
}

impl syn::parse::Parse for BodyItem {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let item = if input.peek(Token![if]) {
            Self::Filter(input.parse()?)
        } else if input.peek(Token![let]) {
            Self::Let(input.parse()?)
        } else if input.peek(Token![!]) {
            Self::Negation(input.parse()?)
        } else if input.peek(kw::agg) && !input.peek2(Token![!]) {
            Self::Aggregate(input.parse()?)
        } else if input.peek(Ident) && input.peek2(Token![!]) && !input.peek2(Token![!=]) {
            let name = input.parse()?;
            input.parse::<Token![!]>()?;
            let body;
            syn::braced!(body in input);
            Self::Extension {
                name,
                tokens: body.parse()?,
            }
        } else if input.peek(Ident) && input.peek2(token::Paren) {
            Self::Positive {
                atom: input.parse()?,
            }
        } else {
            let constraint: Constraint = input.parse()?;
            if let Some(parameter) = constraint.parameters().next() {
                return Err(syn::Error::new_spanned(
                    parameter,
                    "unexpected token `@`: CQ parameters are bare identifiers",
                ));
            }
            match constraint.equality() {
                Some(e) => Self::Equality(e),
                None => Self::Constraint(constraint),
            }
        };
        if !input.is_empty() && !input.peek(Token![,]) && !input.peek(Token![;]) {
            return Err(cq_syntax::constraint::unexpected(
                input,
                "`,` or `;` after a CQ body item",
            ));
        }
        Ok(item)
    }
}

impl quote::ToTokens for BodyItem {
    fn to_tokens(&self, output: &mut proc_macro2::TokenStream) {
        match self {
            Self::Equality(x) => x.to_tokens(output),
            Self::Constraint(x) => output.extend(x.cq_tokens()),
            Self::Filter(x) => x.to_tokens(output),
            Self::Let(x) => x.to_tokens(output),
            Self::Negation(x) => x.to_tokens(output),
            Self::Aggregate(x) => x.to_tokens(output),
            Self::Positive { atom } => atom.to_tokens(output),
            Self::Extension { name, tokens } => output.extend(quote::quote!(#name! { #tokens })),
        }
    }
}

impl PartialEq for BodyItem {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Equality(a), Self::Equality(b)) => a == b,
            (Self::Constraint(a), Self::Constraint(b)) => a == b,
            (Self::Filter(a), Self::Filter(b)) => a == b,
            (Self::Let(a), Self::Let(b)) => a == b,
            (Self::Negation(a), Self::Negation(b)) => a == b,
            (Self::Aggregate(a), Self::Aggregate(b)) => a == b,
            (Self::Positive { atom: a }, Self::Positive { atom: b }) => a == b,
            (Self::Extension { name: a, tokens: x }, Self::Extension { name: b, tokens: y }) => {
                a == b && x.to_string() == y.to_string()
            }
            _ => false,
        }
    }
}
impl Eq for BodyItem {}

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

/// The complete compiler-visible description of an opaque Rust step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustStep {
    /// Includes host captures. Opaque macro syntax conservatively depends on
    /// its complete written query environment via `dependencies`.
    pub free_variables: rust::FreeVariables,
    pub bindings: Vec<Ident>,
}

impl BodyItem {
    pub fn rust_step(&self) -> Option<RustStep> {
        match self {
            Self::Filter(filter) => Some(RustStep {
                free_variables: rust::free_variables(&filter.condition),
                bindings: Vec::new(),
            }),
            Self::Let(binding) => Some(RustStep {
                free_variables: rust::free_variables(&binding.expression),
                bindings: rust::bindings(&binding.pattern),
            }),
            _ => None,
        }
    }

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
        output.check_result()?;
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
                BodyItem::Equality(_)
                | BodyItem::Constraint(_)
                | BodyItem::Filter(_)
                | BodyItem::Negation(_)
                | BodyItem::Aggregate(_)
                | BodyItem::Extension { .. } => (),
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
            BodyItem::Equality(_)
            | BodyItem::Constraint(_)
            | BodyItem::Filter(_)
            | BodyItem::Negation(_)
            | BodyItem::Aggregate(_)
            | BodyItem::Extension { .. } => Vec::new(),
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
                // Every bare argument of this atom is available to a same-atom
                // computed argument, regardless of its column position.
                let mut local = body_variables.clone();
                local.extend(atom.variables().map(symbol_name));
                for arg in &atom.args {
                    if rust::variable(arg).is_none() {
                        check_uses(arg, &local)?;
                    }
                }
                body_variables = local;
            }
            BodyItem::Equality(equality) => {
                equality.check()?;
                for variable in equality.variables() {
                    if !body_variables.contains(&symbol_name(variable)) {
                        return Err(syn::Error::new_spanned(
                            variable,
                            "relational equality uses a variable before it is bound",
                        ));
                    }
                }
            }
            // Formal rule CQ-Comparison: Constraint terms use bound query names or captured constants.
            BodyItem::Constraint(constraint) => {
                constraint.check()?;
                for variable in constraint.variables() {
                    if !body_variables.contains(&symbol_name(variable)) {
                        return Err(syn::Error::new_spanned(
                            variable,
                            "relational constraint uses a query variable before it is bound",
                        ));
                    }
                }
                if let Some(parameter) = constraint
                    .parameters()
                    .find(|p| logical.contains(&symbol_name(p)))
                {
                    return Err(syn::Error::new_spanned(
                        parameter,
                        "relational parameter names a query variable",
                    ));
                }
            }
            // Formal rule IF-Surface: No unexpanded extension enters the checked core.
            BodyItem::Extension { name, .. } => {
                return Err(syn::Error::new_spanned(
                    name,
                    format!("extension `{name}!` was not expanded"),
                ));
            }
            BodyItem::Negation(_) | BodyItem::Aggregate(_) => {
                return Err(syn::Error::new_spanned(
                    item,
                    "negation and aggregation require the extended CQ semantic contract",
                ));
            }
            // Formal rule IF-Environment: Check logical scope; Rust resolves enclosing values.
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

    for expression in &query.head.args {
        check_uses(expression, &body_variables)?;
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
