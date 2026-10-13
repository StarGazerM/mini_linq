//! Conservative totality recognition over declared standard column types.
use std::collections::{BTreeMap, BTreeSet};
use crate::{BodyItem, CommaList, Constraint, RelationDecl, rust, symbol_name};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { String, Scalar, Bool }

fn kind(ty: &syn::Type) -> Option<Kind> {
    match ty {
        syn::Type::Reference(r) => kind(&r.elem),
        syn::Type::Paren(p) => kind(&p.elem),
        syn::Type::Group(g) => kind(&g.elem),
        syn::Type::Path(p) if p.qself.is_none() => {
            let parts: Vec<_> = p.path.segments.iter().map(|s| symbol_name(&s.ident)).collect();
            let name = parts.join("::");
            match name.as_str() {
                "str" | "String" | "std::string::String" | "alloc::string::String" => Some(Kind::String),
                "bool" => Some(Kind::Bool),
                "i8" | "i16" | "i32" | "i64" | "i128" | "isize" |
                "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "char" => Some(Kind::Scalar),
                _ => None,
            }
        }
        _ => None,
    }
}

fn value(expr: &syn::Expr, types: &BTreeMap<String, syn::Type>) -> Option<Kind> {
    if let Some(id) = rust::variable(expr) {
        return types.get(&symbol_name(id)).and_then(kind);
    }
    match expr {
        syn::Expr::Paren(p) => value(&p.expr, types),
        syn::Expr::Group(g) => value(&g.expr, types),
        syn::Expr::Reference(r) if r.mutability.is_none() =>
            rust::variable(rust::unparen(&r.expr)).and_then(|id| types.get(&symbol_name(id))).and_then(kind),
        syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) =>
            rust::variable(rust::unparen(&u.expr)).and_then(|id| types.get(&symbol_name(id))).and_then(kind),
        syn::Expr::Lit(l) => match l.lit {
            syn::Lit::Str(_) => Some(Kind::String),
            syn::Lit::Bool(_) => Some(Kind::Bool),
            syn::Lit::Int(_) | syn::Lit::Char(_) => Some(Kind::Scalar),
            _ => None,
        },
        _ => None,
    }
}

/// Formal rule CQ-Total: Recognize only total standard string operations and Boolean/comparison compositions over declared primitive columns, or trust an explicit `#[total]` guard assertion.
fn allowlisted(expr: &syn::Expr, types: &BTreeMap<String, syn::Type>) -> bool {
    if rust::free_variables(expr).opaque { return false; }
    match expr {
        syn::Expr::Paren(p) => allowlisted(&p.expr, types),
        syn::Expr::Group(g) => allowlisted(&g.expr, types),
        syn::Expr::Lit(l) => matches!(l.lit, syn::Lit::Bool(_)),
        syn::Expr::MethodCall(m) => m.turbofish.is_none()
            && matches!(symbol_name(&m.method).as_str(), "contains" | "starts_with" | "ends_with")
            && value(&m.receiver, types) == Some(Kind::String)
            && m.args.len() == 1
            && value(&m.args[0], types) == Some(Kind::String),
        syn::Expr::Binary(b) => match b.op {
            syn::BinOp::And(_) | syn::BinOp::Or(_) => allowlisted(&b.left, types) && allowlisted(&b.right, types),
            syn::BinOp::Eq(_) | syn::BinOp::Ne(_) | syn::BinOp::Lt(_) | syn::BinOp::Le(_) |
            syn::BinOp::Gt(_) | syn::BinOp::Ge(_) => value(&b.left, types).is_some()
                && value(&b.left, types) == value(&b.right, types),
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn recognize(
    schema: &BTreeMap<String, &RelationDecl>,
    body: &mut CommaList<BodyItem>,
    prefix: &BTreeMap<String, syn::Type>,
) -> syn::Result<()> {
    let Ok(types) = crate::blocks::types(schema, body, prefix) else { return Ok(()); };
    let logical: BTreeSet<_> = types.keys().cloned().collect();
    for item in body {
        match item {
            BodyItem::Filter(filter) if allowlisted(&filter.condition, &types) => {
                *item = BodyItem::Constraint(Constraint::total(filter.condition.clone(), &logical));
            }
            BodyItem::Block(block) => {
                for branch in &mut block.branches { recognize(schema, &mut branch.body, &types)?; }
            }
            _ => (),
        }
    }
    Ok(())
}
