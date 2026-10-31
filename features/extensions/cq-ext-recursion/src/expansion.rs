//! DESIGN contract7-recursion-projection
//! Contract 7 and doc/PLANNING-REDESIGN.md: active-domain heads are authored
//! relational variables. DO NOT recognize Rust copies or inspect column types.
//! EXT-ActiveDomain and EXT-AnswerOwnership (doc/formal/datalog.tex) separate
//! finite-domain membership from the recorded borrowed/owned calling convention.
use std::collections::BTreeSet;

use cq_ext_support::{
    Fresh, Leaf, Rules, check_fresh, check_leaves, ensure, hoist, same_symbol, with_query,
};
use cq_lang_cq::{BodyItem, Program, Query};
use cq_syntax::{RelationDecl, rust, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Expr, Ident, Token, parse::Parse};

mod kw {
    syn::custom_keyword!(observe);
}
#[derive(Clone)]
pub struct Source {
    pub rules: Rules,
    pub observer: Option<Expr>,
}
impl Parse for Source {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let observer = if input.peek(kw::observe) {
            input.parse::<kw::observe>()?;
            input.parse::<Token![=]>()?;
            let observer = input.parse()?;
            input.parse::<Token![;]>()?;
            Some(observer)
        } else {
            None
        };
        Ok(Self {
            rules: input.parse()?,
            observer,
        })
    }
}
impl ToTokens for Source {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { rules, observer } = self;
        if let Some(observer) = observer {
            tokens.extend(quote!(observe = (#observer);));
        }
        rules.to_tokens(tokens);
    }
}
/// DL-AnswerOwnership vocabulary: this extension only admits atom projections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnswerColumn {
    Borrowed,
    Owned,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub declarations: Vec<RelationDecl>,
    pub leaves: Vec<Leaf>,
    pub delta: Ident,
    pub seeds: Vec<(usize, Program)>,
    pub steps: Vec<(usize, Program)>,
    pub observer: Option<Expr>,
    /// Source projection representation; each hole records its owned realization.
    pub answers: Vec<Vec<AnswerColumn>>,
}
fn recursion_count(query: &Query, result: &Ident) -> usize {
    query
        .body
        .iter()
        .filter(|item| matches!(item, BodyItem::Positive { atom } if same_symbol(&atom.relation,result)))
        .count()
}
/// Positive atoms can also join a value previously bound by a generator.
/// The first binder determines ownership, not membership in the active domain.
fn borrowed_columns(query: &Query) -> BTreeSet<String> {
    let mut defined = BTreeSet::new();
    let mut borrowed = BTreeSet::new();
    for item in &query.body {
        let (names, atom): (Vec<_>, bool) = match item {
            BodyItem::Positive { atom } => (atom.variables().map(symbol_name).collect(), true),
            BodyItem::Let(binding) => (
                rust::bindings(&binding.pattern).iter().map(symbol_name).collect(), false,
            ),
            BodyItem::For(binding) => (
                rust::bindings(&binding.pattern).iter().map(symbol_name).collect(), false,
            ),
            BodyItem::Block(block) => (block.analyze(&defined).interface.into_keys().collect(), false),
            _ => continue,
        };
        for name in names {
            if defined.insert(name.clone()) && atom {
                borrowed.insert(name);
            }
        }
    }
    borrowed
}
/// Formal rule EXT-ActiveDomain: every head is a plain variable occurring in a
/// positive atom; there is at least one seed and one linear recursive rule.
fn domain(source: &Source) -> syn::Result<()> {
    let first = &source.rules.first;
    let output = first.output().ok_or_else(|| {
        syn::Error::new_spanned(first, "recursion requires a declared owned result schema")
    })?;
    output.check_result()?;
    let mut seeds = 0;
    let mut steps = 0;
    for query in source.rules.queries() {
        ensure(
            same_symbol(&query.head.relation, &first.query.head.relation)
                && query.head.args.len() == output.arity(),
            query,
            "recursive rules must share the declared head",
        )?;
        match recursion_count(query, &first.query.head.relation) {
            0 => seeds += 1,
            1 => steps += 1,
            _ => {
                return Err(syn::Error::new_spanned(
                    query,
                    "nonlinear recursion: more than one recursive atom",
                ));
            }
        }
        let columns: BTreeSet<_> = query
            .body
            .iter()
            .filter_map(|item| {
                if let BodyItem::Positive { atom } = item {
                    Some(atom)
                } else {
                    None
                }
            })
            .flat_map(|atom| atom.variables())
            .map(symbol_name)
            .collect();
        for argument in &query.head.args {
            let id = rust::variable(argument).ok_or_else(|| {
                syn::Error::new_spanned(
                    argument,
                    "recursive heads must project an active-domain column: write the plain variable, not *x or .clone()",
                )
            })?;
            ensure(
                columns.contains(&symbol_name(id)),
                id,
                "recursive head column must come from a positive atom",
            )?;
        }
    }
    ensure(
        seeds > 0 && steps > 0,
        source,
        "recursion requires seed and recursive rules",
    )
}
fn output(source: &Source, name: &Ident) -> RelationDecl {
    let mut declaration = source
        .rules
        .first
        .output()
        .expect("checked result schema")
        .clone();
    declaration.name = name.clone();
    declaration
}
pub fn compile(source: &Source) -> syn::Result<Context> {
    domain(source)?;
    let mut fresh = Fresh::new(source);
    let (declarations, leaves) = hoist(&source.rules.first, &mut fresh);
    let delta = fresh.id("delta");
    let mut seeds = Vec::new();
    let mut steps = Vec::new();
    let mut answers = Vec::new();
    for (position, query) in source.rules.queries().enumerate() {
        let recursive = recursion_count(query, &source.rules.first.query.head.relation) == 1;
        let borrowed = borrowed_columns(query);
        let answer: Vec<_> = query.head.variables().map(|id| {
            if borrowed.contains(&symbol_name(id)) {
                AnswerColumn::Borrowed
            } else {
                AnswerColumn::Owned
            }
        }).collect();
        let mut query = query.clone();
        let name = fresh.id("round_rows");
        query.head.relation = name.clone();
        // Realize relational ownership before CQ planning/canonicalization.
        // This is generated from the record, never recognized from user Rust.
        for (argument, ownership) in query.head.args.iter_mut().zip(&answer) {
            if matches!(ownership, AnswerColumn::Borrowed) {
                let variable = rust::variable(argument).expect("checked relational head");
                *argument = syn::parse_quote!(::core::clone::Clone::clone(#variable));
            }
        }
        answers.push(answer);
        let mut hole = with_query(
            &source.rules.first,
            &declarations,
            query,
            Some(output(source, &name)),
        );
        if recursive {
            let mut input = output(source, &delta);
            input.value = Some(syn::parse_quote!(= (& #delta)));
            hole.declarations.insert(0, input);
            for item in &mut hole.query.body {
                if let BodyItem::Positive { atom } = item {
                    if same_symbol(&atom.relation, &source.rules.first.query.head.relation) {
                        atom.relation = delta.clone();
                    }
                }
            }
            steps.push((position, hole));
        } else {
            seeds.push((position, hole));
        }
    }
    Ok(Context {
        declarations,
        leaves,
        delta,
        seeds,
        steps,
        answers,
        observer: source.observer.clone(),
    })
}
/// Erase delta renames to result reads, verify the rule partition, schema and
/// observer, and require an active-domain, linear source. Fixpoint execution
/// and ownership are validated by runtime oracles and Rust.
/// Formal rule EXT-AnswerOwnership: check one ownership record per rule/head
/// column against its first defining binder, independently of compilation.
pub fn check(source: &Source, target: &Context) -> syn::Result<()> {
    domain(source)?;
    ensure(target.answers.len() == source.rules.queries().count(), source, "recursive answer ownership rule count changed")?;
    for (query, answer) in source.rules.queries().zip(&target.answers) {
        let borrowed = borrowed_columns(query);
        ensure(
            answer.len() == query.head.args.len()
                && query.head.variables().zip(answer).all(|(id, ownership)| {
                    matches!(ownership, AnswerColumn::Borrowed) == borrowed.contains(&symbol_name(id))
                }),
            query,
            "recursive answer ownership does not match its defining binders",
        )?;
    }
    check_fresh(
        source,
        target
            .leaves
            .iter()
            .map(|leaf| &leaf.binding)
            .chain(std::iter::once(&target.delta)),
    )?;
    check_leaves(&source.rules.first, &target.declarations, &target.leaves)?;
    ensure(
        target.observer == source.observer,
        source,
        "round observer changed",
    )?;
    let queries: Vec<_> = source.rules.queries().collect();
    let mut positions = BTreeSet::new();
    for (recursive, holes) in [(false, &target.seeds), (true, &target.steps)] {
        for (position, hole) in holes {
            check_fresh(source, std::iter::once(&hole.query.head.relation))?;
            ensure(
                *position < queries.len() && positions.insert(*position),
                hole,
                "extra or duplicate round rule",
            )?;
            let original = queries[*position];
            ensure(
                (recursion_count(original, &source.rules.first.query.head.relation) == 1)
                    == recursive,
                hole,
                "wrong seed/delta partition",
            )?;
            let mut erased = hole.clone();
            ensure(
                erased.query.head.args.len() == original.head.args.len(),
                hole,
                "recursive owned-head arity changed",
            )?;
            for ((argument, projection), ownership) in erased.query.head.args.iter_mut()
                .zip(&original.head.args)
                .zip(&target.answers[*position])
            {
                let expected = match ownership {
                    AnswerColumn::Borrowed => syn::parse_quote!(::core::clone::Clone::clone(#projection)),
                    AnswerColumn::Owned => projection.clone(),
                };
                ensure(
                    *argument == expected,
                    &*argument,
                    "recursive owned-head realization changed",
                )?;
                *argument = projection.clone();
            }
            let name = erased.query.head.relation.clone();
            let out = erased
                .declarations
                .pop()
                .ok_or_else(|| syn::Error::new_spanned(hole, "missing round result"))?;
            ensure(
                out == output(source, &name),
                &out,
                "round result schema changed",
            )?;
            if recursive {
                ensure(
                    !erased.declarations.is_empty(),
                    hole,
                    "missing delta declaration",
                )?;
                let input = erased.declarations.remove(0);
                let mut expected = output(source, &target.delta);
                let delta = &target.delta;
                expected.value = Some(syn::parse_quote!(= (&#delta)));
                ensure(
                    input == expected,
                    &input,
                    "delta schema or leaf binding changed",
                )?;
                ensure(
                    recursion_count(&erased.query, &target.delta) == 1,
                    &erased.query,
                    "round must read delta exactly once",
                )?;
                for (position, item) in erased.query.body.iter_mut().enumerate() {
                    if let BodyItem::Positive { atom } = item {
                        if atom.relation == target.delta {
                            let Some(BodyItem::Positive {
                                atom: original_atom,
                            }) = original.body.get(position)
                            else {
                                return Err(syn::Error::new_spanned(
                                    &*atom,
                                    "recursive atom position changed",
                                ));
                            };
                            ensure(
                                same_symbol(
                                    &original_atom.relation,
                                    &source.rules.first.query.head.relation,
                                ),
                                &*atom,
                                "delta reads a different source atom",
                            )?;
                            atom.relation = original_atom.relation.clone();
                        }
                    }
                }
            }
            let mut declarations: Vec<_> = target
                .declarations
                .iter()
                .filter(|d| !same_symbol(&d.name, &source.rules.first.query.head.relation))
                .cloned()
                .collect();
            ensure(
                erased.declarations == declarations,
                &erased,
                "round input schema changed",
            )?;
            declarations = source.rules.first.declarations.clone();
            erased.declarations = declarations;
            erased.query.head.relation = original.head.relation.clone();
            let mut expected = source.rules.first.clone();
            expected.query = (*original).clone();
            ensure(erased == expected, hole, "round rule changed")?;
        }
    }
    ensure(
        positions.len() == queries.len(),
        source,
        "missing round rule",
    )
}
pub fn render(context: &Context, pipeline: &cq_stage::Pipeline) -> syn::Result<TokenStream> {
    let leaves = &context.leaves;
    let delta = &context.delta;
    let mut fresh = Fresh::new(
        context
            .seeds
            .iter()
            .chain(&context.steps)
            .map(|(_, hole)| hole.to_token_stream())
            .collect::<TokenStream>(),
    );
    let seen = fresh.id("fixpoint_seen");
    let all = fresh.id("fixpoint_rows");
    let next = fresh.id("next_delta");
    let round = fresh.id("round");
    let observer = fresh.id("observer");
    let seed_runs = context
        .seeds
        .iter()
        .map(|(_, hole)| {
            let invocation = pipeline.clone().start(hole.to_token_stream())?;
            Ok(quote!(
                for row in #invocation {
                    if #seen.insert(::core::clone::Clone::clone(&row)) { #all.push(row); }
                }
            ))
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let step_runs = context
        .steps
        .iter()
        .map(|(_, hole)| {
            let invocation = pipeline.clone().start(hole.to_token_stream())?;
            Ok(quote!(
                for row in #invocation {
                    if #seen.insert(::core::clone::Clone::clone(&row)) {
                        #all.push(::core::clone::Clone::clone(&row)); #next.push(row);
                    }
                }
            ))
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let observe_binding = context.observer.as_ref().map(|expression| {
        quote!(#[allow(unused_mut)] let mut #observer = #expression;)
    });
    let observe_call = context
        .observer
        .as_ref()
        .map(|_| quote!((#observer)(#round, #delta.len());));
    Ok(quote!({
        #(#leaves)* #observe_binding
        let mut #seen = ::std::collections::HashSet::new();
        let mut #all = ::std::vec::Vec::new(); #(#seed_runs)*
        let mut #delta = ::cq_storage::Answers::collect_from(#all.iter().cloned());
        let mut #round = 0usize;
        while !#delta.is_empty() {
            #observe_call
            let mut #next = ::std::vec::Vec::new(); #(#step_runs)*
            #delta = ::cq_storage::Answers::collect_from(#next);
            #round += 1;
        }
        #all.into_iter()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_result_aliases_erase_to_each_rules_authored_spelling() {
        let source: Source = syn::parse_quote! {
            struct Raw; relation Seed(x:i32); relation E(x:i32,y:i32); relation r#reach(x:i32);
            reach(x) :- Seed(x); r#reach(y) :- r#reach(x), E(x,y);
        };
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
    }
    fn source() -> Source {
        syn::parse_quote! {
            struct Reach; relation E(a:i32,b:i32); relation reach(a:i32,b:i32);
            reach(x,y) :- E(x,y); reach(x,z) :- reach(x,y), E(y,z);
        }
    }
    #[test]
    fn testkit_checks_delta_correspondence() {
        cq_testkit::PassCase {
            source: source(),
            expected_next: None,
            compile: Box::new(compile),
            check: Box::new(check),
            mutations: vec![Box::new(|context: &Context| {
                let mut changed = context.clone();
                changed.steps[0].1.declarations[0].value = None;
                changed
            })],
            answers: None,
        }
        .run(None)
        .unwrap();
    }
    #[test]
    fn correspondence_rejects_partition_delta_and_rule_mutations() {
        let source = source();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        assert!(format!("{target:?}").contains("Borrowed"));
        let mut changed = target.clone();
        changed.answers.pop();
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.answers[0].pop();
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.answers[0][0] = AnswerColumn::Owned;
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.seeds.clear();
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.steps[0].0 = 0;
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.steps[0].1.declarations[0].value = None;
        assert!(check(&source, &changed).is_err());
        let mut changed = target;
        changed.steps[0].1.query.head.args[0] = syn::parse_quote!(*z);
        assert!(check(&source, &changed).is_err());
    }
    #[test]
    fn joined_generator_binding_is_owned_but_still_active_domain() {
        let source: Source = syn::parse_quote! {
            struct R; relation E(x:i32); relation reach(x:i32);
            reach(x) :- for x in [1], E(x);
            reach(x) :- reach(x);
        };
        let target = compile(&source).unwrap();
        assert_eq!(target.answers, vec![vec![AnswerColumn::Owned], vec![AnswerColumn::Borrowed]]);
        check(&source, &target).unwrap();
        let mut changed = target;
        changed.answers[0][0] = AnswerColumn::Borrowed;
        assert!(check(&source, &changed).is_err());
    }
    #[test]
    fn nonlinear_and_function_generating_rules_are_rejected() {
        for recursive in [
            "reach(x,z) :- reach(x,y), reach(y,z);",
            "reach(*x + 1,z) :- reach(x,y), E(y,z);",
            "reach(*x,z) :- reach(x,y), E(y,z);",
            "reach((*x).clone(),z) :- reach(x,y), E(y,z);",
            "reach(x.clone(),z) :- reach(x,y), E(y,z);",
        ] {
            let source = syn::parse_str(&format!("struct R; relation E(a:i32,b:i32); relation reach(a:i32,b:i32); reach(x,y) :- E(x,y); {recursive}")).unwrap();
            assert!(compile(&source).is_err());
        }
    }
}
