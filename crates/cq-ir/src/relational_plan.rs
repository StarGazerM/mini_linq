//! A named relational-algebra fragment for one complete conjunctive query.
//!
//! This language separates logical query planning from index selection and
//! Rust execution. Declared relation names are atomic input expressions;
//! [`Rename`] turns one occurrence into a query-variable heading. Result IDs
//! such as `r0` are serialized DAG references, not algebra operators or an
//! assumed prerequisite for relational algebra.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::TokenStream;
use quote::{ToTokens, TokenStreamExt};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token, token};

use crate::{CommaList, cq, symbol_name};

/// Keywords owned only by the relational-plan language.
pub mod kw {
    syn::custom_keyword!(keep);
    syn::custom_keyword!(natural_join);
    syn::custom_keyword!(output);
    syn::custom_keyword!(project);
    syn::custom_keyword!(relational);
    syn::custom_keyword!(rename);
    syn::custom_keyword!(unit);
    syn::custom_keyword!(with);
}

/// A complete CQ program followed by its named relational-algebra residual.
///
/// The source program remains present so this stage is independently
/// inspectable and its local contract can check declared inputs, inferred
/// headings, and output metadata.
///
/// ```text
/// struct TwoHopRoads;
/// relation road(src: City, dst: City);
/// two_hop(from, to) :- road(from, via), road(via, to).
/// relational {
///     r0 = rename road {src -> from, dst -> via};
///     r1 = rename road {src -> via, dst -> to};
///     r2 = natural_join r0 with r1;
///     r3 = project r2 keep {from, to};
///     output r3 as two_hop(from, to).
/// }
/// ```
///
/// ```text
/// Module ::= Program "relational" "{" Plan "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Module {
    pub program: cq::Program,
    pub relational_token: kw::relational,
    #[syn(braced)]
    pub relational_brace: token::Brace,
    #[syn(in = relational_brace)]
    pub plan: Plan,
}

/// A named relational-algebra dataflow ending in output metadata.
///
/// ```text
/// Plan       ::= Definition* Output
/// Definition ::= RelationId "=" Operator ";"
/// Output     ::= "output" RelationId "as" Atom "."
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub definitions: Vec<Definition>,
    pub output: Output,
}

impl Parse for Plan {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut definitions = Vec::new();
        while !input.peek(kw::output) {
            if input.is_empty() {
                return Err(input.error("relational plan must end with `output result as head.`"));
            }
            definitions.push(input.parse()?);
        }
        let output = input.parse()?;
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after relational plan output"));
        }
        Ok(Self {
            definitions,
            output,
        })
    }
}

impl ToTokens for Plan {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append_all(&self.definitions);
        self.output.to_tokens(tokens);
    }
}

/// One named relation result and the logical operator that defines it.
///
/// ```text
/// Definition ::= RelationId "=" (Unit | Rename | NaturalJoin | Project) ";"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Definition {
    pub result: Ident,
    pub eq_token: Token![=],
    pub operator: Operator,
    pub semi_token: Token![;],
}

/// One logical set operator.
///
/// ```text
/// Operator ::= Unit | Rename | NaturalJoin | Project
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operator {
    // Unit is supplied for global optional-HW4 clauses. OPTIONAL HW4 —
    // STUDENT 3/6 (LOGICAL): add AntiSemijoin and AggregateApply as described
    // in doc/HW4-OPTIONAL.md.
    Unit(Unit),
    Rename(Box<Rename>),
    NaturalJoin(Box<NaturalJoin>),
    Project(Box<Project>),
}

impl Parse for Operator {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::unit) {
            input.parse().map(Self::Unit)
        } else if input.peek(kw::rename) {
            input.parse().map(Box::new).map(Self::Rename)
        } else if input.peek(kw::natural_join) {
            input.parse().map(Box::new).map(Self::NaturalJoin)
        } else if input.peek(kw::project) {
            input.parse().map(Box::new).map(Self::Project)
        } else {
            Err(input.error("expected `unit`, `rename`, `natural_join`, or `project`"))
        }
    }
}

impl ToTokens for Operator {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Unit(operator) => operator.to_tokens(tokens),
            Self::Rename(operator) => operator.to_tokens(tokens),
            Self::NaturalJoin(operator) => operator.to_tokens(tokens),
            Self::Project(operator) => operator.to_tokens(tokens),
        }
    }
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
///              AttributeRename ("," AttributeRename)* [","]
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

/// A binary natural join of two earlier relation results.
///
/// Shared attributes and their equality condition are inferred from headings.
///
/// ```text
/// NaturalJoin ::= "natural_join" RelationId "with" RelationId
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct NaturalJoin {
    pub natural_join_token: kw::natural_join,
    pub left: Ident,
    pub with_token: kw::with,
    pub right: Ident,
}

/// Set projection of an earlier result to a distinct subset of attributes.
///
/// The attribute braces denote a set. Ordered output tuple layout belongs to
/// the separate [`Output`] metadata.
///
/// ```text
/// Project ::= "project" RelationId "keep" "{"
///               Ident ("," Ident)* [","]
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

/// The result selected for the source head's ordered tuple layout.
///
/// Output is metadata, not a relational operator.
///
/// ```text
/// Output ::= "output" RelationId "as" Atom "."
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Output {
    pub output_token: kw::output,
    pub input: Ident,
    pub as_token: Token![as],
    pub head: cq::Atom,
    pub dot_token: Token![.],
}

/// The RelationalPlan contract in diagnostic and predicate form.
pub mod contract {
    /// Check the source program and its named relational-algebra residual.
    pub fn check(module: &super::Module) -> syn::Result<()> {
        super::cq::check_program(&module.program)?;
        super::check_plan(module)
    }

    /// Whether a parsed module satisfies the RelationalPlan contract.
    pub fn well_formed(module: &super::Module) -> bool {
        check(module).is_ok()
    }
}

type Heading = BTreeSet<String>;

#[derive(Clone)]
struct RelationInfo {
    heading: Heading,
    operands: Vec<String>,
}

fn check_plan(module: &Module) -> syn::Result<()> {
    let declarations = module
        .program
        .inputs
        .iter()
        .map(|input| (symbol_name(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let mut relations = BTreeMap::<String, RelationInfo>::new();

    for definition in &module.plan.definitions {
        let result_name = symbol_name(&definition.result);
        if relations.contains_key(&result_name) {
            return Err(syn::Error::new_spanned(
                &definition.result,
                format!("relation result `{result_name}` is defined more than once"),
            ));
        }

        // OPTIONAL HW4 — STUDENT 3/6 (LOGICAL): extend the local heading
        // rules for AntiSemijoin and AggregateApply exactly as specified in
        // doc/HW4-OPTIONAL.md.
        let (heading, operands) = match &definition.operator {
            Operator::Unit(_) => (Heading::new(), Vec::new()),
            Operator::Rename(rename) => {
                let Some(declaration) = declarations.get(&symbol_name(&rename.relation)) else {
                    return Err(syn::Error::new_spanned(
                        &rename.relation,
                        format!("rename names undeclared relation `{}`", rename.relation),
                    ));
                };
                (check_rename(rename, declaration)?, Vec::new())
            }
            Operator::NaturalJoin(join) => {
                let (left_name, left) = earlier_relation(&join.left, &relations)?;
                let (right_name, right) = earlier_relation(&join.right, &relations)?;
                let mut heading = left.heading.clone();
                heading.extend(right.heading.iter().cloned());
                (heading, vec![left_name, right_name])
            }
            Operator::Project(project) => {
                let (input_name, input) = earlier_relation(&project.input, &relations)?;
                let mut heading = Heading::new();
                for attribute in &project.attributes {
                    let name = symbol_name(attribute);
                    if !heading.insert(name.clone()) {
                        return Err(syn::Error::new_spanned(
                            attribute,
                            format!("project repeats attribute `{name}`"),
                        ));
                    }
                    if !input.heading.contains(&name) {
                        return Err(syn::Error::new_spanned(
                            attribute,
                            format!(
                                "project attribute `{name}` is not present in input result `{}`",
                                project.input
                            ),
                        ));
                    }
                }
                (heading, vec![input_name])
            }
        };

        relations.insert(result_name, RelationInfo { heading, operands });
    }

    let output_name = symbol_name(&module.plan.output.input);
    let Some(output) = relations.get(&output_name) else {
        return Err(syn::Error::new_spanned(
            &module.plan.output.input,
            format!("output result `{output_name}` is not defined"),
        ));
    };
    check_output_head(&module.program.query.head, &module.plan.output.head)?;

    let output_attributes = module
        .plan
        .output
        .head
        .variables()
        .map(symbol_name)
        .collect::<BTreeSet<_>>();
    if output_attributes != output.heading {
        return Err(syn::Error::new_spanned(
            &module.plan.output,
            "output tuple attributes must exactly match its input heading",
        ));
    }

    check_all_reachable(module, &relations, &output_name)
}

fn check_rename(rename: &Rename, declaration: &cq::RelationDecl) -> syn::Result<Heading> {
    let declared_names = declaration
        .columns
        .iter()
        .map(|column| symbol_name(&column.name))
        .collect::<BTreeSet<_>>();
    let mut by_source = BTreeMap::<String, &AttributeRename>::new();
    let mut targets = BTreeSet::new();

    for attribute in &rename.mapping {
        let source = symbol_name(&attribute.source);
        if !declared_names.contains(&source) {
            return Err(syn::Error::new_spanned(
                &attribute.source,
                format!(
                    "rename source attribute `{source}` is not declared by relation `{}`",
                    rename.relation
                ),
            ));
        }
        if by_source.insert(source.clone(), attribute).is_some() {
            return Err(syn::Error::new_spanned(
                &attribute.source,
                format!("rename maps source attribute `{source}` more than once"),
            ));
        }
        let target = symbol_name(&attribute.target);
        if !targets.insert(target.clone()) {
            return Err(syn::Error::new_spanned(
                &attribute.target,
                format!("rename target attribute `{target}` is not distinct"),
            ));
        }
    }

    if by_source.len() != declaration.columns.len() {
        return Err(syn::Error::new_spanned(
            rename,
            format!(
                "rename of `{}` must map every declared source attribute exactly once",
                rename.relation
            ),
        ));
    }

    let mut heading = Heading::new();
    for column in &declaration.columns {
        let source = symbol_name(&column.name);
        let Some(attribute) = by_source.get(&source) else {
            return Err(syn::Error::new_spanned(
                rename,
                format!(
                    "rename of `{}` must map every declared source attribute exactly once",
                    rename.relation
                ),
            ));
        };
        heading.insert(symbol_name(&attribute.target));
    }
    Ok(heading)
}

fn earlier_relation<'a>(
    operand: &Ident,
    relations: &'a BTreeMap<String, RelationInfo>,
) -> syn::Result<(String, &'a RelationInfo)> {
    let name = symbol_name(operand);
    let Some(info) = relations.get(&name) else {
        return Err(syn::Error::new_spanned(
            operand,
            format!("operand `{name}` must name a result defined earlier in the plan"),
        ));
    };
    Ok((name, info))
}

fn check_output_head(expected: &cq::Atom, actual: &cq::Atom) -> syn::Result<()> {
    let same_relation = symbol_name(&expected.relation) == symbol_name(&actual.relation);
    let expected_variables = expected.variables().map(symbol_name).collect::<Vec<_>>();
    let actual_variables = actual.variables().map(symbol_name).collect::<Vec<_>>();
    if same_relation && expected_variables == actual_variables {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        actual,
        format!(
            "output head must be the source query head `{}({})`",
            expected.relation,
            expected_variables.join(", ")
        ),
    ))
}

fn check_all_reachable(
    module: &Module,
    relations: &BTreeMap<String, RelationInfo>,
    root: &str,
) -> syn::Result<()> {
    let mut reachable = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(name) = pending.pop() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        pending.extend(relations[&name].operands.iter().cloned());
    }

    if reachable.len() == relations.len() {
        return Ok(());
    }
    let unused = module
        .plan
        .definitions
        .iter()
        .find(|definition| !reachable.contains(&symbol_name(&definition.result)))
        .expect("a size mismatch guarantees an unreachable definition");
    Err(syn::Error::new_spanned(
        &unused.result,
        format!(
            "relation result `{}` is not reachable from output result `{root}`",
            unused.result
        ),
    ))
}
