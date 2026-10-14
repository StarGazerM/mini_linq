//! RA prime: named relational operators with an ordered Rust ancestry chain.
//!
//! The denotation is Kleisli over `T X = bottom + Set X`: one panic aborts
//! evaluation. Rust binds and guards are opaque partial functions of their free
//! variables. I/O, mutation and evaluation counts are outside this meaning.
//! Relational laws apply only to relational operators. Moving a relational step
//! earlier across Rust may shrink its evaluation domain; moving it later is not
//! allowed. Preserving the Rust chain and every source-before relational step is
//! a sufficient refinement condition, checked by [`check_rewrite`] for RA′
//! rewrites and by cq-elaborate for VO elaboration.
//! These are structural contracts for occurrence-preserving passes, not CQ
//! equivalence deciders. Declared inputs carry a unique source occurrence:
//! `rename R@2 { a -> x }`. Checks compare fixed column bindings, equality
//! closure, relational constraints and their first observing Rust region.
//! They visit the DAG and its derived headings without unfolding shared rows
//! or searching for atom mappings. Split-witness wiring admits local set
//! identities: redundant projections/selections, a join projected to its left
//! heading as a semijoin, and support projections that hide no shared name.
//! Predicate ownership and independent witnesses remain structural obligations;
//! other existential splits can be rejected despite equivalent answer sets.
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
//! `contract::check` derives headings, types and source identifier bindings
//! entirely from this program. VO correspondence belongs to cq-elaborate.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! RA grammar adds union r1, r2, ...
//! with identical headings and anti [F] r, s.
//! Block origin records free/interface headings, ordered fallback, prefix correlation and optional finite context domain.
//! Projections remove local witnesses; negative operands name unmodified original results.
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
//! `scope[names] @c.j.k` records a Rust step's lexical inputs and source path.
//! Prefix Rust reached through a positive semijoin remains outside the arm;
//! no such context reduction may modify a negated original branch.
//!
pub use cq_syntax::algebra::{
    Anti, AttributeRename, Bind, BlockOrigin, Correlation, Domain, Environment, Guard, Project,
    Rename, Select, Theta, Union, Unit,
};
pub use cq_syntax::{Equality, EqualityValue};

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, symbol_name};

/// Keywords owned only by the relational-plan language.
pub mod kw {
    pub use cq_syntax::algebra::kw::*;
    syn::custom_keyword!(natural_join);
    syn::custom_keyword!(output);
    syn::custom_keyword!(relational);
    syn::custom_keyword!(semijoin);
}

/// A complete named relational program.
///
/// ```text
/// Program ::= "relational" "{" RelationDecl* Definition* Output "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Program {
    pub relational_token: kw::relational,
    #[syn(braced)]
    pub relational_brace: token::Brace,
    #[syn(in = relational_brace)]
    pub plan: Plan,
}

/// A named relational-algebra dataflow ending in output metadata.
///
/// ```text
/// Plan       ::= RelationDecl* Definition* Output
/// Definition ::= RelationId "=" Operator ";"
/// Output     ::= "output" RelationId "as" Atom ";"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Plan {
    #[parse(cq_syntax::parse_declarations)]
    #[to_tokens(|tokens, declarations| quote::TokenStreamExt::append_all(tokens, declarations))]
    pub declarations: Vec<cq_syntax::RelationDecl>,
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
/// Operator ::= Unit | Rename | NaturalJoin | Project | Semijoin | Select | Bind | Guard
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Operator {
    #[parse(peek = kw::union)]
    Union(Box<Union>),
    #[parse(peek = kw::anti)]
    Anti(Box<Anti>),
    // Unit is supplied for global optional-HW4 clauses. OPTIONAL HW4 —
    // STUDENT 3/6 (LOGICAL): add AntiSemijoin and AggregateApply as
    // course/hw4_optional.rs requires.
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
    #[parse(peek = kw::bind)]
    Bind(Box<Bind>),
    #[parse(peek = kw::guard)]
    Guard(Box<Guard>),
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

/// Checked headings and column types of all named results.
pub type Analysis = BTreeMap<String, RelationInfo>;
pub mod contract {
    /// Structural facts of a checked relational schedule. DAG nodes refer to
    /// earlier nodes; VO states its obligations here without running elaboration.
    #[derive(Default, Clone)]
    pub struct Schedule {
        pub(super) nodes: Vec<super::structural::Node>,
        pub(super) atoms: std::collections::HashMap<usize, super::structural::Atom>,
        pub(super) chain: Vec<super::structural::Rust>,
        pub(super) reductions: Vec<super::structural::Reduction>,
        pub(super) output: Option<usize>,
        pub(super) head: String,
        pub(super) split_nodes: Vec<(usize, usize)>,
        pub(super) full_reads: Vec<(usize, usize)>,
        pub(super) row_views: Vec<super::structural::RowView>,
        pub(super) view_requests: Vec<(usize, usize, bool)>,
        pub(super) paths: std::collections::BTreeMap<cq_syntax::Occurrence, usize>,
    }

    pub fn check(program: &super::Program) -> syn::Result<super::Analysis> {
        super::check_in(&program.plan.declarations, &program.plan)
    }
}

type Heading = BTreeSet<String>;

#[derive(Clone, Debug)]
pub struct RelationInfo {
    pub bindings: BTreeMap<String, Ident>,
    pub heading: Heading,
    pub types: BTreeMap<String, syn::Type>,
    /// Columns introduced by Bind. This records provenance, not the ownership
    /// mode chosen by a realization when subsequent expressions read the column.
    pub computed: BTreeSet<String>,
    operands: Vec<String>,
    /// Rust nodes on this result's single ancestry chain, in written order.
    pub chain: Vec<String>,
}

/// Formal rule WF-RA: Check declarations, local operators, output heading and definition reachability.
fn check_in<'a>(
    inputs: impl IntoIterator<Item = &'a cq_syntax::RelationDecl>,
    plan: &Plan,
) -> syn::Result<BTreeMap<String, RelationInfo>> {
    let declarations = cq_syntax::check_input_declarations(inputs)?;
    if let Some(output) = declarations.get(&symbol_name(&plan.output.head.relation)) {
        output.check_result()?;
    }
    let mut relations = BTreeMap::<String, RelationInfo>::new();
    let logical: BTreeSet<_> = plan
        .definitions
        .iter()
        .flat_map(|d| match &d.operator {
            Operator::Rename(r) => r.mapping.iter().map(|m| symbol_name(&m.target)).collect(),
            Operator::Bind(e) => cq_syntax::rust::bindings(&e.pattern)
                .iter()
                .map(symbol_name)
                .collect(),
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

    let mut chain = Vec::<String>::new();
    let mut occurrences = BTreeSet::new();
    for definition in &plan.definitions {
        let result_name = symbol_name(&definition.result);
        if relations.contains_key(&result_name) {
            return Err(syn::Error::new_spanned(
                &definition.result,
                format!("relation result `{result_name}` is defined more than once"),
            ));
        }

        // OPTIONAL HW4 — STUDENT 3/6 (LOGICAL): extend the local heading
        // rules for AntiSemijoin and AggregateApply as
        // course/hw4_optional.rs requires.
        let (heading, operands) = match &definition.operator {
            Operator::Union(union) => {
                if union.operands.len() < 2 {
                    return Err(syn::Error::new_spanned(
                        union,
                        "union requires at least two operands",
                    ));
                }
                let mut operands = Vec::new();
                let mut heading = None;
                let mut schema = cq_syntax::algebra::UnionSchema::default();
                for operand in &union.operands {
                    let (name, info) = earlier_relation(operand, &relations)?;
                    schema
                        .include(&info.heading, &info.types, &info.computed)
                        .map_err(|message| syn::Error::new_spanned(operand, message))?;
                    heading = Some(info.heading.clone());
                    operands.push(name);
                }
                if let Some(origin) = &union.origin {
                    origin.occurrence.check()?;
                    let free: BTreeSet<_> = origin.free.iter().map(symbol_name).collect();
                    let interface: BTreeSet<_> = origin.interface.iter().map(symbol_name).collect();
                    if free.len() != origin.free.len()
                        || interface.len() != origin.interface.len()
                        || !free.is_disjoint(&interface)
                    {
                        return Err(syn::Error::new_spanned(
                            origin,
                            "block free and interface columns must be distinct",
                        ));
                    }
                    if let Some(domain) = &origin.domain {
                        if origin.over.is_some()
                            || earlier_relation(&domain.input, &relations)?.1.heading != free
                        {
                            return Err(syn::Error::new_spanned(
                                domain,
                                "a hoisted block domain must have exactly its free heading",
                            ));
                        }
                    }
                    let mut expected: BTreeSet<_> = origin
                        .free
                        .iter()
                        .chain(&origin.interface)
                        .map(symbol_name)
                        .collect();
                    if let Some(over) = &origin.over {
                        let prefix = &earlier_relation(&over.prefix, &relations)?.1.heading;
                        if !free.is_subset(prefix) || !interface.is_disjoint(prefix) {
                            return Err(syn::Error::new_spanned(
                                origin,
                                "a correlated block requires its free columns and supplies fresh interface columns",
                            ));
                        }
                        expected.extend(prefix.iter().cloned());
                    }
                    if heading.as_ref() != Some(&expected) {
                        return Err(syn::Error::new_spanned(
                            origin,
                            "block union must project exactly its context and interface",
                        ));
                    }
                }
                (heading.expect("nonempty union"), operands)
            }
            Operator::Anti(anti) => {
                let (left_name, left) = earlier_relation(&anti.left, &relations)?;
                let (right_name, right) = earlier_relation(&anti.right, &relations)?;
                let mut columns = BTreeSet::new();
                for column in &anti.columns {
                    let name = symbol_name(column);
                    if !columns.insert(name.clone())
                        || !left.heading.contains(&name)
                        || !right.heading.contains(&name)
                    {
                        return Err(syn::Error::new_spanned(
                            column,
                            "anti key must list distinct columns available in both operands",
                        ));
                    }
                    check_leibniz(&left.types[&name], column)?;
                    check_leibniz(&right.types[&name], column)?;
                }
                (left.heading.clone(), vec![left_name, right_name])
            }
            Operator::Unit(_) => (Heading::new(), Vec::new()),
            Operator::Rename(rename) => {
                rename.occurrence.check()?;
                let occurrence = &rename.occurrence;
                if !occurrences.insert(occurrence.clone()) {
                    return Err(syn::Error::new_spanned(
                        &rename.occurrence,
                        format!("atom occurrence @{occurrence} is realized more than once"),
                    ));
                }
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
                if select.condition.is_empty() {
                    return Err(syn::Error::new_spanned(
                        select,
                        "selection requires a relational constraint",
                    ));
                }
                // Formal rule RA-Constraint: Central theta has available columns and checked Eq/Ord premises.
                select.condition.check()?;
                for c in &select.condition.constraints {
                    for p in c.parameters() {
                        if input.heading.contains(&symbol_name(p)) {
                            return Err(syn::Error::new_spanned(
                                p,
                                "relational parameter names an input column",
                            ));
                        }
                    }
                    for column in c.variables() {
                        let n = symbol_name(column);
                        let Some(ty) = input.types.get(&n) else {
                            return Err(syn::Error::new_spanned(
                                column,
                                format!("selection uses unavailable column `{n}`"),
                            ));
                        };
                        check_leibniz(ty, column)?;
                    }
                    for column in c.ordered_variables() {
                        cq_syntax::algebra::check_ord_type(
                            &input.types[&symbol_name(column)],
                            column,
                        )?;
                    }
                }
                for equality in &select.condition.equalities {
                    equality.check()?;
                    for column in equality.variables() {
                        let name = symbol_name(column);
                        if !input.heading.contains(&name) {
                            return Err(syn::Error::new_spanned(
                                column,
                                format!("selection uses unavailable column `{name}`"),
                            ));
                        }
                        check_leibniz(&input.types[&name], column)?;
                    }
                }
                (input.heading.clone(), vec![name])
            }
            Operator::Guard(guard) => {
                let (name, input) = earlier_relation(&guard.input, &relations)?;
                if guard.environment.is_none() {
                    check_uses(&guard.condition, &input.heading)?;
                }
                if let Some(environment) = &guard.environment {
                    environment.check(&input.heading, &guard.condition)?;
                }
                (input.heading.clone(), vec![name])
            }
            Operator::Bind(bind) => {
                let (name, input) = earlier_relation(&bind.input, &relations)?;
                if bind.environment.is_none() {
                    check_uses(&bind.expression, &input.heading)?;
                }
                if let Some(environment) = &bind.environment {
                    environment.check(&input.heading, &bind.expression)?;
                }
                let mut heading = input.heading.clone();
                for column in cq_syntax::rust::bindings(&bind.pattern) {
                    if !heading.insert(symbol_name(&column)) {
                        return Err(syn::Error::new_spanned(
                            &column,
                            format!("bind `{result_name}` requires fresh attributes: `{column}`"),
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
        if let Operator::Bind(bind) = &definition.operator {
            for (name, ty) in bind.binding_types() {
                types.insert(symbol_name(&name), ty);
                computed.insert(symbol_name(&name));
            }
        }
        let mut bindings = BTreeMap::new();
        for operand in operands.iter().rev() {
            for (name, id) in &relations[operand].bindings {
                if heading.contains(name) {
                    bindings.insert(name.clone(), id.clone());
                }
            }
        }
        match &definition.operator {
            Operator::Rename(r) => {
                for m in &r.mapping {
                    bindings.insert(symbol_name(&m.target), m.target.clone());
                }
            }
            Operator::Bind(e) => {
                for name in cq_syntax::rust::bindings(&e.pattern) {
                    bindings.insert(symbol_name(&name), name);
                }
            }
            _ => (),
        }
        let mut ancestry = Vec::new();
        for operand in &operands {
            let branch = &relations[operand].chain;
            if ancestry.starts_with(branch) {
                continue;
            }
            if branch.starts_with(&ancestry) {
                ancestry = branch.clone();
                continue;
            }
            if matches!(definition.operator, Operator::Union(_)) {
                for name in branch {
                    if !ancestry.contains(name) {
                        ancestry.push(name.clone());
                    }
                }
            } else {
                return Err(syn::Error::new_spanned(
                    &definition.operator,
                    "Rust chain nodes must form one ancestry chain",
                ));
            }
        }
        if matches!(definition.operator, Operator::Bind(_) | Operator::Guard(_)) {
            if ancestry != chain
                && !plan
                    .definitions
                    .iter()
                    .any(|d| matches!(d.operator, Operator::Union(_)))
            {
                return Err(syn::Error::new_spanned(
                    &definition.operator,
                    "Rust chain nodes must form one ancestry chain in written order",
                ));
            }
            chain.push(result_name.clone());
            ancestry.push(result_name.clone());
        }
        match &definition.operator {
            Operator::NaturalJoin(_) | Operator::Semijoin(_) => {
                for name in relations[&operands[0]]
                    .heading
                    .intersection(&relations[&operands[1]].heading)
                {
                    check_leibniz(&relations[&operands[0]].types[name], &bindings[name])?;
                    check_leibniz(&relations[&operands[1]].types[name], &bindings[name])?;
                }
            }
            Operator::Project(project) => {
                for column in &project.attributes {
                    check_leibniz(&types[&symbol_name(column)], column)?;
                }
            }
            _ => {}
        }
        relations.insert(
            result_name,
            RelationInfo {
                bindings,
                heading,
                types,
                computed,
                operands,
                chain: ancestry,
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

    if plan.output.head.variables().count() != plan.output.head.args.len() {
        return Err(syn::Error::new_spanned(
            &plan.output.head,
            "RA output arguments must be variables; computations belong in final bind nodes",
        ));
    }
    let output_attributes = plan
        .output
        .head
        .variables()
        .map(symbol_name)
        .collect::<BTreeSet<_>>();
    if output_attributes != output.heading {
        return Err(syn::Error::new_spanned(
            &plan.output,
            format!("output `{output_name}` tuple attributes must exactly match its input heading"),
        ));
    }
    for column in plan.output.head.variables() {
        check_leibniz(&output.types[&symbol_name(column)], column)?;
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
        let attribute = by_source[&source];
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

/// Reject known failures of Leibniz admissibility. Generated Rust checks the
/// necessary trait bounds; admissibility of user implementations is a premise.
fn check_leibniz(ty: &syn::Type, column: &Ident) -> syn::Result<()> {
    cq_syntax::algebra::check_leibniz_type(ty, column)
}

mod constants;
mod structural;
pub use constants::disjoint_unions;

/// Check local contracts and fixed structural obligations of an RA rewrite.
///
/// A localized identity certificate also accepts absorption of a reduced
/// support operand when exact named-result lineage supplies all shared columns.
/// It preserves the remaining definitions, Rust payloads and negative ancestry;
/// independent projected witnesses are never merged to establish containment.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    let source_schedule = contract::Schedule::from_program(source)?;
    let target_schedule = contract::Schedule::from_program(target)?;
    if source.plan.declarations != target.plan.declarations {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            "RA rewrite changes declarations",
        ));
    }
    source_schedule
        .check(&target_schedule, &[])
        .or_else(|message| {
            if structural::check_reduction_rewrite(source, target, &source_schedule) {
                Ok(())
            } else {
                Err(message)
            }
        })
        .map_err(|message| {
            syn::Error::new_spanned(
                &target.plan.output,
                format!("result `{}`: {message}", target.plan.output.input),
            )
        })
}
impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
