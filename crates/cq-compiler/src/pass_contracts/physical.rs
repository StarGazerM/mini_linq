//! Source-target relations for the result-index physical routes.
//!
//! These check the concrete storage layouts and local Access regions in the
//! existing staged Rust file. They do not prove the surrounding API wrappers,
//! Rust typing/hygiene, or equivalence of arbitrary host-language effects.
//! The checker reads actual Rust syntax and never invokes a physical emitter.
use cq_ir::{
    cq, index_requirements as ir, relational_plan as ra, rust_access_plan as ac, symbol_name,
};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, Ident, Stmt};

mod basic;
#[cfg(test)]
mod tests;

pub(crate) fn basic(source: &ir::Module, target: &syn::File) -> syn::Result<()> {
    basic::check(source, target)
}

type Information = BTreeMap<String, ra::RelationInfo>;
type Names = BTreeMap<String, Ident>;

/// Environments of the M/L judgments; borrowed facts, not another IR.
struct RuleContext<'a> {
    source: &'a ir::Module,
    info: &'a Information,
    variables: &'a Names,
    inputs: &'a Names,
    results: &'a Names,
}

pub(crate) fn materialized(source: &ir::Module, target: &syn::File) -> syn::Result<()> {
    check(source, target, false)
}

pub(crate) fn lazy(source: &ir::Module, target: &syn::File) -> syn::Result<()> {
    check(source, target, true)
}

fn check(source: &ir::Module, target: &syn::File, lazy: bool) -> syn::Result<()> {
    let mut phase = "storage";
    let result = (|| -> Option<()> {
        let module = &source.relational_plan;
        let info = ra::check_in(module.program.inputs(), &module.plan).ok()?;
        let variables: Names = info
            .values()
            .flat_map(|r| r.heading.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .enumerate()
            .map(|(i, name)| (name, format_ident!("v{i}")))
            .collect();
        let build = method(target, &module.program.name, "build")?;
        let mut statements = build.block.stmts.iter();
        let inputs: Names = module
            .program
            .inputs()
            .enumerate()
            .map(|(i, d)| (symbol_name(&d.name), format_ident!("input{i}")))
            .collect();
        let results: Names = module
            .plan
            .definitions
            .iter()
            .enumerate()
            .map(|(i, d)| {
                (
                    symbol_name(&d.result),
                    if lazy {
                        format_ident!("rows{i}")
                    } else {
                        format_ident!("relation{i}")
                    },
                )
            })
            .collect();
        let context = RuleContext {
            source,
            info: &info,
            variables: &variables,
            inputs: &inputs,
            results: &results,
        };
        if !lazy {
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
        }
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
            let local = next_local(&mut statements, id)?;
            equal(
                initializer(local)?,
                &syn::parse_quote!(#id.into_iter().collect()),
            )?;
            tuple_set_type(
                local,
                &input
                    .columns
                    .iter()
                    .map(|c| c.ty.clone())
                    .collect::<Vec<_>>(),
            )?;
            if !lazy {
                check_indexes(&context, &mut statements, false, &name, id, input.arity())?;
            }
        }
        let mut fields = Vec::new();
        for d in &module.plan.definitions {
            let name = symbol_name(&d.result);
            let id = &results[&name];
            let row_info = &info[&name];
            if lazy {
                let ra::Operator::Rename(rename) = &d.operator else {
                    continue;
                };
                let input = &inputs[&symbol_name(&rename.relation)];
                let declaration = module
                    .program
                    .inputs()
                    .find(|d| d.name == rename.relation)?;
                let columns: Vec<_> = row_info
                    .heading
                    .iter()
                    .map(|v| {
                        let attribute = rename
                            .mapping
                            .iter()
                            .find(|m| symbol_name(&m.target) == *v)?;
                        declaration
                            .columns
                            .iter()
                            .position(|c| c.name == attribute.source)
                    })
                    .collect::<Option<_>>()?;
                let tuple = cloned_columns("row", &columns);
                let local = next_local(&mut statements, id)?;
                equal(
                    initializer(local)?,
                    &syn::parse_quote!(#input.iter().map(|row| #tuple).collect()),
                )?;
                tuple_set_type(
                    local,
                    &row_info
                        .heading
                        .iter()
                        .map(|v| row_info.types[v].clone())
                        .collect::<Vec<_>>(),
                )?;
                fields.push(id.clone());
            } else {
                let local = next_local(&mut statements, id)?;
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
                let Expr::Call(call) = last_expression(&block.block.stmts)? else {
                    return None;
                };
                equal(
                    &call.func,
                    &syn::parse_quote!(::core::iter::Iterator::collect),
                )?;
                if call.args.len() != 1 {
                    return None;
                }
                let plan = pull(&call.args[0])?;
                check_materialized_clause(&context, d, &block.block.stmts, &plan)?;
                let arity = row_info.heading.len();
                expect_statement(
                    &mut statements,
                    &syn::parse_quote!(__stats.results.push((#name, #arity, #id.len()));),
                )?;
                check_indexes(&context, &mut statements, true, &name, id, arity)?;
            }
        }
        let storage = format_ident!("{}Storage", module.program.name);
        if lazy {
            for (i, index) in source.indexes.iter().enumerate() {
                let name = symbol_name(&index.relation);
                let key: Vec<_> = index.key_columns.iter().map(|c| c.index as usize).collect();
                let id = format_ident!("index{i}");
                let arity = info[&name].heading.len();
                check_index(
                    &mut statements,
                    &id,
                    &results[&name],
                    arity,
                    &key,
                    key.len() < arity,
                    true,
                )?;
                fields.push(id);
            }
            expect_statement(
                &mut statements,
                &Stmt::Expr(syn::parse_quote!(#storage { #(#fields,)* }), None),
            )?;
            phase = "access";
            let query = method(target, &storage, "query")?;
            let mut query_statements = query.block.stmts.iter();
            for id in &fields {
                expect_statement(
                    &mut query_statements,
                    &syn::parse_quote!(let #id = &self.#id;),
                )?;
            }
            let plan = match query_statements.next()? {
                Stmt::Macro(m) if m.semi_token.is_none() => pull_macro(&m.mac)?,
                Stmt::Expr(e, None) => pull(e)?,
                _ => return None,
            };
            if query_statements.next().is_some() {
                return None;
            }
            check_lazy_clauses(&context, &plan)?;
        } else {
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
                &info,
                &results,
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
        }
        statements.next().is_none().then_some(())
    })();
    result.ok_or_else(|| {
        syn::Error::new_spanned(
            target,
            format!("physical {phase} source-target relation violated"),
        )
    })
}

fn check_indexes(
    context: &RuleContext<'_>,
    statements: &mut std::slice::Iter<'_, Stmt>,
    result: bool,
    name: &str,
    rows: &Ident,
    arity: usize,
) -> Option<()> {
    let source = context.source;
    let info = context.info;
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
            false,
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

fn join_key(info: &Information, left: &Ident, right: &Ident) -> Vec<usize> {
    info[&symbol_name(right)]
        .heading
        .iter()
        .enumerate()
        .filter_map(|(i, v)| info[&symbol_name(left)].heading.contains(v).then_some(i))
        .collect()
}

fn check_index(
    statements: &mut std::slice::Iter<'_, Stmt>,
    index: &Ident,
    rows: &Ident,
    arity: usize,
    key: &[usize],
    payload: bool,
    lazy: bool,
) -> Option<()> {
    let local = next_local(statements, index)?;
    if key.len() == arity && !lazy {
        if local.pat != syn::parse_quote!(#index) || !local.attrs.is_empty() {
            return None;
        }
        return equal(initializer(local)?, &syn::parse_quote!(&#rows));
    }
    if lazy || !payload {
        let syn::Pat::Type(typed) = &local.pat else {
            return None;
        };
        let Expr::Path(expected) = (if payload {
            syn::parse_quote!(::std::collections::HashMap)
        } else {
            syn::parse_quote!(::std::collections::HashSet)
        }) else {
            return None;
        };
        let syn::Type::Path(actual) = typed.ty.as_ref() else {
            return None;
        };
        let actual_path: Vec<_> = actual.path.segments.iter().map(|s| &s.ident).collect();
        let expected_path: Vec<_> = expected.path.segments.iter().map(|s| &s.ident).collect();
        if actual.qself.is_some()
            || actual.path.leading_colon != expected.path.leading_colon
            || actual_path != expected_path
        {
            return None;
        }
        let pattern: syn::Pat = if payload {
            syn::parse_quote!(mut #index)
        } else {
            syn::parse_quote!(#index)
        };
        if typed.pat.as_ref() != &pattern {
            return None;
        }
        let syn::PathArguments::AngleBracketed(arguments) = &actual.path.segments.last()?.arguments
        else {
            return None;
        };
        if arguments.args.len() != if payload { 2 } else { 1 } {
            return None;
        }
        let syn::GenericArgument::Type(key_type) = &arguments.args[0] else {
            return None;
        };
        if lazy {
            let syn::Type::Tuple(tuple) = key_type else {
                return None;
            };
            if tuple.elems.len() != key.len() {
                return None;
            }
        } else if !matches!(key_type, syn::Type::Infer(_)) {
            return None;
        }
        if payload {
            let syn::GenericArgument::Type(syn::Type::Path(value)) = &arguments.args[1] else {
                return None;
            };
            let path: Vec<_> = value
                .path
                .segments
                .iter()
                .map(|s| symbol_name(&s.ident))
                .collect();
            if value.qself.is_some()
                || value.path.leading_colon.is_none()
                || path != ["std", "vec", "Vec"]
            {
                return None;
            }
            let syn::PathArguments::AngleBracketed(element) =
                &value.path.segments.last()?.arguments
            else {
                return None;
            };
            if element.args.len() != 1 {
                return None;
            }
            let syn::GenericArgument::Type(syn::Type::Tuple(tuple)) = &element.args[0] else {
                return None;
            };
            if tuple.elems.len() != arity - key.len() {
                return None;
            }
        }
    } else if local.pat != syn::parse_quote!(mut #index) {
        return None;
    }
    let row = if lazy { "row" } else { "__row" };
    let row_id = format_ident!("{row}");
    let keys = cloned_columns(row, key);
    if !payload {
        equal(
            initializer(local)?,
            &syn::parse_quote!(#rows.iter().map(|#row_id| #keys).collect()),
        )?;
    } else {
        let complement: Vec<_> = (0..arity).filter(|i| !key.contains(i)).collect();
        let values = cloned_columns(row, &complement);
        equal(
            initializer(local)?,
            &if lazy {
                syn::parse_quote!(::std::collections::HashMap::new())
            } else {
                syn::parse_quote!(::std::collections::HashMap::<_, ::std::vec::Vec<_>>::new())
            },
        )?;
        expect_statement(
            statements,
            &syn::parse_quote!(for #row_id in &#rows { #index.entry(#keys).or_default().push(#values); }),
        )?;
    }
    if !lazy {
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
) -> Option<()> {
    let (source, info, vars, inputs, results) = (
        context.source,
        context.info,
        context.variables,
        context.inputs,
        context.results,
    );
    ac::contract::check(plan).ok()?;
    let aliases = locals(statements);
    let expected_aliases = match d.operator {
        ra::Operator::Unit(_) => 0,
        ra::Operator::NaturalJoin(_) | ra::Operator::Semijoin(_) => 3,
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
            check_guard(
                clauses.next()?,
                &s.condition,
                &info[&symbol_name(&s.input)],
                vars,
                false,
            )?;
        }
        ra::Operator::Extend(e) => {
            input_scan(&e.input, &mut clauses)?;
            let names: Vec<_> = e
                .columns
                .iter()
                .map(|c| vars[&symbol_name(&c.name)].clone())
                .collect();
            check_extend(
                clauses.next()?,
                &names,
                &e.value,
                &info[&symbol_name(&e.input)],
                vars,
                false,
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
                alias("__probes_ref", &format_ident!("__probes"))?;
                check_if(
                    clauses.next()?,
                    &syn::parse_quote!({ __probes_ref.set(__probes_ref.get()+1); #exists }),
                )?;
            } else {
                if added.is_empty() {
                    check_if(clauses.next()?, &exists)?;
                } else {
                    check_for(clauses.next()?, &added, &lookup)?;
                }
                alias("__pairs_ref", &format_ident!("__pairs"))?;
                pair_count = true;
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

fn check_lazy_clauses(context: &RuleContext<'_>, plan: &ac::Plan) -> Option<()> {
    let (source, info, vars, results) = (
        context.source,
        context.info,
        context.variables,
        context.results,
    );
    ac::contract::check(plan).ok()?;
    let mut clauses = plan.body.iter();
    let mut root = true;
    for d in &source.relational_plan.plan.definitions {
        match &d.operator {
            ra::Operator::Unit(_) => {
                check_if(clauses.next()?, &syn::parse_quote!(true))?;
                root = false;
            }
            ra::Operator::Rename(_) if root => {
                let rows = &results[&symbol_name(&d.result)];
                check_scan(
                    clauses.next()?,
                    &tuple_names(&info[&symbol_name(&d.result)].heading, vars),
                    &syn::parse_quote!(#rows),
                )?;
                root = false;
            }
            ra::Operator::Rename(_) | ra::Operator::Project(_) => (),
            ra::Operator::Select(s) => check_guard(
                clauses.next()?,
                &s.condition,
                &info[&symbol_name(&s.input)],
                vars,
                true,
            )?,
            ra::Operator::Extend(e) => {
                let fresh: Vec<_> = e
                    .columns
                    .iter()
                    .map(|c| vars[&symbol_name(&c.name)].clone())
                    .collect();
                check_extend(
                    clauses.next()?,
                    &fresh,
                    &e.value,
                    &info[&symbol_name(&e.input)],
                    vars,
                    true,
                )?;
            }
            ra::Operator::NaturalJoin(j) => {
                let left = &info[&symbol_name(&j.left)];
                let right = &info[&symbol_name(&j.right)];
                let shared: BTreeSet<_> =
                    left.heading.intersection(&right.heading).cloned().collect();
                let fresh = tuple_names(
                    &right.heading.difference(&left.heading).cloned().collect(),
                    vars,
                );
                if shared.is_empty() {
                    let rows = &results[&symbol_name(&j.right)];
                    check_for(clauses.next()?, &fresh, &syn::parse_quote!(#rows.iter()))?;
                } else {
                    let columns = join_key(info, &j.left, &j.right);
                    let (i, _) = source.indexes.iter().enumerate().find(|(_, k)| {
                        k.result_token.is_some()
                            && k.relation == j.right
                            && k.key_columns
                                .iter()
                                .map(|c| c.index as usize)
                                .eq(columns.iter().copied())
                    })?;
                    let index = format_ident!("index{i}");
                    let key = shared.iter().map(|v| {
                        let id = &vars[v];
                        if left.computed.contains(v) {
                            quote!(#id.clone())
                        } else {
                            quote!(::core::clone::Clone::clone(#id))
                        }
                    });
                    let key: Expr = syn::parse_quote!((#(#key,)*));
                    if fresh.is_empty() {
                        check_if(clauses.next()?, &syn::parse_quote!(#index.contains(&#key)))?;
                    } else {
                        check_for(
                            clauses.next()?,
                            &fresh,
                            &syn::parse_quote!(#index.get(&#key).into_iter().flatten()),
                        )?;
                    }
                }
            }
            ra::Operator::Semijoin(_) => return None,
        }
    }
    if clauses.next().is_some() {
        return None;
    }
    let output = &info[&symbol_name(&source.relational_plan.plan.output.input)];
    check_head(plan, &tuple_names(&output.heading, vars))?;
    let tuple_expr = match &plan.head.output {
        Expr::Paren(p) => p.expr.as_ref(),
        e => e,
    };
    let Expr::Tuple(tuple) = tuple_expr else {
        return None;
    };
    let head = &source.relational_plan.plan.output.head.args;
    if tuple.elems.len() != head.len() {
        return None;
    }
    for (actual, original) in tuple.elems.iter().zip(head.iter()) {
        if let Some(v) =
            cq::rust::variable(original).filter(|v| output.heading.contains(&symbol_name(v)))
        {
            let id = &vars[&symbol_name(v)];
            equal(actual, &syn::parse_quote!(#id.clone()))?;
        } else {
            host_expression(actual, original, output, vars, true)?;
        }
    }
    Some(())
}

fn check_guard(
    clause: &ac::Clause,
    original: &Expr,
    input: &ra::RelationInfo,
    vars: &Names,
    lazy: bool,
) -> Option<()> {
    let (cq::BodyItem::Filter(label), ac::RustAccess::If { condition, .. }) =
        (&clause.item, &clause.access)
    else {
        return None;
    };
    equal(&label.condition, &syn::parse_quote!((#condition)))?;
    host_expression(condition, original, input, vars, lazy)
}

fn check_extend(
    clause: &ac::Clause,
    fresh: &[Ident],
    original: &Expr,
    input: &ra::RelationInfo,
    vars: &Names,
    lazy: bool,
) -> Option<()> {
    let cq::BodyItem::Let(label) = &clause.item else {
        return None;
    };
    let ac::RustAccess::For { source, .. } = &clause.access else {
        return None;
    };
    let Expr::Call(call) = source else {
        return None;
    };
    equal(&call.func, &syn::parse_quote!(::core::iter::once))?;
    if call.args.len() != 1 {
        return None;
    }
    let value = &call.args[0];
    check_for(clause, fresh, source)?;
    if label.pattern != syn::parse_quote!((#(#fresh,)*)) {
        return None;
    }
    equal(&label.expression, &syn::parse_quote!((#value)))?;
    host_expression(value, original, input, vars, lazy)
}

// Host syntax is checked structurally only. Scope/type/effect adequacy remains
// the host-language obligation; this verifies the source leaf and aliases.
fn host_expression(
    actual: &Expr,
    original: &Expr,
    input: &ra::RelationInfo,
    vars: &Names,
    lazy: bool,
) -> Option<()> {
    if let Some(p) = cq::Predicate::from_expr(original)
        .filter(|p| p.variables().iter().all(|v| input.heading.contains(v)))
    {
        let primitive = !lazy
            && p.variables()
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
                } else if lazy && input.computed.contains(&name) {
                    quote!(&#id)
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
        if lazy && input.computed.contains(name) {
            syn::parse_quote!(&#variable)
        } else {
            syn::parse_quote!(#variable)
        }
    })
}

fn alias_expression(
    actual: &Expr,
    original: &Expr,
    input: &ra::RelationInfo,
    value: impl Fn(&str) -> Expr,
) -> Option<()> {
    let Expr::Block(block) = actual else {
        return None;
    };
    equal(last_expression(&block.block.stmts)?, original)?;
    let used = cq::rust::free_variables(original).dependencies(&input.heading);
    let mut bound = BTreeSet::new();
    for statement in &block.block.stmts[..block.block.stmts.len().checked_sub(1)?] {
        let Stmt::Local(local) = statement else {
            return None;
        };
        let syn::Pat::Ident(pattern) = &local.pat else {
            return None;
        };
        if pattern.by_ref.is_some() || pattern.mutability.is_some() || pattern.subpat.is_some() {
            return None;
        }
        let name = symbol_name(&pattern.ident);
        if !used.contains(&name) {
            return None;
        }
        let mut expected = value(&name);
        if input.computed.contains(&name) {
            expected = syn::parse_quote!(::core::clone::Clone::clone(#expected));
        }
        equal(initializer(local)?, &expected)?;
        bound.insert(name);
    }
    used.is_subset(&bound).then_some(())
}

fn check_head(plan: &ac::Plan, names: &[Ident]) -> Option<()> {
    plan.head.atom.variables().eq(names.iter()).then_some(())
}

fn check_scan(clause: &ac::Clause, names: &[Ident], rows: &Expr) -> Option<()> {
    if names.is_empty() {
        check_if(clause, &syn::parse_quote!(!#rows.is_empty()))
    } else {
        check_for(clause, names, &syn::parse_quote!(#rows.iter()))
    }
}

fn check_for(clause: &ac::Clause, names: &[Ident], expected: &Expr) -> Option<()> {
    let ac::RustAccess::For {
        pattern, source, ..
    } = &clause.access
    else {
        return None;
    };
    if pattern != &syn::parse_quote!((#(#names,)*)) {
        return None;
    }
    equal(source, expected)
}

fn check_if(clause: &ac::Clause, expected: &Expr) -> Option<()> {
    let ac::RustAccess::If { condition, .. } = &clause.access else {
        return None;
    };
    equal(condition, expected)
}

fn tuple_names(heading: &BTreeSet<String>, vars: &Names) -> Vec<Ident> {
    heading.iter().map(|v| vars[v].clone()).collect()
}

fn cloned_names(names: &[Ident]) -> Expr {
    syn::parse_quote!((#(::core::clone::Clone::clone(#names),)*))
}

fn cloned_columns(row: &str, columns: &[usize]) -> Expr {
    let row = format_ident!("{row}");
    let columns = columns.iter().copied().map(syn::Index::from);
    syn::parse_quote!((#(::core::clone::Clone::clone(&#row.#columns),)*))
}

fn equal(actual: &Expr, expected: &Expr) -> Option<()> {
    (actual == expected).then_some(())
}

fn method<'a>(file: &'a syn::File, owner: &Ident, name: &str) -> Option<&'a syn::ImplItemFn> {
    let mut methods = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item)
                if item.trait_.is_none() && item.self_ty.as_ref() == &syn::parse_quote!(#owner) =>
            {
                Some(item)
            }
            _ => None,
        })
        .flat_map(|item| item.items.iter())
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method) if method.sig.ident == name => Some(method),
            _ => None,
        });
    let found = methods.next()?;
    methods.next().is_none().then_some(found)
}

fn locals(statements: &[Stmt]) -> BTreeMap<String, &syn::Local> {
    statements
        .iter()
        .filter_map(|s| match s {
            Stmt::Local(local) => local_name(local).map(|id| (symbol_name(id), local)),
            _ => None,
        })
        .collect()
}

fn local_name(local: &syn::Local) -> Option<&Ident> {
    let pattern = match &local.pat {
        syn::Pat::Type(p) => p.pat.as_ref(),
        p => p,
    };
    match pattern {
        syn::Pat::Ident(p) if p.by_ref.is_none() && p.subpat.is_none() => Some(&p.ident),
        _ => None,
    }
}

fn initializer(local: &syn::Local) -> Option<&Expr> {
    let init = local.init.as_ref()?;
    init.diverge.is_none().then_some(init.expr.as_ref())
}

fn tuple_set_type(local: &syn::Local, types: &[syn::Type]) -> Option<()> {
    let syn::Pat::Type(p) = &local.pat else {
        return None;
    };
    (p.ty.as_ref() == &syn::parse_quote!(::std::collections::HashSet<(#(#types,)*)>)).then_some(())
}

fn last_expression(statements: &[Stmt]) -> Option<&Expr> {
    match statements.last()? {
        Stmt::Expr(expr, None) => Some(expr),
        _ => None,
    }
}

fn pull(expr: &Expr) -> Option<ac::Plan> {
    let Expr::Macro(mac) = expr else {
        return None;
    };
    pull_macro(&mac.mac)
}

fn pull_macro(mac: &syn::Macro) -> Option<ac::Plan> {
    (mac.path == syn::parse_quote!(::mini_linq::pull)).then_some(())?;
    syn::parse2(mac.tokens.clone()).ok()
}

fn next_local<'a>(
    statements: &mut std::slice::Iter<'a, Stmt>,
    name: &Ident,
) -> Option<&'a syn::Local> {
    let Stmt::Local(local) = statements.next()? else {
        return None;
    };
    (local.attrs.is_empty() && local_name(local)? == name).then_some(local)
}

fn expect_statement(statements: &mut std::slice::Iter<'_, Stmt>, expected: &Stmt) -> Option<()> {
    (statements.next()? == expected).then_some(())
}
