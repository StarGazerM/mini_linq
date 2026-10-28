//! Invocation-local host values passed to integer-only device row functions.
use std::collections::{BTreeMap, BTreeSet};
use cq_lang_structures::syntax::{arithmetic, rust};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{Expr, Ident, Type};

#[derive(Default)]
pub(crate) struct Captures {
    pub setup: TokenStream,
    pub parameters: Vec<(Ident, Type, TokenStream)>,
    values: BTreeMap<String, usize>,
    pub used: BTreeSet<usize>,
}

/// Formal rule GPU-Capture: capture dependencies are recorded while checking each physical expression; launch arguments consume the required nonempty enabled domains.
impl Captures {
    pub fn value(&mut self, expression: &Expr, logical: &Type, codecs: &crate::runs::Codecs) -> Expr {
        let key = format!("{}:{}", logical.to_token_stream(), expression.to_token_stream());
        if let Some(&index) = self.values.get(&key) {
            self.used.insert(index);
            let name = &self.parameters[index].0;
            return syn::parse_quote!(*#name);
        }
        let name = format_ident!("__cq_capture_{}", self.parameters.len());
        let cache = format_ident!("__cq_capture_cache_{}", self.parameters.len());
        let (ty, value) = if let Some(codec) = codecs.get(logical) {
            (syn::parse_quote!(u32), quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#logical>>::code(&(#expression))))
        } else { (logical.clone(), quote!(#expression)) };
        self.setup.extend(quote!(let #cache = ::std::cell::OnceCell::<#ty>::new();));
        self.parameters.push((name.clone(), ty, quote!(*#cache.get_or_init(|| #value))));
        let index = self.parameters.len() - 1;
        self.values.insert(key, index);
        self.used.insert(index);
        syn::parse_quote!(*#name)
    }

    pub fn expression(&mut self, expression: &Expr, types: &BTreeMap<String, Type>, expected: Option<&Type>, codecs: &crate::runs::Codecs) -> syn::Result<Expr> {
        let names: BTreeSet<_> = types.keys().cloned().collect();
        let closed = rust::free_variables(expression).dependencies(&names).is_empty();
        if closed {
            // Keep integer literals in CQ-Arith so literal nonzero division retains its totality proof.
            if cq_lang_structures::syntax::equality::literal(rust::unparen(expression))
                && expected.is_none_or(|ty| codecs.get(ty).is_none()) {
                return Ok(expression.clone());
            }
            let ty = expected.ok_or_else(|| syn::Error::new_spanned(expression, "GPU-Capture: closed expression requires a declared scalar context"))?;
            return Ok(self.value(expression, ty, codecs));
        }
        let mut result = expression.clone();
        match &mut result {
            Expr::Paren(value) => value.expr = Box::new(self.expression(&value.expr, types, expected, codecs)?),
            Expr::Group(value) => value.expr = Box::new(self.expression(&value.expr, types, expected, codecs)?),
            Expr::Binary(value) => {
                let boolean = matches!(value.op, syn::BinOp::And(_) | syn::BinOp::Or(_));
                let comparison = matches!(value.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_) | syn::BinOp::Lt(_) | syn::BinOp::Le(_) | syn::BinOp::Gt(_) | syn::BinOp::Ge(_));
                let ty = if boolean { Some(syn::parse_quote!(bool)) } else {
                    infer(&value.left, types).or_else(|| infer(&value.right, types)).or_else(|| (!comparison).then(|| expected.cloned()).flatten())
                };
                value.left = Box::new(self.expression(&value.left, types, ty.as_ref(), codecs)?);
                value.right = Box::new(self.expression(&value.right, types, ty.as_ref(), codecs)?);
            }
            Expr::Unary(value) if matches!(value.op, syn::UnOp::Not(_)) => {
                value.expr = Box::new(self.expression(&value.expr, types, Some(&syn::parse_quote!(bool)), codecs)?);
            }
            Expr::If(value) => {
                value.cond = Box::new(self.expression(&value.cond, types, Some(&syn::parse_quote!(bool)), codecs)?);
                for statement in &mut value.then_branch.stmts {
                    if let syn::Stmt::Expr(expr, None) = statement { *expr = self.expression(expr, types, expected, codecs)?; }
                }
                if let Some((_, other)) = &mut value.else_branch { **other = self.expression(other, types, expected, codecs)?; }
            }
            Expr::Block(value) => {
                for statement in &mut value.block.stmts {
                    if let syn::Stmt::Expr(expr, None) = statement { *expr = self.expression(expr, types, expected, codecs)?; }
                }
            }
            _ => {}
        }
        Ok(result)
    }

    pub fn types(&self, types: &BTreeMap<String, Type>, codecs: &crate::runs::Codecs) -> BTreeMap<String, Type> {
        types.iter().map(|(name, ty)| (name.clone(), if codecs.get(ty).is_some() { syn::parse_quote!(u32) } else { ty.clone() }))
            .chain(self.parameters.iter().map(|(name, ty, _)| (name.to_string(), ty.clone()))).collect()
    }
}

fn infer(expression: &Expr, types: &BTreeMap<String, Type>) -> Option<Type> {
    match expression {
        Expr::Path(path) => types.get(&path.path.get_ident()?.to_string()).cloned(),
        Expr::Paren(value) => infer(&value.expr, types),
        Expr::Group(value) => infer(&value.expr, types),
        Expr::Unary(value) => infer(&value.expr, types),
        Expr::Binary(value) => infer(&value.left, types).or_else(|| infer(&value.right, types)),
        Expr::If(value) => value.then_branch.stmts.iter().find_map(|statement| match statement {
            syn::Stmt::Expr(value, None) => infer(value, types),
            _ => None,
        }).or_else(|| value.else_branch.as_ref().and_then(|(_, value)| infer(value, types))),
        Expr::Block(value) => value.block.stmts.iter().find_map(|statement| match statement {
            syn::Stmt::Expr(value, None) => infer(value, types),
            _ => None,
        }),
        Expr::Call(call) if call.args.len() == 1 => {
            let Expr::Path(function) = &*call.func else { return None };
            let mut path = function.path.clone();
            if path.segments.last()?.ident != "from" { return None }
            path.segments.pop();
            path.segments.pop_punct();
            Some(Type::Path(syn::TypePath { qself: None, path }))
        }
        Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(value), .. }) if !value.suffix().is_empty() => syn::parse_str(value.suffix()).ok(),
        _ => None,
    }
}

pub(crate) fn predicate(expression: &Expr, types: &BTreeMap<String, Type>) -> Option<Expr> {
    match expression {
        Expr::Lit(value) if matches!(value.lit, syn::Lit::Bool(_)) => Some(expression.clone()),
        Expr::Paren(value) => predicate(&value.expr, types),
        Expr::Unary(value) if matches!(value.op, syn::UnOp::Deref(_)) && infer(&value.expr, types) == Some(syn::parse_quote!(bool)) => Some(expression.clone()),
        Expr::Unary(value) if matches!(value.op, syn::UnOp::Not(_)) => {
            let inner = predicate(&value.expr, types)?;
            Some(syn::parse_quote!(!(#inner)))
        }
        Expr::Binary(value) if matches!(value.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) => {
            let left = predicate(&value.left, types)?;
            let right = predicate(&value.right, types)?;
            let op = &value.op;
            Some(syn::parse_quote!((#left) #op (#right)))
        }
        _ => arithmetic::predicate(expression, types),
    }
}

pub(crate) fn unborrow(expression: &mut Expr, captures: &Captures) {
    use syn::visit_mut::VisitMut;
    struct Unborrow<'a>(&'a Captures);
    impl VisitMut for Unborrow<'_> {
        fn visit_expr_mut(&mut self, expression: &mut Expr) {
            if let Expr::Unary(value) = expression
                && matches!(value.op, syn::UnOp::Deref(_))
                && let Expr::Path(path) = &*value.expr
                && path.path.get_ident().is_some_and(|name|
                    self.0.parameters.iter().any(|(capture, _, _)| capture == name))
            {
                *expression = (*value.expr).clone();
                return;
            }
            syn::visit_mut::visit_expr_mut(self, expression);
        }
    }
    Unborrow(captures).visit_expr_mut(expression);
}


pub(crate) fn simplify_references(expression: &mut Expr) {
    use syn::visit_mut::VisitMut;
    struct Simplify;
    impl VisitMut for Simplify {
        fn visit_expr_mut(&mut self, expression: &mut Expr) {
            syn::visit_mut::visit_expr_mut(self, expression);
            if let Expr::Unary(value) = expression
                && matches!(value.op, syn::UnOp::Deref(_))
                && let Expr::Reference(reference) = &*value.expr {
                let inner = rust::unparen(&reference.expr);
                *expression = if matches!(inner, Expr::Path(_) | Expr::Lit(_) | Expr::Unary(_)) {
                    inner.clone()
                } else { (*reference.expr).clone() };
            }
        }
    }
    Simplify.visit_expr_mut(expression);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbolic_guard_uses_declared_codec_and_integer_parameter() {
        let types = BTreeMap::from([("t".into(), syn::parse_quote!(TestSymbol))]);
        let mut codecs = crate::runs::Codecs::default();
        codecs.record(&syn::parse_quote!(TestSymbol), &syn::parse_quote!(TestCodec)).unwrap();
        let mut captures = Captures::default();
        let source = syn::parse_quote!(*t == sym("x"));
        let rewritten = captures.expression(&source, &types, Some(&syn::parse_quote!(bool)), &codecs).unwrap();
        let mut lowered = predicate(&rewritten, &captures.types(&types, &codecs)).unwrap();
        unborrow(&mut lowered, &captures);
        assert_eq!(captures.parameters.len(), 1);
        assert_eq!(captures.parameters[0].1, syn::parse_quote!(u32));
        let host = captures.parameters[0].2.to_string();
        assert!(host.contains("SymbolCodec < TestSymbol >"));
        assert!(host.contains("sym (\"x\")"));
        assert!(!lowered.to_token_stream().to_string().contains("sym"));
        captures.expression(&source, &types, Some(&syn::parse_quote!(bool)), &codecs).unwrap();
        assert_eq!(captures.parameters.len(), 1);
    }


    #[test]
    fn row_dependent_calls_are_not_extracted_as_host_captures() {
        let types = BTreeMap::from([("x".into(), syn::parse_quote!(i32))]);
        let mut captures = Captures::default();
        let source = syn::parse_quote!(*x == opaque(*x));
        let expression = captures.expression(&source, &types, Some(&syn::parse_quote!(bool)), &crate::runs::Codecs::default()).unwrap();
        assert!(captures.parameters.is_empty());
        assert!(predicate(&expression, &types).is_none());
        assert!(!arithmetic::capture_predicate(&source, &types));
    }
}

