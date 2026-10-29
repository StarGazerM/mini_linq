//! Complete CQ programs. Embedded Rust expressions are opaque, pure and total
//! by premise. They are preserved verbatim and scheduled by free variables.
//! Only explicit `where` predicates use relational representation equality and
//! ordering; Rust `if` conditions are never recognized or split.

//! CQ grammar: Block ::= { Body } ( | { Body } )+ | { Body } ( else { Body } )+.
//! Branches are nonempty; mixed operators require nesting.
//! Interface declared types agree; inferred let types are checked by rustc.
//! Multi-rule heads desugar to a union with a head-only interface.
//!
//! Core blocks union projected answers or select the first nonempty arm.
//! Rust expressions in each arm depend only on their free-variable binders.
//!
//! A single rule head may use `count()`, `sum(e)`, `min(e)` or `max(e)`.
//! Other head variables are ordered group keys. Result declarations are
//! required: count is `usize`, sum is `T`, and extrema are `Option<T>`.
//! Normalization records `group [valuation] by [keys] { name: Type = fold(e); }`
//! before the rule terminator and replaces head folds with fresh result names.
//! The valuation retains every named body binder, not anonymous witnesses.
//! All folds consume that same DISTINCT valuation set.
//!
pub mod blocks;
pub mod group;
pub mod hypergraph;
pub mod normal;
pub use blocks::{Block, Branch};
mod sugar;
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
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Program {
    pub visibility: Visibility,
    pub struct_token: Token![struct],
    pub name: Ident,
    pub declaration_semi: Token![;],
    #[to_tokens(|tokens, declarations| quote::TokenStreamExt::append_all(tokens, declarations))]
    pub declarations: Vec<RelationDecl>,
    pub query: Query,
}

impl syn::parse::Parse for Program {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut program = Self::parse_single(input)?;
        let mut queries = vec![program.query.clone()];
        while input.peek(Ident) && input.peek2(token::Paren) {
            queries.push(input.parse()?);
        }
        if queries.len() > 1 {
            for query in &queries {
                if query.group.is_some()
                    || query
                        .head
                        .args
                        .iter()
                        .map(group::aggregate)
                        .collect::<syn::Result<Vec<_>>>()?
                        .iter()
                        .any(Option::is_some)
                {
                    return Err(syn::Error::new_spanned(
                        &query.head,
                        "grouped heads require a single rule; aggregate scope across multiple rules is not defined",
                    ));
                }
            }
            sugar::combine(&mut program, queries)?;
        }
        Ok(program)
    }
}

fn parse_declarations(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<RelationDecl>> {
    let mut declarations = Vec::new();
    while input.peek(Token![#]) || input.peek(kw::relation) {
        declarations.push(input.parse()?);
    }
    Ok(declarations)
}

impl Program {
    /// Surface extensions owning recursive rule syntax parse one rule before
    /// their own expansion; contracted core parsing combines multiple heads.
    pub fn parse_single(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            visibility: input.parse()?,
            struct_token: input.parse()?,
            name: input.parse()?,
            declaration_semi: input.parse()?,
            declarations: parse_declarations(input)?,
            query: input.parse()?,
        })
    }
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
    pub group: Option<group::HeadGroup>,
    pub semi_token: Token![;],
}

impl syn::parse::Parse for Query {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut query = Self {
            head: input.parse()?,
            colon_token: input.parse()?,
            minus_token: input.parse()?,
            body: CommaList::parse_separated_nonempty(input)?,
            group: if input.peek(cq_syntax::algebra::kw::group) {
                Some(input.parse()?)
            } else {
                None
            },
            semi_token: input.parse()?,
        };
        // Scope is determined by binders, never by identifiers in predicates.
        // A later query binder is still a query variable and must pass the
        // written-scope check; an enclosing value is a captured parameter.
        let logical = blocks::bound(&query.body).into_keys().collect();
        blocks::resolve_constraints(&mut query.body, &logical);
        Ok(query)
    }
}

impl Query {
    /// Columns whose complete valuations must survive logical planning.
    pub fn valuation_variables(&self) -> impl Iterator<Item = &Ident> {
        self.group
            .iter()
            .flat_map(|group| group.valuation.iter())
            .chain(self.head.variables().filter(move |_| self.group.is_none()))
    }
    /// The positive, distinct-variable/literal fragment accepted by the basic route.
    pub fn is_basic(&self) -> bool {
        let bound: BTreeSet<_> = self
            .body
            .iter()
            .filter_map(BodyItem::atom)
            .flat_map(Atom::variables)
            .map(symbol_name)
            .collect();
        self.group.is_none()
            && self.head.is_normalized()
            && self.head.variables().count() == self.head.args.len()
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
/// Rust expressions retain their syntax and are pure and total by premise.
/// Relational constraints are written explicitly with `where`.
///
/// ```text
/// BodyItem ::= Atom | "where" Predicate | "if" RustExpr
///            | "let" RustPat [":" RustType] "=" RustExpr | "for" RustPat "in" RustExpr
///            | Ident "!" "{" TokenTree* "}"
///            | "!" Atom | "agg" RustPat "=" Aggregator "(" [Ident ("," Ident)* [","]] ")" "in" Atom
/// ```
// Syntax-tree enum parsed once per clause, like syn's AST; boxing `Filter`
// would change every public constructor and match for no measured gain.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum BodyItem {
    Block(Block),
    /// Relational identity introduced by atom normalization.
    Equality(Equality),
    /// Explicit relational predicate.
    Constraint(Constraint),
    Filter(Filter),
    Let(Box<Let>),
    /// Formal rule CQ-For: Generator uses its written environment and introduces fresh pattern bindings.
    For(Box<For>),
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
        let item = if input.peek(token::Brace) {
            Self::Block(input.parse()?)
        } else if input.peek(Token![where]) {
            input.parse::<Token![where]>()?;
            Self::Constraint(cq_syntax::constraint::parse_where(input)?)
        } else if input.peek(Token![if]) || input.peek(Token![#]) {
            Self::Filter(input.parse()?)
        } else if input.peek(Token![let]) {
            Self::Let(input.parse()?)
        } else if input.peek(Token![for]) {
            Self::For(input.parse()?)
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
            return Err(cq_syntax::constraint::unexpected(
                input,
                "an atom, `where` predicate, `if` Rust guard, `let` binding, `for` generator, or extension",
            ));
        };
        if !input.is_empty()
            && !input.peek(Token![,])
            && !input.peek(Token![;])
            && !input.peek(cq_syntax::algebra::kw::group)
        {
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
            Self::Block(x) => x.to_tokens(output),
            Self::Equality(x) => {
                let expr = Constraint::from_equality(x).cq_tokens();
                output.extend(quote::quote!(where #expr));
            }
            Self::Constraint(x) => {
                let expr = x.cq_tokens();
                output.extend(quote::quote!(where #expr));
            }
            Self::Filter(x) => x.to_tokens(output),
            Self::Let(x) => x.to_tokens(output),
            Self::For(x) => x.to_tokens(output),
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
            (Self::Block(a), Self::Block(b)) => a == b,
            (Self::Equality(a), Self::Equality(b)) => a == b,
            (Self::Constraint(a), Self::Constraint(b)) => a == b,
            (Self::Equality(equality), Self::Constraint(constraint))
            | (Self::Constraint(constraint), Self::Equality(equality)) => {
                use cq_syntax::constraint::{Comparison, Term};
                matches!(constraint,
                Constraint::Compare {
                    left: Term::Variable(left),
                    comparison: Comparison::Eq(_),
                    right,
                } if left == &equality.left && match (&equality.right, right) {
                    (EqualityValue::Variable(a), Term::Variable(b)) => a == b,
                    (EqualityValue::Literal(a), Term::Literal(b)) => a == b.as_ref(),
                    _ => false,
                })
            }
            (Self::Filter(a), Self::Filter(b)) => a == b,
            (Self::Let(a), Self::Let(b)) => a == b,
            (Self::For(a), Self::For(b)) => a == b,
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

pub use cq_syntax::{Filter, For, Let, TypeAnnotation};

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Negation {
    pub not_token: Token![!],
    pub atom: Atom,
}

impl Negation {
    /// Query variables are names with query binders; other bare names capture
    /// host parameters, just as in relational guards. Negation binds nothing.
    pub fn variables<'a>(
        &'a self,
        logical: &'a BTreeSet<String>,
    ) -> impl Iterator<Item = &'a Ident> + 'a {
        self.atom
            .variables()
            .filter(move |id| logical.contains(&symbol_name(id)))
    }
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
            Self::For(binding) => Some(RustStep {
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

/// Formal rule WF-CQ: Check CQ declarations, written scope, output arity and symbolic type availability.
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
    if let Some(group) = &program.query.group {
        group.check(program)?;
    } else {
        let mut grouped = false;
        for expression in &program.query.head.args {
            grouped |= group::aggregate(expression)?.is_some();
        }
        if grouped {
            if program.output().is_none() {
                return Err(syn::Error::new_spanned(
                    &program.query.head,
                    "aggregated output requires a result declaration",
                ));
            }
            let bound = blocks::bound(&program.query.body);
            for expression in &program.query.head.args {
                if group::aggregate(expression)?.is_none()
                    && !rust::variable(expression)
                        .is_some_and(|v| bound.contains_key(&symbol_name(v)))
                {
                    return Err(syn::Error::new_spanned(
                        expression,
                        "group keys must be named body variables",
                    ));
                }
            }
        }
    }
    if program.output().is_none() {
        let types = blocks::types(&schema, &program.query.body, &BTreeMap::new())?;
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
    let logical: BTreeSet<_> = blocks::bound(&query.body).into_keys().collect();
    blocks::types(schema, &query.body, &BTreeMap::new())?;
    let body_variables = check_body(schema, &query.body, &logical, BTreeSet::new())?;
    let check_uses = |expr: &syn::Expr, bound: &BTreeSet<String>| -> syn::Result<()> {
        if let Some(name) = rust::known_uses(expr, &logical).difference(bound).next() {
            return Err(syn::Error::new_spanned(
                expr,
                format!(
                    "logical variable `{name}` is used before it is bound (branch-local names cannot escape a block)"
                ),
            ));
        }
        Ok(())
    };
    let all_names: BTreeSet<_> = blocks::walk(&query.body)
        .into_iter()
        .flat_map(|item| match item {
            BodyItem::Positive { atom } => atom.variables().map(symbol_name).collect::<Vec<_>>(),
            BodyItem::Let(binding) => rust::bindings(&binding.pattern)
                .iter()
                .map(symbol_name)
                .collect(),
            BodyItem::For(binding) => rust::bindings(&binding.pattern)
                .iter()
                .map(symbol_name)
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    for expression in &query.head.args {
        if let Some(name) = rust::known_uses(expression, &all_names)
            .difference(&body_variables)
            .next()
        {
            return Err(syn::Error::new_spanned(
                expression,
                format!("branch-local variable `{name}` cannot escape its block"),
            ));
        }
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
    if !blocks::walk(&query.body)
        .into_iter()
        .any(|item| item.atom().is_some() || matches!(item, BodyItem::Negation(_)))
    {
        return Err(syn::Error::new_spanned(
            query,
            "CQ requires a relational occurrence",
        ));
    }
    Ok(())
}

/// Formal rule CQ-Atom: Atom columns bind simultaneously; computed arguments
/// use the incoming scope plus this atom's bare binders, never later clauses.
fn check_body(
    schema: &BTreeMap<String, &RelationDecl>,
    body: &CommaList<BodyItem>,
    logical: &BTreeSet<String>,
    mut body_variables: BTreeSet<String>,
) -> syn::Result<BTreeSet<String>> {
    let check_uses = |expr: &syn::Expr,
                      bound: &BTreeSet<String>,
                      forbidden: &BTreeSet<String>|
     -> syn::Result<()> {
        let names = logical.union(forbidden).cloned().collect();
        if let Some(name) = rust::known_uses(expr, &names).difference(bound).next() {
            return Err(syn::Error::new_spanned(
                expr,
                format!("logical variable `{name}` is used before it is bound"),
            ));
        }
        Ok(())
    };
    let mut forbidden = BTreeSet::new();
    for item in body {
        // OPTIONAL HW4 — STUDENT 2/6 (WF): add Aggregate.
        // Sum keeps its value local, requires correlated terms already bound,
        // and then binds one fresh result. Negation is part of core CQ.
        match item {
            BodyItem::Block(block) => {
                if block.branches.len() < 2
                    || block.branches.iter().any(|branch| branch.body.is_empty())
                {
                    return Err(syn::Error::new_spanned(
                        block,
                        "a block requires at least two nonempty branches",
                    ));
                }
                let mut common = None::<BTreeSet<String>>;
                for branch in &block.branches {
                    let branch_logical: BTreeSet<_> = logical
                        .iter()
                        .cloned()
                        .chain(blocks::bound(&branch.body).into_keys())
                        .collect();
                    let bound = check_body(
                        schema,
                        &branch.body,
                        &branch_logical,
                        body_variables.clone(),
                    )?;
                    common = Some(match common {
                        None => bound,
                        Some(names) => names.intersection(&bound).cloned().collect(),
                    });
                }
                let common = common.expect("nonempty branches checked");
                forbidden.extend(block.analyze(&body_variables).locals.into_iter().flatten());
                body_variables = common;
            }
            BodyItem::Positive { atom } => {
                let binders: BTreeSet<_> = atom.variables().map(symbol_name).collect();
                if let Some(variable) = binders.intersection(&forbidden).next() {
                    return Err(syn::Error::new_spanned(
                        atom,
                        format!(
                            "branch-local variable `{variable}` cannot be used after its block"
                        ),
                    ));
                }
                check_input_atom(schema, atom)?;
                // An atom is one tuple match, not a sequence of column reads.
                // A-Local evaluates dependent computed arguments after the match.
                // Derive its simultaneous binder scope once for both checks.
                let mut local = body_variables.clone();
                local.extend(binders);
                for arg in &atom.args {
                    if rust::variable(arg).is_none() {
                        check_uses(arg, &local, &forbidden)?;
                    }
                }
                body_variables = local;
            }
            // Formal rule CQ-Neg: Test a declared input using bound query
            // variables, captured parameters, literals and existential wildcards.
            BodyItem::Negation(negation) => {
                check_input_atom(schema, &negation.atom)?;
                for argument in &negation.atom.args {
                    if matches!(argument, syn::Expr::Infer(_)) {
                        continue;
                    }
                    if let Some(variable) = rust::variable(argument) {
                        let name = symbol_name(variable);
                        if (logical.contains(&name) || forbidden.contains(&name))
                            && !body_variables.contains(&name)
                        {
                            return Err(syn::Error::new_spanned(
                                variable,
                                format!(
                                    "negation uses logical variable `{name}` before it is bound"
                                ),
                            ));
                        }
                    } else if !cq_syntax::equality::literal(argument) {
                        return Err(syn::Error::new_spanned(
                            argument,
                            "negation arguments must be bound query variables, bare host parameters, literals, or `_`",
                        ));
                    }
                }
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
            // Formal rule CQ-Where: Only explicit relational predicates constrain query columns.
            // Formal rule CQ-Comparison: Relational terms name bound columns or closed operands.
            BodyItem::Constraint(constraint) => {
                constraint.check()?;
                for variable in constraint.variables() {
                    if !body_variables.contains(&symbol_name(variable)) {
                        return Err(syn::Error::new_spanned(
                            variable,
                            format!(
                                "relational constraint uses query variable `{variable}` before it is bound"
                            ),
                        ));
                    }
                }
                // Formal rule CQ-Closed: A host operand has no query free variables.
                let names = logical.union(&forbidden).cloned().collect();
                if let Some(expression) = constraint.parameters().find(|expression| {
                    !rust::free_variables(expression)
                        .dependencies(&names)
                        .is_empty()
                }) {
                    return Err(syn::Error::new_spanned(
                        expression,
                        "relational closed operand depends on a query variable",
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
            BodyItem::Aggregate(_) => {
                return Err(syn::Error::new_spanned(
                    item,
                    "aggregation requires the optional extended CQ semantic contract",
                ));
            }
            // Formal rule IF-Environment: Check logical scope; Rust resolves enclosing values.
            BodyItem::Filter(filter) => {
                check_uses(&filter.condition, &body_variables, &forbidden)?;
            }
            BodyItem::Let(binding) => {
                check_uses(&binding.expression, &body_variables, &forbidden)?;
                for variable in rust::bindings(&binding.pattern) {
                    if forbidden.contains(&symbol_name(&variable)) {
                        return Err(syn::Error::new_spanned(
                            variable,
                            "branch-local variable cannot be rebound after its block",
                        ));
                    }
                    if !body_variables.insert(symbol_name(&variable)) {
                        return Err(syn::Error::new_spanned(
                            variable,
                            "let binding shadows an existing logical variable",
                        ));
                    }
                }
            }
            // Check the same lexical freshness obligation as scalar let;
            // Rust validates the pattern itself.
            BodyItem::For(binding) => {
                check_uses(&binding.expression, &body_variables, &forbidden)?;
                for variable in rust::bindings(&binding.pattern) {
                    if forbidden.contains(&symbol_name(&variable)) {
                        return Err(syn::Error::new_spanned(
                            variable,
                            "branch-local variable cannot be rebound after its block",
                        ));
                    }
                    if !body_variables.insert(symbol_name(&variable)) {
                        return Err(syn::Error::new_spanned(
                            variable,
                            "for binding shadows an existing logical variable",
                        ));
                    }
                }
            }
        }
    }

    Ok(body_variables)
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

pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    if source != target {
        return Err(syn::Error::new_spanned(
            target,
            "CQ rewrite requires an unchanged contracted query; surface expansion precedes this boundary",
        ));
    }
    Ok(())
}
impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
