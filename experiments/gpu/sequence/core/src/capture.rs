//! Host captures and opaque Rust callables for device row expressions.
//!
//! DESIGN contract7-gpu-captures
//! Owns host captures and verbatim device Rust callables (contract 7, GPU-Capture/BE-RustRepresentation).
//! Source: doc/PLANNING-REDESIGN.md Contract 7; doc/formal/CONFORMANCE.md GPU-Capture.
//! Closed relational operands use the compared column's recorded codec; Rust reads require identity.
//! DO NOT infer semantics from types, recognize `if` comparisons, or rewrite Rust arithmetic.
//! Capturing query-closed inputs does not authorize splitting Rust into relational predicates.
use std::collections::{BTreeMap, BTreeSet};
use cq_lang_structures::syntax::{rust, algebra::Theta, constraint::{Comparison, Constraint, Term}, EqualityValue};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{Expr, Ident, Type, visit_mut::VisitMut};

#[derive(Default)]
pub(crate) struct Captures {
    pub setup: TokenStream,
    pub parameters: Vec<(Ident, Type, TokenStream)>,
    pub rust_functions: Vec<(Ident, TokenStream, Ident)>,
    pub disabled: BTreeMap<usize, TokenStream>,
    values: BTreeMap<String, usize>,
    pub used: BTreeSet<usize>,
}

impl Captures {
    /// Formal rule GPU-Capture: closed relational operands are evaluated once on
    /// the host and encoded with the compared column's recorded codec.
    pub fn value(&mut self, expression: &Expr, logical: &Type, codecs: &crate::runs::Codecs) -> Expr {
        let key = format!("{}:{}", logical.to_token_stream(), expression.to_token_stream());
        if let Some(&index) = self.values.get(&key) {
            self.used.insert(index);
            let name = &self.parameters[index].0;
            return syn::parse_quote!(#name);
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
        syn::parse_quote!(#name)
    }
    pub fn host_relational(&mut self, theta: &mut Theta, types: &BTreeMap<String, Type>, codecs: &crate::runs::Codecs) {
        fn bound(captures: &mut Captures, value: &mut Expr, ty: &Type, codecs: &crate::runs::Codecs) {
            let argument = captures.value(value, ty, codecs);
            let Expr::Path(path) = argument else { unreachable!() };
            let name = path.path.get_ident().expect("capture parameter");
            let (_, _, host) = captures.parameters.iter().find(|(id, _, _)| id == name).expect("recorded capture");
            *value = syn::parse_quote!(#host);
        }
        fn term(captures: &mut Captures, value: &mut Term, ty: &Type, codecs: &crate::runs::Codecs) {
            match value {
                Term::Closed(value) => bound(captures, value, ty, codecs),
                Term::Literal(value) => bound(captures, value, ty, codecs),
                Term::Variable(_) => {}
            }
        }
        fn constraint(captures: &mut Captures, value: &mut Constraint, types: &BTreeMap<String, Type>, codecs: &crate::runs::Codecs) {
            match value {
                Constraint::Compare { left, right, .. } => {
                    let column = match (&*left, &*right) {
                        (Term::Variable(name), _) | (_, Term::Variable(name)) => name.to_string(),
                        _ => return,
                    };
                    term(captures, left, &types[&column], codecs);
                    term(captures, right, &types[&column], codecs);
                }
                Constraint::Between { term: Term::Variable(name), low, high } => {
                    term(captures, low, &types[&name.to_string()], codecs);
                    term(captures, high, &types[&name.to_string()], codecs);
                }
                Constraint::In { term: Term::Variable(name), literals } => {
                    for value in literals.iter_mut() { bound(captures, value, &types[&name.to_string()], codecs); }
                }
                Constraint::Or(branches) => {
                    for value in branches.iter_mut().flatten() { constraint(captures, value, types, codecs); }
                }
                _ => {}
            }
        }
        for equality in &mut theta.equalities {
            if let EqualityValue::Literal(value) = &mut equality.right {
                bound(self, value, &types[&equality.left.to_string()], codecs);
            }
        }
        for value in &mut theta.constraints { constraint(self, value, types, codecs); }
    }

    /// Rust remains a single callable whose body is compiled by cuda-oxide.
    /// Only query-closed subexpressions cross the host capture boundary; rustc,
    /// not the query compiler, infers their types and the callable's operations.
    pub fn rust_expression(&mut self, expression: &Expr, types: &BTreeMap<String, Type>, bindings: &BTreeMap<String, Ident>, output: &Type, codecs: &crate::runs::Codecs) -> Expr {
        struct Locals(BTreeSet<String>);
        impl VisitMut for Locals {
            fn visit_pat_mut(&mut self, pattern: &mut syn::Pat) {
                self.0.extend(rust::bindings(pattern).iter().map(ToString::to_string));
                syn::visit_mut::visit_pat_mut(self, pattern);
            }
        }
        struct Closed {
            scope: BTreeSet<String>,
            setup: TokenStream,
            count: usize,
        }
        impl VisitMut for Closed {
            fn visit_expr_mut(&mut self, expression: &mut Expr) {
                if !matches!(expression, Expr::Lit(_))
                    && rust::free_variables(expression).dependencies(&self.scope).is_empty()
                {
                    let name = format_ident!("__cq_closed_{}", self.count);
                    self.count += 1;
                    self.setup.extend(quote!(let #name = __cq_enabled.then(|| #expression);));
                    // The launch domain enables every callable used by a live
                    // row. Disabled witnesses establish only its Rust type.
                    *expression = syn::parse_quote!(unsafe { #name.unwrap_unchecked() });
                    return;
                }
                syn::visit_mut::visit_expr_mut(self, expression);
            }
            fn visit_expr_call_mut(&mut self, call: &mut syn::ExprCall) {
                // A callee is Rust syntax, not a value supplied by the query.
                if !matches!(&*call.func, Expr::Path(_)) { self.visit_expr_mut(&mut call.func); }
                for argument in &mut call.args { self.visit_expr_mut(argument); }
            }
        }
        let scope: BTreeSet<_> = types.keys().cloned().collect();
        let reads = rust::free_variables(expression).dependencies(&scope);
        if reads.is_empty() { return self.value(expression, output, codecs); }
        let mut body = expression.clone();
        let mut locals = Locals(scope);
        locals.visit_expr_mut(&mut body);
        let mut closed = Closed { scope: locals.0, setup: TokenStream::new(), count: 0 };
        closed.visit_expr_mut(&mut body);
        let setup = closed.setup;
        let names: Vec<_> = reads.iter().map(|name| bindings[name].clone()).collect();
        let types: Vec<_> = reads.iter().map(|name| &types[name]).collect();
        let index = self.parameters.len();
        let name = format_ident!("__cq_rust_{index}");
        let generic = format_ident!("__CqRust{index}");
        let factory = format_ident!("__cq_rust_factory_{index}");
        let cache = format_ident!("__cq_rust_cache_{index}");
        self.setup.extend(quote! {
            let #factory = |__cq_enabled: bool| {
                #setup move |#(#names: &#types),*| -> #output { #body }
            };
            let #name = #factory(false);
            let #cache = {
                // A checked callable can be unused by the chosen schedule.
                // Infer its cache from the disabled witness, not a later launch.
                fn cache<T>(_: &T) -> ::std::cell::OnceCell<T> {
                    ::std::cell::OnceCell::new()
                }
                cache(&#name)
            };
        });
        self.rust_functions.push((generic.clone(), quote!(Fn(#(&#types),*) -> #output), name.clone()));
        self.parameters.push((name.clone(), syn::parse_quote!(#generic), quote!(*#cache.get_or_init(|| #factory(true)))));
        self.disabled.insert(index, quote!(#name));
        self.used.insert(index);
        syn::parse_quote!(#name(#(#names),*))
    }

    pub fn relational(&mut self, theta: &Theta, types: &BTreeMap<String, Type>, codecs: &crate::runs::Codecs) -> syn::Result<Expr> {
        let mut predicates = Vec::new();
        for equality in &theta.equalities {
            let left = &equality.left;
            let right = match &equality.right {
                EqualityValue::Variable(right) => quote!(*#right),
                EqualityValue::Literal(value) if codecs.get(&types[&left.to_string()]).is_none() => quote!(#value),
                EqualityValue::Literal(value) => {
                    let value = self.value(value, &types[&left.to_string()], codecs);
                    quote!(#value)
                }
            };
            predicates.push(quote!(*#left == #right));
        }
        for constraint in &theta.constraints { predicates.push(self.constraint(constraint, types, codecs)?); }
        syn::parse2(quote!(true #(&& (#predicates))*))
    }

    fn constraint(&mut self, constraint: &Constraint, types: &BTreeMap<String, Type>, codecs: &crate::runs::Codecs) -> syn::Result<TokenStream> {
        let context = |term: &Term| match term { Term::Variable(name) => types.get(&name.to_string()), _ => None };
        match constraint {
            Constraint::Compare { left, comparison, right } => {
                let ty = context(left).or_else(|| context(right)).ok_or_else(|| syn::Error::new_spanned(constraint, "GPU-Capture: relational comparison requires a column"))?;
                let left = self.term(left, ty, codecs);
                let right = self.term(right, ty, codecs);
                let op = match comparison {
                    Comparison::Eq(_) => quote!(==), Comparison::Ne(_) => quote!(!=),
                    Comparison::Lt(_) => quote!(<), Comparison::Le(_) => quote!(<=),
                    Comparison::Gt(_) => quote!(>), Comparison::Ge(_) => quote!(>=),
                };
                Ok(quote!(#left #op #right))
            }
            Constraint::Between { term, low, high } => {
                let ty = context(term).ok_or_else(|| syn::Error::new_spanned(term, "GPU-Capture: range requires a column"))?;
                let term = self.term(term, ty, codecs);
                let low = self.term(low, ty, codecs);
                let high = self.term(high, ty, codecs);
                Ok(quote!((#term) >= (#low) && (#term) <= (#high)))
            }
            Constraint::In { term, literals } => {
                let ty = context(term).ok_or_else(|| syn::Error::new_spanned(term, "GPU-Capture: membership requires a column"))?;
                let term = self.term(term, ty, codecs);
                let values: Vec<_> = literals.iter().map(|value| {
                    if codecs.get(ty).is_none() { value.clone() } else { self.value(value, ty, codecs) }
                }).collect();
                Ok(quote!(false #(|| #term == #values)*))
            }
            Constraint::Or(branches) => {
                let branches = branches.iter().map(|branch| {
                    let predicates = branch.iter().map(|predicate| self.constraint(predicate, types, codecs)).collect::<syn::Result<Vec<_>>>()?;
                    Ok(quote!(true #(&& (#predicates))*))
                }).collect::<syn::Result<Vec<TokenStream>>>()?;
                Ok(quote!(false #(|| (#branches))*))
            }
        }
    }

    fn term(&mut self, term: &Term, ty: &Type, codecs: &crate::runs::Codecs) -> TokenStream {
        match term {
            Term::Variable(name) => quote!(*#name),
            Term::Closed(value) => self.value(value, ty, codecs).into_token_stream(),
            Term::Literal(value) if codecs.get(ty).is_none() => quote!(#value),
            Term::Literal(value) => self.value(value, ty, codecs).into_token_stream(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relational_symbol_operand_uses_recorded_codec() {
        let mut captures = Captures::default();
        let mut codecs = crate::runs::Codecs::default();
        codecs.record(&syn::parse_quote!(TestSymbol), &syn::parse_quote!(TestCodec)).unwrap();
        captures.value(&syn::parse_quote!(sym("x")), &syn::parse_quote!(TestSymbol), &codecs);
        assert_eq!(captures.parameters[0].1, syn::parse_quote!(u32));
        assert!(captures.parameters[0].2.to_string().contains("SymbolCodec < TestSymbol >"));
    }
    #[test]
    fn rust_callable_preserves_arbitrary_operations_and_local_scope() {
        let mut captures = Captures::default();
        let types = BTreeMap::from([("x".into(), syn::parse_quote!(i64))]);
        captures.rust_expression(&syn::parse_quote!({ let local = opaque(*x); local + host_value() }), &types, &BTreeMap::from([("x".into(), syn::parse_quote!(x))]), &syn::parse_quote!(i64), &crate::runs::Codecs::default());
        let setup = captures.setup.to_string();
        assert!(setup.contains("opaque (* x)"), "{setup}");
        assert!(setup.contains("let __cq_closed_0 = __cq_enabled . then (|| host_value ())"), "{setup}");
        assert!(!setup.contains("wrapping"));
        assert_eq!(captures.rust_functions.len(), 1);
    }
}
