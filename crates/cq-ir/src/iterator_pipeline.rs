use std::collections::BTreeSet;

use proc_macro2::{Ident, TokenStream};
use quote::{ToTokens, TokenStreamExt};
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Pat, Token};

use crate::symbol_name;

pub mod kw {
    syn::custom_keyword!(distinct);
    syn::custom_keyword!(filter);
    syn::custom_keyword!(join);
    syn::custom_keyword!(project);
    syn::custom_keyword!(scan);
    syn::custom_keyword!(unit);
    syn::custom_keyword!(with);
}

/// A named, left-deep Volcano iterator pipeline.
///
/// Every physical operator defines one stream and every non-source operator
/// explicitly names its predecessor. Complete tuple bindings make each
/// intermediate stream's item schema visible. Rust iterator combinators are
/// deliberately absent: choosing them belongs to the next lowering.
///
/// ```text
/// iter0 = scan (x, y,) in (rows.iter()) yield (x, y,);
/// iter1 = join iter0 as (x, y,) with (z,) in (lookup(y.clone())) yield (x, y, z,);
/// iter2 = project iter1 as (x, y, z,) yield ((x.clone(), y.clone(), z.clone(),));
/// iter3 = distinct iter2;
/// return iter3.
/// ```
///
/// ```text
/// Plan       ::= Definition* Return
/// Definition ::= StreamId "=" Operator ";"
/// Operator   ::= Unit | Scan | Join | Filter | Project | Distinct
/// Unit       ::= "unit" "yield" BindingExpr
/// Scan       ::= "scan" RustPat "in" "(" RustExpr ")" "yield" BindingExpr
/// Join       ::= "join" StreamId "as" BindingPat "with" RustPat
///                "in" "(" RustExpr ")" "yield" BindingExpr
/// Filter     ::= "filter" StreamId "as" BindingPat "if" "(" RustExpr ")"
/// Project    ::= "project" StreamId "as" BindingPat "yield" "(" RustExpr ")"
/// Distinct   ::= "distinct" StreamId
/// Return     ::= "return" StreamId "."
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pipeline {
    pub definitions: Vec<Definition>,
    pub return_stream: ReturnStream,
}

impl Parse for Pipeline {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut definitions = Vec::new();
        while !input.peek(Token![return]) {
            if input.is_empty() {
                return Err(input.error("named iterator plan must end with `return stream.`"));
            }
            definitions.push(input.parse()?);
        }
        let return_stream = input.parse()?;
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after named iterator plan"));
        }
        Ok(Self {
            definitions,
            return_stream,
        })
    }
}

impl ToTokens for Pipeline {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append_all(&self.definitions);
        self.return_stream.to_tokens(tokens);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Definition {
    pub stream: Ident,
    pub eq_token: Token![=],
    pub operator: Operator,
    pub semi_token: Token![;],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operator {
    Unit(Box<Unit>),
    Scan(Box<Scan>),
    Join(Box<Join>),
    Filter(Box<Filter>),
    Project(Box<Project>),
    Distinct(Box<Distinct>),
}

impl Parse for Operator {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::unit) {
            input.parse().map(Box::new).map(Self::Unit)
        } else if input.peek(kw::scan) {
            input.parse().map(Box::new).map(Self::Scan)
        } else if input.peek(kw::join) {
            input.parse().map(Box::new).map(Self::Join)
        } else if input.peek(kw::filter) {
            input.parse().map(Box::new).map(Self::Filter)
        } else if input.peek(kw::project) {
            input.parse().map(Box::new).map(Self::Project)
        } else if input.peek(kw::distinct) {
            input.parse().map(Box::new).map(Self::Distinct)
        } else {
            Err(input.error("expected `unit`, `scan`, `join`, `filter`, `project`, or `distinct`"))
        }
    }
}

impl ToTokens for Operator {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Unit(operator) => operator.to_tokens(tokens),
            Self::Scan(operator) => operator.to_tokens(tokens),
            Self::Join(operator) => operator.to_tokens(tokens),
            Self::Filter(operator) => operator.to_tokens(tokens),
            Self::Project(operator) => operator.to_tokens(tokens),
            Self::Distinct(operator) => operator.to_tokens(tokens),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Unit {
    pub unit_token: kw::unit,
    pub yield_token: Token![yield],
    pub binding: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Scan {
    pub scan_token: kw::scan,
    #[parse(Pat::parse_multi_with_leading_vert)]
    pub item_pattern: Pat,
    pub in_token: Token![in],
    #[syn(parenthesized)]
    pub source_paren: syn::token::Paren,
    #[syn(in = source_paren)]
    pub source: Expr,
    pub yield_token: Token![yield],
    pub binding: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Join {
    pub join_token: kw::join,
    pub input_stream: Ident,
    pub as_token: Token![as],
    #[parse(Pat::parse_multi_with_leading_vert)]
    pub input_pattern: Pat,
    pub with_token: kw::with,
    #[parse(Pat::parse_multi_with_leading_vert)]
    pub item_pattern: Pat,
    pub in_token: Token![in],
    #[syn(parenthesized)]
    pub source_paren: syn::token::Paren,
    #[syn(in = source_paren)]
    pub source: Expr,
    pub yield_token: Token![yield],
    pub binding: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Filter {
    pub filter_token: kw::filter,
    pub input_stream: Ident,
    pub as_token: Token![as],
    #[parse(Pat::parse_multi_with_leading_vert)]
    pub input_pattern: Pat,
    pub if_token: Token![if],
    #[syn(parenthesized)]
    pub condition_paren: syn::token::Paren,
    #[syn(in = condition_paren)]
    pub condition: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Project {
    pub project_token: kw::project,
    pub input_stream: Ident,
    pub as_token: Token![as],
    #[parse(Pat::parse_multi_with_leading_vert)]
    pub input_pattern: Pat,
    pub yield_token: Token![yield],
    #[syn(parenthesized)]
    pub output_paren: syn::token::Paren,
    #[syn(in = output_paren)]
    pub output: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Distinct {
    pub distinct_token: kw::distinct,
    pub input_stream: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ReturnStream {
    pub return_token: Token![return],
    pub stream: Ident,
    pub dot_token: Token![.],
}

pub mod contract {
    pub fn check(plan: &super::Pipeline) -> syn::Result<()> {
        super::check_plan(plan)
    }

    pub fn well_formed(plan: &super::Pipeline) -> bool {
        check(plan).is_ok()
    }
}

fn check_plan(plan: &Pipeline) -> syn::Result<()> {
    let Some((first, remaining)) = plan.definitions.split_first() else {
        return Err(syn::Error::new_spanned(
            &plan.return_stream,
            "named iterator plan must define a source and projection",
        ));
    };

    let mut defined = BTreeSet::new();
    check_unique_stream(first, &mut defined)?;
    check_stream_number(&first.stream, 0)?;
    let mut binding = match &first.operator {
        Operator::Unit(source) => {
            let actual = expression_binding(&source.binding)?;
            if !actual.is_empty() {
                return Err(syn::Error::new_spanned(
                    &source.binding,
                    "the unit source must yield the empty binding `()`",
                ));
            }
            actual
        }
        Operator::Scan(source) => {
            let introduced = enumerating_pattern_binding(&source.item_pattern)?;
            check_fresh(&introduced, &BTreeSet::new(), &source.item_pattern)?;
            let actual = expression_binding(&source.binding)?;
            check_same_binding("scan output", &actual, &introduced, &source.binding)?;
            actual
        }
        _ => {
            return Err(syn::Error::new_spanned(
                &first.operator,
                "named iterator plan must start with `unit` or `scan`",
            ));
        }
    };

    let mut previous = first.stream.clone();
    let mut saw_project = false;
    let mut saw_distinct = false;
    for (position, definition) in remaining.iter().enumerate() {
        check_unique_stream(definition, &mut defined)?;
        check_stream_number(&definition.stream, position + 1)?;
        if saw_distinct {
            return Err(syn::Error::new_spanned(
                definition,
                "the distinct result-set operator must be final",
            ));
        }
        match &definition.operator {
            Operator::Join(operator) => {
                if saw_project {
                    return Err(syn::Error::new_spanned(
                        operator,
                        "a join cannot follow projection",
                    ));
                }
                check_input_stream(&operator.input_stream, &previous)?;
                check_input_pattern(&operator.input_pattern, &binding)?;
                let introduced = enumerating_pattern_binding(&operator.item_pattern)?;
                let already_bound = binding.iter().cloned().collect();
                check_fresh(&introduced, &already_bound, &operator.item_pattern)?;
                let mut expected = binding.clone();
                expected.extend(introduced);
                let actual = expression_binding(&operator.binding)?;
                check_same_binding("join output", &actual, &expected, &operator.binding)?;
                binding = actual;
            }
            Operator::Filter(operator) => {
                if saw_project {
                    return Err(syn::Error::new_spanned(
                        operator,
                        "a filter cannot follow projection",
                    ));
                }
                check_input_stream(&operator.input_stream, &previous)?;
                check_input_pattern(&operator.input_pattern, &binding)?;
            }
            Operator::Project(operator) => {
                if saw_project {
                    return Err(syn::Error::new_spanned(
                        operator,
                        "an iterator pipeline has exactly one projection",
                    ));
                }
                check_input_stream(&operator.input_stream, &previous)?;
                check_input_pattern(&operator.input_pattern, &binding)?;
                saw_project = true;
            }
            Operator::Distinct(operator) => {
                if !saw_project {
                    return Err(syn::Error::new_spanned(
                        operator,
                        "distinct must immediately follow projection",
                    ));
                }
                check_input_stream(&operator.input_stream, &previous)?;
                saw_distinct = true;
            }
            Operator::Unit(_) | Operator::Scan(_) => {
                return Err(syn::Error::new_spanned(
                    &definition.operator,
                    "`unit` or `scan` may appear only as the first operator",
                ));
            }
        }
        previous = definition.stream.clone();
    }

    if !saw_project {
        return Err(syn::Error::new_spanned(
            &plan.return_stream,
            "named iterator plan must contain one `project` operator",
        ));
    }
    if !saw_distinct {
        return Err(syn::Error::new_spanned(
            &plan.return_stream,
            "named iterator plan must end with `distinct`",
        ));
    }
    check_input_stream(&plan.return_stream.stream, &previous)
}

fn check_stream_number(stream: &Ident, position: usize) -> syn::Result<()> {
    let expected = format!("iter{position}");
    if symbol_name(stream) == expected {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        stream,
        format!("definition {position} must name its result `{expected}`"),
    ))
}

fn check_unique_stream(definition: &Definition, defined: &mut BTreeSet<String>) -> syn::Result<()> {
    let stream = symbol_name(&definition.stream);
    if defined.insert(stream.clone()) {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        &definition.stream,
        format!("iterator stream `{stream}` is defined more than once"),
    ))
}

fn check_input_stream(actual: &Ident, expected: &Ident) -> syn::Result<()> {
    if symbol_name(actual) == symbol_name(expected) {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        actual,
        format!("operator must consume the preceding stream `{expected}`"),
    ))
}

fn check_input_pattern(pattern: &Pat, expected: &[String]) -> syn::Result<()> {
    let actual = tuple_pattern_binding(pattern)?;
    check_same_binding("operator input", &actual, expected, pattern)
}

fn check_same_binding<T: ToTokens>(
    context: &str,
    actual: &[String],
    expected: &[String],
    tokens: &T,
) -> syn::Result<()> {
    if actual == expected {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        tokens,
        format!(
            "{context} must be the complete binding `({})`",
            tuple_display(expected)
        ),
    ))
}

fn check_fresh<T: ToTokens>(
    introduced: &[String],
    already_bound: &BTreeSet<String>,
    tokens: &T,
) -> syn::Result<()> {
    let mut seen = already_bound.clone();
    for name in introduced {
        if !seen.insert(name.clone()) {
            return Err(syn::Error::new_spanned(
                tokens,
                format!("enumerating pattern introduces duplicate binding `{name}`"),
            ));
        }
    }
    Ok(())
}

fn enumerating_pattern_binding(pattern: &Pat) -> syn::Result<Vec<String>> {
    match pattern {
        Pat::Tuple(pattern) => pattern.elems.iter().map(simple_pattern_name).collect(),
        Pat::Ident(_) => simple_pattern_name(pattern).map(|name| vec![name]),
        _ => Err(syn::Error::new_spanned(
            pattern,
            "enumerating source pattern must be a simple tuple pattern",
        )),
    }
}

fn tuple_pattern_binding(pattern: &Pat) -> syn::Result<Vec<String>> {
    let Pat::Tuple(pattern) = pattern else {
        return Err(syn::Error::new_spanned(
            pattern,
            "operator binding must be a tuple pattern",
        ));
    };
    pattern.elems.iter().map(simple_pattern_name).collect()
}

fn expression_binding(expression: &Expr) -> syn::Result<Vec<String>> {
    let Expr::Tuple(expression) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "intermediate binding must be a tuple expression",
        ));
    };
    expression
        .elems
        .iter()
        .map(|element| {
            let Expr::Path(path) = element else {
                return Err(syn::Error::new_spanned(
                    element,
                    "intermediate binding elements must be simple identifiers",
                ));
            };
            path.path.get_ident().map(symbol_name).ok_or_else(|| {
                syn::Error::new_spanned(
                    element,
                    "intermediate binding elements must be simple identifiers",
                )
            })
        })
        .collect()
}

fn simple_pattern_name(pattern: &Pat) -> syn::Result<String> {
    let Pat::Ident(pattern) = pattern else {
        return Err(syn::Error::new_spanned(
            pattern,
            "binding elements must be simple identifiers",
        ));
    };
    if !pattern.attrs.is_empty()
        || pattern.by_ref.is_some()
        || pattern.mutability.is_some()
        || pattern.subpat.is_some()
    {
        return Err(syn::Error::new_spanned(
            pattern,
            "binding elements must be simple immutable identifiers",
        ));
    }
    Ok(symbol_name(&pattern.ident))
}

fn tuple_display(binding: &[String]) -> String {
    match binding {
        [] => String::new(),
        [only] => format!("{only},"),
        many => many.join(", "),
    }
}
