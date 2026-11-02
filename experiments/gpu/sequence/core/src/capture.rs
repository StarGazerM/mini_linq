//! Host captures and opaque Rust callables for device row expressions.
//!
//! DESIGN contract7-gpu-captures
//! Owns host captures and verbatim device Rust callables (contract 7, GPU-Capture/BE-RustRepresentation).
//! Source: doc/PLANNING-REDESIGN.md Contract 7; doc/formal/CONFORMANCE.md GPU-Capture.
//! Closed relational operands use the compared column's recorded codec; Rust reads require identity.
//! DO NOT infer semantics from types, recognize `if` comparisons, or rewrite Rust arithmetic.
//! Capturing query-closed inputs does not authorize splitting Rust into relational predicates.
//! Extraction runs once in GPU realization and records `realization::Captures`;
//! printing consumes those records without re-extracting.
use cq_lang_sequence::gpu::realization::Capture;
use cq_lang_structures::syntax::{
    EqualityValue,
    algebra::Theta,
    constraint::{Comparison, Constraint, Term},
    rust,
};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, Ident, Type, visit_mut::VisitMut};

/// Capture extraction state: recorded captures in parameter order, the
/// deduplicated closed values, and the indices read by the current definition.
/// A frozen state resolves closed values against supplied records only and
/// flags any value they lack; the checker uses it to validate records.
#[derive(Default)]
pub(crate) struct Captures {
    pub records: Vec<Capture>,
    values: BTreeMap<String, usize>,
    pub used: BTreeSet<usize>,
    frozen: bool,
    missing: bool,
}

/// Printed capture support: setup statements, kernel parameters with their
/// host values, device Rust callable bounds, and disabled-launch witnesses.
pub(crate) struct Printed {
    pub setup: TokenStream,
    pub parameters: Vec<(Ident, Type, TokenStream)>,
    pub rust_functions: Vec<(Ident, TokenStream, Ident)>,
    pub disabled: BTreeMap<usize, TokenStream>,
}

/// The host value of a closed operand. Query captures are bound once as `&T`
/// (closed-capture convention), other closed operands as values; borrowing
/// the operand at the column's logical type accepts both by deref coercion.
fn value_carrier(expression: &Expr, logical: &Type, codec: Option<&Type>) -> (Type, TokenStream) {
    match codec {
        Some(codec) => (
            syn::parse_quote!(u32),
            quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#logical>>::code(&(#expression))),
        ),
        None => (
            logical.clone(),
            quote!(<#logical as ::core::clone::Clone>::clone(&(#expression))),
        ),
    }
}

/// Host value of a recorded closed capture, shared by every invocation use.
fn host_value(
    index: usize,
    expression: &Expr,
    logical: &Type,
    codec: Option<&Type>,
) -> TokenStream {
    let cache = format_ident!("__cq_capture_cache_{index}");
    let (_, value) = value_carrier(expression, logical, codec);
    quote!(*#cache.get_or_init(|| #value))
}

/// Print recorded captures in parameter order.
pub(crate) fn print(records: &[Capture]) -> Printed {
    let mut printed = Printed {
        setup: TokenStream::new(),
        parameters: Vec::new(),
        rust_functions: Vec::new(),
        disabled: BTreeMap::new(),
    };
    for (index, capture) in records.iter().enumerate() {
        match capture {
            Capture::Value {
                expression,
                logical,
                codec,
            } => {
                let name = format_ident!("__cq_capture_{index}");
                let cache = format_ident!("__cq_capture_cache_{index}");
                let (ty, _) = value_carrier(expression, logical, codec.as_ref());
                printed
                    .setup
                    .extend(quote!(let #cache = ::std::cell::OnceCell::<#ty>::new();));
                printed.parameters.push((
                    name,
                    ty,
                    host_value(index, expression, logical, codec.as_ref()),
                ));
            }
            Capture::Callable {
                reads,
                output,
                closed,
                body,
            } => {
                let closed = closed.iter().enumerate().map(|(at, expression)| {
                    let name = format_ident!("__cq_closed_{at}");
                    // A disabled witness only fixes the Rust type; MaybeUninit
                    // carries no discriminant (cuda-oxide rejects 128-bit ones).
                    quote!(let #name = if __cq_enabled {
                        ::core::mem::MaybeUninit::new(#expression)
                    } else {
                        ::core::mem::MaybeUninit::uninit()
                    };)
                });
                let names = reads.iter().map(|(name, _)| name);
                let types: Vec<_> = reads.iter().map(|(_, ty)| ty).collect();
                let name = format_ident!("__cq_rust_{index}");
                let generic = format_ident!("__CqRust{index}");
                let factory = format_ident!("__cq_rust_factory_{index}");
                let cache = format_ident!("__cq_rust_cache_{index}");
                printed.setup.extend(quote! {
                    let #factory = |__cq_enabled: bool| {
                        #(#closed)* move |#(#names: &#types),*| -> #output { #body }
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
                printed.rust_functions.push((
                    generic.clone(),
                    quote!(Fn(#(&#types),*) -> #output),
                    name.clone(),
                ));
                printed.parameters.push((
                    name.clone(),
                    syn::parse_quote!(#generic),
                    quote!(*#cache.get_or_init(|| #factory(true))),
                ));
                printed.disabled.insert(index, quote!(#name));
            }
        }
    }
    printed
}

fn value_key(expression: &Expr, logical: &Type) -> String {
    format!(
        "{}:{}",
        logical.to_token_stream(),
        expression.to_token_stream()
    )
}

/// GPU-Capture checker: validate recorded captures without re-extracting
/// them. Closed values have no row reads and use their column's codec
/// identity. Each row function's device expression is either one closed
/// value of its expression or a callable that, with its closed
/// subexpressions restored, is that expression over its recorded reads. Each
/// selection's device and host forms resolve every closed operand to an
/// existing Value capture at the compared column's type, and its uses are
/// exactly the captures those forms read.
pub(crate) fn check(
    records: &cq_lang_sequence::gpu::realization::Captures,
    definitions: &BTreeMap<String, cq_lang_structures::physical::Definition>,
    analysis: &cq_lang_sequence::Analysis,
    codecs: &crate::runs::Codecs,
) -> syn::Result<()> {
    use cq_lang_structures::physical::{Operator, symbol_name};
    let error = |message: &str| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("GPU-Capture: {message}"),
        )
    };
    let rows = &analysis.structures.physical.rows;
    let columns: BTreeSet<_> = rows
        .values()
        .flat_map(|row| row.types.keys().cloned())
        .collect();
    for capture in &records.captures {
        match capture {
            Capture::Value {
                expression,
                logical,
                codec,
            } => {
                if !rust::free_variables(expression)
                    .dependencies(&columns)
                    .is_empty()
                {
                    return Err(error("closed value reads a row column"));
                }
                if codec.as_ref() != codecs.get(logical) {
                    return Err(error(
                        "closed value codec differs from its column codec identity",
                    ));
                }
            }
            Capture::Callable { closed, .. } => {
                if closed.iter().any(|expression| {
                    !rust::free_variables(expression)
                        .dependencies(&columns)
                        .is_empty()
                }) {
                    return Err(error("callable host subexpression reads a row column"));
                }
            }
        }
    }
    let mut expected_device = BTreeSet::new();
    let mut expected_host = BTreeSet::new();
    for (name, definition) in definitions {
        let Some(device) = records.device.get(name) else {
            continue;
        };
        let uses = records.uses.get(name);
        match &definition.operator {
            Operator::Guard(_) | Operator::Bind(_)
                if !analysis.rust_requires_host(&definition.operator) =>
            {
                let (expression, input, output): (&Expr, _, Type) = match &definition.operator {
                    Operator::Guard(guard) => {
                        (&guard.condition, &guard.input, syn::parse_quote!(bool))
                    }
                    Operator::Bind(bind) => {
                        let names = rust::bindings(&bind.pattern);
                        let output = rows[name].types[&symbol_name(&names[0])].clone();
                        (&bind.expression, &bind.input, output)
                    }
                    _ => unreachable!(),
                };
                let row = &rows[&symbol_name(input)];
                check_row_function(records, device, expression, row, &output)
                    .map_err(|message| error(message))?;
                let index =
                    capture_reference(device).ok_or_else(|| error("unrecorded row function"))?;
                if uses != Some(&BTreeSet::from([index])) {
                    return Err(error("row function uses differ from its device expression"));
                }
                expected_device.insert(name.clone());
            }
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                let types = &rows[&symbol_name(&select.input)].types;
                let mut frozen = Captures::frozen(&records.captures);
                let rendered = frozen.relational(&select.condition, types, codecs)?;
                // Uses are the device form's captures; the host form reads caches.
                let used = std::mem::take(&mut frozen.used);
                let mut host = select.condition.clone();
                frozen.host_relational(&mut host, types, codecs);
                if frozen.missing
                    || rendered != *device
                    || records.host.get(name) != Some(&host)
                    || uses.unwrap_or(&BTreeSet::new()) != &used
                {
                    return Err(error(
                        "selection forms do not resolve to recorded closed values",
                    ));
                }
                expected_device.insert(name.clone());
                expected_host.insert(name.clone());
            }
            _ => return Err(error("device expression of a definition without one")),
        }
    }
    for (name, definition) in definitions {
        if let Operator::Select(_) | Operator::Point(..) | Operator::Range(..) =
            &definition.operator
        {
            expected_host.insert(name.clone());
        }
    }
    if records
        .device
        .keys()
        .any(|name| !expected_device.contains(name))
        || records.host.keys().cloned().collect::<BTreeSet<_>>() != expected_host
        || records
            .uses
            .keys()
            .any(|name| !records.device.contains_key(name))
    {
        return Err(error("capture records name other definitions"));
    }
    if records
        .uses
        .values()
        .flatten()
        .any(|&index| index >= records.captures.len())
    {
        return Err(error("definition reads an unrecorded capture"));
    }
    Ok(())
}

/// The capture index a device row expression names.
fn capture_reference(device: &Expr) -> Option<usize> {
    let path = match device {
        Expr::Path(path) => path,
        Expr::Call(call) => match &*call.func {
            Expr::Path(path) => path,
            _ => return None,
        },
        _ => return None,
    };
    let name = path.path.get_ident()?.to_string();
    name.strip_prefix(if matches!(device, Expr::Call(_)) {
        "__cq_rust_"
    } else {
        "__cq_capture_"
    })?
    .parse()
    .ok()
}

/// A device row expression computes `expression`: a closed value of it at
/// `output`, or a callable over exactly its row reads whose body, with the
/// closed subexpressions restored, is `expression`.
fn check_row_function(
    records: &cq_lang_sequence::gpu::realization::Captures,
    device: &Expr,
    expression: &Expr,
    row: &cq_lang_structures::physical::Row,
    output: &Type,
) -> Result<(), &'static str> {
    let index = capture_reference(device).ok_or("row function names no capture")?;
    let scope: BTreeSet<_> = row.types.keys().cloned().collect();
    let reads = rust::free_variables(expression).dependencies(&scope);
    match (records.captures.get(index), device) {
        (
            Some(Capture::Value {
                expression: value,
                logical,
                ..
            }),
            Expr::Path(_),
        ) => {
            if !reads.is_empty() || value != expression || logical != output {
                return Err("closed row function is not its expression's value");
            }
        }
        (
            Some(Capture::Callable {
                reads: recorded,
                output: recorded_output,
                closed,
                body,
            }),
            Expr::Call(call),
        ) => {
            let expected: Vec<_> = reads
                .iter()
                .map(|name| Some((row.bindings.get(name)?.clone(), row.types[name].clone())))
                .collect::<Option<_>>()
                .ok_or("callable read lacks a row binding")?;
            let arguments: Vec<Expr> = recorded
                .iter()
                .map(|(name, _)| syn::parse_quote!(#name))
                .collect();
            if *recorded != expected
                || recorded_output != output
                || call.args.iter().ne(arguments.iter())
            {
                return Err("callable reads differ from its expression's row reads");
            }
            struct Restore<'a>(&'a [Expr], bool);
            impl VisitMut for Restore<'_> {
                fn visit_expr_mut(&mut self, expression: &mut Expr) {
                    if let Expr::Unsafe(block) = expression
                        && let [syn::Stmt::Expr(Expr::MethodCall(call), None)] =
                            block.block.stmts.as_slice()
                        && call.method == "assume_init"
                        && let Expr::Path(path) = &*call.receiver
                        && let Some(index) = path.path.get_ident().and_then(|name| {
                            name.to_string()
                                .strip_prefix("__cq_closed_")?
                                .parse::<usize>()
                                .ok()
                        })
                    {
                        match self.0.get(index) {
                            Some(closed) => *expression = closed.clone(),
                            None => self.1 = false,
                        }
                        return;
                    }
                    syn::visit_mut::visit_expr_mut(self, expression);
                }
            }
            let mut restored = body.clone();
            let mut restore = Restore(closed, true);
            restore.visit_expr_mut(&mut restored);
            if !restore.1
                || restored.to_token_stream().to_string()
                    != expression.to_token_stream().to_string()
            {
                return Err("callable body is not its row function with closed values restored");
            }
        }
        _ => return Err("row function names a capture of another kind"),
    }
    Ok(())
}

impl Captures {
    /// Lookup-only resolution over supplied records (never records new captures).
    fn frozen(records: &[Capture]) -> Self {
        let values = records
            .iter()
            .enumerate()
            .filter_map(|(index, capture)| match capture {
                Capture::Value {
                    expression,
                    logical,
                    ..
                } => Some((value_key(expression, logical), index)),
                Capture::Callable { .. } => None,
            })
            .collect();
        Self {
            records: records.to_vec(),
            values,
            used: BTreeSet::new(),
            frozen: true,
            missing: false,
        }
    }

    /// The recorded Value capture of a closed operand at `logical`.
    fn value_index(
        &mut self,
        expression: &Expr,
        logical: &Type,
        codecs: &crate::runs::Codecs,
    ) -> usize {
        let key = value_key(expression, logical);
        let index = match self.values.get(&key) {
            Some(&index) => index,
            None if self.frozen => {
                self.missing = true;
                self.records.len()
            }
            None => {
                self.records.push(Capture::Value {
                    expression: expression.clone(),
                    logical: logical.clone(),
                    codec: codecs.get(logical).cloned(),
                });
                let index = self.records.len() - 1;
                self.values.insert(key, index);
                index
            }
        };
        self.used.insert(index);
        index
    }

    /// Formal rule GPU-Capture: closed relational operands are evaluated once on
    /// the host and encoded with the compared column's recorded codec.
    pub fn value(
        &mut self,
        expression: &Expr,
        logical: &Type,
        codecs: &crate::runs::Codecs,
    ) -> Expr {
        let index = self.value_index(expression, logical, codecs);
        let name = format_ident!("__cq_capture_{index}");
        syn::parse_quote!(#name)
    }
    pub fn host_relational(
        &mut self,
        theta: &mut Theta,
        types: &BTreeMap<String, Type>,
        codecs: &crate::runs::Codecs,
    ) {
        fn bound(
            captures: &mut Captures,
            value: &mut Expr,
            ty: &Type,
            codecs: &crate::runs::Codecs,
        ) {
            let index = captures.value_index(value, ty, codecs);
            let Some(Capture::Value {
                expression,
                logical,
                codec,
            }) = captures.records.get(index)
            else {
                return;
            };
            let host = host_value(index, expression, logical, codec.as_ref());
            *value = syn::parse_quote!(#host);
        }
        fn term(
            captures: &mut Captures,
            value: &mut Term,
            ty: &Type,
            codecs: &crate::runs::Codecs,
        ) {
            match value {
                Term::Closed(value) => bound(captures, value, ty, codecs),
                Term::Literal(value) => bound(captures, value, ty, codecs),
                Term::Variable(_) => {}
            }
        }
        fn constraint(
            captures: &mut Captures,
            value: &mut Constraint,
            types: &BTreeMap<String, Type>,
            codecs: &crate::runs::Codecs,
        ) {
            match value {
                Constraint::Compare { left, right, .. } => {
                    let column = match (&*left, &*right) {
                        (Term::Variable(name), _) | (_, Term::Variable(name)) => name.to_string(),
                        _ => return,
                    };
                    term(captures, left, &types[&column], codecs);
                    term(captures, right, &types[&column], codecs);
                }
                Constraint::Between {
                    term: Term::Variable(name),
                    low,
                    high,
                } => {
                    term(captures, low, &types[&name.to_string()], codecs);
                    term(captures, high, &types[&name.to_string()], codecs);
                }
                Constraint::In {
                    term: Term::Variable(name),
                    literals,
                } => {
                    for value in literals.iter_mut() {
                        bound(captures, value, &types[&name.to_string()], codecs);
                    }
                }
                Constraint::Or(branches) => {
                    for value in branches.iter_mut().flatten() {
                        constraint(captures, value, types, codecs);
                    }
                }
                _ => {}
            }
        }
        for equality in &mut theta.equalities {
            if let EqualityValue::Literal(value) = &mut equality.right {
                bound(self, value, &types[&equality.left.to_string()], codecs);
            }
        }
        for value in &mut theta.constraints {
            constraint(self, value, types, codecs);
        }
    }

    /// Rust remains a single callable whose body is compiled by cuda-oxide.
    /// Only query-closed subexpressions cross the host capture boundary; rustc,
    /// not the query compiler, infers their types and the callable's operations.
    pub fn rust_expression(
        &mut self,
        expression: &Expr,
        types: &BTreeMap<String, Type>,
        bindings: &BTreeMap<String, Ident>,
        output: &Type,
        codecs: &crate::runs::Codecs,
    ) -> Expr {
        struct Locals(BTreeSet<String>);
        impl VisitMut for Locals {
            fn visit_pat_mut(&mut self, pattern: &mut syn::Pat) {
                self.0
                    .extend(rust::bindings(pattern).iter().map(ToString::to_string));
                syn::visit_mut::visit_pat_mut(self, pattern);
            }
        }
        struct Closed {
            scope: BTreeSet<String>,
            closed: Vec<Expr>,
        }
        impl VisitMut for Closed {
            fn visit_expr_mut(&mut self, expression: &mut Expr) {
                if rust::free_variables(expression)
                    .dependencies(&self.scope)
                    .is_empty()
                {
                    let name = format_ident!("__cq_closed_{}", self.closed.len());
                    self.closed.push(expression.clone());
                    // The launch domain enables every callable used by a live
                    // row. Disabled witnesses establish only its Rust type.
                    *expression = syn::parse_quote!(unsafe { #name.assume_init() });
                    return;
                }
                syn::visit_mut::visit_expr_mut(self, expression);
            }
            fn visit_expr_call_mut(&mut self, call: &mut syn::ExprCall) {
                // A callee is Rust syntax, not a value supplied by the query.
                if !matches!(&*call.func, Expr::Path(_)) {
                    self.visit_expr_mut(&mut call.func);
                }
                for argument in &mut call.args {
                    self.visit_expr_mut(argument);
                }
            }
        }
        let scope: BTreeSet<_> = types.keys().cloned().collect();
        let reads = rust::free_variables(expression).dependencies(&scope);
        if reads.is_empty() {
            return self.value(expression, output, codecs);
        }
        let mut body = expression.clone();
        let mut locals = Locals(scope);
        locals.visit_expr_mut(&mut body);
        let mut closed = Closed {
            scope: locals.0,
            closed: Vec::new(),
        };
        closed.visit_expr_mut(&mut body);
        let names: Vec<_> = reads.iter().map(|name| bindings[name].clone()).collect();
        self.records.push(Capture::Callable {
            reads: reads
                .iter()
                .map(|name| (bindings[name].clone(), types[name].clone()))
                .collect(),
            output: output.clone(),
            closed: closed.closed,
            body,
        });
        let index = self.records.len() - 1;
        let name = format_ident!("__cq_rust_{index}");
        self.used.insert(index);
        syn::parse_quote!(#name(#(#names),*))
    }

    pub fn relational(
        &mut self,
        theta: &Theta,
        types: &BTreeMap<String, Type>,
        codecs: &crate::runs::Codecs,
    ) -> syn::Result<Expr> {
        let mut predicates = Vec::new();
        for equality in &theta.equalities {
            let left = &equality.left;
            let right = match &equality.right {
                EqualityValue::Variable(right) => quote!(*#right),
                EqualityValue::Literal(value)
                    if codecs.get(&types[&left.to_string()]).is_none() =>
                {
                    quote!(#value)
                }
                EqualityValue::Literal(value) => {
                    let value = self.value(value, &types[&left.to_string()], codecs);
                    quote!(#value)
                }
            };
            predicates.push(quote!(*#left == #right));
        }
        for constraint in &theta.constraints {
            predicates.push(self.constraint(constraint, types, codecs)?);
        }
        syn::parse2(quote!(true #(&& (#predicates))*))
    }

    fn constraint(
        &mut self,
        constraint: &Constraint,
        types: &BTreeMap<String, Type>,
        codecs: &crate::runs::Codecs,
    ) -> syn::Result<TokenStream> {
        let context = |term: &Term| match term {
            Term::Variable(name) => types.get(&name.to_string()),
            _ => None,
        };
        match constraint {
            Constraint::Compare {
                left,
                comparison,
                right,
            } => {
                let ty = context(left).or_else(|| context(right)).ok_or_else(|| {
                    syn::Error::new_spanned(
                        constraint,
                        "GPU-Capture: relational comparison requires a column",
                    )
                })?;
                let left = self.term(left, ty, codecs);
                let right = self.term(right, ty, codecs);
                let op = match comparison {
                    Comparison::Eq(_) => quote!(==),
                    Comparison::Ne(_) => quote!(!=),
                    Comparison::Lt(_) => quote!(<),
                    Comparison::Le(_) => quote!(<=),
                    Comparison::Gt(_) => quote!(>),
                    Comparison::Ge(_) => quote!(>=),
                };
                Ok(quote!(#left #op #right))
            }
            Constraint::Between { term, low, high } => {
                let ty = context(term).ok_or_else(|| {
                    syn::Error::new_spanned(term, "GPU-Capture: range requires a column")
                })?;
                let term = self.term(term, ty, codecs);
                let low = self.term(low, ty, codecs);
                let high = self.term(high, ty, codecs);
                Ok(quote!((#term) >= (#low) && (#term) <= (#high)))
            }
            Constraint::In { term, literals } => {
                let ty = context(term).ok_or_else(|| {
                    syn::Error::new_spanned(term, "GPU-Capture: membership requires a column")
                })?;
                let term = self.term(term, ty, codecs);
                let values: Vec<_> = literals
                    .iter()
                    .map(|value| {
                        if codecs.get(ty).is_none() {
                            value.clone()
                        } else {
                            self.value(value, ty, codecs)
                        }
                    })
                    .collect();
                Ok(quote!(false #(|| #term == #values)*))
            }
            Constraint::Or(branches) => {
                let branches = branches
                    .iter()
                    .map(|branch| {
                        let predicates = branch
                            .iter()
                            .map(|predicate| self.constraint(predicate, types, codecs))
                            .collect::<syn::Result<Vec<_>>>()?;
                        Ok(quote!(true #(&& (#predicates))*))
                    })
                    .collect::<syn::Result<Vec<TokenStream>>>()?;
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
        codecs
            .record(
                &syn::parse_quote!(TestSymbol),
                &syn::parse_quote!(TestCodec),
            )
            .unwrap();
        captures.value(
            &syn::parse_quote!(sym("x")),
            &syn::parse_quote!(TestSymbol),
            &codecs,
        );
        let printed = print(&captures.records);
        assert_eq!(printed.parameters[0].1, syn::parse_quote!(u32));
        assert!(
            printed.parameters[0]
                .2
                .to_string()
                .contains("SymbolCodec < TestSymbol >")
        );
    }
    #[test]
    fn rust_callable_preserves_arbitrary_operations_and_local_scope() {
        let mut captures = Captures::default();
        let types = BTreeMap::from([("x".into(), syn::parse_quote!(i64))]);
        captures.rust_expression(
            &syn::parse_quote!({
                let local = opaque(*x);
                local + host_value()
            }),
            &types,
            &BTreeMap::from([("x".into(), syn::parse_quote!(x))]),
            &syn::parse_quote!(i64),
            &crate::runs::Codecs::default(),
        );
        let printed = print(&captures.records);
        let setup = printed.setup.to_string();
        assert!(setup.contains("opaque (* x)"), "{setup}");
        assert!(
            setup.contains(
                "let __cq_closed_0 = if __cq_enabled { :: core :: mem :: MaybeUninit :: new (host_value ()) }"
            ),
            "{setup}"
        );
        assert!(!setup.contains("wrapping"));
        assert_eq!(printed.rust_functions.len(), 1);
    }
}
