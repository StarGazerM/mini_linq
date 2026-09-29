//! A named relational-algebra fragment for one complete conjunctive query.
//!
//! This language separates logical query planning from index selection and
//! Rust execution. Declared relation names are atomic input expressions;
//! [`Rename`] turns one occurrence into a query-variable heading. Result IDs
//! such as `r0` are serialized DAG references, not algebra operators or an
//! assumed prerequisite for relational algebra.
//!
//! A result name denotes a set-valued definition that may have several uses.
//! It promises neither materialization nor a reusable iterator instance. The
//! selected realization decides whether those uses recompute or share retained
//! data; each independent traversal needs its own cursor state.
//!
//! [`erase`] and [`kernel`] read back the normal-form query a plan computes,
//! so a plan can be checked against the valuation it elaborates.

#[cfg(feature = "reference-prefix")]
mod kernel;
#[cfg(feature = "reference-prefix")]
pub use kernel::{erase, kernel};

#[cfg(feature = "reference-prefix")]
pub use cq_lang_cq as cq;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, symbol_name};

/// Keywords owned only by the relational-plan language.
pub mod kw {
    syn::custom_keyword!(keep);
    syn::custom_keyword!(natural_join);
    syn::custom_keyword!(output);
    syn::custom_keyword!(project);
    syn::custom_keyword!(relational);
    syn::custom_keyword!(rename);
    syn::custom_keyword!(unit);
    syn::custom_keyword!(semijoin);
    syn::custom_keyword!(select);
    syn::custom_keyword!(with);
    syn::custom_keyword!(extend);
    syn::custom_keyword!(value);
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
/// two_hop(from, to) :- road(from, via), road(via, to);
/// relational {
///     r0 = rename road {src -> from, dst -> via};
///     r1 = rename road {src -> via, dst -> to};
///     r2 = natural_join r0 with r1;
///     r3 = project r2 keep {from, to};
///     output r3 as two_hop(from, to);
/// }
/// ```
///
/// ```text
/// Module ::= Program "relational" "{" Plan "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Section {
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
/// Output     ::= "output" RelationId "as" Atom ";"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Plan {
    #[syn(repeat, until = kw::output)]
    pub definitions: Vec<Definition>,
    pub output: Output,
}

/// One named relation result and the logical operator that defines it.
///
/// ```text
/// Definition ::= RelationId "=" Operator ";"
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
/// Operator ::= Unit | Rename | NaturalJoin | Project | Semijoin | Select | Extend
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Operator {
    // Unit is supplied for global optional-HW4 clauses. OPTIONAL HW4 —
    // STUDENT 3/6 (LOGICAL): add AntiSemijoin and AggregateApply as described
    // in doc/HW4-OPTIONAL.md.
    #[parse(peek = kw::unit)]
    Unit(Unit),
    #[parse(peek = kw::rename)]
    Rename(Box<Rename>),
    #[parse(peek = kw::natural_join)]
    NaturalJoin(Box<NaturalJoin>),
    #[parse(peek = kw::project)]
    Project(Box<Project>),
    #[parse(peek = kw::semijoin)]
    Semijoin(Box<Semijoin>),
    #[parse(peek = kw::select)]
    Select(Box<Select>),
    #[parse(peek = kw::extend)]
    Extend(Box<Extend>),
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

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Semijoin {
    pub semijoin_token: kw::semijoin,
    pub left: Ident,
    pub with_token: kw::with,
    pub right: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Select {
    pub select_token: kw::select,
    pub input: Ident,
    pub if_token: Token![if],
    #[syn(parenthesized)]
    pub condition_paren: token::Paren,
    #[syn(in = condition_paren)]
    pub condition: syn::Expr,
}

/// Compute one tuple of fresh attributes for each input tuple. Rust owns the
/// expression and its types; `_` permits local Rust inference for a binding.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Extend {
    pub extend_token: kw::extend,
    pub input: Ident,
    pub with_token: kw::with,
    #[syn(braced)]
    pub columns_brace: token::Brace,
    #[syn(in = columns_brace)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<cq_syntax::ColumnDecl>,
    pub value_token: kw::value,
    #[syn(parenthesized)]
    pub value_paren: token::Paren,
    #[syn(in = value_paren)]
    pub value: syn::Expr,
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
/// Output ::= "output" RelationId "as" Atom ";"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Output {
    pub output_token: kw::output,
    pub input: Ident,
    pub as_token: Token![as],
    pub head: cq_syntax::Atom,
    pub semi_token: Token![;],
}

/// The RelationalPlan contract in diagnostic and predicate form.
/// Input declarations and expected output supplied by a source language.
pub trait CqEnv {
    fn inputs(&self) -> &[cq_syntax::RelationDecl];
    fn head(&self) -> &cq_syntax::Atom;
}
pub mod contract {
    pub fn check(section: &super::Section, env: &impl super::CqEnv) -> syn::Result<()> {
        super::check_output_head(env.head(), &section.plan.output.head)?;
        super::check_in(env.inputs(), &section.plan).map(|_| ())
    }
    pub fn well_formed(section: &super::Section, env: &impl super::CqEnv) -> bool {
        check(section, env).is_ok()
    }
    #[cfg(feature = "reference-prefix")]
    pub fn check_module(module: &super::Module) -> syn::Result<()> {
        super::cq::check_program(&module.program)?;
        check(
            &module.section,
            &super::reference::Env::new(&module.program),
        )
    }
    #[cfg(feature = "reference-prefix")]
    pub fn well_formed_module(module: &super::Module) -> bool {
        check_module(module).is_ok()
    }
}

type Heading = BTreeSet<String>;

#[derive(Clone, Debug)]
pub struct RelationInfo {
    pub heading: Heading,
    pub types: BTreeMap<String, syn::Type>,
    /// Columns introduced by Extend. This records provenance, not the ownership
    /// mode chosen by a realization when subsequent expressions read the column.
    pub computed: BTreeSet<String>,
    operands: Vec<String>,
}

/// A proc macro cannot infer a Rust function's return type; A declared result
/// schema supplies that public API boundary; simple projections inherit types.
#[cfg(feature = "reference-prefix")]
pub fn output_types(program: &cq::Program, input: &RelationInfo) -> syn::Result<Vec<syn::Type>> {
    if let Some(output) = &program.output() {
        return Ok(output.columns.iter().map(|c| c.ty.clone()).collect());
    }
    program.query.head.args.iter().map(|arg| {
        cq_syntax::rust::variable(arg).and_then(|v| input.types.get(&symbol_name(v)))
            .filter(|ty| !matches!(ty, syn::Type::Infer(_))).cloned()
            .ok_or_else(|| syn::Error::new_spanned(arg,
                "declare the result relation's column types for a computed head, e.g. relation answer(value: String);"))
    }).collect()
}

pub fn check_in<'a>(
    inputs: impl IntoIterator<Item = &'a cq_syntax::RelationDecl>,
    plan: &Plan,
) -> syn::Result<BTreeMap<String, RelationInfo>> {
    let declarations = cq_syntax::check_input_declarations(inputs)?;
    let mut relations = BTreeMap::<String, RelationInfo>::new();
    let logical: BTreeSet<_> = plan
        .definitions
        .iter()
        .flat_map(|d| match &d.operator {
            Operator::Rename(r) => r.mapping.iter().map(|m| symbol_name(&m.target)).collect(),
            Operator::Extend(e) => e.columns.iter().map(|c| symbol_name(&c.name)).collect(),
            _ => Vec::new(),
        })
        .collect();
    let check_uses = |expr: &syn::Expr, heading: &Heading| -> syn::Result<()> {
        if let Some(name) = cq_syntax::rust::known_uses(expr, &logical)
            .difference(heading)
            .next()
        {
            return Err(syn::Error::new_spanned(
                expr,
                format!("Rust expression uses unavailable attribute `{name}`"),
            ));
        }
        Ok(())
    };

    for definition in &plan.definitions {
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
            Operator::Semijoin(join) => {
                let (left_name, left) = earlier_relation(&join.left, &relations)?;
                let (right_name, _) = earlier_relation(&join.right, &relations)?;
                (left.heading.clone(), vec![left_name, right_name])
            }
            Operator::Select(select) => {
                let (name, input) = earlier_relation(&select.input, &relations)?;
                check_uses(&select.condition, &input.heading)?;
                (input.heading.clone(), vec![name])
            }
            Operator::Extend(extend) => {
                let (name, input) = earlier_relation(&extend.input, &relations)?;
                check_uses(&extend.value, &input.heading)?;
                let mut heading = input.heading.clone();
                for column in &extend.columns {
                    if !heading.insert(symbol_name(&column.name)) {
                        return Err(syn::Error::new_spanned(
                            column,
                            "extend requires fresh, distinct attributes",
                        ));
                    }
                }
                (heading, vec![name])
            }
        };

        let mut types = BTreeMap::new();
        if let Operator::Rename(rename) = &definition.operator {
            let declaration = declarations[&symbol_name(&rename.relation)];
            for mapping in &rename.mapping {
                let column = declaration
                    .columns
                    .iter()
                    .find(|c| symbol_name(&c.name) == symbol_name(&mapping.source))
                    .unwrap();
                types.insert(symbol_name(&mapping.target), column.ty.clone());
            }
        } else {
            for operand in &operands {
                for (name, ty) in &relations[operand].types {
                    if heading.contains(name) {
                        types.entry(name.clone()).or_insert_with(|| ty.clone());
                    }
                }
            }
        }
        let mut computed: BTreeSet<_> = operands
            .iter()
            .flat_map(|r| {
                relations[r]
                    .computed
                    .iter()
                    .filter(|v| heading.contains(*v))
                    .cloned()
            })
            .collect();
        if let Operator::Extend(extend) = &definition.operator {
            for column in &extend.columns {
                types.insert(symbol_name(&column.name), column.ty.clone());
                computed.insert(symbol_name(&column.name));
            }
        }
        relations.insert(
            result_name,
            RelationInfo {
                heading,
                types,
                computed,
                operands,
            },
        );
    }

    let output_name = symbol_name(&plan.output.input);
    let Some(output) = relations.get(&output_name) else {
        return Err(syn::Error::new_spanned(
            &plan.output.input,
            format!("output result `{output_name}` is not defined"),
        ));
    };

    for expr in &plan.output.head.args {
        check_uses(expr, &output.heading)?;
    }
    let output_attributes = plan
        .output
        .head
        .args
        .iter()
        .flat_map(|e| cq_syntax::rust::free_variables(e).dependencies(&output.heading))
        .collect::<BTreeSet<_>>();
    if !output_attributes.is_subset(&output.heading)
        || (plan.output.head.variables().count() == plan.output.head.args.len()
            && output_attributes != output.heading)
    {
        return Err(syn::Error::new_spanned(
            &plan.output,
            "output tuple attributes must exactly match its input heading",
        ));
    }

    check_all_reachable(plan, &relations, &output_name)?;
    Ok(relations)
}

fn check_rename(rename: &Rename, declaration: &cq_syntax::RelationDecl) -> syn::Result<Heading> {
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

fn check_output_head(expected: &cq_syntax::Atom, actual: &cq_syntax::Atom) -> syn::Result<()> {
    let same_relation = symbol_name(&expected.relation) == symbol_name(&actual.relation);
    let expected_variables = expected.variables().map(symbol_name).collect::<Vec<_>>();
    if same_relation && expected.args == actual.args {
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
    plan: &Plan,
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
    let unused = plan
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

/// Shared syntax used by this language.
pub use cq_syntax as syntax;

#[cfg(feature = "reference-prefix")]
pub mod reference;
#[cfg(feature = "reference-prefix")]
pub use reference::Module;
