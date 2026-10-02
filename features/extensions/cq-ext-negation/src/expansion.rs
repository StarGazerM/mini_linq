use std::collections::{BTreeMap, BTreeSet};

use cq_ext_support::{
    Fresh, Leaf, bound, check_fresh, check_leaves, distinct, ensure, hoist, same_symbol, with_query,
};
use cq_lang_cq::{BodyItem, Program};
use cq_syntax::{Atom, RelationDecl, rust, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Token, parse::Parse};

mod kw {
    syn::custom_keyword!(depends);
}
#[derive(Clone)]
struct Negative {
    atom: Atom,
    edges: Vec<(Ident, Ident)>,
}
impl Parse for Negative {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let atom = input.parse()?;
        let mut edges = Vec::new();
        if !input.is_empty() {
            input.parse::<Token![;]>()?;
            if !input.is_empty() {
                input.parse::<kw::depends>()?;
                let graph;
                syn::braced!(graph in input);
                while !graph.is_empty() {
                    let from = graph.parse()?;
                    graph.parse::<Token![->]>()?;
                    let to = graph.parse()?;
                    graph.parse::<Token![;]>()?;
                    edges.push((from, to));
                }
            }
        }
        Ok(Self { atom, edges })
    }
}

#[derive(Clone)]
pub struct Anti {
    pub position: usize,
    pub binding: Ident,
    pub keys: Vec<Ident>,
    pub hole: Program,
}
#[derive(Clone)]
pub struct Context {
    pub main: Program,
    pub leaves: Vec<Leaf>,
    pub negatives: Vec<Anti>,
}

fn clauses(source: &Program) -> syn::Result<Vec<(usize, Negative, Vec<Ident>)>> {
    let mut scope = BTreeSet::new();
    let mut negatives = Vec::new();
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (position, item) in source.query.body.iter().enumerate() {
        if let BodyItem::Extension { name, tokens } = item {
            if name == "neg" {
                let negative: Negative = syn::parse2(tokens.clone())?;
                let declaration = source
                    .inputs()
                    .find(|d| same_symbol(&d.name, &negative.atom.relation))
                    .ok_or_else(|| {
                        syn::Error::new_spanned(
                            &negative.atom,
                            "negated relation must be a declared input, not the query result",
                        )
                    })?;
                ensure(
                    declaration.arity() == negative.atom.args.len(),
                    &negative.atom,
                    "negative atom arity mismatch",
                )?;
                let mut keys = Vec::new();
                for argument in &negative.atom.args {
                    if matches!(argument, syn::Expr::Infer(_)) {
                        continue;
                    }
                    let id = rust::variable(argument).ok_or_else(|| {
                        syn::Error::new_spanned(
                            argument,
                            "neg arguments must be bound variables or _",
                        )
                    })?;
                    ensure(
                        scope.contains(&symbol_name(id)),
                        id,
                        "negation variable is not bound before the clause",
                    )?;
                    if !keys.iter().any(|key| same_symbol(key, id)) {
                        keys.push(id.clone());
                    }
                }
                for (from, to) in &negative.edges {
                    graph
                        .entry(symbol_name(from))
                        .or_default()
                        .push(symbol_name(to));
                }
                negatives.push((position, negative, keys));
            }
        }
        scope.extend(bound(item).iter().map(symbol_name));
    }
    for (_, negative, _) in &negatives {
        let mut pending = vec![symbol_name(&negative.atom.relation)];
        let mut seen = BTreeSet::new();
        while let Some(node) = pending.pop() {
            ensure(
                node != symbol_name(&source.query.head.relation),
                &negative.atom,
                "unstratified negation: relation depends on the query itself",
            )?;
            if seen.insert(node.clone()) {
                pending.extend(graph.get(&node).into_iter().flatten().cloned());
            }
        }
    }
    Ok(negatives)
}

fn inner(
    source: &Program,
    declarations: &[RelationDecl],
    negative: &Negative,
    keys: &[Ident],
    name: &Ident,
    fresh: &mut Fresh,
) -> Program {
    let mut atom = negative.atom.clone();
    for argument in &mut atom.args {
        if matches!(argument, syn::Expr::Infer(_)) {
            let id = fresh.id("wildcard");
            *argument = syn::parse_quote!(#id);
        }
    }
    let (head, output): (Atom, Option<RelationDecl>) = if keys.is_empty() {
        (
            syn::parse_quote!(#name(())),
            Some(syn::parse_quote!(relation #name(unit:());)),
        )
    } else {
        (syn::parse_quote!(#name(#(#keys),*)), None)
    };
    let query = syn::parse_quote!(#head :- #atom;);
    with_query(source, declarations, query, output)
}
fn guard(anti: &Anti) -> BodyItem {
    let Anti { binding, keys, .. } = anti;
    syn::parse_quote!(if !::cq_storage::ExactMembership::contains(&#binding, &(#(#keys,)*)))
}

pub fn compile(source: &Program) -> syn::Result<Context> {
    let clauses = clauses(source)?;
    let mut fresh = Fresh::new(source);
    let (declarations, leaves) = hoist(source, &mut fresh);
    let mut main = source.clone();
    main.declarations = declarations.clone();
    let mut negatives = Vec::new();
    for (position, negative, keys) in clauses {
        let binding = fresh.id("negative");
        let name = fresh.id("negative_rows");
        let hole = inner(source, &declarations, &negative, &keys, &name, &mut fresh);
        let anti = Anti {
            position,
            binding,
            keys,
            hole,
        };
        main.query.body[position] = guard(&anti);
        negatives.push(anti);
    }
    Ok(Context {
        main,
        leaves,
        negatives,
    })
}

/// Erase each membership guard back to its source clause and verify the
/// projected negative atoms, wildcard binding, leaf sharing, and dependency DAG.
#[cfg_attr(not(test), allow(dead_code))]
pub fn check(source: &Program, target: &Context) -> syn::Result<()> {
    let clauses = clauses(source)?;
    check_fresh(
        source,
        target
            .leaves
            .iter()
            .map(|leaf| &leaf.binding)
            .chain(target.negatives.iter().map(|anti| &anti.binding)),
    )?;
    check_leaves(source, &target.main.declarations, &target.leaves)?;
    ensure(
        clauses.len() == target.negatives.len(),
        source,
        "negative hole count changed",
    )?;
    let mut erased = target.main.clone();
    erased.declarations = source.declarations.clone();
    for ((position, negative, keys), anti) in clauses.iter().zip(&target.negatives) {
        ensure(
            anti.position == *position && anti.keys == *keys,
            source,
            "negative key or position changed",
        )?;
        ensure(
            target.main.query.body.get(*position) == Some(&guard(anti)),
            source,
            "negative membership guard changed",
        )?;
        let atoms: Vec<_> = anti
            .hole
            .query
            .body
            .iter()
            .filter_map(|item| {
                if let BodyItem::Positive { atom } = item {
                    Some(atom)
                } else {
                    None
                }
            })
            .collect();
        ensure(
            atoms.len() == 1 && anti.hole.query.body.len() == 1,
            &anti.hole,
            "negative hole must contain exactly its atom",
        )?;
        let atom = atoms[0];
        ensure(
            atom.relation == negative.atom.relation && atom.args.len() == negative.atom.args.len(),
            atom,
            "negative atom changed",
        )?;
        let mut wildcards = BTreeSet::new();
        for (old, new) in negative.atom.args.iter().zip(&atom.args) {
            if matches!(old, syn::Expr::Infer(_)) {
                let id = rust::variable(new).ok_or_else(|| {
                    syn::Error::new_spanned(new, "wildcard must bind a fresh variable")
                })?;
                ensure(
                    !keys.contains(id) && wildcards.insert(symbol_name(id)),
                    id,
                    "wildcard aliases a key or another wildcard",
                )?;
            } else {
                ensure(old == new, new, "negative argument changed")?;
            }
        }
        let name = &anti.hole.query.head.relation;
        let wildcard_ids: Vec<_> = atom
            .args
            .iter()
            .zip(&negative.atom.args)
            .filter_map(|(new, old)| {
                matches!(old, syn::Expr::Infer(_))
                    .then(|| rust::variable(new))
                    .flatten()
            })
            .collect();
        check_fresh(
            source,
            wildcard_ids.into_iter().chain(std::iter::once(name)),
        )?;
        let expected_head: Atom = if keys.is_empty() {
            syn::parse_quote!(#name(()))
        } else {
            syn::parse_quote!(#name(#(#keys),*))
        };
        ensure(
            anti.hole.query.head == expected_head,
            &anti.hole,
            "negative projection changed",
        )?;
        let output = if keys.is_empty() {
            Some(syn::parse_quote!(relation #name(unit:());))
        } else {
            None
        };
        let expected = with_query(
            source,
            &target.main.declarations,
            anti.hole.query.clone(),
            output,
        );
        ensure(
            anti.hole == expected,
            &anti.hole,
            "negative schema or header changed",
        )?;
        erased.query.body[*position] = source.query.body[*position].clone();
    }
    ensure(
        erased == *source,
        &target.main,
        "main query changed outside negative clauses",
    )
}

pub fn render(context: &Context) -> TokenStream {
    let leaves = &context.leaves;
    let initializations = context.negatives.iter().map(|anti| {
        let Anti { binding, hole, keys, .. } = anti;
        if keys.is_empty() { quote!(let #binding = ::cq_storage::GroupedAnswers::collect_from(mini_linq::negation_hole! { #hole }.map(|(_unit,)| ((), ())));) }
        else { quote!(let #binding = ::cq_storage::GroupedAnswers::collect_from(mini_linq::negation_hole! { #hole }.map(|row| (row, ())));) }
    });
    let main = &context.main;
    let answers = distinct(
        quote!(mini_linq::negation_hole! { #main }),
        &mut Fresh::new(main),
        "negative_answers",
    );
    // A surface extension may itself own leaves; close their lifetimes here.
    quote!({ #(#leaves)* #(#initializations)* #answers.collect::<::std::vec::Vec<_>>().into_iter() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliasing_negative_hosts_is_rejected_even_when_both_guards_match() {
        let source: Program = syn::parse_quote! {
            struct N; relation A(x:i32); relation B(x:i32); relation C(x:i32);
            answer(x) :- A(x), neg!{B(x)}, neg!{C(x)};
        };
        let mut target = compile(&source).unwrap();
        target.negatives[1].binding = target.negatives[0].binding.clone();
        target.main.query.body[2] = guard(&target.negatives[1]);
        assert!(check(&source, &target).is_err());
    }
    #[test]
    fn correspondence_checks_anti_projection_guard_and_wildcards() {
        let source: Program = syn::parse_quote! { struct N; relation R(x:i32); relation S(x:i32,y:i32); answer(x) :- R(x), neg!{ S(x, _) }; };
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        let mut changed = target.clone();
        changed.main.query.body[1] = syn::parse_quote!(if true);
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.negatives[0].keys.clear();
        assert!(check(&source, &changed).is_err());
        let mut changed = target.clone();
        changed.negatives[0].hole.query.head = syn::parse_quote!(wrong(1));
        assert!(check(&source, &changed).is_err());
        let mut changed = target;
        changed.negatives.clear();
        assert!(check(&source, &changed).is_err());
    }
    #[test]
    fn unsafe_and_transitively_unstratified_negation_are_rejected() {
        for clause in [
            "S(y, _)",
            "S(x, _); depends { S -> T; T -> answer; }",
            "answer(x)",
        ] {
            let source = syn::parse_str(&format!("struct N; relation R(x:i32); relation S(x:i32,y:i32); relation answer(x:i32); answer(*x) :- R(x), neg!{{ {clause} }};")).unwrap();
            assert!(compile(&source).is_err());
        }
        let source: Program = syn::parse_quote! { struct N; relation R(x:i32); relation S(x:i32,y:i32); answer(x) :- R(x), neg!{ S(_, _); depends { S -> T; T -> S; } }; };
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
    }
}
