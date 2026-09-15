//! Fused linear storage and Access correspondence.
use crate::check_support::{
    Names, RuleContext, access_expression, access_macro, alias_expression, check_for, check_head,
    check_if, check_scan, cloned_columns, collection_key_type, equal, expect_statement,
    extend_value, guard_value, initializer, join_key, method, next_local, tuple_names,
    tuple_set_type,
};
use cq_ir::{
    cq, index_requirements as ir, relational_plan as ra, rust_access_plan as ac, symbol_name,
};
use quote::{format_ident, quote};
use std::collections::BTreeSet;
use syn::{Expr, Ident, Stmt};

pub fn check(source: &ir::Module, target: &syn::File, access: &syn::Path) -> syn::Result<()> {
    if !supported(source) {
        return Err(syn::Error::new_spanned(
            source,
            "unsupported fused linear storage domain",
        ));
    }
    check_file(source, target, access).ok_or_else(|| {
        syn::Error::new_spanned(
            target,
            "physical lazy storage/access source-target relation violated",
        )
    })
}

fn check_file(source: &ir::Module, target: &syn::File, access: &syn::Path) -> Option<()> {
    let module = &source.relational_plan;
    let context = RuleContext::new(source, |i| format_ident!("rows{i}"))?;
    let (info, inputs, results) = (&context.info, &context.inputs, &context.results);
    let build = method(target, &module.program.name, "build")?;
    let mut statements = build.block.stmts.iter();
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
    }
    let mut fields = Vec::new();
    for d in &module.plan.definitions {
        let name = symbol_name(&d.result);
        let id = &results[&name];
        let row_info = &info[&name];
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
    }
    let storage = format_ident!("{}Storage", module.program.name);
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
        )?;
        fields.push(id);
    }
    expect_statement(
        &mut statements,
        &Stmt::Expr(syn::parse_quote!(#storage { #(#fields,)* }), None),
    )?;
    let query = method(target, &storage, "query")?;
    let mut query_statements = query.block.stmts.iter();
    for id in &fields {
        expect_statement(
            &mut query_statements,
            &syn::parse_quote!(let #id = &self.#id;),
        )?;
    }
    let plan = match query_statements.next()? {
        Stmt::Macro(m) if m.semi_token.is_none() => access_macro(&m.mac, access)?,
        Stmt::Expr(e, None) => access_expression(e, access)?,
        _ => return None,
    };
    if query_statements.next().is_some() {
        return None;
    }
    check_lazy_clauses(&context, &plan)?;
    statements.next().is_none().then_some(())
}

fn check_index(
    statements: &mut std::slice::Iter<'_, Stmt>,
    index: &Ident,
    rows: &Ident,
    arity: usize,
    key: &[usize],
    payload: bool,
) -> Option<()> {
    let local = next_local(statements, index)?;
    let key_type = collection_key_type(local, index, payload.then_some(arity - key.len()))?;
    let syn::Type::Tuple(tuple) = key_type else {
        return None;
    };
    if tuple.elems.len() != key.len() {
        return None;
    }
    let keys = cloned_columns("row", key);
    if payload {
        let complement: Vec<_> = (0..arity).filter(|i| !key.contains(i)).collect();
        let values = cloned_columns("row", &complement);
        equal(
            initializer(local)?,
            &syn::parse_quote!(::std::collections::HashMap::new()),
        )?;
        expect_statement(
            statements,
            &syn::parse_quote!(for row in &#rows { #index.entry(#keys).or_default().push(#values); }),
        )?;
    } else {
        equal(
            initializer(local)?,
            &syn::parse_quote!(#rows.iter().map(|row| #keys).collect()),
        )?;
    }
    Some(())
}

fn check_lazy_clauses(context: &RuleContext<'_>, plan: &ac::Plan) -> Option<()> {
    let (source, info, vars, results) = (
        context.source,
        &context.info,
        &context.variables,
        &context.results,
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
            ra::Operator::Select(s) => host_expression(
                guard_value(clauses.next()?)?,
                &s.condition,
                &info[&symbol_name(&s.input)],
                vars,
            )?,
            ra::Operator::Extend(e) => {
                let fresh: Vec<_> = e
                    .columns
                    .iter()
                    .map(|c| vars[&symbol_name(&c.name)].clone())
                    .collect();
                host_expression(
                    extend_value(clauses.next()?, &fresh)?,
                    &e.value,
                    &info[&symbol_name(&e.input)],
                    vars,
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
            host_expression(actual, original, output, vars)?;
        }
    }
    Some(())
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
        let term = |t: &cq::Term| match t {
            cq::Term::Integer(n) => {
                quote!(&#n)
            }
            cq::Term::Variable(v) => {
                let name = symbol_name(v);
                let id = &vars[&name];
                if input.computed.contains(&name) {
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
        if input.computed.contains(name) {
            syn::parse_quote!(&#variable)
        } else {
            syn::parse_quote!(#variable)
        }
    })
}

fn supported(source: &ir::Module) -> bool {
    super::domain::is_linear_prefix(&source.relational_plan)
        && super::coverage::check_coverage(source).is_ok()
        && source.indexes.iter().all(|key| {
            key.result_token.is_some()
                && source.relational_plan.plan.definitions.iter().any(|d| {
                    symbol_name(&d.result) == symbol_name(&key.relation)
                        && matches!(d.operator, ra::Operator::Rename(_))
                })
        })
}
