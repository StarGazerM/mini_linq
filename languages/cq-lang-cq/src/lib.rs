//! Complete CQ programs with denotation `T X = bottom + Set X`.
//! The reference evaluates the desugared body in written order; bottom is one
//! undistinguished panic aborting the evaluation. Rust lets and guards denote
//! opaque partial functions of their free variables and host captures. I/O,
//! mutation, counters and evaluation counts are outside this meaning.
//! Relational steps commute and may move earlier across a Rust step. Rust
//! steps keep their relative order and follow every free-variable binder.
//!
//! Comparisons inside `if` guards use Rust syntax and trait meaning: `==`/`!=`
//! call PartialEq and `<`/`<=`/`>`/`>=` call PartialOrd. Under the existing
//! Leibniz Eq and total, consistent Ord premises, comparisons on query names
//! (bare, `*x`, or `&x`), literals (including negative numbers), and bare host
//! parameters are total relational constraints. `&&`, `||`, `!`, and parentheses
//! over these comparisons are classical connectives. Host parameters are
//! borrowed once outside query loops. Known inadmissible types are rejected;
//! Rust checks Eq/Ord for aliases and user types; admissibility is trusted.
//! A top-level conjunction extracts these relational conjuncts and leaves one
//! opaque residual guard at the written position, in written order. In `a && f`,
//! f already runs only where a holds; in `f && a`, moving total a earlier only
//! shrinks f's domain. Mixed disjunctions and negations stay wholly opaque.
//! Calls, methods (including string membership), arithmetic, casts, indexing,
//! blocks, macros and field access remain opaque. Relational negation `!R(args)`
//! tests absence in a declared input, binds nothing, and is position-independent
//! once its query variables are bound; wildcards are existential.
//! Bare comparisons, `between`, and set membership are not CQ syntax.
//! Normalized internal constraints retain the RA/IX/Physical forms.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! CQ grammar: Block ::= { Body } ( | { Body } )+ | { Body } ( else { Body } )+.
//! Branches are nonempty; mixed operators require nesting.
//! Interface declared types agree; inferred let types are checked by rustc.
//! Multi-rule heads desugar to a union with a head-only interface.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
pub mod blocks;
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
        let logical = blocks::bound(&query.body).into_keys().collect();
        blocks::split_guards(&mut query.body, &logical);
        Ok(query)
    }
}

impl Query {
    /// The positive, distinct-variable/literal fragment accepted by the basic route.
    pub fn is_basic(&self) -> bool {
        let bound: BTreeSet<_> = self
            .body
            .iter()
            .filter_map(BodyItem::atom)
            .flat_map(Atom::variables)
            .map(symbol_name)
            .collect();
        self.head.is_normalized()
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
/// After guard recognition, residual Rust expressions retain their syntax as
/// opaque partial functions. Lets, finite pure generators,
/// and guards share one written chain; relational steps alone may move earlier
/// across that chain under the sufficient refinement check.
///
/// ```text
/// BodyItem ::= Atom | "if" RustExpr | "let" RustPat [":" RustType] "=" RustExpr
///            | "for" RustPat "in" RustExpr
///            | Ident "!" "{" TokenTree* "}"
///            | "!" Atom | "agg" RustPat "=" Aggregator "(" [Ident ("," Ident)* [","]] ")" "in" Atom
/// ```
// Syntax-tree enum parsed once per clause, like syn's AST; boxing `Filter`
// would change every public constructor and match for no measured gain.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum BodyItem {
    Block(Block),
    /// Internal relational identity extracted from a Rust guard or atom.
    Equality(Equality),
    /// Internal total predicate extracted from a Rust guard.
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
                "an atom, `if` Rust guard, `let` binding, `for` generator, or extension",
            ));
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
            Self::Block(x) => x.to_tokens(output),
            Self::Equality(x) => {
                let expr = Constraint::from_equality(x).cq_tokens();
                output.extend(quote::quote!(if #expr));
            }
            Self::Constraint(x) => {
                let expr = x.cq_tokens();
                output.extend(quote::quote!(if #expr));
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
                if let Some(variable) = atom
                    .variables()
                    .find(|id| forbidden.contains(&symbol_name(id)))
                {
                    return Err(syn::Error::new_spanned(
                        variable,
                        "branch-local variable cannot be used after its block",
                    ));
                }
                check_input_atom(schema, atom)?;
                // Every bare argument of this atom is available to a same-atom
                // computed argument, regardless of its column position.
                let mut local = body_variables.clone();
                local.extend(atom.variables().map(symbol_name));
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
                let input = check_input_atom(schema, &negation.atom)?;
                for (argument, column) in negation.atom.args.iter().zip(&input.columns) {
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
                    cq_syntax::algebra::check_leibniz_type(&column.ty, &column.name)?;
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
            // Formal rule CQ-Comparison: Constraint terms use bound query names or captured constants.
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
            BodyItem::Aggregate(_) => {
                return Err(syn::Error::new_spanned(
                    item,
                    "aggregation requires the optional extended CQ semantic contract",
                ));
            }
            // Formal rule IF-Environment: Check logical scope; Rust resolves enclosing values.
            BodyItem::Filter(filter) => {
                if !cq_syntax::guard::split(&filter.condition, logical)
                    .constraints
                    .is_empty()
                {
                    return Err(syn::Error::new_spanned(
                        filter,
                        "recognizable Rust guard must be split into relational constraints and its opaque residual",
                    ));
                }
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
