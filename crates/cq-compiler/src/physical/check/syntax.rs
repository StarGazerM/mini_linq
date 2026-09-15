//! Shared Rust/Access syntax observations for independent physical checks.
//! No lowering or physical strategy selection takes place here.
use super::*;

pub(super) fn alias_expression(
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

pub(super) fn check_head(plan: &ac::Plan, names: &[Ident]) -> Option<()> {
    plan.head.atom.variables().eq(names.iter()).then_some(())
}

pub(super) fn check_scan(clause: &ac::Clause, names: &[Ident], rows: &Expr) -> Option<()> {
    if names.is_empty() {
        check_if(clause, &syn::parse_quote!(!#rows.is_empty()))
    } else {
        check_for(clause, names, &syn::parse_quote!(#rows.iter()))
    }
}

pub(super) fn check_for(clause: &ac::Clause, names: &[Ident], expected: &Expr) -> Option<()> {
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

pub(super) fn check_if(clause: &ac::Clause, expected: &Expr) -> Option<()> {
    let ac::RustAccess::If { condition, .. } = &clause.access else {
        return None;
    };
    equal(condition, expected)
}

pub(super) fn tuple_names(heading: &BTreeSet<String>, vars: &Names) -> Vec<Ident> {
    heading.iter().map(|v| vars[v].clone()).collect()
}

pub(super) fn cloned_names(names: &[Ident]) -> Expr {
    syn::parse_quote!((#(::core::clone::Clone::clone(#names),)*))
}

pub(super) fn cloned_columns(row: &str, columns: &[usize]) -> Expr {
    let row = format_ident!("{row}");
    let columns = columns.iter().copied().map(syn::Index::from);
    syn::parse_quote!((#(::core::clone::Clone::clone(&#row.#columns),)*))
}

pub(super) fn equal(actual: &Expr, expected: &Expr) -> Option<()> {
    (actual == expected).then_some(())
}

pub(super) fn method<'a>(
    file: &'a syn::File,
    owner: &Ident,
    name: &str,
) -> Option<&'a syn::ImplItemFn> {
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

pub(super) fn locals(statements: &[Stmt]) -> BTreeMap<String, &syn::Local> {
    statements
        .iter()
        .filter_map(|s| match s {
            Stmt::Local(local) => local_name(local).map(|id| (symbol_name(id), local)),
            _ => None,
        })
        .collect()
}

pub(super) fn local_name(local: &syn::Local) -> Option<&Ident> {
    let pattern = match &local.pat {
        syn::Pat::Type(p) => p.pat.as_ref(),
        p => p,
    };
    match pattern {
        syn::Pat::Ident(p) if p.by_ref.is_none() && p.subpat.is_none() => Some(&p.ident),
        _ => None,
    }
}

pub(super) fn initializer(local: &syn::Local) -> Option<&Expr> {
    let init = local.init.as_ref()?;
    init.diverge.is_none().then_some(init.expr.as_ref())
}

pub(super) fn tuple_set_type(local: &syn::Local, types: &[syn::Type]) -> Option<()> {
    let syn::Pat::Type(p) = &local.pat else {
        return None;
    };
    (p.ty.as_ref() == &syn::parse_quote!(::std::collections::HashSet<(#(#types,)*)>)).then_some(())
}

pub(super) fn last_expression(statements: &[Stmt]) -> Option<&Expr> {
    match statements.last()? {
        Stmt::Expr(expr, None) => Some(expr),
        _ => None,
    }
}

pub(super) fn pull(expr: &Expr) -> Option<ac::Plan> {
    let Expr::Macro(mac) = expr else {
        return None;
    };
    pull_macro(&mac.mac)
}

pub(super) fn pull_macro(mac: &syn::Macro) -> Option<ac::Plan> {
    access_macro(mac, &syn::parse_quote!(::mini_linq::pull))
}

pub(super) fn access_macro(mac: &syn::Macro, path: &syn::Path) -> Option<ac::Plan> {
    (mac.path == *path).then_some(())?;
    syn::parse2(mac.tokens.clone()).ok()
}

pub(super) fn next_local<'a>(
    statements: &mut std::slice::Iter<'a, Stmt>,
    name: &Ident,
) -> Option<&'a syn::Local> {
    let Stmt::Local(local) = statements.next()? else {
        return None;
    };
    (local.attrs.is_empty() && local_name(local)? == name).then_some(local)
}

pub(super) fn expect_statement(
    statements: &mut std::slice::Iter<'_, Stmt>,
    expected: &Stmt,
) -> Option<()> {
    (statements.next()? == expected).then_some(())
}

/// Inspect a labeled filter without selecting its representation of host values.
pub(super) fn guard_value(clause: &ac::Clause) -> Option<&Expr> {
    let (cq::BodyItem::Filter(label), ac::RustAccess::If { condition, .. }) =
        (&clause.item, &clause.access)
    else {
        return None;
    };
    equal(&label.condition, &syn::parse_quote!((#condition)))?;
    Some(condition)
}

/// Inspect the one-element enumeration used to bind a computed tuple.
pub(super) fn extend_value<'a>(clause: &'a ac::Clause, fresh: &[Ident]) -> Option<&'a Expr> {
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

/// Check the literal collection type and binding pattern, returning its key type.
/// Tuple-key versus inferred-key requirements belong to the concrete caller.
pub(super) fn collection_key_type<'a>(
    local: &'a syn::Local,
    index: &Ident,
    payload_arity: Option<usize>,
) -> Option<&'a syn::Type> {
    let syn::Pat::Type(typed) = &local.pat else {
        return None;
    };
    let expected: syn::Path = if payload_arity.is_some() {
        syn::parse_quote!(::std::collections::HashMap)
    } else {
        syn::parse_quote!(::std::collections::HashSet)
    };
    let syn::Type::Path(actual) = typed.ty.as_ref() else {
        return None;
    };
    let actual_path: Vec<_> = actual.path.segments.iter().map(|s| &s.ident).collect();
    let expected_path: Vec<_> = expected.segments.iter().map(|s| &s.ident).collect();
    if actual.qself.is_some()
        || actual.path.leading_colon != expected.leading_colon
        || actual_path != expected_path
    {
        return None;
    }
    let pattern: syn::Pat = if payload_arity.is_some() {
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
    if arguments.args.len() != if payload_arity.is_some() { 2 } else { 1 } {
        return None;
    }
    let syn::GenericArgument::Type(key_type) = &arguments.args[0] else {
        return None;
    };
    if let Some(arity) = payload_arity {
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
        let syn::PathArguments::AngleBracketed(element) = &value.path.segments.last()?.arguments
        else {
            return None;
        };
        if element.args.len() != 1 {
            return None;
        }
        let syn::GenericArgument::Type(syn::Type::Tuple(tuple)) = &element.args[0] else {
            return None;
        };
        if tuple.elems.len() != arity {
            return None;
        }
    }
    Some(key_type)
}
