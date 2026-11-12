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
    parameters: BTreeMap<String, Ident>,
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
pub(crate) fn print(records: &[Capture], device_bindings: Option<&crate::DeviceCaptureBindings>) -> Printed {
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
                ..
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
                    let expression = device_bindings.and_then(|bindings| bindings.operand(index, at))
                        .unwrap_or_else(|| expression.to_token_stream());
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

/// Print checked query captures in source order. A failed answer gate returns
/// before later captures or any query input is evaluated.
pub(crate) fn print_query(
    captures: &cq_lang_structures::syntax::Captures,
    empty: &TokenStream,
) -> TokenStream {
    let mut setup = TokenStream::new();
    for (index, item) in captures.items.iter().enumerate() {
        if let Some(binding) = item.binding() {
            let id = format_ident!("__cq_query_capture_{index}");
            let annotation = &binding.annotation;
            let expression = &binding.expression;
            let pattern = &binding.pattern;
            setup.extend(quote! {
                let #id #annotation = #expression;
                #[allow(unused_variables)]
                let #pattern = &#id;
            });
        } else if let cq_lang_structures::syntax::Capture::Guard(guard) = item {
            let condition = &guard.condition;
            setup.extend(quote! {
                if !(#condition) { return #empty; }
            });
        }
    }
    setup
}

fn value_key(
    expression: &Expr,
    logical_type_id: Option<cq_syntax::LogicalTypeId>,
    column: Option<&str>,
) -> String {
    format!(
        "{logical_type_id:?}:{:?}:{}",
        column.filter(|_| logical_type_id.is_none()),
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
    structure_analysis: &cq_lang_structures::Analysis,
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
    let rows = &structure_analysis.physical.rows;
    let columns: BTreeSet<_> = rows
        .values()
        .flat_map(|row| row.types.keys().cloned())
        .collect();
    for capture in &records.captures {
        match capture {
            Capture::Value {
                expression,
                logical: _,
                logical_type_id,
                codec,
                codec_id,
                ..
            } => {
                if !rust::free_variables(expression)
                    .dependencies(&columns)
                    .is_empty()
                {
                    return Err(error("closed value reads a row column"));
                }
                if *codec_id != codecs.codec_id(*logical_type_id) {
                    return Err(error(
                        "closed value codec differs from its column codec identity",
                    ));
                }
                if codec.as_ref() != codecs.get(*logical_type_id) {
                    return Err(error(
                        "closed value codec payload differs from its declaration",
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
                if !analysis.rust_requires_host(&structure_analysis.physical, &definition.operator) =>
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
                let output_id = match &definition.operator {
                    Operator::Bind(bind) => rust::bindings(&bind.pattern)
                        .first()
                        .and_then(|column| rows[name].logical_type_ids.get(&symbol_name(column)))
                        .copied(),
                    _ => None,
                };
                check_row_function(records, device, expression, row, &output, output_id, name)
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
                check_range_contexts(&select.condition, types, codecs, &records.captures)
                    .map_err(|message| error(message))?;
                let mut frozen = Captures::frozen(&records.captures, &records.query);
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

/// Independently check the recorded representation of closed range operands.
/// The subject need not be a column: either available bound supplies context.
fn check_range_contexts(
    theta: &Theta,
    types: &BTreeMap<String, Type>,
    codecs: &crate::runs::Codecs,
    records: &[Capture],
) -> Result<(), &'static str> {
    fn check(
        constraint: &Constraint,
        types: &BTreeMap<String, Type>,
        codecs: &crate::runs::Codecs,
        records: &[Capture],
    ) -> Result<(), &'static str> {
        match constraint {
            Constraint::Between { term, low, high } => {
                let operands = [term, low, high];
                let context = operands.iter().find_map(|operand| match operand {
                    Term::Variable(name) => {
                        let column = name.to_string();
                        types.get(&column).map(|ty| (column, ty))
                    }
                    _ => None,
                });
                let Some((column, ty)) = context else {
                    // Fully closed ranges have a single boolean capture.
                    return Ok(());
                };
                let id = codecs.logical_type_id(&column);
                for operand in operands {
                    let expression = match operand {
                        Term::Closed(expression) => expression,
                        Term::Literal(expression) => expression.as_ref(),
                        _ => continue,
                    };
                    let expected = value_key(expression, id, Some(&column));
                    if !records.iter().any(|capture| match capture {
                        Capture::Value {
                            expression,
                            logical,
                            logical_type_id,
                            logical_column,
                            ..
                        } => {
                            value_key(expression, *logical_type_id, logical_column.as_deref())
                                == expected
                                && (id.is_some() || logical == ty)
                        }
                        _ => false,
                    }) {
                        return Err("range operand lacks its recorded column capture context");
                    }
                }
            }
            Constraint::Or(branches) => {
                for constraint in branches.iter().flatten() {
                    check(constraint, types, codecs, records)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    for constraint in &theta.constraints {
        check(constraint, types, codecs, records)?;
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
    output_id: Option<cq_syntax::LogicalTypeId>,
    output_column: &str,
) -> Result<(), &'static str> {
    let index = capture_reference(device).ok_or("row function names no capture")?;
    let scope: BTreeSet<_> = row.types.keys().cloned().collect();
    let reads = rust::free_variables(expression).dependencies(&scope);
    match (records.captures.get(index), device) {
        (
            Some(Capture::Value {
                expression: value,
                logical_type_id,
                logical_column,
                ..
            }),
            Expr::Path(_),
        ) => {
            if !reads.is_empty()
                || value != expression
                || *logical_type_id != output_id
                || (output_id.is_none() && logical_column.as_deref() != Some(output_column))
            {
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
    pub(crate) fn new(query: &cq_lang_structures::syntax::Captures) -> Self {
        Self {
            parameters: query
                .binders()
                .map(|name| (name.to_string(), name))
                .collect(),
            ..Self::default()
        }
    }

    /// Lookup-only resolution over supplied records (never records new captures).
    fn frozen(records: &[Capture], query: &cq_lang_structures::syntax::Captures) -> Self {
        let values = records
            .iter()
            .enumerate()
            .filter_map(|(index, capture)| match capture {
                Capture::Value {
                    expression,
                    logical_type_id,
                    logical_column,
                    ..
                } => Some((
                    value_key(expression, *logical_type_id, logical_column.as_deref()),
                    index,
                )),
                Capture::Callable { .. } => None,
            })
            .collect();
        Self {
            records: records.to_vec(),
            values,
            frozen: true,
            ..Self::new(query)
        }
    }

    /// The recorded Value capture of a closed operand at `logical`.
    fn value_index(
        &mut self,
        expression: &Expr,
        logical: &Type,
        logical_type_id: Option<cq_syntax::LogicalTypeId>,
        logical_column: Option<&str>,
        codecs: &crate::runs::Codecs,
    ) -> usize {
        let key = value_key(expression, logical_type_id, logical_column);
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
                    logical_type_id,
                    logical_column: logical_column.map(str::to_owned),
                    codec: codecs.get(logical_type_id).cloned(),
                    codec_id: codecs.codec_id(logical_type_id),
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
        logical_type_id: Option<cq_syntax::LogicalTypeId>,
        logical_column: Option<&str>,
        codecs: &crate::runs::Codecs,
    ) -> Expr {
        let index = self.value_index(expression, logical, logical_type_id, logical_column, codecs);
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
            logical_type_id: Option<cq_syntax::LogicalTypeId>,
            logical_column: &str,
            codecs: &crate::runs::Codecs,
        ) {
            let index =
                captures.value_index(value, ty, logical_type_id, Some(logical_column), codecs);
            let Some(Capture::Value {
                expression,
                logical,
                codec,
                ..
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
            logical_type_id: Option<cq_syntax::LogicalTypeId>,
            logical_column: &str,
            codecs: &crate::runs::Codecs,
        ) {
            match value {
                Term::Closed(value) => {
                    bound(captures, value, ty, logical_type_id, logical_column, codecs)
                }
                Term::Literal(value) => {
                    bound(captures, value, ty, logical_type_id, logical_column, codecs)
                }
                Term::Variable(_) => {}
            }
        }
        fn constraint(
            captures: &mut Captures,
            value: &mut Constraint,
            types: &BTreeMap<String, Type>,
            codecs: &crate::runs::Codecs,
        ) {
            if value.variables().next().is_none() {
                let tokens =
                    cq_print::expressions::relational_constraint(value, &captures.parameters);
                let expression = syn::parse_quote!(#tokens);
                let index =
                    captures.value_index(&expression, &syn::parse_quote!(bool), None, None, codecs);
                if let Some(Capture::Value {
                    expression,
                    logical,
                    codec,
                    ..
                }) = captures.records.get(index)
                {
                    let host = host_value(index, expression, logical, codec.as_ref());
                    *value = Constraint::Compare {
                        left: Term::Closed(syn::parse_quote!(#host)),
                        comparison: Comparison::Eq(Default::default()),
                        right: Term::Literal(syn::parse_quote!(true)),
                    };
                }
                return;
            }
            match value {
                Constraint::Compare { left, right, .. } => {
                    let column = match (&*left, &*right) {
                        (Term::Variable(name), _) | (_, Term::Variable(name)) => name.to_string(),
                        _ => return,
                    };
                    term(
                        captures,
                        left,
                        &types[&column],
                        codecs.logical_type_id(&column),
                        &column,
                        codecs,
                    );
                    term(
                        captures,
                        right,
                        &types[&column],
                        codecs.logical_type_id(&column),
                        &column,
                        codecs,
                    );
                }
                Constraint::Between { term: subject, low, high } => {
                    // A closed subject still has a row representation when either
                    // bound is a column. Record that column's capture context.
                    let Some(column) = [&*subject, &*low, &*high].into_iter().find_map(|value| {
                        match value {
                            Term::Variable(name) if types.contains_key(&name.to_string()) => {
                                Some(name.to_string())
                            }
                            _ => None,
                        }
                    }) else {
                        return;
                    };
                    for value in [subject, low, high] {
                        term(
                            captures,
                            value,
                            &types[&column],
                            codecs.logical_type_id(&column),
                            &column,
                            codecs,
                        );
                    }
                }
                Constraint::In {
                    term: Term::Variable(name),
                    literals,
                } => {
                    for value in literals.iter_mut() {
                        bound(
                            captures,
                            value,
                            &types[&name.to_string()],
                            codecs.logical_type_id(&name.to_string()),
                            &name.to_string(),
                            codecs,
                        );
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
                bound(
                    self,
                    value,
                    &types[&equality.left.to_string()],
                    codecs.logical_type_id(&equality.left.to_string()),
                    &equality.left.to_string(),
                    codecs,
                );
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
        output_id: Option<cq_syntax::LogicalTypeId>,
        output_column: &str,
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
            return self.value(expression, output, output_id, Some(output_column), codecs);
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
                EqualityValue::Literal(value) if codecs.column(&left.to_string()).is_none() => {
                    quote!(#value)
                }
                EqualityValue::Literal(value) => {
                    let value = self.value(
                        value,
                        &types[&left.to_string()],
                        codecs.logical_type_id(&left.to_string()),
                        Some(&left.to_string()),
                        codecs,
                    );
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
        // A column-free predicate is a host proposition, not a device
        // comparison with an inferred column representation. Reuse the shared
        // relational laws and capture only its boolean result.
        if constraint.variables().next().is_none() {
            let expression = syn::parse2(cq_print::expressions::relational_constraint(
                constraint,
                &self.parameters,
            ))?;
            return Ok(self
                .value(&expression, &syn::parse_quote!(bool), None, None, codecs)
                .into_token_stream());
        }
        let context = |term: &Term| match term {
            Term::Variable(name) => types.get(&name.to_string()).map(|ty| {
                (
                    ty,
                    codecs.logical_type_id(&name.to_string()),
                    name.to_string(),
                )
            }),
            _ => None,
        };
        match constraint {
            Constraint::Compare {
                left,
                comparison,
                right,
            } => {
                let (ty, id, column) =
                    context(left).or_else(|| context(right)).ok_or_else(|| {
                        syn::Error::new_spanned(
                            constraint,
                            "GPU-Capture: relational comparison requires a column",
                        )
                    })?;
                let left = self.term(left, ty, id, &column, codecs);
                let right = self.term(right, ty, id, &column, codecs);
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
                let (ty, id, column) = context(term)
                    .or_else(|| context(low))
                    .or_else(|| context(high))
                    .ok_or_else(|| {
                        syn::Error::new_spanned(constraint, "GPU-Capture: range requires a column")
                    })?;
                let term = self.term(term, ty, id, &column, codecs);
                let low = self.term(low, ty, id, &column, codecs);
                let high = self.term(high, ty, id, &column, codecs);
                Ok(quote!((#term) >= (#low) && (#term) <= (#high)))
            }
            Constraint::In { term, literals } => {
                let (ty, id, column) = context(term).ok_or_else(|| {
                    syn::Error::new_spanned(term, "GPU-Capture: membership requires a column")
                })?;
                let term = self.term(term, ty, id, &column, codecs);
                let values: Vec<_> = literals
                    .iter()
                    .map(|value| {
                        if codecs.get(id).is_none() {
                            value.clone()
                        } else {
                            self.value(value, ty, id, Some(&column), codecs)
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

    fn term(
        &mut self,
        term: &Term,
        ty: &Type,
        id: Option<cq_syntax::LogicalTypeId>,
        column: &str,
        codecs: &crate::runs::Codecs,
    ) -> TokenStream {
        match term {
            Term::Variable(name) => quote!(*#name),
            Term::Closed(value) => self
                .value(value, ty, id, Some(column), codecs)
                .into_token_stream(),
            Term::Literal(value) if codecs.get(id).is_none() => quote!(#value),
            Term::Literal(value) => self
                .value(value, ty, id, Some(column), codecs)
                .into_token_stream(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("../../tests/support/stack.rs");

    #[test]
    fn closed_relational_forms_capture_only_the_host_boolean() {
        let query = syn::parse_quote!(capture { let y: Alias = (limit + 1); });
        let codecs = crate::runs::Codecs::default();
        for predicate in [
            quote!(@(y) > 0),
            quote!(@(y) between 0 and 3),
            quote!(@(y) in { 1, 2 }),
            quote!((@(y) > 0) or (@(y) < -3)),
        ] {
            let theta: Theta = syn::parse2(predicate).unwrap();
            let mut captures = Captures::new(&query);
            let device = captures
                .relational(&theta, &BTreeMap::new(), &codecs)
                .unwrap();
            assert_eq!(captures.records.len(), 1);
            let Capture::Value {
                expression,
                logical,
                codec,
                logical_type_id,
                logical_column,
                codec_id,
            } = &captures.records[0]
            else {
                panic!("a closed restriction must not become a device callable");
            };
            assert_eq!(*logical, syn::parse_quote!(bool));
            assert!(codec.is_none());
            assert!(logical_type_id.is_none());
            assert!(logical_column.is_none());
            assert!(codec_id.is_none());
            let host = expression.to_token_stream().to_string();
            assert!(host.contains("relational_"), "{host}");
            assert!(
                !host.contains("& (y)"),
                "capture binders already borrow values: {host}"
            );
            let mut host_theta = theta.clone();
            captures.host_relational(&mut host_theta, &BTreeMap::new(), &codecs);
            assert_eq!(captures.records.len(), 1);

            let mut frozen = Captures::frozen(&captures.records, &query);
            assert_eq!(
                frozen
                    .relational(&theta, &BTreeMap::new(), &codecs)
                    .unwrap(),
                device
            );
            frozen.host_relational(&mut theta.clone(), &BTreeMap::new(), &codecs);
            assert!(!frozen.missing);
            let mut changed = captures.records.clone();
            let Capture::Value { expression, .. } = &mut changed[0] else {
                unreachable!()
            };
            *expression = syn::parse_quote!(false);
            let mut frozen = Captures::frozen(&changed, &query);
            frozen
                .relational(&theta, &BTreeMap::new(), &codecs)
                .unwrap();
            assert!(
                frozen.missing,
                "lookup-only checking must reject a changed proposition"
            );
        }
    }

    #[test]
    fn closed_range_subject_uses_available_bound_context() {
        let query = syn::parse_quote!(capture { let y: i32 = (limit + 2); });
        let codecs = crate::runs::Codecs::default();
        let types = BTreeMap::from([
            ("low".into(), syn::parse_quote!(i32)),
            ("high".into(), syn::parse_quote!(i32)),
        ]);
        for (predicate, column) in [
            (quote!(@(y) between -2 and high), "high"),
            (quote!(@(y) between low and 2), "low"),
            (quote!(@(y) between low and high), "low"),
        ] {
            let theta: Theta = syn::parse2(predicate).unwrap();
            let mut captures = Captures::new(&query);
            let device = captures.relational(&theta, &types, &codecs).unwrap();
            let mut host = theta.clone();
            captures.host_relational(&mut host, &types, &codecs);
            assert!(captures.records.iter().all(|capture| matches!(
                capture,
                Capture::Value { logical_column: Some(recorded), .. } if recorded == column
            )));
            assert!(check_range_contexts(&theta, &types, &codecs, &captures.records).is_ok());
            let mut frozen = Captures::frozen(&captures.records, &query);
            assert_eq!(frozen.relational(&theta, &types, &codecs).unwrap(), device);
            let mut checked_host = theta.clone();
            frozen.host_relational(&mut checked_host, &types, &codecs);
            assert_eq!(checked_host, host);
            assert!(!frozen.missing);

            for mutation in 0..4 {
                let mut changed = captures.records.clone();
                let Capture::Value {
                    logical_column, logical, expression, ..
                } = &mut changed[0] else {
                    unreachable!()
                };
                match mutation {
                    0 => *logical_column = Some("unavailable".into()),
                    1 => *logical = syn::parse_quote!(u64),
                    2 => *expression = syn::parse_quote!(different()),
                    _ => { changed.remove(0); }
                }
                assert!(check_range_contexts(&theta, &types, &codecs, &changed).is_err());
            }
        }
    }

    #[test]
    fn relational_symbol_operand_uses_recorded_codec() {
        let mut captures = Captures::default();
        let mut codecs = crate::runs::Codecs::default();
        codecs
            .record(
                &syn::parse_quote!(TestSymbol),
                cq_syntax::LogicalTypeId(0),
                Some(cq_lang_structures::provider::CodecId(0)),
                Some(&syn::parse_quote!(TestCodec)),
            )
            .unwrap();
        captures.value(
            &syn::parse_quote!(sym("x")),
            &syn::parse_quote!(TestSymbol),
            Some(cq_syntax::LogicalTypeId(0)),
            Some("x"),
            &codecs,
        );
        let printed = print(&captures.records, None);
        assert_eq!(printed.parameters[0].1, syn::parse_quote!(u32));
        assert!(
            printed.parameters[0]
                .2
                .to_string()
                .contains("SymbolCodec < TestSymbol >")
        );
    }

    #[test]
    fn capture_identity_distinguishes_equal_spelling_ids_and_untyped_columns() {
        let logical = syn::parse_quote!(Logical);
        let expression = syn::parse_quote!(closed());
        let mut codecs = crate::runs::Codecs::default();
        codecs
            .record(
                &logical,
                cq_syntax::LogicalTypeId(1),
                Some(cq_lang_structures::provider::CodecId(1)),
                Some(&syn::parse_quote!(First)),
            )
            .unwrap();
        codecs
            .record(
                &logical,
                cq_syntax::LogicalTypeId(2),
                Some(cq_lang_structures::provider::CodecId(2)),
                Some(&syn::parse_quote!(Second)),
            )
            .unwrap();
        let mut captures = Captures::default();
        let first = captures.value(
            &expression,
            &logical,
            Some(cq_syntax::LogicalTypeId(1)),
            Some("a"),
            &codecs,
        );
        let second = captures.value(
            &expression,
            &logical,
            Some(cq_syntax::LogicalTypeId(2)),
            Some("b"),
            &codecs,
        );
        assert_ne!(first, second);
        let same = captures.value(
            &expression,
            &syn::parse_quote!(Renamed),
            Some(cq_syntax::LogicalTypeId(1)),
            Some("c"),
            &codecs,
        );
        assert_eq!(first, same);
        let unknown_a = captures.value(&expression, &logical, None, Some("computed_a"), &codecs);
        let unknown_b = captures.value(&expression, &logical, None, Some("computed_b"), &codecs);
        assert_ne!(unknown_a, unknown_b);
        let Capture::Value {
            logical_type_id, ..
        } = &mut captures.records[0]
        else {
            unreachable!()
        };
        *logical_type_id = Some(cq_syntax::LogicalTypeId(9));
        let mut frozen = Captures::frozen(&captures.records, &Default::default());
        frozen.value(
            &expression,
            &logical,
            Some(cq_syntax::LogicalTypeId(1)),
            Some("a"),
            &codecs,
        );
        assert!(frozen.missing);
    }
    #[test]
    fn capture_checker_rejects_codec_identity_and_payload_mutations() {
        let logical = syn::parse_quote!(Logical);
        let mut codecs = crate::runs::Codecs::default();
        codecs
            .record(
                &logical,
                cq_syntax::LogicalTypeId(0),
                Some(cq_lang_structures::CodecId(7)),
                Some(&syn::parse_quote!(Codec)),
            )
            .unwrap();
        let mut captures = Captures::default();
        captures.value(
            &syn::parse_quote!(closed()),
            &logical,
            Some(cq_syntax::LogicalTypeId(0)),
            Some("x"),
            &codecs,
        );
        let records = cq_lang_sequence::gpu::realization::Captures {
            captures: captures.records,
            ..Default::default()
        };
        let definitions = BTreeMap::new();
        let Stack { physical, structures, program } = syn::parse_quote! {
            physical {
                relation R(a:i32);
                r = pipe rename R {a -> a};
                result = let canon project r keep {a};
                return result as answer(a);
            }
            structures { providers {
                scan = Scan { reads [], build typed };
                sort = Sort { reads [distinct], build inferred };
            }
                r host scan = @scan as Scan [];
                result rows = @sort as Sort;
                result canon = rows;
            }
            sequence {
                input = store rows r;
                result_rows = store canon map result flatten tile 256 input;
                output count;
            }
        };
        let structure_analysis = cq_lang_structures::contract::check(&physical, &structures).unwrap();
        let analysis = cq_lang_sequence::contract::check(&physical, &structures, &program).unwrap();
        assert!(check(&records, &definitions, &structure_analysis, &analysis, &codecs).is_ok());
        for change_identity in [true, false] {
            let mut changed = records.clone();
            let Capture::Value {
                codec_id, codec, ..
            } = &mut changed.captures[0]
            else {
                unreachable!()
            };
            if change_identity {
                *codec_id = Some(cq_lang_structures::CodecId(8));
            } else {
                *codec = Some(syn::parse_quote!(DifferentPayload));
            }
            assert!(check(&changed, &definitions, &structure_analysis, &analysis, &codecs).is_err());
        }
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
            None,
            "result",
            &crate::runs::Codecs::default(),
        );
        let printed = print(&captures.records, None);
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
