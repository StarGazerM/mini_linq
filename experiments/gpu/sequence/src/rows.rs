use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::{Analysis, Program};
use cq_lang_structures::physical::{symbol_name, Definition, Operator};
use cq_lang_structures::syntax::{
    algebra::Theta,
    constraint::{Comparison, Constraint, Term},
    EqualityValue,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::visit_mut::VisitMut;
use syn::{Ident, Type};

#[derive(Clone)]
struct Interval {
    column: String,
    start: TokenStream,
    end: TokenStream,
}
#[derive(Clone)]
struct Origin {
    host: String,
    columns: BTreeMap<String, String>,
    selection: Option<Interval>,
}
struct Source {
    reads: Vec<(usize, TokenStream)>,
    length: TokenStream,
    positions: Option<Ident>,
}
struct Provider {
    view: Ident,
    payload: Vec<String>,
    keys: Vec<String>,
}

/// Physical adapters only: layouts and predicates are already recorded by the
/// Structures prefix. This module never schedules a Sequence constructor.
pub struct Rows {
    pub width: usize,
    pub columns: BTreeMap<String, usize>,
    pub setup: TokenStream,
    pub build: TokenStream,
    pub parameters: Vec<(Ident, Type, TokenStream)>,
    pub output_columns: Vec<String>,
    sources: BTreeMap<String, Source>,
    providers: BTreeMap<String, Provider>,
    bounds: BTreeMap<String, TokenStream>,
    definitions: BTreeMap<String, Definition>,
    types: BTreeMap<String, BTreeMap<String, Type>>,
}

impl Rows {
    pub fn new(program: &Program, analysis: &Analysis) -> syn::Result<Self> {
        let physical = &program.structures.physical;
        let context = cq_print::Context::new(&program.structures, &analysis.structures);
        let scanned: BTreeSet<_> = analysis
            .dispatches
            .iter()
            .map(|dispatch| {
                analysis
                    .results
                    .get(&dispatch.outer_store)
                    .cloned()
                    .unwrap_or_else(|| dispatch.outer_store.clone())
            })
            .collect();
        let definitions: BTreeMap<_, _> = physical
            .program
            .definitions
            .iter()
            .map(|d| (symbol_name(&d.name), d.clone()))
            .collect();
        let names: BTreeSet<_> = analysis
            .structures
            .physical
            .headings
            .values()
            .flat_map(|heading| heading.iter().cloned())
            .collect();
        let columns: BTreeMap<_, _> = names.into_iter().enumerate().map(|(i, n)| (n, i)).collect();
        let output_columns = physical
            .program
            .head
            .args
            .iter()
            .map(|arg| {
                cq_lang_structures::syntax::rust::variable(arg)
                    .map(symbol_name)
                    .ok_or_else(|| {
                        syn::Error::new_spanned(arg, "T-Gpu: output requires recorded code columns")
                    })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let mut setup = TokenStream::new();
        let mut build = TokenStream::new();
        let mut parameters = Vec::new();
        let mut hosts = BTreeMap::new();
        let mut raw = BTreeMap::new();
        let mut first = true;
        for input in physical.inputs() {
            let name = symbol_name(&input.name);
            let id = format_ident!("__cq_sequence_host_{}", name);
            let length = format_ident!("__cq_sequence_length_{}", name);
            let expression = input
                .value
                .as_ref()
                .map(|leaf| {
                    let expression = &leaf.expression;
                    quote!(#expression)
                })
                .unwrap_or_else(|| {
                    let name = &input.name;
                    quote!(#name)
                });
            setup.extend(quote!(let #id = &(#expression);));
            if first {
                setup.extend(quote! {
                    let __cq_pool = #id.pool().clone();
                    let mut __cq_dictionary = ::cq_gpu_runtime::Dictionary::default();
                    let mut __cq_statuses = ::std::vec::Vec::new();
                });
                first = false;
            }
            setup.extend(quote! {
                if !::std::sync::Arc::ptr_eq(&__cq_pool, #id.pool()) {
                    return Err("T-Gpu: resident inputs must share a pool and stream".into());
                }
                __cq_dictionary.merge(#id.dictionary())?;
                __cq_statuses.extend(#id.storage().statuses.iter().cloned());
                __cq_statuses.push(#id.storage().metadata.clone());
                let #length = #id.checked_len()?;
            });
            let layout = &analysis.structures.host_layouts[&name];
            let mut fields = BTreeMap::new();
            for (index, column) in layout.iter().enumerate() {
                let param = format_ident!("__cq_sequence_column_{}_{}", name, index);
                let view = quote!(unsafe { #id.storage().columns[#index].view() });
                parameters.push((
                    param.clone(),
                    syn::parse_quote!(::cq_gpu_runtime::ColumnView),
                    view,
                ));
                fields.insert(column.clone(), (index, param));
            }
            raw.insert(name.clone(), fields);
            hosts.insert(name, (id, length));
        }
        if first {
            return Err(syn::Error::new_spanned(
                program,
                "T-Gpu: a resident input supplies the execution pool",
            ));
        }
        setup.extend(quote! {
            let __cq_dictionary = ::std::sync::Arc::new(__cq_dictionary);
            ::std::thread_local! {
                static __CQ_OUTPUT_ADAPTER: ::std::cell::RefCell<Option<
                    ::std::rc::Rc<::std::cell::RefCell<::cq_gpu_sequence_runtime::canon::PackedColumnsAdapter>>
                >> = const { ::std::cell::RefCell::new(None) };
            }
            let __cq_range_builder = ::cq_gpu_sequence_runtime::RangeBuilder::new(&__cq_pool)?;
        });
        let output_width = output_columns.len();
        setup.extend(quote! {
            let __cq_output_adapter = __CQ_OUTPUT_ADAPTER.with(|cache| -> ::cq_gpu_sequence_runtime::Result<_> {
                let mut cache = cache.borrow_mut();
                if cache.as_ref().is_none_or(|adapter|
                    !::std::sync::Arc::ptr_eq(adapter.borrow().pool(), &__cq_pool)) {
                    *cache = Some(::std::rc::Rc::new(::std::cell::RefCell::new(
                        ::cq_gpu_sequence_runtime::canon::PackedColumnsAdapter::new(__cq_pool.clone(), #output_width)?)));
                }
                Ok(cache.as_ref().unwrap().clone())
            })?;
        });
        let mut sources = BTreeMap::new();
        let mut providers = BTreeMap::new();
        let mut bound_sources: BTreeMap<String, BTreeSet<(String, usize)>> = BTreeMap::new();
        let required_sources: BTreeSet<_> = analysis.results.values().cloned().collect();
        for (name, definition) in &definitions {
            if !required_sources.contains(name)
                && !analysis.structures.built.contains_key(name)
                && !matches!(definition.operator, Operator::Rename(_))
            {
                continue;
            }
            let Some(origin) = origin(name, &definitions)? else {
                continue;
            };
            let (host, host_length) = &hosts[&origin.host];
            let fields = &raw[&origin.host];
            for (column, source) in &origin.columns {
                bound_sources
                    .entry(column.clone())
                    .or_default()
                    .insert((origin.host.clone(), fields[source].0));
            }
            let selection = if let Some(selection) = &origin.selection {
                let index = fields[&selection.column].0;
                let start = &selection.start;
                let end = &selection.end;
                quote!(Some(::cq_gpu_sequence_runtime::RangeSelection {
                    column: unsafe { ::cq_gpu_sequence_runtime::ColumnDescriptor::new(
                        #host.storage().columns[#index].clone(), #host_length)? },
                    start: #start, end: #end,
                }))
            } else {
                quote!(None)
            };
            let built = analysis
                .structures
                .built
                .get(name)
                .and_then(|layouts| layouts.first());
            if let Some((levels, _)) = built {
                if analysis.structures.built[name].len() != 1 {
                    return Err(syn::Error::new_spanned(
                        definition,
                        "T-Gpu: source has multiple recorded provider layouts; name each provider store separately",
                    ));
                }
                let keys: Vec<_> = levels.iter().flatten().cloned().collect();
                let mut key_descriptors = Vec::new();
                for key in &keys {
                    let source = origin.columns.get(key).ok_or_else(|| {
                        syn::Error::new_spanned(
                            definition,
                            "T-Gpu: provider key has no resident origin",
                        )
                    })?;
                    let index = fields[source].0;
                    key_descriptors.push(quote! {
                        ::cq_gpu_sequence_runtime::KeyDescriptor {
                            column: unsafe { ::cq_gpu_sequence_runtime::ColumnDescriptor::new(
                                #host.storage().columns[#index].clone(), #host_length)? },
                            domain_size: match #host.storage().bounds[#index] {
                                ::cq_gpu_runtime::Bounds::Empty => 0usize,
                                ::cq_gpu_runtime::Bounds::Inclusive { min, max } => {
                                    if min < 0 || max > u32::MAX as i64 {
                                        return Err("T-Gpu: range keys require dense nonnegative u32 codes".into());
                                    }
                                    usize::try_from(max)?.checked_add(1).ok_or("T-Gpu: key domain overflow")?
                                }
                            },
                        }
                    });
                }
                let payload = if scanned.contains(name) {
                    analysis.structures.physical.headings[name].clone()
                } else {
                    let mut payload = context.built_payload(name, levels);
                    // Code equality reconstructs the leading key from the left
                    // row; unlike borrowed host tuples, no identity is lost.
                    payload.retain(|column| keys.first() != Some(column));
                    payload.extend(keys.iter().skip(1).cloned());
                    payload.sort();
                    payload.dedup();
                    payload
                };
                let mut payload_descriptors = Vec::new();
                for column in &payload {
                    let source = &origin.columns[column];
                    let index = fields[source].0;
                    let ty = &analysis.structures.physical.rows[name].types[column];
                    let kind = match primitive(ty).as_deref() {
                        Some("u32" | "u16" | "u8" | "bool") => {
                            quote!(::cq_gpu_sequence_runtime::PayloadKind::U32)
                        }
                        Some("i32" | "i16" | "i8") => {
                            quote!(::cq_gpu_sequence_runtime::PayloadKind::I32)
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(
                                ty,
                                "T-Gpu: range payload needs a recorded u32/i32 dense code",
                            ));
                        }
                    };
                    payload_descriptors.push(quote! {
                        ::cq_gpu_sequence_runtime::PayloadDescriptor {
                            column: unsafe { ::cq_gpu_sequence_runtime::ColumnDescriptor::new(
                                #host.storage().columns[#index].clone(), #host_length)? }, kind: #kind,
                        }
                    });
                }
                let owner = format_ident!("__cq_sequence_provider_{}", name);
                let view = format_ident!("__cq_sequence_provider_view_{}", name);
                let descriptor = format_ident!("__cq_sequence_descriptor_{}", name);
                let workspace = format_ident!("__cq_sequence_workspace_{}", name);
                let cache = format_ident!("__CQ_SEQUENCE_WORKSPACE_{}", name.to_uppercase());
                setup.extend(quote! {
                    let #descriptor = ::cq_gpu_sequence_runtime::RangeDescriptor {
                        key: vec![#(#key_descriptors),*], selection: #selection,
                        payload: vec![#(#payload_descriptors),*],
                    };
                    ::std::thread_local! {
                        static #cache: ::std::cell::RefCell<Option<::std::rc::Rc<::std::cell::RefCell<
                            ::cq_gpu_sequence_runtime::RangeWorkspace
                        >>>> = const { ::std::cell::RefCell::new(None) };
                    }
                    let #workspace = #cache.with(|cache| -> ::cq_gpu_sequence_runtime::Result<_> {
                        let mut cache = cache.borrow_mut();
                        if cache.as_ref().is_none_or(|workspace| !workspace.borrow().matches(&__cq_pool, &#descriptor)) {
                            *cache = Some(::std::rc::Rc::new(::std::cell::RefCell::new(
                                ::cq_gpu_sequence_runtime::RangeWorkspace::new(__cq_pool.clone(), &#descriptor)?)));
                        }
                        Ok(cache.as_ref().unwrap().clone())
                    })?;
                });
                build.extend(quote! {
                    let #owner = __cq_range_builder.build_with_workspace(&#descriptor, &mut #workspace.borrow_mut())?;
                });
                parameters.push((
                    view.clone(),
                    syn::parse_quote!(::cq_gpu_sequence_runtime::RangeView),
                    quote!(unsafe { #owner.view() }),
                ));
                let reads = payload
                    .iter()
                    .enumerate()
                    .map(|(index, column)| {
                        (
                            columns[column],
                            quote!(unsafe { *#view.payload.add(#index) }),
                        )
                    })
                    .collect();
                sources.insert(
                    name.clone(),
                    Source {
                        reads,
                        length: quote!(#owner.len()),
                        positions: None,
                    },
                );
                providers.insert(
                    name.clone(),
                    Provider {
                        view,
                        payload,
                        keys,
                    },
                );
            } else {
                let reads = origin
                    .columns
                    .iter()
                    .filter_map(|(column, source)| {
                        columns.get(column).map(|&slot| {
                            let param = &fields[source].1;
                            (slot, quote!(#param))
                        })
                    })
                    .collect();
                let (positions, length) = if origin.selection.is_some() {
                    let selected = format_ident!("__cq_sequence_selected_{}", name);
                    let param = format_ident!("__cq_sequence_selected_rows_{}", name);
                    build.extend(quote! {
                        let #selected = ::cq_gpu_sequence_runtime::select(
                            __cq_pool.clone(), &(#selection).expect("recorded selection"))?;
                    });
                    parameters.push((
                        param.clone(),
                        syn::parse_quote!(*const u32),
                        quote!(#selected.ptr()),
                    ));
                    (Some(param), quote!(#selected.len()))
                } else {
                    (None, quote!(#host_length))
                };
                sources.insert(
                    name.clone(),
                    Source {
                        reads,
                        length,
                        positions,
                    },
                );
            }
        }
        let mut bounds = BTreeMap::new();
        for (column, origins) in bound_sources {
            let values = origins.iter().map(|(host, index)| {
                let host = &hosts[host].0;
                quote!(#host.storage().bounds[#index])
            });
            bounds.insert(column, quote! {
                [#(#values),*].into_iter().fold(::cq_gpu_runtime::Bounds::Empty, |a,b| match (a,b) {
                    (::cq_gpu_runtime::Bounds::Empty, x) | (x, ::cq_gpu_runtime::Bounds::Empty) => x,
                    (::cq_gpu_runtime::Bounds::Inclusive { min:a, max:b }, ::cq_gpu_runtime::Bounds::Inclusive { min:c, max:d }) =>
                        ::cq_gpu_runtime::Bounds::Inclusive { min:a.min(c), max:b.max(d) },
                })
            });
        }
        build.extend(quote! {
            let __cq_timing_built = __cq_pool.stream().record_event(
                Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
        });
        let types = analysis
            .structures
            .physical
            .rows
            .iter()
            .map(|(name, row)| (name.clone(), row.types.clone()))
            .collect();
        Ok(Self {
            width: columns.len().max(1),
            columns,
            setup,
            build,
            parameters,
            output_columns,
            sources,
            providers,
            bounds,
            definitions,
            types,
        })
    }

    pub fn source(
        &self,
        name: &str,
        position: TokenStream,
        row: TokenStream,
    ) -> syn::Result<TokenStream> {
        let source = self.sources.get(name).ok_or_else(|| {
            error(format!(
                "T-Gpu: {name} has no resident source; materialize its Sequence producer"
            ))
        })?;
        let position = match &source.positions {
            Some(positions) => quote!(unsafe { *#positions.add((#position) as usize) } as u64),
            None => quote!((#position) as u64),
        };
        let reads = source.reads.iter().map(|(slot, view)| quote! {
            #row[#slot] = unsafe { ::cq_gpu_sequence_runtime::device::decode(#view, #position) };
        });
        Ok(quote!(#(#reads)*))
    }
    pub fn source_len(&self, name: &str) -> syn::Result<TokenStream> {
        self.sources
            .get(name)
            .map(|s| s.length.clone())
            .ok_or_else(|| error(format!("T-Gpu: {name} has no recorded source length")))
    }
    pub fn provider(&self, name: &str) -> syn::Result<(Ident, Vec<String>, Vec<String>)> {
        self.providers
            .get(name)
            .map(|p| (p.view.clone(), p.payload.clone(), p.keys.clone()))
            .ok_or_else(|| {
                error(format!(
                    "T-Gpu: {name} has no recorded resident range layout"
                ))
            })
    }
    pub fn bounds(&self, column: &str) -> syn::Result<TokenStream> {
        self.bounds
            .get(column)
            .cloned()
            .ok_or_else(|| error(format!("T-Gpu: {column} has no recorded codec bounds")))
    }
    pub fn map(
        &self,
        name: &str,
        input_result: &str,
        row: TokenStream,
        live: TokenStream,
    ) -> syn::Result<TokenStream> {
        let mut chain = Vec::new();
        let mut current = name;
        while current != input_result {
            let definition = self
                .definitions
                .get(current)
                .ok_or_else(|| error("T-Gpu: unknown Physical row function"))?;
            chain.push(definition);
            let Some(input) = definition.operator.input() else {
                break;
            };
            current = self
                .definitions
                .keys()
                .find(|key| **key == symbol_name(input))
                .map(String::as_str)
                .ok_or_else(|| error("T-Gpu: map does not extend its checked input"))?;
        }
        let mut output = TokenStream::new();
        for definition in chain.into_iter().rev() {
            match &definition.operator {
                Operator::Rename(_) | Operator::Project(_) | Operator::Unit(_) => {}
                Operator::Select(select)
                | Operator::Point(_, select)
                | Operator::Range(_, select) => {
                    validate_theta(&select.condition)?;
                    let input = symbol_name(&select.input);
                    let bindings = self.types[&input].iter().map(|(column, ty)| {
                        let id = Ident::new(column, proc_macro2::Span::call_site());
                        let slot = self.columns[column];
                        quote!(let #id = &(#row[#slot] as #ty);)
                    });
                    let tokens = cq_print::expressions::relational_condition_with(
                        &select.condition,
                        &BTreeMap::new(),
                    );
                    let mut expression: syn::Expr = syn::parse2(tokens)?;
                    DeviceComparisons.visit_expr_mut(&mut expression);
                    output.extend(quote! { if #live { #(#bindings)* #live = #expression; } });
                }
                Operator::Semi(_, _)
                | Operator::Anti(_, _)
                | Operator::Probe(_, _)
                | Operator::Merge(_, _) => {
                    // The Sequence Lookup constructor already executed the keyed operation.
                }
                Operator::Bind(_) | Operator::Guard(_) => {
                    return Err(syn::Error::new_spanned(
                        definition,
                        "T-Gpu: opaque Rust map is not #[device]-compatible; evaluate it at host before dispatch",
                    ));
                }
                Operator::Union(_) => {
                    return Err(syn::Error::new_spanned(
                        definition,
                        "T-Gpu: union needs explicit Sequence arms",
                    ));
                }
            }
        }
        Ok(output)
    }
}

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}
fn primitive(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
            Some(path.path.segments[0].ident.to_string())
        }
        _ => None,
    }
}

fn origin(name: &str, definitions: &BTreeMap<String, Definition>) -> syn::Result<Option<Origin>> {
    let definition = &definitions[name];
    match &definition.operator {
        Operator::Rename(rename) => {
            let mut result = Origin {
                host: symbol_name(&rename.relation),
                columns: rename
                    .mapping
                    .iter()
                    .map(|m| (symbol_name(&m.target), symbol_name(&m.source)))
                    .collect(),
                selection: None,
            };
            if let Some(constants) = &rename.constants {
                for equality in &constants.equalities {
                    let EqualityValue::Literal(value) = &equality.right else {
                        return Err(error(
                            "T-Gpu: source selection must compare a code to a constant",
                        ));
                    };
                    interval(
                        &mut result.selection,
                        symbol_name(&equality.left),
                        quote!((#value) as i64),
                        "eq",
                    )?;
                }
            }
            Ok(Some(result))
        }
        Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
            let Some(mut result) = origin(&symbol_name(&select.input), definitions)? else {
                return Ok(None);
            };
            intervals(&select.condition, &result.columns, &mut result.selection)?;
            Ok(Some(result))
        }
        Operator::Project(project) => origin(&symbol_name(&project.input), definitions),
        _ => Ok(None),
    }
}

fn interval(
    target: &mut Option<Interval>,
    column: String,
    value: TokenStream,
    op: &str,
) -> syn::Result<()> {
    if target.as_ref().is_some_and(|old| old.column != column) {
        return Err(error(
            "T-Gpu: recorded range build selection must be a half-open interval on one source column",
        ));
    }
    let selection = target.get_or_insert(Interval {
        column,
        start: quote!(i64::MIN),
        end: quote!(i64::MAX),
    });
    let next = quote!((#value).checked_add(1).ok_or("T-Gpu: selection bound overflow")?);
    match op {
        "eq" => {
            selection.start = value;
            selection.end = next;
        }
        "ge" | "gt" => {
            let prior = &selection.start;
            let value = if op == "gt" { next } else { value };
            selection.start = quote!((#prior).max(#value));
        }
        "lt" | "le" => {
            let prior = &selection.end;
            let value = if op == "le" { next } else { value };
            selection.end = quote!((#prior).min(#value));
        }
        _ => return Err(error("T-Gpu: provider selection is not a contiguous range")),
    }
    Ok(())
}

fn intervals(
    theta: &Theta,
    columns: &BTreeMap<String, String>,
    target: &mut Option<Interval>,
) -> syn::Result<()> {
    for equality in &theta.equalities {
        let EqualityValue::Literal(value) = &equality.right else {
            return Err(error("T-Gpu: provider equality requires a constant code"));
        };
        interval(
            target,
            columns[&symbol_name(&equality.left)].clone(),
            quote!((#value) as i64),
            "eq",
        )?;
    }
    for constraint in &theta.constraints {
        match constraint {
            Constraint::Compare {
                left: Term::Variable(column),
                comparison,
                right,
            } => {
                let value = host_value(right)?;
                let op = match comparison {
                    Comparison::Eq(_) => "eq",
                    Comparison::Ge(_) => "ge",
                    Comparison::Gt(_) => "gt",
                    Comparison::Lt(_) => "lt",
                    Comparison::Le(_) => "le",
                    _ => "ne",
                };
                interval(target, columns[&symbol_name(column)].clone(), value, op)?;
            }
            Constraint::Between {
                term: Term::Variable(column),
                low,
                high,
            } => {
                interval(
                    target,
                    columns[&symbol_name(column)].clone(),
                    host_value(low)?,
                    "ge",
                )?;
                interval(
                    target,
                    columns[&symbol_name(column)].clone(),
                    host_value(high)?,
                    "le",
                )?;
            }
            Constraint::Total {
                expression,
                bindings,
            } => {
                validate_device(expression)?;
                let aliases = bindings
                    .iter()
                    .filter_map(|(name, term)| match term {
                        Term::Variable(column) => {
                            Some((symbol_name(name), columns[&symbol_name(column)].clone()))
                        }
                        _ => None,
                    })
                    .collect();
                expression_intervals(expression, &aliases, target)?;
            }
            _ => {
                return Err(error(
                    "T-Gpu: provider predicate requires a recorded contiguous range selection",
                ));
            }
        }
    }
    Ok(())
}
fn host_value(term: &Term) -> syn::Result<TokenStream> {
    match term {
        Term::Literal(value) => Ok(quote!((#value) as i64)),
        Term::Parameter(value) => Ok(quote!((#value) as i64)),
        _ => Err(error(
            "T-Gpu: provider bound must be a constant or host parameter",
        )),
    }
}
fn ungroup(expression: &syn::Expr) -> &syn::Expr {
    match expression {
        syn::Expr::Paren(e) => ungroup(&e.expr),
        syn::Expr::Group(e) => ungroup(&e.expr),
        _ => expression,
    }
}
fn expression_intervals(
    expression: &syn::Expr,
    aliases: &BTreeMap<String, String>,
    target: &mut Option<Interval>,
) -> syn::Result<()> {
    let syn::Expr::Binary(binary) = ungroup(expression) else {
        return Err(error(
            "T-Gpu: provider total predicate must be a conjunction of code bounds",
        ));
    };
    if matches!(binary.op, syn::BinOp::And(_)) {
        expression_intervals(&binary.left, aliases, target)?;
        return expression_intervals(&binary.right, aliases, target);
    }
    let left = match ungroup(&binary.left) {
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)) => ungroup(&unary.expr),
        other => other,
    };
    let syn::Expr::Path(path) = left else {
        return Err(error("T-Gpu: provider bound must address one code column"));
    };
    let column = path
        .path
        .get_ident()
        .and_then(|id| aliases.get(&symbol_name(id)))
        .ok_or_else(|| error("T-Gpu: provider bound column is not a checked binding"))?;
    let op = match binary.op {
        syn::BinOp::Eq(_) => "eq",
        syn::BinOp::Ge(_) => "ge",
        syn::BinOp::Gt(_) => "gt",
        syn::BinOp::Lt(_) => "lt",
        syn::BinOp::Le(_) => "le",
        _ => "other",
    };
    let value = &binary.right;
    interval(target, column.clone(), quote!((#value) as i64), op)
}
fn validate_theta(theta: &Theta) -> syn::Result<()> {
    for constraint in &theta.constraints {
        if let Constraint::Total { expression, .. } = constraint {
            validate_device(expression)?;
        }
    }
    Ok(())
}
fn validate_device(expression: &syn::Expr) -> syn::Result<()> {
    match expression {
        syn::Expr::Binary(e) => {
            validate_device(&e.left)?;
            validate_device(&e.right)
        }
        syn::Expr::Unary(e) => validate_device(&e.expr),
        syn::Expr::Paren(e) => validate_device(&e.expr),
        syn::Expr::Group(e) => validate_device(&e.expr),
        syn::Expr::Cast(e) => validate_device(&e.expr),
        syn::Expr::Reference(e) => validate_device(&e.expr),
        syn::Expr::Path(_) => Ok(()),
        syn::Expr::Lit(e) if matches!(e.lit, syn::Lit::Int(_) | syn::Lit::Bool(_)) => Ok(()),
        _ => Err(syn::Error::new_spanned(
            expression,
            "T-Gpu: Rust map is not #[device]-compatible; only total scalar code expressions are supported below host",
        )),
    }
}

struct DeviceComparisons;
impl VisitMut for DeviceComparisons {
    fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
        if let syn::Expr::MethodCall(method) = expression {
            if let syn::Expr::Call(call) = ungroup(&method.receiver) {
                if let syn::Expr::Path(path) = call.func.as_ref() {
                    if path
                        .path
                        .segments
                        .last()
                        .is_some_and(|s| s.ident == "relational_cmp")
                        && call.args.len() == 2
                    {
                        let left = &call.args[0];
                        let right = &call.args[1];
                        let replacement = match method.method.to_string().as_str() {
                            "is_lt" => Some(quote!(*#left < *#right)),
                            "is_gt" => Some(quote!(*#left > *#right)),
                            _ => None,
                        };
                        if let Some(tokens) = replacement {
                            *expression = syn::parse2(tokens).expect("comparison expression");
                            return;
                        }
                    }
                }
            }
        }
        if let syn::Expr::Call(call) = expression {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                if path
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "relational_eq")
                    && call.args.len() == 2
                {
                    let left = &call.args[0];
                    let right = &call.args[1];
                    *expression = syn::parse_quote!(*#left == *#right);
                    return;
                }
            }
        }
        syn::visit_mut::visit_expr_mut(self, expression);
    }
}
