mod predicate;
pub use predicate::{Comparison, Predicate, Term};
pub mod normal;
pub mod rust;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, Type, Visibility, token};

use crate::{CommaList, kw, symbol_name};

/// A logical query paired with its program declaration and input schema.
///
/// ```text
/// Program ::= RustVisibility? "struct" Ident ";"
///             RelationDecl+
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
    let mut declarations = vec![input.parse()?];
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
/// An `#[index(…)]` attribute is advice for structure selection: whichever
/// structures a later pass builds over this relation use the Rust type it
/// names, with the operations it claims. It is not part of the query's
/// meaning and names no columns — which columns get indexed is a planner
/// decision the query cannot see. CQ checks only that the attribute is a
/// well-formed `index(…)` list; its argument belongs to the pass that reads it.
///
/// ```text
/// RelationDecl ::= ["#" "[" "index" "(" … ")" "]"]
///                  "relation" Ident "(" [ColumnDecl ("," ColumnDecl)* [","]] ")" ";"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct RelationDecl {
    #[parse(syn::Attribute::parse_outer)]
    #[to_tokens(|tokens, attrs| quote::TokenStreamExt::append_all(tokens, attrs))]
    pub attrs: Vec<syn::Attribute>,
    pub relation_token: kw::relation,
    pub name: Ident,
    #[syn(parenthesized)]
    pub columns_paren: token::Paren,
    #[syn(in = columns_paren)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<ColumnDecl>,
    pub semi_token: Token![;],
}

impl RelationDecl {
    /// The relation arity derived from its declared columns.
    pub fn arity(&self) -> usize {
        self.columns.len()
    }

    /// The structure advice attached to this relation, if any. Shape is the
    /// CQ contract's concern; the argument is read by the pass that consults
    /// it.
    pub fn index_advice(&self) -> syn::Result<Option<&syn::Attribute>> {
        let mut advice = None;
        for attr in &self.attrs {
            if !attr.path().is_ident("index") {
                return Err(syn::Error::new_spanned(
                    attr,
                    "relation declarations accept only `#[index(…)]`",
                ));
            }
            attr.meta.require_list()?;
            if advice.is_some() {
                return Err(syn::Error::new_spanned(
                    attr,
                    format!("relation `{}` advises an index twice", self.name),
                ));
            }
            advice = Some(attr);
        }
        Ok(advice)
    }
}

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
///            | "!" Atom | "agg" RustPat "=" Aggregator "(" [Ident ("," Ident)*] ")" "in" Atom
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
    /// Written binding types; unannotated bindings retain Rust type inference.
    pub fn binding_types(&self) -> Vec<(Ident, syn::Type)> {
        let mut vars: Vec<_> = rust::bindings(&self.pattern)
            .into_iter()
            .map(|id| (id, syn::parse_quote!(_)))
            .collect();
        if let Some(annotation) = &self.annotation {
            if vars.len() == 1 && matches!(self.pattern, syn::Pat::Ident(_)) {
                vars[0].1 = annotation.ty.clone();
            }
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
    pub fn positive_atom(&self) -> syn::Result<&Atom> {
        match self {
            Self::Positive { atom } => Ok(atom),
            _ => Err(syn::Error::new_spanned(
                self,
                "this R1 route supports positive atoms only; use the Stage I route for selections",
            )),
        }
    }
}

/// A complete positive conjunctive-query language object.
///
/// ```text
/// struct TriangleProgram;
/// relation R(src: i32, dst: i32);
/// relation S(src: i32, dst: i32);
/// relation T(src: i32, dst: i32);
/// triangle(x, y, z) :- R(x, y), S(y, z), T(z, x);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Module {
    pub program: Program,
}

/// The CQ language contract in diagnostic and predicate form.
pub mod contract {
    /// Check the CQ language contract and retain precise diagnostic spans.
    pub fn check(module: &super::Module) -> syn::Result<()> {
        super::check_program(&module.program)
    }

    /// Whether a parsed module satisfies the CQ language contract.
    pub fn well_formed(module: &super::Module) -> bool {
        check(module).is_ok()
    }
}

pub(crate) fn check_program(program: &Program) -> syn::Result<()> {
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

pub(crate) fn check_input_declarations<'a>(
    inputs: impl IntoIterator<Item = &'a RelationDecl>,
) -> syn::Result<BTreeMap<String, &'a RelationDecl>> {
    let mut schema = BTreeMap::new();
    for input in inputs {
        input.index_advice()?;
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
