//! Shared Rust/Access syntax observations for independent physical checks.
//! No lowering or physical strategy selection takes place here.
use super::*;

#[cfg(any(feature = "prefix-hash", feature = "materialized-hash"))]
pub fn alias_expression(
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

#[cfg(any(feature = "prefix-hash", feature = "materialized-hash"))]
pub fn check_head(plan: &ac::Plan, names: &[Ident]) -> Option<()> {
    plan.head.atom.variables().eq(names.iter()).then_some(())
}

#[cfg(any(feature = "prefix-hash", feature = "materialized-hash"))]
pub fn check_scan(clause: &ac::Clause, names: &[Ident], rows: &Expr) -> Option<()> {
    if names.is_empty() {
        check_if(clause, &syn::parse_quote!(!#rows.is_empty()))
    } else {
        check_for(clause, names, &syn::parse_quote!(#rows.iter()))
    }
}

pub fn check_for(clause: &ac::Clause, names: &[Ident], expected: &Expr) -> Option<()> {
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

pub fn check_if(clause: &ac::Clause, expected: &Expr) -> Option<()> {
    let ac::RustAccess::If { condition, .. } = &clause.access else {
        return None;
    };
    equal(condition, expected)
}

#[cfg(any(feature = "prefix-hash", feature = "materialized-hash"))]
pub fn tuple_names(heading: &BTreeSet<String>, vars: &Names) -> Vec<Ident> {
    heading.iter().map(|v| vars[v].clone()).collect()
}

#[cfg(any(feature = "base-hash", feature = "materialized-hash"))]
pub fn cloned_names(names: &[Ident]) -> Expr {
    syn::parse_quote!((#(::core::clone::Clone::clone(#names),)*))
}

#[cfg(any(feature = "prefix-hash", feature = "materialized-hash"))]
pub fn cloned_columns(row: &str, columns: &[usize]) -> Expr {
    let row = format_ident!("{row}");
    let columns = columns.iter().copied().map(syn::Index::from);
    syn::parse_quote!((#(::core::clone::Clone::clone(&#row.#columns),)*))
}

pub fn equal(actual: &Expr, expected: &Expr) -> Option<()> {
    (actual == expected).then_some(())
}

pub fn method<'a>(file: &'a syn::File, owner: &Ident, name: &str) -> Option<&'a syn::ImplItemFn> {
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

#[cfg(feature = "materialized-hash")]
pub fn locals(statements: &[Stmt]) -> BTreeMap<String, &syn::Local> {
    statements
        .iter()
        .filter_map(|s| match s {
            Stmt::Local(local) => local_name(local).map(|id| (symbol_name(id), local)),
            _ => None,
        })
        .collect()
}

pub fn local_name(local: &syn::Local) -> Option<&Ident> {
    let pattern = match &local.pat {
        syn::Pat::Type(p) => p.pat.as_ref(),
        p => p,
    };
    match pattern {
        syn::Pat::Ident(p) if p.by_ref.is_none() && p.subpat.is_none() => Some(&p.ident),
        _ => None,
    }
}

pub fn initializer(local: &syn::Local) -> Option<&Expr> {
    let init = local.init.as_ref()?;
    init.diverge.is_none().then_some(init.expr.as_ref())
}

#[cfg(any(feature = "prefix-hash", feature = "materialized-hash"))]
pub fn last_expression(statements: &[Stmt]) -> Option<&Expr> {
    match statements.last()? {
        Stmt::Expr(expr, None) => Some(expr),
        _ => None,
    }
}

#[cfg(any(feature = "base-hash", feature = "prefix-hash"))]
pub fn access_expression(expr: &Expr, path: &syn::Path) -> Option<ac::Plan> {
    let Expr::Macro(mac) = expr else {
        return None;
    };
    access_macro(&mac.mac, path)
}

pub fn access_macro(mac: &syn::Macro, path: &syn::Path) -> Option<ac::Plan> {
    (mac.path == *path).then_some(())?;
    syn::parse2(mac.tokens.clone()).ok()
}

pub fn next_local<'a>(
    statements: &mut std::slice::Iter<'a, Stmt>,
    name: &Ident,
) -> Option<&'a syn::Local> {
    let Stmt::Local(local) = statements.next()? else {
        return None;
    };
    (local.attrs.is_empty() && local_name(local)? == name).then_some(local)
}

pub fn expect_statement(
    statements: &mut std::slice::Iter<'_, Stmt>,
    expected: &Stmt,
) -> Option<()> {
    (statements.next()? == expected).then_some(())
}

/// Inspect a labeled filter without selecting its representation of host values.
#[cfg(any(feature = "prefix-hash", feature = "materialized-hash"))]
pub fn guard_value(clause: &ac::Clause) -> Option<&Expr> {
    let (cq::BodyItem::Filter(label), ac::RustAccess::If { condition, .. }) =
        (&clause.item, &clause.access)
    else {
        return None;
    };
    equal(&label.condition, &syn::parse_quote!((#condition)))?;
    Some(condition)
}

/// Inspect the one-element enumeration used to bind a computed tuple.
#[cfg(any(feature = "prefix-hash", feature = "materialized-hash"))]
pub fn extend_value<'a>(clause: &'a ac::Clause, fresh: &[Ident]) -> Option<&'a Expr> {
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
    Some(value)
}
