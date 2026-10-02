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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub declarations: Vec<RelationDecl>,
    pub leaves: Vec<Leaf>,
    pub delta: Ident,
    pub seeds: Vec<(usize, Program)>,
    pub steps: Vec<(usize, Program)>,
    pub observer: Option<Expr>,
}
fn recursion_count(query: &Query, result: &Ident) -> usize {
    query
        .body
        .iter()
        .filter(|item| matches!(item, BodyItem::Positive { atom } if same_symbol(&atom.relation,result)))
        .count()
}
fn unparen(expression: &Expr) -> &Expr {
    match expression {
        Expr::Paren(p) => unparen(&p.expr),
        Expr::Group(g) => unparen(&g.expr),
        _ => expression,
    }
}
fn active_column(expression: &Expr) -> Option<&Ident> {
    match unparen(expression) {
        Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)) => {
            rust::variable(unparen(&unary.expr))
        }
        Expr::MethodCall(call)
            if call.method == "clone" && call.args.is_empty() && call.turbofish.is_none() =>
        {
            match unparen(&call.receiver) {
                Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)) => {
                    rust::variable(unparen(&unary.expr))
                }
                _ => None,
            }
        }
        _ => None,
    }
}
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
            let id = active_column(argument).ok_or_else(|| {
                syn::Error::new_spanned(
                    argument,
                    "recursive heads must copy an active-domain column: *x or (*x).clone()",
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
    for (position, query) in source.rules.queries().enumerate() {
        let recursive = recursion_count(query, &source.rules.first.query.head.relation) == 1;
        let mut query = query.clone();
        let name = fresh.id("round_rows");
        query.head.relation = name.clone();
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
        observer: source.observer.clone(),
    })
}
/// Erase delta renames to result reads, verify the rule partition, schema and
/// observer, and require an active-domain, linear source. Fixpoint execution
/// and ownership are validated by runtime oracles and Rust.
pub fn check(source: &Source, target: &Context) -> syn::Result<()> {
    domain(source)?;
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
        let expression = unparen(expression);
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
            reach(*x) :- Seed(x); r#reach(*y) :- r#reach(x), E(x,y);
        };
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
    }
    fn source() -> Source {
        syn::parse_quote! {
            struct Reach; relation E(a:i32,b:i32); relation reach(a:i32,b:i32);
            reach(*x,*y) :- E(x,y); reach(*x,*z) :- reach(x,y), E(y,z);
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
    fn nonlinear_and_function_generating_rules_are_rejected() {
        for recursive in [
            "reach(*x,*z) :- reach(x,y), reach(y,z);",
            "reach(*x + 1,*z) :- reach(x,y), E(y,z);",
            "reach(x,z) :- reach(x,y), E(y,z);",
        ] {
            let source = syn::parse_str(&format!("struct R; relation E(a:i32,b:i32); relation reach(a:i32,b:i32); reach(*x,*y) :- E(x,y); {recursive}")).unwrap();
            assert!(compile(&source).is_err());
        }
    }
}
