//! CQ's total, fixed-width integer arithmetic. Decimal coefficients use their
//! declared integer representation; arbitrary Rust calls are never recognized.
use crate::{algebra::Bind, constraint::Comparison, rust, symbol_name};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
use syn::{BinOp, Expr, Type, UnOp};

fn plain(mut e: &Expr) -> &Expr {
    loop {
        match e {
            Expr::Paren(p) if p.attrs.is_empty() => e = &p.expr,
            Expr::Group(p) if p.attrs.is_empty() => e = &p.expr,
            _ => return e,
        }
    }
}

fn integer(ty: &Type) -> bool {
    let Type::Path(p) = ty else { return false };
    p.qself.is_none()
        && p.path.leading_colon.is_none()
        && p.path.segments.len() == 1
        && matches!(p.path.segments[0].arguments, syn::PathArguments::None)
        && matches!(
            p.path.segments[0].ident.to_string().as_str(),
            "i8" | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
        )
}

fn decimal_coefficient(e: &Expr, types: &BTreeMap<String, Type>) -> Option<Expr> {
    let e = plain(e);
    let receiver = match e {
        Expr::MethodCall(call)
            if call.attrs.is_empty()
                && call.method == "scaled"
                && call.turbofish.is_none()
                && call.args.is_empty() =>
        {
            &*call.receiver
        }
        _ => e,
    };
    let name = variable(receiver)?;
    let Type::Path(path) = types.get(&symbol_name(name))? else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    let segments = &path.path.segments;
    let decimal = segments.last()?;
    if decimal.ident != "Decimal"
        || !(segments.len() == 1 || (segments.len() == 2 && segments[0].ident == "cq_load"))
    {
        return None;
    }
    // The emitted canonical type enforces the language's representation, even
    // if the source's unqualified Decimal name is shadowed by a Rust type.
    Some(syn::parse_quote!(::cq_load::Decimal::scaled(*#name)))
}

fn literal(e: &Expr) -> Option<&syn::LitInt> {
    match plain(e) {
        Expr::Lit(l) if l.attrs.is_empty() => match &l.lit {
            syn::Lit::Int(n) => Some(n),
            _ => None,
        },
        Expr::Unary(u) if u.attrs.is_empty() && matches!(u.op, UnOp::Neg(_)) => {
            match plain(&u.expr) {
                Expr::Lit(l) if l.attrs.is_empty() => match &l.lit {
                    syn::Lit::Int(n) => Some(n),
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

fn variable(e: &Expr) -> Option<&syn::Ident> {
    let e = plain(e);
    let value = match e {
        Expr::Unary(u) if u.attrs.is_empty() && matches!(u.op, UnOp::Deref(_)) => plain(&u.expr),
        _ => e,
    };
    match value {
        Expr::Path(path) if path.attrs.is_empty() => rust::variable(value),
        _ => None,
    }
}

fn branch(block: &syn::Block) -> Option<&Expr> {
    match block.stmts.as_slice() {
        [syn::Stmt::Expr(value, None)] => Some(value),
        _ => None,
    }
}

fn widening<'a>(e: &'a Expr, types: &BTreeMap<String, Type>) -> Option<(Type, Type, &'a Expr)> {
    let Expr::Call(call) = plain(e) else {
        return None;
    };
    if !call.attrs.is_empty() || call.args.len() != 1 {
        return None;
    }
    let Expr::Path(function) = &*call.func else {
        return None;
    };
    let segments = &function.path.segments;
    if !function.attrs.is_empty()
        || function.qself.is_some()
        || segments.len() != 2
        || segments[1].ident != "from"
        || segments
            .iter()
            .any(|s| !matches!(s.arguments, syn::PathArguments::None))
    {
        return None;
    }
    let target = &segments[0].ident;
    let target: Type = syn::parse_quote!(#target);
    let argument = call.args.first()?;
    let source = infer(argument, types)?;
    fn shape(ty: &Type) -> Option<(bool, u32)> {
        if !integer(ty) {
            return None;
        }
        let Type::Path(path) = ty else { return None };
        let name = path.path.segments[0].ident.to_string();
        Some((name.starts_with('i'), name[1..].parse().ok()?))
    }
    let (signed_from, width_from) = shape(&source)?;
    let (signed_to, width_to) = shape(&target)?;
    let lossless = if signed_from == signed_to {
        width_to >= width_from
    } else {
        !signed_from && signed_to && width_to > width_from
    };
    lossless.then_some((source, target, argument))
}

fn infer(e: &Expr, types: &BTreeMap<String, Type>) -> Option<Type> {
    if decimal_coefficient(e, types).is_some() {
        return Some(syn::parse_quote!(i64));
    }
    if let Some((_, target, _)) = widening(e, types) {
        return Some(target);
    }
    if let Some(v) = variable(e) {
        return types.get(&symbol_name(v)).filter(|ty| integer(ty)).cloned();
    }
    if let Some(n) = literal(e) {
        return (!n.suffix().is_empty())
            .then(|| syn::parse_str(n.suffix()).ok())
            .flatten();
    }
    if let Expr::Binary(b) = plain(e) {
        return infer(&b.left, types).or_else(|| infer(&b.right, types));
    }
    if let Expr::If(conditional) = plain(e) {
        return infer(branch(&conditional.then_branch)?, types).or_else(|| {
            let otherwise = &conditional.else_branch.as_ref()?.1;
            match plain(otherwise) {
                Expr::Block(block) => infer(branch(&block.block)?, types),
                other => infer(other, types),
            }
        });
    }
    None
}

fn lower(e: &Expr, types: &BTreeMap<String, Type>, ty: &Type) -> Option<Expr> {
    let e = plain(e);
    if *ty == syn::parse_quote!(i64) {
        if let Some(coefficient) = decimal_coefficient(e, types) {
            return Some(coefficient);
        }
    }
    if let Some(v) = variable(e) {
        return (types.get(&symbol_name(v)) == Some(ty)).then(|| syn::parse_quote!(*#v));
    }
    if let Some((source, target, argument)) = widening(e, types) {
        if &target != ty {
            return None;
        }
        let value = lower(argument, types, &source)?;
        let (Type::Path(source), Type::Path(target)) = (source, target) else {
            return None;
        };
        let source = &source.path.segments[0].ident;
        let target = &target.path.segments[0].ident;
        return Some(syn::parse_quote!(
            <::core::primitive::#target as ::core::convert::From<::core::primitive::#source>>::from(#value)
        ));
    }
    if let Expr::If(conditional) = e {
        if !conditional.attrs.is_empty() {
            return None;
        }
        let condition = predicate(&conditional.cond, types)?;
        let yes = lower(branch(&conditional.then_branch)?, types, ty)?;
        let otherwise = &conditional.else_branch.as_ref()?.1;
        let no = match plain(otherwise) {
            Expr::Block(block) if block.attrs.is_empty() && block.label.is_none() => {
                lower(branch(&block.block)?, types, ty)?
            }
            Expr::If(_) => lower(otherwise, types, ty)?,
            _ => return None,
        };
        return Some(syn::parse_quote!(if #condition { #yes } else { #no }));
    }
    if let Some(n) = literal(e) {
        if !n.suffix().is_empty() && syn::parse_str::<Type>(n.suffix()).ok().as_ref() != Some(ty) {
            return None;
        }
        return Some(e.clone());
    }
    let Expr::Binary(b) = e else { return None };
    if !b.attrs.is_empty() {
        return None;
    }
    let method = match b.op {
        BinOp::Add(_) => "wrapping_add",
        BinOp::Sub(_) => "wrapping_sub",
        BinOp::Mul(_) => "wrapping_mul",
        BinOp::Div(_) if literal(&b.right)?.base10_parse::<u128>().ok()? != 0 => "wrapping_div",
        _ => return None,
    };
    let method = syn::Ident::new(method, proc_macro2::Span::call_site());
    let left = lower(&b.left, types, ty)?;
    let right = lower(&b.right, types, ty)?;
    let Type::Path(path) = ty else { return None };
    let primitive = &path.path.segments[0].ident;
    Some(syn::parse_quote!(::core::primitive::#primitive::#method(#left, #right)))
}

/// Formal rule CQ-Arith: Defined integer operations have modular, total meaning.
/// Only declared integer names and integer literals participate. Division needs
/// a nonzero literal; signed MIN / -1 wraps to MIN rather than panicking.
pub fn expression(
    e: &Expr,
    types: &BTreeMap<String, Type>,
    expected: Option<&Type>,
) -> Option<Expr> {
    let ty = expected.cloned().or_else(|| infer(e, types))?;
    if !integer(&ty) {
        return None;
    }
    lower(e, types, &ty)
}

/// A scalar Bind earns totality from its grammar and its declared input types.
pub fn binding(bind: &Bind, types: &BTreeMap<String, Type>) -> Option<Expr> {
    if bind.for_token.is_some() || !matches!(bind.pattern, syn::Pat::Ident(_)) {
        return None;
    }
    expression(
        &bind.expression,
        types,
        Some(&bind.annotation.as_ref()?.ty),
    )
}

/// The arithmetic extension of CQ-GuardRecognize. Both Boolean branches must
/// be recognized; no opaque short-circuit evaluation domain is enlarged.
pub fn predicate(e: &Expr, types: &BTreeMap<String, Type>) -> Option<Expr> {
    let e = plain(e);
    if let Expr::Unary(u) = e {
        if u.attrs.is_empty() && matches!(u.op, UnOp::Not(_)) {
            let value = predicate(&u.expr, types)?;
            return Some(syn::parse_quote!(!(#value)));
        }
    }
    let Expr::Binary(b) = e else { return None };
    if !b.attrs.is_empty() {
        return None;
    }
    let op = &b.op;
    if matches!(op, BinOp::And(_) | BinOp::Or(_)) {
        let left = predicate(&b.left, types)?;
        let right = predicate(&b.right, types)?;
        return Some(syn::parse_quote!((#left) #op (#right)));
    }
    if !matches!(
        op,
        BinOp::Eq(_) | BinOp::Ne(_) | BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_)
    ) {
        return None;
    }
    let comparison = match op {
        BinOp::Eq(_) => Comparison::Eq(Default::default()),
        BinOp::Ne(_) => Comparison::Ne(Default::default()),
        BinOp::Lt(_) => Comparison::Lt(Default::default()),
        BinOp::Le(_) => Comparison::Le(Default::default()),
        BinOp::Gt(_) => Comparison::Gt(Default::default()),
        BinOp::Ge(_) => Comparison::Ge(Default::default()),
        _ => return None,
    };
    if let Some(ty) = infer(&b.left, types).or_else(|| infer(&b.right, types)) {
        if let (Some(left), Some(right)) = (
            expression(&b.left, types, Some(&ty)),
            expression(&b.right, types, Some(&ty)),
        ) {
            return syn::parse2(comparison.expression(quote!(&(#left)), quote!(&(#right)), false)).ok();
        }
    }
    let names = types.keys().cloned().collect();
    let left = crate::guard::term(&b.left, &names, &BTreeSet::new())?;
    let right = crate::guard::term(&b.right, &names, &BTreeSet::new())?;
    let parameters = BTreeMap::new();
    syn::parse2(comparison.expression(
        left.reference(&parameters), right.reference(&parameters),
        left.string_literal() || right.string_literal(),
    )).ok()
}
