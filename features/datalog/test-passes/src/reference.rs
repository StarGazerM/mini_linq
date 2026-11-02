use cq_datalog_support::Fresh;
use cq_lang_datalog_source::{self as source, BodyItem};
use cq_syntax::{rust, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;

pub fn emit(program: &source::Program) -> syn::Result<TokenStream> {
    let analysis = source::contract::check(program)?;
    let mut ranks = vec![0; program.declarations.len()];
    // Least signed dependency ranks, not the production SCC stratifier. All
    // positive recursion at one rank is solved together; negative inputs freeze.
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
    emit_ranked(program, &ranks)
}

pub fn emit_strata(program: &cq_lang_datalog_strata::Program) -> syn::Result<TokenStream> {
    let analysis = cq_lang_datalog_strata::contract::check(program)?;
    let mut ranks = vec![0; program.source.declarations.len()];
    for (rank, members) in analysis.components.iter().enumerate() {
        for &member in members {
            ranks[member] = rank;
        }
    }
    emit_ranked(&program.source, &ranks)
}

fn emit_ranked(program: &source::Program, ranks: &[usize]) -> syn::Result<TokenStream> {
    let analysis = source::contract::check(program)?;
    let mut fresh = Fresh::new(program);
    let state = fresh.id("reference_state");
    let changed = fresh.id("reference_changed");
    let component = fresh.id("reference_component");
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
        let domains = domains(&program.declarations);
        for rule in &program.rules {
            let head = analysis.positions[&symbol_name(&rule.head.relation)];
            if ranks[head] != rank {
                continue;
            }
            rules.push(body(
                &rule.body.iter().collect::<Vec<_>>(),
                &rule.head.args.iter().collect::<Vec<_>>(),
                &candidates[&head],
                Inputs {
                    state: &state,
                    positions: &analysis.positions,
                    domains: &domains,
                    borrowed_negative_inputs: &[],
                },
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

/// Evaluate a recorded CQ query over borrowed, typed input sets. Reference
/// outputs (negation-key projections) retain the input column borrow instead
/// of cloning its referent. The ordinary source/Incremental path stays owned.
pub(crate) fn emit_query(
    query: &source::Query,
    state: &syn::Ident,
    positions: &BTreeMap<String, usize>,
    domains: &Domains,
    candidates: &syn::Ident,
    borrowed_heads: &[bool],
    borrowed_negative_inputs: &[String],
    fresh: &mut Fresh,
) -> syn::Result<TokenStream> {
    let heads = query
        .head
        .args
        .iter()
        .zip(borrowed_heads)
        .map(|(value, borrowed)| {
            if *borrowed && rust::variable(value).is_some() {
                syn::parse_quote!(&*(#value))
            } else {
                value.clone()
            }
        })
        .collect::<Vec<syn::Expr>>();
    body(
        &query.body.iter().collect::<Vec<_>>(),
        &heads.iter().collect::<Vec<_>>(),
        candidates,
        Inputs {
            state,
            positions,
            domains,
            borrowed_negative_inputs,
        },
        BTreeMap::new(),
        fresh,
    )
}

#[derive(Clone, Copy)]
pub(crate) struct Inputs<'a> {
    pub state: &'a syn::Ident,
    pub positions: &'a BTreeMap<String, usize>,
    pub domains: &'a Domains,
    pub borrowed_negative_inputs: &'a [String],
}

/// The authored code domain (`col: T in (D)`) of every input column, keyed
/// like `Inputs::positions`.
pub(crate) type Domains = BTreeMap<String, Vec<Option<syn::Expr>>>;

pub(crate) fn domains<'d>(
    declarations: impl IntoIterator<Item = &'d cq_syntax::RelationDecl>,
) -> Domains {
    declarations
        .into_iter()
        .map(|declaration| {
            let columns = declaration.columns.iter();
            let columns = columns.map(|c| c.domain.as_ref().map(|d| d.expression.clone()));
            (symbol_name(&declaration.name), columns.collect())
        })
        .collect()
}

/// The domain each body variable's columns store. Reference evaluation
/// translates every closed operand compared with such a variable or column
/// through the authored domain (the A-Domain table, evaluated directly).
fn variable_domains(items: &[&BodyItem], domains: &Domains) -> BTreeMap<String, syn::Expr> {
    let mut variables = BTreeMap::new();
    for item in items {
        let atom = match item {
            BodyItem::Positive { atom } => atom,
            BodyItem::Negation(negative) => &negative.atom,
            _ => continue,
        };
        let Some(columns) = domains.get(&symbol_name(&atom.relation)) else {
            continue;
        };
        for (argument, domain) in atom.args.iter().zip(columns) {
            if let (Some(variable), Some(domain)) = (rust::variable(argument), domain) {
                variables.insert(symbol_name(variable), domain.clone());
            }
        }
    }
    variables
}

fn translate(function: &str, domain: Option<&syn::Expr>, value: TokenStream) -> TokenStream {
    let function = syn::Ident::new(function, proc_macro2::Span::call_site());
    match domain {
        Some(domain) => quote!(::cq_storage::domain::#function(&(#domain), &(#value))),
        None => value,
    }
}

/// Reference evaluation follows A-Capture: closed top-level lets, guards and
/// head operands are evaluated once, before every loop of the rule.
pub(crate) fn body(
    items: &[&BodyItem],
    heads: &[&syn::Expr],
    candidates: &syn::Ident,
    inputs: Inputs<'_>,
    scope: BTreeMap<String, bool>,
    fresh: &mut Fresh,
) -> syn::Result<TokenStream> {
    let list: cq_syntax::CommaList<BodyItem> = items.iter().map(|item| (*item).clone()).collect();
    let positions = cq_lang_cq::normal::capture_positions(&list);
    let rest: cq_syntax::CommaList<BodyItem> = list
        .iter()
        .enumerate()
        .filter(|(index, _)| !positions.contains(index))
        .map(|(_, item)| item.clone())
        .collect();
    let logical: std::collections::BTreeSet<String> =
        cq_lang_cq::blocks::bound(&rest).into_keys().collect();
    let named = cq_lang_cq::blocks::bound(&list);
    // A closed head operand is bound once after the captured lets it may read.
    let mut hoisted = Vec::new();
    let mut owned = Vec::new();
    for value in heads {
        let named = rust::variable(value).is_some_and(|v| {
            named.contains_key(&symbol_name(v)) || scope.contains_key(&symbol_name(v))
        });
        if !named
            && rust::free_variables(value)
                .dependencies(&logical)
                .is_empty()
        {
            let id = fresh.id("reference_capture");
            hoisted.push(BodyItem::Let(Box::new(
                syn::parse_quote!(let #id = ::std::clone::Clone::clone(&(#value))),
            )));
            owned.push(syn::parse_quote!(#id));
        } else {
            owned.push((*value).clone());
        }
    }
    let ordered: Vec<&BodyItem> = positions
        .iter()
        .map(|&position| items[position])
        .chain(&hoisted)
        .chain(
            items
                .iter()
                .enumerate()
                .filter(|(index, _)| !positions.contains(index))
                .map(|(_, item)| *item),
        )
        .collect();
    let heads: Vec<&syn::Expr> = owned.iter().collect();
    let variables = variable_domains(&ordered, inputs.domains);
    nested(
        &ordered, &heads, candidates, inputs, &variables, scope, fresh,
    )
}

// Scope values record borrowed atom/generator columns (true) versus native
// scalar let values (false). No expression is interpreted as text.
fn nested(
    items: &[&BodyItem],
    heads: &[&syn::Expr],
    candidates: &syn::Ident,
    inputs: Inputs<'_>,
    variables: &BTreeMap<String, syn::Expr>,
    mut scope: BTreeMap<String, bool>,
    fresh: &mut Fresh,
) -> syn::Result<TokenStream> {
    let Inputs {
        state,
        positions,
        domains,
        borrowed_negative_inputs,
    } = inputs;
    let Some((item, rest)) = items.split_first() else {
        let values = heads.iter().map(|value| {
            if rust::variable(value).is_some_and(|v| scope.get(&symbol_name(v)) == Some(&true)) {
                quote!((*(#value)).clone())
            } else {
                quote!(::std::clone::Clone::clone(&(#value)))
            }
        });
        return Ok(quote!(#candidates.insert((#(#values,)*));));
    };
    match item {
        BodyItem::Positive { atom } => {
            let field = syn::Index::from(positions[&symbol_name(&atom.relation)]);
            let columns = &domains[&symbol_name(&atom.relation)];
            let row = fresh.id("reference_row");
            let mut prefix = Vec::new();
            for (column, argument) in atom.args.iter().enumerate() {
                let domain = columns[column].as_ref();
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
                    let argument = translate("exact", domain, quote!(#argument));
                    prefix.push(quote!(if #row.#column != (#argument) { continue; }));
                }
            }
            let next = nested(rest, heads, candidates, inputs, variables, scope, fresh)?;
            Ok(quote!(for #row in #state.#field.iter() { #(#prefix)* #next }))
        }
        BodyItem::Negation(negative) => {
            let atom = &negative.atom;
            let field = syn::Index::from(positions[&symbol_name(&atom.relation)]);
            let columns = &domains[&symbol_name(&atom.relation)];
            let row = fresh.id("reference_negative");
            let borrowed = borrowed_negative_inputs.contains(&symbol_name(&atom.relation));
            let conditions = atom
                .args
                .iter()
                .enumerate()
                .filter_map(|(column, argument)| {
                    if matches!(rust::unparen(argument), syn::Expr::Infer(_)) {
                        return None;
                    }
                    let domain = columns[column].as_ref();
                    let column = syn::Index::from(column);
                    let value = if borrowed {
                        quote!(*#row.#column)
                    } else {
                        quote!(#row.#column)
                    };
                    let bound = rust::variable(argument).and_then(|v| scope.get(&symbol_name(v)));
                    Some(match bound {
                        Some(true) => quote!(&(#value) == #argument),
                        Some(false) => quote!((#value) == (#argument)),
                        None => {
                            let argument = translate("exact", domain, quote!(#argument));
                            quote!((#value) == (#argument))
                        }
                    })
                })
                .collect::<Vec<_>>();
            let next = nested(rest, heads, candidates, inputs, variables, scope, fresh)?;
            Ok(quote!(if !#state.#field.iter().any(|#row| true #(&& (#conditions))*) { #next }))
        }
        BodyItem::Filter(filter) => {
            let condition = &filter.condition;
            let next = nested(rest, heads, candidates, inputs, variables, scope, fresh)?;
            Ok(quote!(if #condition { #next }))
        }
        BodyItem::Let(binding) => {
            for name in rust::bindings(&binding.pattern) {
                scope.insert(symbol_name(&name), false);
            }
            let next = nested(rest, heads, candidates, inputs, variables, scope, fresh)?;
            Ok(quote!({ #binding; #next }))
        }
        BodyItem::For(generator) => {
            for name in rust::bindings(&generator.pattern) {
                scope.insert(symbol_name(&name), true);
            }
            let value = fresh.id("reference_generated");
            let expression = &generator.expression;
            let pattern = &generator.pattern;
            let next = nested(rest, heads, candidates, inputs, variables, scope, fresh)?;
            Ok(quote!(for #value in #expression {
                let #pattern = &#value;
                #next
            }))
        }
        BodyItem::Equality(equality) => {
            let equality = cq_syntax::Constraint::from_equality(equality);
            let condition = constraint(&equality, &scope, variables);
            let next = nested(rest, heads, candidates, inputs, variables, scope, fresh)?;
            Ok(quote!(if #condition { #next }))
        }
        BodyItem::Constraint(constraint) => {
            let condition = self::constraint(constraint, &scope, variables);
            let next = nested(rest, heads, candidates, inputs, variables, scope, fresh)?;
            Ok(quote!(if #condition { #next }))
        }
        _ => Err(syn::Error::new_spanned(
            item,
            "unsupported DL-Source clause",
        )),
    }
}

// Relational predicates compare column representations. Restore a uniform
// borrowed comparison here, rather than treating cq_tokens() (the logical
// wire spelling) as executable Rust. A closed operand compared with a variable
// of a code domain is translated through the domain: `x <= v ⇔ x < upper(v)`,
// `x > v ⇔ x >= upper(v)`.
fn constraint(
    value: &cq_syntax::Constraint,
    scope: &BTreeMap<String, bool>,
    variables: &BTreeMap<String, syn::Expr>,
) -> TokenStream {
    use cq_syntax::constraint::{Comparison, Constraint, Term};
    let term = |value: &Term| match value {
        Term::Variable(name) if scope.get(&symbol_name(name)) == Some(&true) => quote!(#name),
        Term::Variable(name) => quote!(&#name),
        Term::Closed(expression) => quote!(&(#expression)),
        Term::Literal(value) => quote!(&(#value)),
    };
    let domain = |value: &Term| match value {
        Term::Variable(name) => variables.get(&symbol_name(name)),
        _ => None,
    };
    let closed = |value: &Term| match value {
        Term::Variable(name) => !scope.contains_key(&symbol_name(name)),
        Term::Closed(_) | Term::Literal(_) => true,
    };
    let code = |function: &str, domain: &syn::Expr, value: &Term| {
        let value = match value {
            Term::Variable(name) => quote!(#name),
            Term::Closed(expression) => quote!(#expression),
            Term::Literal(value) => quote!(#value),
        };
        let value = translate(function, Some(domain), value);
        quote!(&(#value))
    };
    let operator = |comparison: Comparison| match comparison {
        Comparison::Eq(_) => quote!(==),
        Comparison::Ne(_) => quote!(!=),
        Comparison::Lt(_) => quote!(<),
        Comparison::Le(_) => quote!(<=),
        Comparison::Gt(_) => quote!(>),
        Comparison::Ge(_) => quote!(>=),
    };
    let translation = |comparison: Comparison| match comparison {
        Comparison::Eq(_) => (quote!(==), "exact"),
        Comparison::Ne(_) => (quote!(!=), "exact"),
        Comparison::Lt(_) => (quote!(<), "lower"),
        Comparison::Ge(_) => (quote!(>=), "lower"),
        Comparison::Le(_) => (quote!(<), "upper"),
        Comparison::Gt(_) => (quote!(>=), "upper"),
    };
    let flip = |comparison: Comparison| match comparison {
        Comparison::Lt(_) => Comparison::Gt(Default::default()),
        Comparison::Le(_) => Comparison::Ge(Default::default()),
        Comparison::Gt(_) => Comparison::Lt(Default::default()),
        Comparison::Ge(_) => Comparison::Le(Default::default()),
        same => same,
    };
    match value {
        Constraint::Compare {
            left,
            comparison,
            right,
        } => match (domain(left), domain(right)) {
            (Some(domain), _) if closed(right) => {
                let (operator, function) = translation(*comparison);
                let (left, right) = (term(left), code(function, domain, right));
                quote!((#left) #operator (#right))
            }
            (_, Some(domain)) if closed(left) => {
                let (operator, function) = translation(flip(*comparison));
                let (variable, left) = (term(right), code(function, domain, left));
                quote!((#variable) #operator (#left))
            }
            _ => {
                let (left, right) = (term(left), term(right));
                let operator = operator(*comparison);
                quote!((#left) #operator (#right))
            }
        },
        Constraint::Between {
            term: value,
            low,
            high,
        } => match domain(value) {
            Some(domain) if closed(low) && closed(high) => {
                let (low, high) = (code("lower", domain, low), code("upper", domain, high));
                let value = term(value);
                quote!((#value) >= (#low) && (#value) < (#high))
            }
            _ => {
                let value = term(value);
                let low = term(low);
                let high = term(high);
                quote!((#value) >= (#low) && (#value) <= (#high))
            }
        },
        Constraint::In {
            term: value,
            literals,
        } => {
            let domain = domain(value);
            let value = term(value);
            let literals = literals
                .iter()
                .map(|literal| translate("exact", domain, quote!(#literal)));
            quote!(false #(|| (#value) == &(#literals))*)
        }
        Constraint::Or(branches) => {
            let branches = branches.iter().map(|branch| {
                let terms = branch
                    .iter()
                    .map(|value| constraint(value, scope, variables));
                quote!(true #(&& (#terms))*)
            });
            quote!(false #(|| (#branches))*)
        }
    }
}
