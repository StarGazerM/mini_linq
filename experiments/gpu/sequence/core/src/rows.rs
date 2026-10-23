use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::{Analysis, Obligation, Program};
use cq_lang_structures::physical::{Definition, Operator, symbol_name};
use cq_lang_structures::syntax as cq_syntax;
use cq_lang_structures::syntax::{
    EqualityValue,
    algebra::Theta,
    constraint::{Comparison, Constraint, Term},
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

/// Device bindings are scalar codes or direct reads through a parent's owner
/// index. Missing bindings are dead columns, never a second physical row.
#[derive(Clone, Default)]
pub struct DeviceRow(pub BTreeMap<usize, TokenStream>);

impl DeviceRow {
    pub fn writes(tokens: TokenStream) -> syn::Result<BTreeSet<usize>> {
        struct Writes(BTreeSet<usize>);
        impl VisitMut for Writes {
            fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
                if let syn::Expr::Assign(assign) = expression
                    && let Some(slot) = row_slot(&assign.left)
                {
                    self.0.insert(slot);
                }
                syn::visit_mut::visit_expr_mut(self, expression);
            }
        }
        let mut block: syn::Block = syn::parse2(quote!({#tokens}))?;
        let mut writes = Writes(BTreeSet::new());
        writes.visit_block_mut(&mut block);
        Ok(writes.0)
    }
    pub fn uses(tokens: TokenStream) -> syn::Result<BTreeSet<usize>> {
        struct Uses(BTreeSet<usize>);
        impl VisitMut for Uses {
            fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
                if let Some(slot) = row_slot(expression) {
                    self.0.insert(slot);
                }
                syn::visit_mut::visit_expr_mut(self, expression);
            }
        }
        let mut block: syn::Block = syn::parse2(quote!({#tokens}))?;
        let mut uses = Uses(BTreeSet::new());
        uses.visit_block_mut(&mut block);
        Ok(uses.0)
    }
    pub fn rewrite(&self, tokens: TokenStream) -> syn::Result<TokenStream> {
        struct Rewrite<'a>(&'a DeviceRow);
        impl VisitMut for Rewrite<'_> {
            fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
                if let syn::Expr::Assign(assign) = expression
                    && let Some(slot) = row_slot(&assign.left)
                {
                    if let Some(value) = self.0.0.get(&slot) {
                        let right = &assign.right;
                        *expression = syn::parse_quote!(#value = (#right) as _);
                    } else {
                        *expression = syn::parse_quote!(());
                    }
                    return;
                }
                if let Some(slot) = row_slot(expression) {
                    *expression = self.0.0.get(&slot).map_or_else(
                        || syn::parse_quote!(0i64),
                        |value| syn::parse_quote!(#value),
                    );
                    return;
                }
                syn::visit_mut::visit_expr_mut(self, expression);
            }
        }
        let mut block: syn::Block = syn::parse2(quote!({#tokens}))?;
        Rewrite(self).visit_block_mut(&mut block);
        let statements = block.stmts;
        Ok(quote!(#(#statements)*))
    }
}

fn row_slot(expression: &syn::Expr) -> Option<usize> {
    let syn::Expr::Index(index) = expression else {
        return None;
    };
    let syn::Expr::Path(path) = &*index.expr else {
        return None;
    };
    if !path.path.is_ident("__row") {
        return None;
    }
    let syn::Expr::Lit(literal) = &*index.index else {
        return None;
    };
    let syn::Lit::Int(index) = &literal.lit else {
        return None;
    };
    index.base10_parse().ok()
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
    pub dense: BTreeSet<usize>,
    sources: BTreeMap<String, Source>,
    providers: BTreeMap<String, Provider>,
    bounds: BTreeMap<String, TokenStream>,
    definitions: BTreeMap<String, Definition>,
    types: BTreeMap<String, BTreeMap<String, Type>>,
    resident_hosts: BTreeSet<String>,
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
        let device_results: BTreeSet<_> = analysis
            .dispatches
            .iter()
            .flat_map(|dispatch| {
                let source = analysis.results.get(&dispatch.outer_store);
                dispatch
                    .inner_stores
                    .iter()
                    .filter_map(|name| analysis.results.get(name))
                    .filter(move |result| Some(*result) != source)
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
            let resident = program.structures.structures.entries.iter().any(|entry| {
                analysis.structures.physical.hosts.get(&symbol_name(&entry.name)) == Some(&name)
                    && matches!(&entry.structure,
                        cq_lang_structures::Structure::Host { reader, keyed, .. }
                        if reader == "scan" && matches!(&keyed.ty, Type::Path(path)
                            if path.path.segments.last().is_some_and(|segment| segment.ident == "DeviceColumns")))
            });
            if !resident {
                continue;
            }
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
        // Only host stores and dispatch inputs need resident source adapters.
        // Intermediate map/compact results execute their predicates per lane;
        // interpreting them as host range selections loses row-valued bounds.
        let required_sources: BTreeSet<_> = scanned
            .iter()
            .cloned()
            .chain(analysis.stores.keys().filter_map(|name| {
                analysis
                    .levels
                    .get(name)
                    .is_some_and(|level| level.last().is_some_and(|level| level == "host"))
                    .then(|| analysis.results.get(name).cloned())
                    .flatten()
            }))
            .collect();
        let resident_hosts: BTreeSet<_> = hosts.keys().cloned().collect();
        for (name, definition) in &definitions {
            if !matches!(definition.operator, Operator::Rename(_))
                && (device_results.contains(name)
                    || (!required_sources.contains(name)
                        && !analysis.structures.built.contains_key(name)))
            {
                continue;
            }
            let dependencies = host_dependencies(name, &definitions);
            if !matches!(definition.operator, Operator::Rename(_))
                && dependencies
                    .iter()
                    .any(|result| device_results.contains(result))
            {
                continue;
            }
            let needs_host = dependencies.iter().any(|result| {
                analysis
                    .structures
                    .physical
                    .hosts
                    .get(result)
                    .is_some_and(|host| !resident_hosts.contains(host))
            });
            let origin = if needs_host {
                let layout = &analysis.structures.layouts[name];
                if (!required_sources.contains(name)
                    && !analysis.structures.built.contains_key(name))
                    || layout.iter().any(|column| {
                        !matches!(
                            primitive(&analysis.structures.physical.rows[name].types[column])
                                .as_deref(),
                            Some("i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "bool")
                        )
                    })
                {
                    continue;
                }
                let rows = host_rows(
                    program,
                    analysis,
                    name,
                    &definitions,
                    &resident_hosts,
                    &BTreeMap::new(),
                    !layout.is_empty(),
                )?;
                let id = format_ident!("__cq_sequence_uploaded_{name}");
                let length = format_ident!("__cq_sequence_uploaded_length_{name}");
                let types = layout
                    .iter()
                    .map(|column| &analysis.structures.physical.rows[name].types[column]);
                let upload = if layout.is_empty() {
                    quote!(::cq_gpu_runtime::DeviceColumns::from_rows(
                        __cq_pool.clone(),
                        &rows,
                        &mut ::cq_gpu_runtime::Dictionary::default(),
                        &[]
                    )?)
                } else {
                    quote!(::cq_gpu_sequence_runtime::upload_numeric_columns::<(#(#types,)*), _>(
                        __cq_pool.clone(), rows)?)
                };
                build.extend(quote! {
                    let #id = {
                        let rows = #rows;
                        #upload
                    };
                    let #length = #id.checked_len()?;
                    __cq_statuses.push(#id.storage().metadata.clone());
                });
                let mut fields = BTreeMap::new();
                for (index, column) in layout.iter().enumerate() {
                    let param = format_ident!("__cq_sequence_uploaded_column_{name}_{index}");
                    parameters.push((
                        param.clone(),
                        syn::parse_quote!(::cq_gpu_runtime::ColumnView),
                        quote!(unsafe { #id.storage().columns[#index].view() }),
                    ));
                    fields.insert(column.clone(), (index, param));
                }
                raw.insert(name.clone(), fields);
                hosts.insert(name.clone(), (id, length));
                Origin {
                    host: name.clone(),
                    columns: layout
                        .iter()
                        .map(|column| (column.clone(), column.clone()))
                        .collect(),
                    selection: None,
                }
            } else {
                let Some(origin) = origin(name, &definitions)? else {
                    continue;
                };
                origin
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
            let built = (!device_results.contains(name))
                .then(|| analysis.structures.built.get(name))
                .flatten()
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
                    analysis.structures.layouts[name].clone()
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
                    let kind = payload_kind(ty)?;
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
                let preparation = if needs_host { &mut build } else { &mut setup };
                preparation.extend(quote! {
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
        // Computed CQ-Arith values have their declared primitive range, not
        // the source codec's range: wrapping can cross either signed endpoint.
        for row in analysis.structures.physical.rows.values() {
            for column in &row.computed {
                let ty = &row.types[column];
                if matches!(
                    primitive(ty).as_deref(),
                    Some("i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32")
                ) {
                    bounds.entry(column.clone()).or_insert_with(|| {
                        quote! {
                            ::cq_gpu_runtime::Bounds::Inclusive {
                                min: <#ty>::MIN as i64, max: <#ty>::MAX as i64,
                            }
                        }
                    });
                }
            }
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
        let codec_columns: BTreeSet<_> = analysis
            .obligations
            .iter()
            .filter_map(|obligation| {
                if let Obligation::Codec { columns, .. } = obligation {
                    Some(columns)
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        let dense = columns
            .iter()
            .filter_map(|(column, slot)| {
                // A codec obligation alone does not prove nonnegative code width.
                // Use the checked physical code type as well.
                (codec_columns.contains(column)
                    && analysis
                        .structures
                        .physical
                        .rows
                        .values()
                        .filter_map(|row| row.types.get(column))
                        .all(|ty| crate::layout::code_bytes(ty, true) == 4))
                .then_some(*slot)
            })
            .collect();
        Ok(Self {
            width: columns.len().max(1),
            columns,
            setup,
            build,
            parameters,
            output_columns,
            dense,
            sources,
            providers,
            bounds,
            definitions,
            types,
            resident_hosts,
        })
    }

    pub fn has_source(&self, name: &str) -> bool {
        self.sources.contains_key(name)
    }

    pub fn host_dependencies(&self, name: &str) -> BTreeSet<String> {
        host_dependencies(name, &self.definitions)
    }

    pub fn bound_host_dependencies(
        &self,
        name: &str,
        bindings: &BTreeMap<String, Ident>,
    ) -> BTreeSet<String> {
        host_dependencies_from(name, &self.definitions, bindings)
    }

    /// Realize the recorded host boundary after its device operands publish.
    /// Bindings are decoded complete publications, never re-evaluated producers.
    pub fn publish_host(
        &mut self,
        program: &Program,
        analysis: &Analysis,
        name: &str,
        bindings: &BTreeMap<String, Ident>,
    ) -> syn::Result<TokenStream> {
        let rows = host_rows(
            program,
            analysis,
            name,
            &self.definitions,
            &self.resident_hosts,
            bindings,
            false,
        )?;
        let layout = &analysis.structures.layouts[name];
        if layout.iter().any(|column| {
            !matches!(
                primitive(&analysis.structures.physical.rows[name].types[column]).as_deref(),
                Some("i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "bool")
            )
        }) {
            return Err(error(format!(
                "T-Gpu: host publication {name} needs recorded device-representable code columns"
            )));
        }
        let id = format_ident!("__cq_sequence_uploaded_{name}");
        let length = format_ident!("__cq_sequence_uploaded_length_{name}");
        let codecs = layout.iter().map(|_| quote!(::cq_gpu_runtime::Codec::For));
        let mut reads = Vec::new();
        for (index, column) in layout.iter().enumerate() {
            let param = format_ident!("__cq_sequence_uploaded_column_{name}_{index}");
            self.parameters.push((
                param.clone(),
                syn::parse_quote!(::cq_gpu_runtime::ColumnView),
                quote!(unsafe { #id.storage().columns[#index].view() }),
            ));
            reads.push((self.columns[column], quote!(#param)));
            let bound = quote!(#id.storage().bounds[#index]);
            let bound = if let Some(previous) = self.bounds.get(column) {
                quote! {
                    match (#previous, #bound) {
                        (::cq_gpu_runtime::Bounds::Empty, x) | (x, ::cq_gpu_runtime::Bounds::Empty) => x,
                        (::cq_gpu_runtime::Bounds::Inclusive {min:a,max:b}, ::cq_gpu_runtime::Bounds::Inclusive {min:c,max:d}) =>
                            ::cq_gpu_runtime::Bounds::Inclusive {min:a.min(c),max:b.max(d)},
                    }
                }
            } else {
                bound
            };
            self.bounds.insert(column.clone(), bound);
        }
        self.sources.insert(
            name.to_owned(),
            Source {
                reads,
                length: quote!(#length),
                positions: None,
            },
        );
        Ok(quote! {
            let #id = {
                let rows = #rows;
                ::cq_gpu_runtime::DeviceColumns::from_rows(
                    __cq_pool.clone(), &rows,
                    &mut ::cq_gpu_runtime::Dictionary::default(), &[#(#codecs),*])?
            };
            let #length = #id.checked_len()?;
            __cq_statuses.push(#id.storage().metadata.clone());
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
        let reads = source.reads.iter().map(|(slot, view)| {
            let decode = if self.dense.contains(slot) { format_ident!("decode_u32") } else { format_ident!("decode") };
            quote! {
                #row[#slot] = unsafe { ::cq_gpu_sequence_runtime::device::#decode(#view, #position) } as i64;
            }
        });
        Ok(quote!(#(#reads)*))
    }
    pub fn source_len(&self, name: &str) -> syn::Result<TokenStream> {
        self.sources
            .get(name)
            .map(|s| s.length.clone())
            .ok_or_else(|| error(format!("T-Gpu: {name} has no recorded source length")))
    }
    pub fn provider_name(&self, name: &str, keys: &[String]) -> syn::Result<String> {
        if self.providers.contains_key(name) {
            return Ok(name.to_owned());
        }
        let mut keys = keys.to_vec();
        keys.sort();
        let name = format!("{name}__{}", keys.join("__"));
        self.providers
            .get(&name)
            .ok_or_else(|| error("T-Gpu: lookup has no matching recorded provider layout"))?;
        Ok(name)
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
    /// Realize the recorded provider only after its device store has published
    /// its live count. Descriptors borrow the stored layout without transposing
    /// or copying it before the ordinary radix/lower-bound range build.
    pub fn publish_provider(
        &mut self,
        analysis: &Analysis,
        program: &Program,
        result: &str,
        publication: &str,
        index: usize,
        packed: Option<&[String]>,
    ) -> syn::Result<TokenStream> {
        let Some(layouts) = analysis.structures.built.get(result) else {
            return Ok(TokenStream::new());
        };
        let rows = format_ident!("__cq_store_rows_{index}");
        let mut output = TokenStream::new();
        if packed.is_none() {
            output.extend(quote! { let #rows = ::std::sync::Arc::new(#rows); });
        }
        for (levels, _) in layouts {
            let keys: Vec<_> = levels.iter().flatten().cloned().collect();
            let mut payload = cq_print::Context::new(&program.structures, &analysis.structures)
                .built_payload(result, levels);
            payload.retain(|column| keys.first() != Some(column));
            payload.extend(keys.iter().skip(1).cloned());
            payload.sort();
            payload.dedup();
            let length = format_ident!("__cq_store_length_{index}");
            let rows = format_ident!("__cq_store_rows_{index}");
            let canonical = format_ident!("__cq_canonical_{index}");
            let fields = format_ident!("__cq_fields_{index}");
            let width = self.width;
            let mut sorted_keys = keys.clone();
            sorted_keys.sort();
            let provider_name = if layouts.len() == 1 {
                result.to_owned()
            } else {
                format!("{result}__{}", sorted_keys.join("__"))
            };
            let descriptor = format_ident!("__cq_sequence_descriptor_{provider_name}");
            let owner = format_ident!("__cq_sequence_provider_{provider_name}");
            let view = format_ident!("__cq_sequence_provider_view_{provider_name}");
            let column = |column: &str| -> syn::Result<TokenStream> {
                let bounds = self.bounds(column)?;
                Ok(if let Some(packed) = packed {
                    let field = packed
                        .iter()
                        .position(|name| name == column)
                        .ok_or_else(|| {
                            error("T-Gpu: provider column absent from stored packed row")
                        })?;
                    quote! {
                        unsafe { ::cq_gpu_sequence_runtime::ColumnDescriptor::from_packed(
                            #canonical.keys().clone(), #length, #canonical.lanes(), #fields[#field], #bounds)? }
                    }
                } else {
                    let slot = self.columns[column];
                    quote! {
                        unsafe { ::cq_gpu_sequence_runtime::ColumnDescriptor::from_rows(
                            #rows.clone(), #length, #width, #slot, #bounds)? }
                    }
                })
            };
            let mut key_descriptors = Vec::new();
            for key in &keys {
                let source = column(key)?;
                let bounds = self.bounds(key)?;
                key_descriptors.push(quote! {
                ::cq_gpu_sequence_runtime::KeyDescriptor {
                    column: #source,
                    domain_size: match #bounds {
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
            let mut payload_descriptors = Vec::new();
            for name in &payload {
                let source = column(name)?;
                let kind = payload_kind(&analysis.structures.physical.rows[result].types[name])?;
                payload_descriptors.push(quote! {
                    ::cq_gpu_sequence_runtime::PayloadDescriptor { column: #source, kind: #kind }
                });
            }
            self.parameters.push((
                view.clone(),
                syn::parse_quote!(::cq_gpu_sequence_runtime::RangeView),
                quote!(unsafe { #owner.view() }),
            ));
            for name in [result, publication] {
                let name = if layouts.len() == 1 {
                    name.to_owned()
                } else {
                    format!("{name}__{}", sorted_keys.join("__"))
                };
                self.providers.insert(
                    name,
                    Provider {
                        view: view.clone(),
                        payload: payload.clone(),
                        keys: keys.clone(),
                    },
                );
            }
            output.extend(quote! {
                let #descriptor = ::cq_gpu_sequence_runtime::RangeDescriptor {
                    key: vec![#(#key_descriptors),*],
                    selection: None,
                    payload: vec![#(#payload_descriptors),*],
                };
                let #owner = __cq_range_builder.build(&#descriptor)?;
            });
        }
        Ok(output)
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
                    let input = symbol_name(&select.input);
                    let tokens = cq_print::expressions::relational_condition_with(
                        &select.condition,
                        &BTreeMap::new(),
                    );
                    struct Uses(BTreeSet<String>);
                    impl VisitMut for Uses {
                        fn visit_expr_path_mut(&mut self, path: &mut syn::ExprPath) {
                            if let Some(id) = path.path.get_ident() {
                                self.0.insert(id.to_string());
                            }
                            syn::visit_mut::visit_expr_path_mut(self, path);
                        }
                    }
                    let mut uses = Uses(BTreeSet::new());
                    uses.visit_expr_mut(&mut syn::parse2(tokens)?);
                    let bindings = self.types[&input]
                        .iter()
                        .filter(|(column, _)| uses.0.contains(*column))
                        .map(|(column, ty)| {
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
                Operator::Bind(bind) => {
                    let input = symbol_name(&bind.input);
                    let mut expression = cq_syntax::arithmetic::binding(bind, &self.types[&input])
                        .ok_or_else(|| {
                            error("T-Gpu: opaque Rust binding requires host evaluation")
                        })?;
                    DeviceComparisons.visit_expr_mut(&mut expression);
                    let dependencies = cq_syntax::rust::free_variables(&bind.expression)
                        .dependencies(&self.types[&input].keys().cloned().collect());
                    let bindings = dependencies.iter().map(|column| {
                        let id = Ident::new(column, proc_macro2::Span::call_site());
                        let ty = &self.types[&input][column];
                        let slot = self.columns[column];
                        quote!(let #id = &(#row[#slot] as #ty);)
                    });
                    let names = cq_syntax::rust::bindings(&bind.pattern);
                    let slot = self.columns[&symbol_name(&names[0])];
                    output.extend(quote! {
                        if #live { #(#bindings)* #row[#slot] = (#expression) as i64; }
                    });
                }
                Operator::Guard(guard) => {
                    let input = symbol_name(&guard.input);
                    let mut expression =
                        cq_syntax::arithmetic::predicate(&guard.condition, &self.types[&input])
                            .ok_or_else(|| {
                                error("T-Gpu: opaque Rust guard requires host evaluation")
                            })?;
                    DeviceComparisons.visit_expr_mut(&mut expression);
                    let dependencies = cq_syntax::rust::free_variables(&guard.condition)
                        .dependencies(&self.types[&input].keys().cloned().collect());
                    let bindings = dependencies.iter().map(|column| {
                        let id = Ident::new(column, proc_macro2::Span::call_site());
                        let ty = &self.types[&input][column];
                        let slot = self.columns[column];
                        quote!(let #id = &(#row[#slot] as #ty);)
                    });
                    output.extend(quote! { if #live { #(#bindings)* #live = #expression; } });
                }
                Operator::Group(_) => {
                    return Err(syn::Error::new_spanned(
                        definition,
                        "T-Gpu: group requires an explicit Sequence grouped publication, not a row map",
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

/// Realize a host subgraph with the shared Rust row printer before uploading a
/// device-representable boundary. String guards execute before any upload.
fn host_rows(
    program: &Program,
    analysis: &Analysis,
    name: &str,
    definitions: &BTreeMap<String, Definition>,
    resident_hosts: &BTreeSet<String>,
    bindings: &BTreeMap<String, Ident>,
    columnar: bool,
) -> syn::Result<TokenStream> {
    let dependencies = host_dependencies_from(name, definitions, bindings);
    for result in &dependencies {
        if bindings.contains_key(result) {
            continue;
        }
        if let Operator::Probe(_, keyed)
        | Operator::Merge(_, keyed)
        | Operator::Semi(_, keyed)
        | Operator::Anti(_, keyed) = &definitions[result].operator
        {
            let provider = symbol_name(&keyed.right);
            let keys = keyed.columns.iter().map(symbol_name).collect::<Vec<_>>();
            if analysis.structures.built_on(&provider, &keys).is_some_and(|(ty, _)| {
                matches!(ty, Type::Path(path)
                    if path.path.segments.last().is_some_and(|segment| segment.ident == "RangeProvider"))
            }) {
                return Err(error(format!(
                    "T-Gpu: host publication {name} reads device-only provider {provider}; record a device grade or a host-capable provider"
                )));
            }
        }
    }
    let mut host_structures = program.structures.clone();
    for entry in &mut host_structures.structures.entries {
        if analysis
            .structures
            .physical
            .hosts
            .get(&symbol_name(&entry.name))
            .is_some_and(|host| resident_hosts.contains(host))
            && let cq_lang_structures::Structure::Host { reader, keyed, .. } = &mut entry.structure
            && reader == "scan"
        {
            // Downloaded rows use the same scan adapter as default Structures;
            // the resident annotation describes only the original GPU view.
            keyed.ty = syn::parse_quote!(::core::iter::Map<_, _>);
        }
    }
    cq_lang_structures::check_rewrite(&program.structures, &host_structures)?;
    let host_analysis = cq_lang_structures::contract::check(&host_structures)?;
    let mut context = cq_print::Context::new(&host_structures, &host_analysis);
    // This host/device boundary publishes its complete checked layout, even
    // when the final answer projects columns away. Seed that demand before
    // RowEmitter prunes carriers through the producer's probes and guards.
    context.demand_columns(&host_structures, name, &analysis.structures.layouts[name])?;
    let needed_hosts: BTreeSet<_> = dependencies
        .iter()
        .filter(|result| !bindings.contains_key(*result))
        .filter_map(|result| host_analysis.physical.hosts.get(result))
        .collect();
    for host in needed_hosts {
        if resident_hosts.contains(host) {
            let value = &context.hosts[host].value;
            context
                .statements
                .push(syn::parse_quote!(let #value = &#value.to_host()?;));
        }
    }
    let mut emitter = cq_physical_realization::RowEmitter::new(context);
    let mut output = None;
    for definition in &host_structures.physical.program.definitions {
        let result = symbol_name(&definition.name);
        if !dependencies.contains(&result) {
            continue;
        }
        emitter.context_mut().register(&result);
        let rename = matches!(definition.operator, Operator::Rename(_));
        let rows = if let Some(binding) = bindings.get(&result) {
            let values = emitter.context().carried(&result).iter().map(|column| {
                let index = host_analysis.layouts[&result]
                    .iter()
                    .position(|stored| stored == column)
                    .expect("carried column belongs to downloaded layout");
                let index = syn::Index::from(index);
                if emitter.context().infos[&result].computed.contains(column) {
                    quote!(::core::clone::Clone::clone(&__cq_bound.#index))
                } else {
                    quote!(&__cq_bound.#index)
                }
            });
            quote!(::core::iter::Iterator::map(#binding.iter(), |__cq_bound| (#(#values,)*)))
        } else if rename {
            emitter
                .context_mut()
                .reprs
                .insert(result.clone(), cq_print::Repr::Host);
            emitter.context().host_scan(&result)
        } else {
            emitter.operator(&result, None)?
        };
        if result == name {
            output = Some(rows);
            break;
        }
        if host_analysis.built.contains_key(&result) || host_analysis.retained.contains_key(&result)
        {
            let repr = emitter.context_mut().materialize(&result, rows);
            emitter.context_mut().reprs.insert(result, repr);
        } else if !rename || bindings.contains_key(&result) {
            emitter
                .context_mut()
                .reprs
                .insert(result, cq_print::Repr::Pipe(Some(rows)));
        }
    }
    let rows = output.ok_or_else(|| error("T-Gpu: missing host boundary"))?;
    let context = emitter.context();
    let pattern = context.pattern(name);
    let values: Vec<_> = analysis.structures.layouts[name]
        .iter()
        .map(|column| context.key_field(name, column, true))
        .collect();
    let statements = &context.statements;
    if columnar {
        // Numeric host boundaries already have their final lane types. Emit
        // columns directly, without a tuple copy followed by dynamic row
        // encoding and a second transpose through a scratch value vector.
        let columns: Vec<_> = (0..values.len())
            .map(|index| format_ident!("__cq_host_column_{index}"))
            .collect();
        if let [value] = values.as_slice() {
            return Ok(quote!(|__cq_columns: &mut [::std::vec::Vec<i64>]| {
                #(#statements)*
                __cq_columns[0].extend(
                    ::core::iter::Iterator::map(#rows, |#pattern| #value as i64));
                Ok(())
            }));
        }
        return Ok(quote!(|__cq_columns: &mut [::std::vec::Vec<i64>]| {
            #(#statements)*
            let rows = #rows;
            let capacity = ::core::iter::Iterator::size_hint(&rows).0;
            let [#(#columns),*] = __cq_columns else { unreachable!() };
            #(#columns.reserve(capacity);)*
            for #pattern in rows {
                #(#columns.push(#values as i64);)*
            }
            Ok(())
        }));
    }
    Ok(quote!({
        #(#statements)*
        ::core::iter::Iterator::map(#rows, |#pattern| (#(#values,)*))
            .collect::<::std::vec::Vec<_>>()
    }))
}

fn host_dependencies(name: &str, definitions: &BTreeMap<String, Definition>) -> BTreeSet<String> {
    host_dependencies_from(name, definitions, &BTreeMap::new())
}

fn host_dependencies_from(
    name: &str,
    definitions: &BTreeMap<String, Definition>,
    bindings: &BTreeMap<String, Ident>,
) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    let mut pending = vec![name.to_owned()];
    while let Some(name) = pending.pop() {
        if !result.insert(name.clone()) {
            continue;
        }
        if bindings.contains_key(&name) {
            continue;
        }
        pending.extend(
            definitions[&name]
                .operator
                .operands()
                .into_iter()
                .map(symbol_name),
        );
    }
    result
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
                            syn::visit_mut::visit_expr_mut(self, expression);
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
                    syn::visit_mut::visit_expr_mut(self, expression);
                    return;
                }
            }
        }
        syn::visit_mut::visit_expr_mut(self, expression);
    }
}

fn payload_kind(ty: &Type) -> syn::Result<TokenStream> {
    match primitive(ty).as_deref() {
        Some("u32" | "u16" | "u8" | "bool") => {
            Ok(quote!(::cq_gpu_sequence_runtime::PayloadKind::U32))
        }
        Some("i32" | "i16" | "i8") => Ok(quote!(::cq_gpu_sequence_runtime::PayloadKind::I32)),
        Some("i64") => Ok(quote!(::cq_gpu_sequence_runtime::PayloadKind::I64)),
        _ => Err(syn::Error::new_spanned(
            ty,
            "T-Gpu: range payload needs a recorded u32/i32/i64 code",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_conditionals_use_native_comparisons_at_every_depth() {
        let types = [("x".into(), syn::parse_quote!(i64))].into();
        let mut expression = cq_syntax::arithmetic::predicate(
            &syn::parse_quote!((if *x > 0 { *x * 2 } else { *x / -1 }) == 3),
            &types,
        )
        .unwrap();
        DeviceComparisons.visit_expr_mut(&mut expression);
        let tokens = quote!(#expression).to_string();
        assert!(!tokens.contains("cq_storage"), "{tokens}");
        assert!(tokens.contains("wrapping_mul"), "{tokens}");
        assert!(tokens.contains("wrapping_div"), "{tokens}");
    }

    #[test]
    fn downloaded_probe_carriers_preserve_the_host_publication_demand() {
        // D72 shape: eight columns cross a device publication, a host probe
        // publishes all eight, but the final answer keeps only three.
        let program: Program = syn::parse_quote! {
            physical {
                relation Driver(available:i64,customer_key:i64,customer_nation:i64,
                    order_date:i64,order_key:i64,part_key:i64,supplier_key:i64,supplier_nation:i64) = (&data.driver);
                relation Nation(supplier_nation:i64) = (&data.nation);
                driver = pipe rename Driver@0 {available -> available,customer_key -> customer_key,
                    customer_nation -> customer_nation,order_date -> order_date,order_key -> order_key,
                    part_key -> part_key,supplier_key -> supplier_key,supplier_nation -> supplier_nation};
                nation = let rename Nation@1 {supplier_nation -> supplier_nation};
                filtered = let select driver if (order_key > 0);
                joined = pipe probe first [supplier_nation] filtered,nation;
                result = pipe canon project joined keep {order_key,part_key,supplier_nation};
                return result as answer(order_key,part_key,supplier_nation);
            }
            structures {
                driver host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64,i64,i64,i64,i64,i64,i64)> [];
                nation host scan = ::core::iter::Map<_,_> [];
                nation = ::cq_storage::SortedRun<_,_> [supplier_nation];
                filtered rows = ::std::vec::Vec<_>;
                result canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                input = store rows driver;
                index = store rows nation;
                feed = compact filtered flatten tile 256 input;
                device_rows = store feed;
                host_rows = store flatten lookup nation [supplier_nation] device_rows;
                projected = canon map result host_rows;
                output_rows = store projected;
                output count;
            }
            graded gpu {
                input at host;
                index at host;
                feed at host.device.block;
                device_rows at host;
                host_rows at host;
                projected at host;
                output_rows at host.device;
            }
        };
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let definitions = program
            .structures
            .physical
            .program
            .definitions
            .iter()
            .map(|definition| (symbol_name(&definition.name), definition.clone()))
            .collect();
        let bindings = BTreeMap::from([("filtered".to_owned(), syn::parse_quote!(downloaded))]);
        let resident_hosts = BTreeSet::from(["Driver".to_owned()]);
        let publication = host_rows(
            &program,
            &analysis,
            "joined",
            &definitions,
            &resident_hosts,
            &bindings,
            false,
        )
        .unwrap();
        syn::parse2::<syn::Expr>(publication.clone()).unwrap();
        let publication = publication.to_string();
        let full_pattern = quote!(|(available,customer_key,customer_nation,order_date,
            order_key,part_key,supplier_key,supplier_nation,)|)
        .to_string();
        assert!(publication.contains(&full_pattern), "{publication}");
        let full_download = quote!(|__cq_bound| (
            &__cq_bound.0,
            &__cq_bound.1,
            &__cq_bound.2,
            &__cq_bound.3,
            &__cq_bound.4,
            &__cq_bound.5,
            &__cq_bound.6,
            &__cq_bound.7,
        ))
        .to_string();
        assert!(publication.contains(&full_download), "{publication}");

        // A narrower boundary must select the carried slots from the same
        // stored layout, not hand an eight-field tuple to a three-field probe.
        let projected = host_rows(
            &program,
            &analysis,
            "result",
            &definitions,
            &resident_hosts,
            &bindings,
            false,
        )
        .unwrap()
        .to_string();
        let projected_download =
            quote!(|__cq_bound| (&__cq_bound.4, &__cq_bound.5, &__cq_bound.7,)).to_string();
        assert!(projected.contains(&projected_download), "{projected}");
    }
}
