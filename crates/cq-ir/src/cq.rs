mod predicate;
pub use predicate::{Comparison, Predicate, Term};
pub mod rust;

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::TokenStream;
use quote::{ToTokens, TokenStreamExt};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token, Type, Visibility, token};

use crate::{CommaList, InputList, kw, symbol_name};

/// A logical query paired with its program declaration and input schema.
///
/// ```text
/// Program ::= RustVisibility? "struct" Ident ";"
///             RelationDecl+
///             Query
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub visibility: Visibility,
    pub struct_token: Token![struct],
    pub name: Ident,
    pub declaration_semi: Token![;],
    pub inputs: InputList,
    /// Optional declaration of the head relation's Rust column types.
    pub output: Option<RelationDecl>,
    pub query: Query,
}

impl Parse for Program {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let visibility = input.parse()?;
        let struct_token = input.parse()?;
        let name = input.parse()?;
        let declaration_semi = input.parse()?;

        let mut inputs = Vec::new();
        while input.peek(kw::relation) {
            inputs.push(input.parse()?);
        }
        if inputs.is_empty() {
            return Err(input.error("CQ program must declare at least one input relation"));
        }

        let query: Query = input.parse()?;
        let output = inputs
            .iter()
            .position(|d: &RelationDecl| symbol_name(&d.name) == symbol_name(&query.head.relation))
            .map(|i| inputs.remove(i));
        Ok(Self {
            visibility,
            struct_token,
            name,
            declaration_semi,
            inputs,
            output,
            query,
        })
    }
}

impl ToTokens for Program {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.visibility.to_tokens(tokens);
        self.struct_token.to_tokens(tokens);
        self.name.to_tokens(tokens);
        self.declaration_semi.to_tokens(tokens);
        tokens.append_all(&self.inputs);
        self.output.to_tokens(tokens);
        self.query.to_tokens(tokens);
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
/// ```text
/// RelationDecl ::= "relation" Ident "(" [ColumnDecl ("," ColumnDecl)* [","]] ")" ";"
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
    pub semi_token: Token![;],
}

impl RelationDecl {
    /// The relation arity derived from its declared columns.
    pub fn arity(&self) -> usize {
        self.columns.len()
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

/// One positive conjunctive query, with a nonempty syntactic body.
///
/// ```text
/// Query ::= Atom ":-" BodyItem ("," BodyItem)* ("." | ";")
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Query {
    pub head: Atom,
    pub colon_token: Token![:],
    pub minus_token: Token![-],
    #[parse(CommaList::parse_separated_nonempty)]
    pub body: CommaList<BodyItem>,
    pub end: RuleEnd,
}

/// Semicolon permits an unparenthesized Rust expression at the end of a rule
/// without confusing `1.` (a Rust float literal) with the DSL's legacy period.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum RuleEnd {
    #[parse(peek = Token![.])]
    Dot(Token![.]),
    #[parse(peek = Token![;])]
    Semi(Token![;]),
}

/// One source-ordered logical body clause.
///
/// Expressions retain Rust syntax. Whether a condition can move is a planner
/// premise, not a parser restriction.
///
/// ```text
/// BodyItem ::= Atom | "if" RustExpr | "let" RustPat "=" RustExpr
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum BodyItem {
    #[parse(peek = Token![if])]
    Filter(Filter),
    #[parse(peek = Token![let])]
    Let(Let),
    // OPTIONAL HW4 — STUDENT 1/6 (IR): add the more-specific Ascent-style
    // Negation and Aggregate variants *before* this positive-atom fallback,
    // reusing Atom and its Rust expression arguments. `agg` is
    // also an identifier token, so enum parse order matters. Update this
    // file's BodyItem/Query/Module EBNF and examples with the grammar. Follow
    // doc/HW4-OPTIONAL.md exactly.
    #[parse(peek = Ident)]
    Positive { atom: Atom },
}

/// An embedded Rust condition. Parsing does not establish reorderability.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Filter {
    pub if_token: Token![if],
    #[parse(rust::parse_expression)]
    pub condition: syn::Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Let {
    pub let_token: Token![let],
    #[parse(rust::parse_binding_pattern)]
    pub pattern: syn::Pat,
    pub eq_token: Token![=],
    #[parse(rust::parse_expression)]
    pub expression: syn::Expr,
}

impl BodyItem {
    pub fn atom(&self) -> Option<&Atom> {
        match self {
            Self::Positive { atom } => Some(atom),
            Self::Filter(_) | Self::Let(_) => None,
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
/// triangle(x, y, z) :- R(x, y), S(y, z), T(z, x).
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
    let schema = check_input_declarations(&program.inputs)?;
    if let Some(output) = &program.output {
        check_input_declarations(std::slice::from_ref(output))?;
        if symbol_name(&output.name) != symbol_name(&program.query.head.relation) {
            return Err(syn::Error::new_spanned(
                output,
                "result declaration must name the query head relation",
            ));
        }
        if output.arity() != program.query.head.args.len() {
            return Err(syn::Error::new_spanned(
                output,
                "result declaration and head have different arities",
            ));
        }
    }
    check_query(&schema, &program.query, program.output.is_some())?;
    if program.output.is_none() {
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
                    rust::binding_types(&binding.pattern)
                        .into_iter()
                        .map(|(id, ty)| (symbol_name(&id), ty)),
                ),
                BodyItem::Filter(_) => (),
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

pub(crate) fn check_input_declarations(
    inputs: &[RelationDecl],
) -> syn::Result<BTreeMap<String, &RelationDecl>> {
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
            BodyItem::Filter(_) => Vec::new(),
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
