use cq_datalog_support::Fresh;
use cq_lang_datalog_source::{self as source, BodyItem};
use cq_syntax::{rust, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;

pub fn emit(program: &source::Program) -> syn::Result<TokenStream> {
    let analysis = source::contract::check(program)?;
    let mut fresh = Fresh::new(program);
    let state = fresh.id("reference_state");
    let changed = fresh.id("reference_changed");
    let component = fresh.id("reference_component");
    // Least signed dependency ranks, not the production SCC stratifier. All
    // positive recursion at one rank is solved together; negative inputs freeze.
    let mut ranks = vec![0; program.declarations.len()];
    loop {
        let mut changed = false;
        for &(head, input, negative) in &analysis.dependencies {
            let required = ranks[input] + usize::from(negative);
            if ranks[head] < required {
                ranks[head] = required;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let count = ranks.iter().copied().max().unwrap_or(0) + 1;
    let initial = program
        .declarations
        .iter()
        .map(|declaration| {
            let types = declaration.columns.iter().map(|c| &c.ty);
            let tuple = quote!((#(#types,)*));
            if let Some(value) = &declaration.value {
                let value = &value.expression;
                let row = fresh.id("reference_input");
                quote!(::std::iter::IntoIterator::into_iter(#value)
                .map(|#row| ::std::borrow::Borrow::<#tuple>::borrow(&#row).clone())
                .collect::<::std::collections::HashSet<#tuple>>())
            } else {
                quote!(::std::collections::HashSet::<#tuple>::new())
            }
        })
        .collect::<Vec<_>>();
    let mut arms = Vec::new();
    for rank in 0..count {
        let members: Vec<_> = program
            .declarations
            .iter()
            .enumerate()
            .filter(|(i, d)| d.value.is_none() && ranks[*i] == rank)
            .collect();
        let candidates: BTreeMap<_, _> = members
            .iter()
            .map(|(i, _)| (*i, fresh.id("reference_candidates")))
            .collect();
        let declarations = members.iter().map(|(i, d)| {
            let name = &candidates[i];
            let types = d.columns.iter().map(|c| &c.ty);
            quote!(let mut #name = ::std::collections::HashSet::<(#(#types,)*)>::new();)
        });
        let mut rules = Vec::new();
        for rule in &program.rules {
            let head = analysis.positions[&symbol_name(&rule.head.relation)];
            if ranks[head] != rank {
                continue;
            }
            rules.push(body(
                &rule.body.iter().collect::<Vec<_>>(),
                &rule.head.args.iter().collect::<Vec<_>>(),
                &candidates[&head],
                &state,
                &analysis.positions,
                BTreeMap::new(),
                &mut fresh,
            )?);
        }
        let publication = members.iter().map(|(i, _)| {
            let field = syn::Index::from(*i);
            let candidates = &candidates[i];
            quote!(for row in #candidates { #changed |= #state.#field.insert(row); })
        });
        arms.push(quote!(#rank => {
            #(#declarations)*
            #(#rules)*
            let mut #changed = false;
            #(#publication)*
            #changed
        }));
    }
    let outputs = program
        .outputs
        .iter()
        .map(|name| {
            let field = syn::Index::from(analysis.positions[&symbol_name(name)]);
            quote!(#state.#field)
        })
        .collect::<Vec<_>>();
    let answer = if outputs.len() == 1 {
        outputs[0].clone()
    } else {
        quote!((#(#outputs,)*))
    };
    Ok(quote!({
        let #state = ::cq_datalog_testkit::eval_source(
            || (#(#initial,)*), #count,
            |#state, #component| match #component {
                #(#arms,)*
                _ => unreachable!("reference stratum index"),
            },
        );
        #answer
    }))
}

// Scope values record whether a binding is a borrowed atom column (true) or
// native Rust let/for value (false). No expression is interpreted as text.
fn body(
    items: &[&BodyItem],
    heads: &[&syn::Expr],
    candidates: &syn::Ident,
    state: &syn::Ident,
    positions: &BTreeMap<String, usize>,
    mut scope: BTreeMap<String, bool>,
    fresh: &mut Fresh,
) -> syn::Result<TokenStream> {
    let Some((item, rest)) = items.split_first() else {
        let values = heads.iter().map(|value| {
            if rust::variable(value).is_some_and(|v| scope.get(&symbol_name(v)) == Some(&true)) {
                quote!((*(#value)).clone())
            } else {
                quote!((#value).clone())
            }
        });
        return Ok(quote!(#candidates.insert((#(#values,)*));));
    };
    match item {
        BodyItem::Positive { atom } => {
            let field = syn::Index::from(positions[&symbol_name(&atom.relation)]);
            let row = fresh.id("reference_row");
            let mut prefix = Vec::new();
            for (column, argument) in atom.args.iter().enumerate() {
                let column = syn::Index::from(column);
                if matches!(rust::unparen(argument), syn::Expr::Infer(_)) {
                    continue;
                }
                if let Some(variable) = rust::variable(argument) {
                    match scope.get(&symbol_name(variable)) {
                        Some(true) => {
                            prefix.push(quote!(if &#row.#column != #variable { continue; }))
                        }
                        Some(false) => {
                            prefix.push(quote!(if #row.#column != #variable { continue; }))
                        }
                        None => {
                            prefix.push(quote!(let #variable = &#row.#column;));
                            scope.insert(symbol_name(variable), true);
                        }
                    }
                } else {
                    prefix.push(quote!(if #row.#column != (#argument) { continue; }));
                }
            }
            let next = body(rest, heads, candidates, state, positions, scope, fresh)?;
            Ok(quote!(for #row in &#state.#field { #(#prefix)* #next }))
        }
        BodyItem::Negation(negative) => {
            let atom = &negative.atom;
            let field = syn::Index::from(positions[&symbol_name(&atom.relation)]);
            let row = fresh.id("reference_negative");
            let conditions = atom
                .args
                .iter()
                .enumerate()
                .filter_map(|(column, argument)| {
                    if matches!(rust::unparen(argument), syn::Expr::Infer(_)) {
                        return None;
                    }
                    let column = syn::Index::from(column);
                    Some(
                        if rust::variable(argument)
                            .is_some_and(|v| scope.get(&symbol_name(v)) == Some(&true))
                        {
                            quote!(&#row.#column == #argument)
                        } else {
                            quote!(#row.#column == (#argument))
                        },
                    )
                })
                .collect::<Vec<_>>();
            let next = body(rest, heads, candidates, state, positions, scope, fresh)?;
            Ok(quote!(if !#state.#field.iter().any(|#row| true #(&& (#conditions))*) { #next }))
        }
        BodyItem::Filter(filter) => {
            let condition = &filter.condition;
            let next = body(rest, heads, candidates, state, positions, scope, fresh)?;
            Ok(quote!(if #condition { #next }))
        }
        BodyItem::Let(binding) => {
            for name in rust::bindings(&binding.pattern) {
                scope.insert(symbol_name(&name), false);
            }
            let next = body(rest, heads, candidates, state, positions, scope, fresh)?;
            Ok(quote!({ #binding; #next }))
        }
        BodyItem::For(generator) => {
            for name in rust::bindings(&generator.pattern) {
                scope.insert(symbol_name(&name), false);
            }
            let next = body(rest, heads, candidates, state, positions, scope, fresh)?;
            Ok(quote!(#generator { #next }))
        }
        BodyItem::Equality(equality) => {
            let condition = constraint(&cq_syntax::Constraint::from_equality(equality), &scope);
            let next = body(rest, heads, candidates, state, positions, scope, fresh)?;
            Ok(quote!(if #condition { #next }))
        }
        BodyItem::Constraint(constraint) => {
            let condition = self::constraint(constraint, &scope);
            let next = body(rest, heads, candidates, state, positions, scope, fresh)?;
            Ok(quote!(if #condition { #next }))
        }
        _ => Err(syn::Error::new_spanned(
            item,
            "unsupported DL-Source clause",
        )),
    }
}

// Query parsing recognizes total guards and strips reference syntax from their
// relational terms. Restore a uniform borrowed comparison here, rather than
// treating cq_tokens() (the logical wire spelling) as executable Rust.
fn constraint(value: &cq_syntax::Constraint, scope: &BTreeMap<String, bool>) -> TokenStream {
    use cq_syntax::constraint::{Comparison, Constraint, Term};
    let term = |value: &Term| match value {
        Term::Variable(name) if scope.get(&symbol_name(name)) == Some(&true) => quote!(#name),
        Term::Variable(name) | Term::Parameter(name) => quote!(&#name),
        Term::Literal(value) => quote!(&(#value)),
    };
    match value {
        Constraint::Compare {
            left,
            comparison,
            right,
        } => {
            let left = term(left);
            let right = term(right);
            let operator = match comparison {
                Comparison::Eq(_) => quote!(==),
                _ => quote!(#comparison),
            };
            quote!((#left) #operator (#right))
        }
        Constraint::Between {
            term: value,
            low,
            high,
        } => {
            let value = term(value);
            let low = term(low);
            let high = term(high);
            quote!((#value) >= (#low) && (#value) <= (#high))
        }
        Constraint::In {
            term: value,
            literals,
        } => {
            let value = term(value);
            let literals = literals.iter();
            quote!(false #(|| (#value) == &(#literals))*)
        }
        Constraint::Or(branches) => {
            let branches = branches.iter().map(|branch| {
                let terms = branch.iter().map(|value| constraint(value, scope));
                quote!(true #(&& (#terms))*)
            });
            quote!(false #(|| (#branches))*)
        }
        Constraint::Total { expression, .. } => quote!(#expression),
    }
}
