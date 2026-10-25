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
    values: BTreeMap<String, Ident>,
}

// Formal rule GPU-Capture: closed row subexpressions are evaluated on the host once per nonempty invocation and passed as integer kernel parameters.
impl Captures {
    pub fn value(&mut self, expression: &Expr, logical: &Type, codecs: &BTreeMap<String, Type>) -> Expr {
        let key = format!("{}:{}", logical.to_token_stream(), expression.to_token_stream());
        if let Some(name) = self.values.get(&key) { return syn::parse_quote!(*#name); }
        let name = format_ident!("__cq_capture_{}", self.parameters.len());
        let cache = format_ident!("__cq_capture_cache_{}", self.parameters.len());
        let (ty, value) = if let Some(codec) = codecs.get(&logical.to_token_stream().to_string()) {
            (syn::parse_quote!(u32), quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#logical>>::code(&(#expression))))
        } else { (logical.clone(), quote!(#expression)) };
        self.setup.extend(quote!(let #cache = ::std::cell::OnceCell::<#ty>::new();));
        self.parameters.push((name.clone(), ty, quote!(*#cache.get_or_init(|| #value))));
        self.values.insert(key, name.clone());
        syn::parse_quote!(*#name)
    }

    pub fn expression(&mut self, expression: &Expr, types: &BTreeMap<String, Type>, expected: Option<&Type>, codecs: &BTreeMap<String, Type>) -> syn::Result<Expr> {
        let names: BTreeSet<_> = types.keys().cloned().collect();
        let closed = rust::free_variables(expression).dependencies(&names).is_empty();
        if closed {
            // Keep integer literals in CQ-Arith so literal nonzero division retains its totality proof.
            if cq_lang_structures::syntax::equality::literal(rust::unparen(expression))
                && expected.is_none_or(|ty| !codecs.contains_key(&ty.to_token_stream().to_string())) {
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

    pub fn types(&self, types: &BTreeMap<String, Type>, codecs: &BTreeMap<String, Type>) -> BTreeMap<String, Type> {
        types.iter().map(|(name, ty)| (name.clone(), if codecs.contains_key(&ty.to_token_stream().to_string()) { syn::parse_quote!(u32) } else { ty.clone() }))
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

pub(crate) fn unborrow(expression: &mut Expr) {
    use syn::visit_mut::VisitMut;
    struct Unborrow;
    impl VisitMut for Unborrow {
        fn visit_expr_mut(&mut self, expression: &mut Expr) {
            if let Expr::Unary(value) = expression
                && matches!(value.op, syn::UnOp::Deref(_))
                && let Expr::Path(path) = &*value.expr
                && path.path.get_ident().is_some_and(|name| {
                    name.to_string().strip_prefix("__cq_capture_")
                        .is_some_and(|suffix| suffix.parse::<usize>().is_ok())
                })
            {
                *expression = (*value.expr).clone();
                return;
            }
            syn::visit_mut::visit_expr_mut(self, expression);
        }
    }
    Unborrow.visit_expr_mut(expression);
}

/// Guard host argument evaluation by the actual dispatch input, including
/// disabled choice arms. OnceCell survives output-capacity retries.
pub(crate) fn launches(kernels: &[TokenStream], launches: &mut [TokenStream]) -> syn::Result<()> {
    use syn::visit_mut::VisitMut;
    use syn::visit::Visit;
    #[derive(Default)]
    struct Uses {
        branch: Option<String>,
        captures: BTreeMap<String, BTreeSet<String>>,
    }
    impl<'ast> Visit<'ast> for Uses {
        fn visit_expr_if(&mut self, expression: &'ast syn::ExprIf) {
            let branch = if let Expr::Path(path) = &*expression.cond {
                path.path.get_ident().and_then(|name| name.to_string().strip_prefix("__cq_enabled_").map(str::to_owned))
            } else { None };
            let previous = self.branch.clone();
            if branch.is_some() { self.branch = branch; }
            syn::visit::visit_expr_if(self, expression);
            self.branch = previous;
        }
        fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
            if let Some(name) = expression.path.get_ident()
                && name.to_string().starts_with("__cq_capture_")
                && let Some(branch) = &self.branch {
                self.captures.entry(name.to_string()).or_default().insert(branch.clone());
            }
        }
    }
    struct Signature {
        parameters: Vec<String>,
        captures: BTreeMap<String, BTreeSet<String>>,
    }
    let signatures = kernels.iter().map(|tokens| {
        let kernel: syn::ItemFn = syn::parse2(tokens.clone())?;
        let parameters = kernel.sig.inputs.iter().map(|argument| {
            let syn::FnArg::Typed(argument) = argument else { unreachable!() };
            let syn::Pat::Ident(name) = &*argument.pat else { unreachable!() };
            name.ident.to_string()
        }).collect::<Vec<_>>();
        let mut uses = Uses::default();
        uses.visit_block(&kernel.block);
        Ok((kernel.sig.ident.to_string(), Signature { parameters, captures: uses.captures }))
    }).collect::<syn::Result<BTreeMap<_, _>>>()?;
    struct Launches<'a>(&'a BTreeMap<String, Signature>);
    impl VisitMut for Launches<'_> {
        fn visit_expr_method_call_mut(&mut self, call: &mut syn::ExprMethodCall) {
            syn::visit_mut::visit_expr_method_call_mut(self, call);
            let Some(signature) = self.0.get(&call.method.to_string()) else { return };
            let parameters = &signature.parameters;
            let mut conditions = BTreeMap::new();
            for (index, name) in parameters.iter().enumerate() {
                if let Some(suffix) = name.strip_prefix("__cq_domain_") {
                    let length = &call.args[index + 2];
                    let enabled_name = format!("__cq_enabled_{suffix}");
                    let enabled = parameters.iter().position(|name| name == &enabled_name).map(|index| &call.args[index + 2]);
                    conditions.insert(suffix.to_owned(), match enabled {
                        Some(enabled) => quote!((#length != 0 && #enabled)),
                        None => quote!((#length != 0)),
                    });
                } else if name == "length" || name == "__length" {
                    let length = &call.args[index + 2];
                    conditions.insert(String::new(), quote!((#length != 0)));
                }
            }
            for (index, name) in parameters.iter().enumerate() {
                if name.starts_with("__cq_capture_") {
                    let conditions = conditions.iter().filter_map(|(branch, condition)| {
                        signature.captures.get(name).is_none_or(|branches| branches.contains(branch))
                            .then_some(condition)
                    }).collect::<Vec<_>>();
                    let value = &call.args[index + 2];
                    call.args[index + 2] = syn::parse_quote!(if false #(|| #conditions)* { #value } else { ::core::default::Default::default() });
                }
            }
        }
    }
    for tokens in launches {
        let mut block: syn::Block = syn::parse2(quote!({ #tokens }))?;
        Launches(&signatures).visit_block_mut(&mut block);
        let statements = block.stmts;
        *tokens = quote!(#(#statements)*);
    }
    Ok(())
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
        let codecs = BTreeMap::from([("TestSymbol".into(), syn::parse_quote!(TestCodec))]);
        let mut captures = Captures::default();
        let source = syn::parse_quote!(*t == sym("x"));
        let rewritten = captures.expression(&source, &types, Some(&syn::parse_quote!(bool)), &codecs).unwrap();
        let mut lowered = predicate(&rewritten, &captures.types(&types, &codecs)).unwrap();
        unborrow(&mut lowered);
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
    fn launch_capture_evaluation_is_guarded_by_nonempty_enabled_input() {
        let kernels = vec![quote! {
            fn dispatch(__cq_capture_0: i32, __cq_domain_0: u32, __cq_enabled_0: bool) {}
        }];
        let mut calls = vec![quote! {
            module.dispatch(stream, prepared, *cache.get_or_init(|| threshold()), length, enabled)?;
        }];
        launches(&kernels, &mut calls).unwrap();
        let generated = calls[0].to_string();
        assert!(generated.contains("length != 0 && enabled"));
        assert!(generated.contains("get_or_init"));
        assert!(generated.contains("Default :: default"));
    }

    #[test]
    fn row_dependent_calls_are_not_extracted_as_host_captures() {
        let types = BTreeMap::from([("x".into(), syn::parse_quote!(i32))]);
        let mut captures = Captures::default();
        let source = syn::parse_quote!(*x == opaque(*x));
        let expression = captures.expression(&source, &types, Some(&syn::parse_quote!(bool)), &BTreeMap::new()).unwrap();
        assert!(captures.parameters.is_empty());
        assert!(predicate(&expression, &types).is_none());
        assert!(!arithmetic::capture_predicate(&source, &types));
    }
}

#[cfg(test)]
mod branch_tests {
    use super::*;

    #[test]
    fn another_nonempty_choice_arm_does_not_force_a_capture() {
        let kernels = vec![quote! {
            fn dispatch(__cq_capture_0: i32, __cq_domain_0: u32, __cq_enabled_0: bool,
                __cq_domain_1: u32, __cq_enabled_1: bool) {
                if __cq_enabled_0 { let _ = __cq_domain_0; }
                if __cq_enabled_1 { let _ = (__cq_domain_1, __cq_capture_0); }
            }
        }];
        let mut calls = vec![quote! {
            module.dispatch(stream, prepared, capture(), left_len, left_enabled, right_len, right_enabled)?;
        }];
        launches(&kernels, &mut calls).unwrap();
        let generated = calls[0].to_string();
        assert!(generated.contains("right_len != 0 && right_enabled"));
        assert!(!generated.contains("left_len != 0"));
    }
}
