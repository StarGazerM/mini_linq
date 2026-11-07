//! Operator details shared by relational and physical programs.
use crate::CommaList;
use syn::{Ident, Token, token};
pub mod kw {
    syn::custom_keyword!(keep);
    syn::custom_keyword!(project);
    syn::custom_keyword!(rename);
    syn::custom_keyword!(constants);
    syn::custom_keyword!(unit);
    syn::custom_keyword!(select);
    syn::custom_keyword!(with);
    syn::custom_keyword!(bind);
    syn::custom_keyword!(guard);
    syn::custom_keyword!(union);
    syn::custom_keyword!(anti);
    syn::custom_keyword!(block);
    syn::custom_keyword!(free);
    syn::custom_keyword!(interface);
    syn::custom_keyword!(fallback);
    syn::custom_keyword!(over);
    syn::custom_keyword!(domain);
    syn::custom_keyword!(scope);
    syn::custom_keyword!(group);
    syn::custom_keyword!(by);
    syn::custom_keyword!(count);
    syn::custom_keyword!(sum);
    syn::custom_keyword!(min);
    syn::custom_keyword!(max);
}

/// Group a set of complete named valuations; aggregate arguments retain
/// multiplicity across distinct valuations. An empty global group has one row.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Group {
    pub group_token: kw::group,
    pub input: Ident,
    pub by_token: kw::by,
    #[syn(bracketed)]
    pub bracket_token: token::Bracket,
    #[syn(in = bracket_token)]
    #[parse(CommaList::parse_terminated)]
    pub keys: CommaList<Ident>,
    #[syn(braced)]
    pub brace_token: token::Brace,
    #[syn(in = brace_token)]
    #[parse(parse_aggregates)]
    #[to_tokens(|tokens, bindings| quote::TokenStreamExt::append_all(tokens, bindings))]
    pub aggregates: Vec<AggregateBinding>,
}

/// A typed fold result; extrema declare `Option<T>`, count declares `usize`.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct AggregateBinding {
    pub name: Ident,
    pub colon_token: Token![:],
    pub ty: syn::Type,
    pub eq_token: Token![=],
    pub kind: AggregateKind,
    #[syn(parenthesized)]
    pub paren_token: token::Paren,
    #[syn(in = paren_token)]
    #[parse(parse_aggregate_expression)]
    pub expression: Option<syn::Expr>,
    pub semi_token: Token![;],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggregateKind {
    Count,
    Sum,
    Min,
    Max,
}

impl syn::parse::Parse for AggregateKind {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::count) {
            input.parse::<kw::count>()?;
            Ok(Self::Count)
        } else if input.peek(kw::sum) {
            input.parse::<kw::sum>()?;
            Ok(Self::Sum)
        } else if input.peek(kw::min) {
            input.parse::<kw::min>()?;
            Ok(Self::Min)
        } else if input.peek(kw::max) {
            input.parse::<kw::max>()?;
            Ok(Self::Max)
        } else {
            Err(input.error("expected count, sum, min, or max"))
        }
    }
}

impl quote::ToTokens for AggregateKind {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let name = match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Min => "min",
            Self::Max => "max",
        };
        quote::ToTokens::to_tokens(&Ident::new(name, proc_macro2::Span::call_site()), tokens);
    }
}

fn parse_aggregates(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<AggregateBinding>> {
    let mut aggregates = Vec::new();
    while !input.is_empty() {
        aggregates.push(input.parse()?);
    }
    Ok(aggregates)
}

fn parse_aggregate_expression(
    input: syn::parse::ParseStream<'_>,
) -> syn::Result<Option<syn::Expr>> {
    if input.is_empty() {
        Ok(None)
    } else {
        input.parse().map(Some)
    }
}

impl Group {
    /// Check the complete input valuation schema and typed fold payloads.
    /// Keys and aggregate arguments name input columns or host captures
    /// (WF-Capture); a capture is one value shared by every valuation.
    pub fn check(
        &self,
        input: &std::collections::BTreeMap<String, syn::Type>,
        captures: &std::collections::BTreeSet<String>,
    ) -> syn::Result<()> {
        let available = |name: &String| input.contains_key(name) || captures.contains(name);
        let mut outputs = std::collections::BTreeSet::new();
        for key in &self.keys {
            if !available(&crate::symbol_name(key)) || !outputs.insert(crate::symbol_name(key)) {
                return Err(syn::Error::new_spanned(
                    key,
                    "group keys must be distinct input columns or captures",
                ));
            }
        }
        if self.aggregates.is_empty() {
            return Err(syn::Error::new_spanned(self, "group requires an aggregate"));
        }
        for aggregate in &self.aggregates {
            let name = crate::symbol_name(&aggregate.name);
            if available(&name) || !outputs.insert(name) {
                return Err(syn::Error::new_spanned(
                    &aggregate.name,
                    "aggregate outputs must be fresh and distinct",
                ));
            }
            if aggregate.kind == AggregateKind::Count {
                if aggregate.expression.is_some() {
                    return Err(syn::Error::new_spanned(
                        aggregate,
                        "count() requires no argument",
                    ));
                }
                continue;
            }
            let variable = aggregate
                .expression
                .as_ref()
                .and_then(crate::rust::variable)
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        aggregate,
                        "aggregate argument must be a normalized input variable",
                    )
                })?;
            if !available(&crate::symbol_name(variable)) {
                return Err(syn::Error::new_spanned(
                    variable,
                    "aggregate argument is not in the input valuation or captures",
                ));
            }
        }
        Ok(())
    }
}

/// Set union of earlier named results with identical headings.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Union {
    pub union_token: kw::union,
    #[parse(CommaList::parse_separated_nonempty)]
    pub operands: CommaList<Ident>,
    #[parse(parse_origin)]
    pub origin: Option<BlockOrigin>,
}

/// Branch provenance, common heading and optional shared prefix correlation.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct BlockOrigin {
    pub block_token: kw::block,
    pub free_token: kw::free,
    #[syn(bracketed)]
    pub free_bracket: token::Bracket,
    #[syn(in = free_bracket)]
    #[parse(CommaList::parse_terminated)]
    pub free: CommaList<Ident>,
    pub interface_token: kw::interface,
    #[syn(bracketed)]
    pub interface_bracket: token::Bracket,
    #[syn(in = interface_bracket)]
    #[parse(CommaList::parse_terminated)]
    pub interface: CommaList<Ident>,
    pub fallback: Option<kw::fallback>,
    #[parse(parse_correlation)]
    pub over: Option<Correlation>,
    #[parse(parse_domain)]
    pub domain: Option<Domain>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Correlation {
    pub over_token: kw::over,
    pub prefix: Ident,
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Domain {
    pub domain_token: kw::domain,
    pub input: Ident,
}
fn parse_domain(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<Domain>> {
    if input.peek(kw::domain) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}
fn parse_origin(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<BlockOrigin>> {
    if input.peek(kw::block) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}
fn parse_correlation(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<Correlation>> {
    if input.peek(kw::over) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

/// Anti matches exactly the listed F columns; its negative operand is a named
/// original branch result, rather than an operand eligible for reduction.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Anti {
    pub anti_token: kw::anti,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Ident>,
    pub left: Ident,
    pub comma: Token![,],
    pub right: Ident,
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

/// Filter literal source columns, drop them, and rename the remaining columns.
///
/// ```text
/// Rename ::= "rename" RelationName "{"
///              [AttributeRename ("," AttributeRename)* [","]]
///            "}" ["constants" "{" Equality ("," Equality)* [","] "}"]
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
    #[parse(parse_constants)]
    pub constants: Option<OccurrenceConstants>,
}

/// Literal restrictions in source-column names, absent from the result heading.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct OccurrenceConstants {
    pub constants_token: kw::constants,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(CommaList::parse_terminated)]
    pub equalities: CommaList<crate::Equality>,
}

impl OccurrenceConstants {
    pub fn check(&self) -> syn::Result<()> {
        if self.equalities.is_empty() {
            return Err(syn::Error::new_spanned(
                self,
                "occurrence constants must not be empty",
            ));
        }
        let mut sources = std::collections::BTreeSet::new();
        for equality in &self.equalities {
            equality.check()?;
            if !matches!(equality.right, crate::EqualityValue::Literal(_)) {
                return Err(syn::Error::new_spanned(
                    equality,
                    "occurrence constants require literal values",
                ));
            }
            if !sources.insert(crate::symbol_name(&equality.left)) {
                return Err(syn::Error::new_spanned(
                    &equality.left,
                    "occurrence constant source column is repeated",
                ));
            }
        }
        Ok(())
    }
}

fn parse_constants(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<OccurrenceConstants>> {
    if input.peek(kw::constants) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

/// A conjunction of central relational constraints, including boolean
/// disjunctions. Rust guards use [`Guard`]. Commas mean conjunction:
/// `select r if (x >= @(lower), x < @(upper))`. Captures are explicit in IR.
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
            theta.push_constraint(c);
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
    /// Insert a relational predicate in conjunction normal form. A one-branch
    /// `Or` denotes conjunction, not an alternative or a Rust expression.
    pub fn push_constraint(&mut self, constraint: crate::Constraint) {
        match constraint {
            crate::Constraint::Or(mut branches) if branches.len() == 1 => {
                for conjunct in branches.pop().unwrap() {
                    self.push_constraint(conjunct);
                }
            }
            constraint => {
                if let Some(equality) = constraint.equality() {
                    self.equalities.push(equality);
                } else {
                    self.constraints.push(constraint);
                }
            }
        }
    }

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

    /// Point key, when every conjunct equates a column to a constant: a literal
    /// or a once-bound closed operand (IF-Parameter: closed operands are host
    /// values usable as keys). Duplicate conjuncts retain one probe value;
    /// selection still checks all conjuncts, including inconsistent constants
    /// on the same column. Membership is multi-point and is not a key.
    pub fn constant_keys(
        &self,
    ) -> Option<std::collections::BTreeMap<String, crate::constraint::Term>> {
        use crate::constraint::{Comparison, Constraint, Term};
        if self.is_empty() {
            return None;
        }
        let equalities = self.equalities.iter().map(|e| match &e.right {
            crate::EqualityValue::Literal(value) => Some((
                crate::symbol_name(&e.left),
                Term::Literal(Box::new(value.clone())),
            )),
            crate::EqualityValue::Variable(_) => None,
        });
        let constraints = self.constraints.iter().map(|c| match c {
            Constraint::Compare {
                left: Term::Variable(v),
                comparison: Comparison::Eq(_),
                right,
            } if right.constant() => Some((crate::symbol_name(v), right.clone())),
            Constraint::Compare {
                left,
                comparison: Comparison::Eq(_),
                right: Term::Variable(v),
            } if left.constant() => Some((crate::symbol_name(v), left.clone())),
            _ => None,
        });
        equalities.chain(constraints).collect()
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

/// Opaque, pure, total scalar computation placed by its free variables.
/// I/O, mutation and evaluation counts are outside this meaning.
/// With `for`, unnest a finite pure iterator instead of binding one scalar.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Bind {
    pub bind_token: kw::bind,
    pub input: Ident,
    pub with_token: kw::with,
    #[syn(optional, peek = Token![for])]
    pub for_token: Option<Token![for]>,
    #[parse(syn::Pat::parse_single)]
    pub pattern: syn::Pat,
    #[syn(optional, peek = Token![:])]
    pub annotation: Option<crate::TypeAnnotation>,
    pub eq_token: Token![=],
    /// IF-Capture: the bound value is a copy of the named query capture.
    #[syn(optional, peek = crate::capture::kw::capture)]
    pub capture: Option<crate::capture::kw::capture>,
    #[syn(parenthesized)]
    pub expression_paren: token::Paren,
    #[syn(in = expression_paren)]
    pub expression: syn::Expr,
    #[parse(parse_environment)]
    pub environment: Option<Environment>,
}

impl Bind {
    pub fn binding(&self) -> crate::Let {
        crate::Let {
            let_token: Default::default(),
            pattern: self.pattern.clone(),
            annotation: self.annotation.clone(),
            eq_token: self.eq_token,
            capture: self.capture,
            expression: self.expression.clone(),
        }
    }

    /// The Rust value a printer binds (IF-Capture).
    pub fn value(&self) -> syn::Expr {
        crate::capture::realize_value(self.capture.as_ref(), &self.expression)
    }

    pub fn binding_types(&self) -> Vec<(Ident, syn::Type)> {
        self.binding().binding_types()
    }
}

/// Predicate: typed CQ-Arith is total; other Rust retains its ancestry position.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Guard {
    pub guard_token: kw::guard,
    pub input: Ident,
    pub if_token: Token![if],
    /// IF-Capture: the condition is the named query capture's `bool`.
    #[syn(optional, peek = crate::capture::kw::capture)]
    pub capture: Option<crate::capture::kw::capture>,
    #[syn(parenthesized)]
    pub condition_paren: token::Paren,
    #[syn(in = condition_paren)]
    pub condition: syn::Expr,
    #[parse(parse_environment)]
    pub environment: Option<Environment>,
}

impl Guard {
    /// The Rust condition a printer evaluates (IF-Capture).
    pub fn test(&self) -> syn::Expr {
        crate::capture::realize_test(self.capture.as_ref(), &self.condition)
    }
}

/// Written lexical query environment of an opaque step. Planned binders added
/// earlier are passed through its row but do not shadow enclosing Rust values.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Environment {
    pub scope_token: kw::scope,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub variables: CommaList<Ident>,
}
impl Environment {
    pub fn check(
        &self,
        heading: &std::collections::BTreeSet<String>,
        _expression: &syn::Expr,
    ) -> syn::Result<()> {
        let names: std::collections::BTreeSet<_> =
            self.variables.iter().map(crate::symbol_name).collect();
        if names.len() != self.variables.len() || !names.is_subset(heading) {
            return Err(syn::Error::new_spanned(
                self,
                "Rust scope must list distinct available query names",
            ));
        }
        Ok(())
    }
}
fn parse_environment(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<Environment>> {
    if input.peek(kw::scope) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
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

/// Union arms are disjoint only by construction: a fallback block runs a later
/// arm only when the earlier arms are empty. Relational literals are operands
/// like any other; the compiler never decodes their values, so differing
/// literals prove nothing and such unions are canonicalized.
pub fn disjoint_unions<'a>(
    unions: impl IntoIterator<Item = (&'a Ident, Option<&'a BlockOrigin>)>,
) -> std::collections::BTreeSet<String> {
    unions
        .into_iter()
        .filter(|(_, origin)| origin.is_some_and(|o| o.fallback.is_some()))
        .map(|(name, _)| crate::symbol_name(name))
        .collect()
}

/// Accumulated heading and declared (noncomputed) column types of a union.
#[derive(Default)]
pub struct UnionSchema {
    heading: Option<std::collections::BTreeSet<String>>,
    logical_type_ids: std::collections::BTreeMap<String, crate::LogicalTypeId>,
}
impl UnionSchema {
    pub fn include<'a>(
        &mut self,
        heading: impl IntoIterator<Item = &'a String>,
        logical_type_ids: &std::collections::BTreeMap<String, crate::LogicalTypeId>,
        computed: &std::collections::BTreeSet<String>,
    ) -> Result<(), String> {
        let heading = heading.into_iter().cloned().collect();
        if self.heading.as_ref().is_some_and(|h| h != &heading) {
            return Err("union operand headings must be identical".into());
        }
        for (column, id) in logical_type_ids {
            if !computed.contains(column) {
                if self
                    .logical_type_ids
                    .get(column)
                    .is_some_and(|previous| previous != id)
                {
                    return Err(format!(
                        "union interface `{column}` has different declared types"
                    ));
                }
                self.logical_type_ids.insert(column.clone(), *id);
            }
        }
        self.heading = Some(heading);
        Ok(())
    }
}
