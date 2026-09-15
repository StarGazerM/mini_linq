//! Materialized binary algebra storage and local Access correspondence.
use super::*;

pub(crate) fn check(
    source: &ir::Module,
    target: &syn::File,
    access: &syn::Path,
) -> syn::Result<()> {
    check_file(source, target, access).ok_or_else(|| {
        syn::Error::new_spanned(
            target,
            "physical materialized storage/access source-target relation violated",
        )
    })
}

fn check_file(source: &ir::Module, target: &syn::File, access: &syn::Path) -> Option<()> {
    let module = &source.relational_plan;
    let context = RuleContext::new(source, |i| format_ident!("relation{i}"))?;
    let (info, results) = (&context.info, &context.results);
    let build = method(target, &module.program.name, "build")?;
    let mut statements = build.block.stmts.iter();
    let stats = format_ident!("{}ExecutionStats", module.program.name);
    expect_statement(
        &mut statements,
        &syn::parse_quote!(let mut __stats = #stats::default();),
    )?;
    expect_statement(
        &mut statements,
        &syn::parse_quote!(let __pairs = ::core::cell::Cell::new(0usize);),
    )?;
    expect_statement(
        &mut statements,
        &syn::parse_quote!(let __probes = ::core::cell::Cell::new(0usize);),
    )?;
    check_body(&context, &mut statements, access, Observation::Counted)?;
    let storage = format_ident!("{}Storage", module.program.name);
    expect_statement(
        &mut statements,
        &syn::parse_quote!(__stats.join_pairs = __pairs.get();),
    )?;
    expect_statement(
        &mut statements,
        &syn::parse_quote!(__stats.semijoin_probes = __probes.get();),
    )?;
    check_answer(
        module,
        info,
        results,
        initializer(next_local(&mut statements, &format_ident!("answer"))?)?,
    )?;
    expect_statement(
        &mut statements,
        &Stmt::Expr(syn::parse_quote!(#storage { answer, stats: __stats }), None),
    )?;
    let query = method(target, &storage, "query")?;
    if query.block.stmts.len() != 1 {
        return None;
    }
    equal(
        last_expression(&query.block.stmts)?,
        &syn::parse_quote!(self.answer.iter().cloned()),
    )?;
    statements.next().is_none().then_some(())
}

/// The two hosts observe the same materialized computation differently.
/// This checker-local choice describes target syntax, not a lowering strategy.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Observation {
    Plain,
    Counted,
}

pub(crate) fn query(
    source: &ir::Module,
    target: &Expr,
    access: &syn::Path,
    inputs: &[Expr],
) -> syn::Result<()> {
    check_query(source, target, access, inputs).ok_or_else(|| {
        syn::Error::new_spanned(target, "materialized query source-target relation violated")
    })
}

/// Check the callable binding boundary separately from the query computation.
pub(crate) fn callable(source: &ir::Module, target: &Expr, access: &syn::Path) -> syn::Result<()> {
    let accepted = (|| -> Option<()> {
        let Expr::Closure(closure) = target else {
            return None;
        };
        if !closure.attrs.is_empty()
            || closure.lifetimes.is_some()
            || closure.constness.is_some()
            || closure.movability.is_some()
            || closure.asyncness.is_some()
            || closure.capture.is_some()
        {
            return None;
        }
        let module = &source.relational_plan;
        let names: Vec<_> = (0..module.program.inputs().count())
            .map(|i| format_ident!("input{i}"))
            .collect();
        if closure.inputs.len() != names.len()
            || closure
                .inputs
                .iter()
                .zip(&names)
                .any(|(pattern, name)| pattern != &syn::parse_quote!(#name))
        {
            return None;
        }
        let info = ra::check_in(module.program.inputs(), &module.plan).ok()?;
        let types = ra::output_types(
            &module.program,
            &info[&symbol_name(&module.plan.output.input)],
        )
        .ok()?;
        let syn::ReturnType::Type(_, actual) = &closure.output else {
            return None;
        };
        let expected: syn::Type = syn::parse_quote!(::std::collections::HashSet<(#(#types,)*)>);
        if actual.as_ref() != &expected {
            return None;
        }
        let inputs: Vec<_> = names.iter().map(|name| syn::parse_quote!(#name)).collect();
        check_query(source, &closure.body, access, &inputs)
    })();
    accepted.ok_or_else(|| {
        syn::Error::new_spanned(
            target,
            "materialized callable source-target relation violated",
        )
    })
}

fn check_query(
    source: &ir::Module,
    target: &Expr,
    access: &syn::Path,
    inputs: &[Expr],
) -> Option<()> {
    let Expr::Block(block) = target else {
        return None;
    };
    if block.label.is_some() || !block.attrs.is_empty() {
        return None;
    }
    let module = &source.relational_plan;
    let context = RuleContext::new(source, |i| format_ident!("relation{i}"))?;
    let names: Vec<_> = (0..module.program.inputs().count())
        .map(|i| format_ident!("input{i}"))
        .collect();
    if names.len() != inputs.len() {
        return None;
    }
    let mut statements = block.block.stmts.iter();
    expect_statement(
        &mut statements,
        &syn::parse_quote!(let (#(#names,)*) = (#((#inputs),)*);),
    )?;
    check_body(&context, &mut statements, access, Observation::Plain)?;
    let answer = next_local(&mut statements, &format_ident!("answer"))?;
    let types = ra::output_types(
        &module.program,
        &context.info[&symbol_name(&module.plan.output.input)],
    )
    .ok()?;
    tuple_set_type(answer, &types)?;
    check_answer(
        module,
        &context.info,
        &context.results,
        initializer(answer)?,
    )?;
    expect_statement(
        &mut statements,
        &Stmt::Expr(syn::parse_quote!(answer), None),
    )?;
    statements.next().is_none().then_some(())
}

fn check_body(
    context: &RuleContext<'_>,
    statements: &mut std::slice::Iter<'_, Stmt>,
    access: &syn::Path,
    observation: Observation,
) -> Option<()> {
    let source = context.source;
    let module = &source.relational_plan;
    let (info, inputs, results) = (&context.info, &context.inputs, &context.results);
    let used: BTreeSet<_> = module
        .plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Rename(r) => Some(symbol_name(&r.relation)),
            _ => None,
        })
        .chain(
            source
                .indexes
                .iter()
                .filter(|k| k.result_token.is_none())
                .map(|k| symbol_name(&k.relation)),
        )
        .collect();
    for input in module
        .program
        .inputs()
        .filter(|d| used.contains(&symbol_name(&d.name)))
    {
        let name = symbol_name(&input.name);
        let id = &inputs[&name];
        let local = next_local(statements, id)?;
        equal(
            initializer(local)?,
            &syn::parse_quote!(::core::iter::Iterator::collect(::core::iter::IntoIterator::into_iter(#id))),
        )?;
        tuple_set_type(
            local,
            &input
                .columns
                .iter()
                .map(|c| c.ty.clone())
                .collect::<Vec<_>>(),
        )?;
        check_indexes(
            context,
            statements,
            false,
            &name,
            id,
            input.arity(),
            observation,
        )?;
    }
    for d in &module.plan.definitions {
        let name = symbol_name(&d.result);
        let id = &results[&name];
        let row_info = &info[&name];
        let local = next_local(statements, id)?;
        tuple_set_type(
            local,
            &row_info
                .heading
                .iter()
                .map(|v| row_info.types[v].clone())
                .collect::<Vec<_>>(),
        )?;
        let Expr::Block(block) = initializer(local)? else {
            return None;
        };
        let Expr::Macro(mac) = last_expression(&block.block.stmts)? else {
            return None;
        };
        let plan = access_macro(&mac.mac, access)?;
        check_materialized_clause(context, d, &block.block.stmts, &plan, observation)?;
        let arity = row_info.heading.len();
        if observation == Observation::Counted {
            expect_statement(
                statements,
                &syn::parse_quote!(__stats.results.push((#name, #arity, #id.len()));),
            )?;
        }
        check_indexes(context, statements, true, &name, id, arity, observation)?;
    }
    Some(())
}

fn check_indexes(
    context: &RuleContext<'_>,
    statements: &mut std::slice::Iter<'_, Stmt>,
    result: bool,
    name: &str,
    rows: &Ident,
    arity: usize,
    observation: Observation,
) -> Option<()> {
    let source = context.source;
    let info = &context.info;
    let mut required: Vec<_> = source
        .indexes
        .iter()
        .enumerate()
        .filter(|(_, k)| k.result_token.is_some() == result && symbol_name(&k.relation) == name)
        .map(|(i, k)| {
            (
                k.key_columns
                    .iter()
                    .map(|c| c.index as usize)
                    .collect::<Vec<_>>(),
                i,
            )
        })
        .collect();
    required.sort();
    for (key, i) in required {
        let payload =
            key.len() < arity && result && needs_payload(&source.relational_plan, info, name, &key);
        check_index(
            statements,
            &format_ident!("index{i}"),
            rows,
            arity,
            &key,
            payload,
            observation,
        )?;
    }
    Some(())
}

fn check_answer(
    module: &ra::Module,
    info: &Information,
    results: &Names,
    actual: &Expr,
) -> Option<()> {
    let name = symbol_name(&module.plan.output.input);
    let input = &info[&name];
    let store = &results[&name];
    let Expr::MethodCall(collect) = actual else {
        return None;
    };
    if collect.method != "collect" || !collect.args.is_empty() {
        return None;
    }
    let Expr::MethodCall(map) = collect.receiver.as_ref() else {
        return None;
    };
    if map.method != "map" || map.args.len() != 1 {
        return None;
    }
    equal(&map.receiver, &syn::parse_quote!(#store.iter()))?;
    let Expr::Closure(closure) = &map.args[0] else {
        return None;
    };
    if closure.inputs.len() != 1 || closure.inputs[0] != syn::parse_quote!(__row) {
        return None;
    }
    let Expr::Block(block) = closure.body.as_ref() else {
        return None;
    };
    if block.block.stmts.len() != 1 {
        return None;
    }
    let Expr::Tuple(tuple) = last_expression(&block.block.stmts)? else {
        return None;
    };
    if tuple.elems.len() != module.plan.output.head.args.len() {
        return None;
    }
    for (actual, original) in tuple.elems.iter().zip(module.plan.output.head.args.iter()) {
        let column = |name: &str| {
            let index = syn::Index::from(input.heading.iter().position(|v| v == name).unwrap());
            syn::parse_quote!(&__row.#index)
        };
        if let Some(v) =
            cq::rust::variable(original).filter(|v| input.heading.contains(&symbol_name(v)))
        {
            let value: Expr = column(&symbol_name(v));
            equal(
                actual,
                &syn::parse_quote!(::core::clone::Clone::clone(#value)),
            )?;
        } else {
            alias_expression(actual, original, input, column)?;
        }
    }
    Some(())
}

fn needs_payload(module: &ra::Module, info: &Information, result: &str, key: &[usize]) -> bool {
    module.plan.definitions.iter().any(|d| match &d.operator {
        ra::Operator::NaturalJoin(j) if symbol_name(&j.right) == result => {
            join_key(info, &j.left, &j.right) == key
                && !info[result]
                    .heading
                    .is_subset(&info[&symbol_name(&j.left)].heading)
        }
        _ => false,
    })
}

fn check_index(
    statements: &mut std::slice::Iter<'_, Stmt>,
    index: &Ident,
    rows: &Ident,
    arity: usize,
    key: &[usize],
    payload: bool,
    observation: Observation,
) -> Option<()> {
    let local = next_local(statements, index)?;
    if key.len() == arity {
        if local.pat != syn::parse_quote!(#index) || !local.attrs.is_empty() {
            return None;
        }
        return equal(initializer(local)?, &syn::parse_quote!(&#rows));
    }
    if payload {
        if local.pat != syn::parse_quote!(mut #index) {
            return None;
        }
    } else if !matches!(
        collection_key_type(local, index, None)?,
        syn::Type::Infer(_)
    ) {
        return None;
    }
    let keys = cloned_columns("__row", key);
    if payload {
        let complement: Vec<_> = (0..arity).filter(|i| !key.contains(i)).collect();
        let values = cloned_columns("__row", &complement);
        equal(
            initializer(local)?,
            &syn::parse_quote!(::std::collections::HashMap::<_, ::std::vec::Vec<_>>::new()),
        )?;
        expect_statement(
            statements,
            &syn::parse_quote!(for __row in &#rows { #index.entry(#keys).or_default().push(#values); }),
        )?;
    } else {
        equal(
            initializer(local)?,
            &syn::parse_quote!(#rows.iter().map(|__row| #keys).collect()),
        )?;
    }
    if observation == Observation::Counted {
        expect_statement(
            statements,
            &syn::parse_quote!(__stats.index_rows += #rows.len();),
        )?;
    }
    Some(())
}

fn check_materialized_clause(
    context: &RuleContext<'_>,
    d: &ra::Definition,
    statements: &[Stmt],
    plan: &ac::Plan,
    observation: Observation,
) -> Option<()> {
    let (source, info, vars, inputs, results) = (
        context.source,
        &context.info,
        &context.variables,
        &context.inputs,
        &context.results,
    );
    ac::contract::check(plan).ok()?;
    let aliases = locals(statements);
    let expected_aliases = match d.operator {
        ra::Operator::Unit(_) => 0,
        ra::Operator::NaturalJoin(_) | ra::Operator::Semijoin(_) => {
            if observation == Observation::Counted {
                3
            } else {
                2
            }
        }
        _ => 1,
    };
    if aliases.len() != expected_aliases || statements.len() != expected_aliases + 1 {
        return None;
    }
    let alias = |name: &str, value: &Ident| {
        let local = *aliases.get(name)?;
        let name = format_ident!("{name}");
        if local.pat != syn::parse_quote!(#name) || !local.attrs.is_empty() {
            return None;
        }
        equal(initializer(local)?, &syn::parse_quote!(&#value))
    };
    let output = &info[&symbol_name(&d.result)];
    check_head(plan, &tuple_names(&output.heading, vars))?;
    let mut clauses = plan.body.iter();
    let input_scan = |input: &Ident, clauses: &mut syn::punctuated::Iter<'_, ac::Clause>| {
        alias("__left", &results[&symbol_name(input)])?;
        check_scan(
            clauses.next()?,
            &tuple_names(&info[&symbol_name(input)].heading, vars),
            &syn::parse_quote!(__left),
        )
    };
    let mut fresh = BTreeSet::new();
    let mut pair_count = false;
    match &d.operator {
        ra::Operator::Unit(_) => check_if(clauses.next()?, &syn::parse_quote!(true))?,
        ra::Operator::Rename(rename) => {
            alias("__left", &inputs[&symbol_name(&rename.relation)])?;
            let decl = source
                .relational_plan
                .program
                .inputs()
                .find(|r| r.name == rename.relation)?;
            let names: Vec<_> = decl
                .columns
                .iter()
                .map(|c| {
                    let mapping = rename.mapping.iter().find(|m| m.source == c.name)?;
                    Some(vars[&symbol_name(&mapping.target)].clone())
                })
                .collect::<Option<_>>()?;
            check_scan(clauses.next()?, &names, &syn::parse_quote!(__left))?;
        }
        ra::Operator::Project(p) => input_scan(&p.input, &mut clauses)?,
        ra::Operator::Select(s) => {
            input_scan(&s.input, &mut clauses)?;
            host_expression(
                guard_value(clauses.next()?)?,
                &s.condition,
                &info[&symbol_name(&s.input)],
                vars,
            )?;
        }
        ra::Operator::Extend(e) => {
            input_scan(&e.input, &mut clauses)?;
            let names: Vec<_> = e
                .columns
                .iter()
                .map(|c| vars[&symbol_name(&c.name)].clone())
                .collect();
            host_expression(
                extend_value(clauses.next()?, &names)?,
                &e.value,
                &info[&symbol_name(&e.input)],
                vars,
            )?;
            fresh.extend(names.iter().map(symbol_name));
        }
        ra::Operator::NaturalJoin(_) | ra::Operator::Semijoin(_) => {
            let (left, right, semi) = match &d.operator {
                ra::Operator::NaturalJoin(j) => (&j.left, &j.right, false),
                ra::Operator::Semijoin(j) => (&j.left, &j.right, true),
                _ => unreachable!(),
            };
            input_scan(left, &mut clauses)?;
            let lh = &info[&symbol_name(left)].heading;
            let rh = &info[&symbol_name(right)].heading;
            let shared: BTreeSet<_> = lh.intersection(rh).cloned().collect();
            let added = tuple_names(&rh.difference(lh).cloned().collect(), vars);
            let key = tuple_names(&shared, vars);
            let key_value = cloned_names(&key);
            let (exists, lookup): (Expr, Expr) = if shared.is_empty() {
                alias("__right", &results[&symbol_name(right)])?;
                (
                    syn::parse_quote!(!__right.is_empty()),
                    syn::parse_quote!(__right.iter()),
                )
            } else {
                let columns = join_key(info, left, right);
                let (i, _) = source.indexes.iter().enumerate().find(|(_, k)| {
                    k.result_token.is_some()
                        && k.relation == *right
                        && k.key_columns
                            .iter()
                            .map(|c| c.index as usize)
                            .eq(columns.iter().copied())
                })?;
                alias("__index", &format_ident!("index{i}"))?;
                let exists = if needs_payload(
                    &source.relational_plan,
                    info,
                    &symbol_name(right),
                    &columns,
                ) {
                    syn::parse_quote!(__index.contains_key(&#key_value))
                } else {
                    syn::parse_quote!(__index.contains(&#key_value))
                };
                (
                    exists,
                    syn::parse_quote!(__index.get(&#key_value).into_iter().flatten()),
                )
            };
            if semi {
                if observation == Observation::Counted {
                    alias("__probes_ref", &format_ident!("__probes"))?;
                    check_if(
                        clauses.next()?,
                        &syn::parse_quote!({ __probes_ref.set(__probes_ref.get()+1); #exists }),
                    )?;
                } else {
                    check_if(clauses.next()?, &exists)?;
                }
            } else {
                if added.is_empty() {
                    check_if(clauses.next()?, &exists)?;
                } else {
                    check_for(clauses.next()?, &added, &lookup)?;
                }
                if observation == Observation::Counted {
                    alias("__pairs_ref", &format_ident!("__pairs"))?;
                    pair_count = true;
                }
            }
        }
    }
    if clauses.next().is_some() {
        return None;
    }
    let names = tuple_names(&output.heading, vars);
    let fields = names.iter().map(|v| {
        if fresh.contains(&symbol_name(v)) {
            quote!(#v)
        } else {
            quote!(::core::clone::Clone::clone(#v))
        }
    });
    let value: Expr = syn::parse_quote!((#(#fields,)*));
    equal(
        &plan.head.output,
        &if pair_count {
            syn::parse_quote!({ __pairs_ref.set(__pairs_ref.get()+1); #value })
        } else {
            value
        },
    )
}

// Host syntax is checked structurally only. Scope/type/effect adequacy remains
// the host-language obligation; this verifies the source leaf and aliases.
fn host_expression(
    actual: &Expr,
    original: &Expr,
    input: &ra::RelationInfo,
    vars: &Names,
) -> Option<()> {
    if let Some(p) = cq::Predicate::from_expr(original)
        .filter(|p| p.variables().iter().all(|v| input.heading.contains(v)))
    {
        let primitive = p
            .variables()
            .iter()
            .all(|v| input.types.get(v).is_some_and(cq::rust::primitive_i32));
        let term = |t: &cq::Term| match t {
            cq::Term::Integer(n) => {
                if primitive {
                    quote!(#n)
                } else {
                    quote!(&#n)
                }
            }
            cq::Term::Variable(v) => {
                let name = symbol_name(v);
                let id = &vars[&name];
                if primitive {
                    quote!({ let value: ::core::primitive::i32 = *#id; value })
                } else {
                    quote!(#id)
                }
            }
        };
        let left = term(&p.left);
        let right = term(&p.right);
        let op = &p.comparison;
        return equal(actual, &syn::parse_quote!(#left #op #right));
    }
    alias_expression(actual, original, input, |name| {
        let variable = &vars[name];
        syn::parse_quote!(#variable)
    })
}
