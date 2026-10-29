use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::{Analysis, Program};
use cq_lang_structures::physical::{Definition, Operator, Row, symbol_name};
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
pub(crate) struct Interval {
    column: String,
    start: TokenStream,
    end: TokenStream,
}
#[derive(Clone)]
pub(crate) struct Origin {
    pub(crate) host: String,
    pub(crate) columns: BTreeMap<String, String>,
    /// A contiguous key interval folded into the source read.
    pub(crate) selection: Option<Interval>,
    /// A relational selection on the chain that is not a contiguous interval;
    /// it executes as a row predicate, so the source is not the raw host.
    pub(crate) predicate: bool,
}
struct Source {
    reads: Vec<(usize, TokenStream)>,
    length: TokenStream,
    positions: Option<Ident>,
}
struct RunSource {
    view: Ident,
    reads: Vec<(usize, usize)>,
    length: TokenStream,
}
struct Provider {
    view: Ident,
    payload: Vec<String>,
    keys: Vec<String>,
}

#[derive(Clone, Debug)]
enum DirectoryPolicy {
    /// Zero-based nonnegative u32 codes; one terminal lower-bound entry.
    DenseU32 { terminal_entries: usize },
}

impl DirectoryPolicy {
    fn domain(&self, bounds: TokenStream) -> TokenStream {
        match self {
            Self::DenseU32 { .. } => quote! {
                match #bounds {
                    ::cq_gpu_runtime::Bounds::Empty => 0usize,
                    ::cq_gpu_runtime::Bounds::Inclusive { min, max } => {
                        if min < 0 || max > u32::MAX as i64 {
                            return Err("T-Gpu: range keys require dense nonnegative u32 codes".into());
                        }
                        usize::try_from(max)?.checked_add(1).ok_or("T-Gpu: key domain overflow")?
                    }
                }
            },
        }
    }

    fn allocation(&self) -> TokenStream {
        let Self::DenseU32 { terminal_entries } = self;
        quote!(__cq_provider_keys.first().map_or(Ok(0usize), |key|
            key.domain_size.checked_add(#terminal_entries).ok_or("T-Gpu: directory allocation overflow"))?)
    }
}

/// Printable checked realization shared by resident and computed providers.
#[derive(Clone, Debug)]
struct ProviderLayout {
    keys: Vec<String>,
    payload: Vec<String>,
    directory: DirectoryPolicy,
}

impl ProviderLayout {
    /// Formal rule GPU-ProviderLayout: derive key reconstruction, payload and dense directory allocation once before translating either provider path.
    fn check(program: &Program, analysis: &Analysis, scanned: &BTreeSet<String>) -> BTreeMap<String, Vec<Self>> {
        let context = cq_print::Context::new(&program.structures, &analysis.structures);
        analysis.structures.built.iter().map(|(name, layouts)| {
            let layouts = layouts.iter().map(|(levels, _)| {
                let keys: Vec<_> = levels.iter().flatten().cloned().collect();
                let payload = if scanned.contains(name) {
                    analysis.structures.layouts[name].clone()
                } else {
                    let mut payload = context.built_payload(name, levels);
                    payload.retain(|column| keys.first() != Some(column));
                    payload.extend(keys.iter().skip(1).cloned());
                    payload.sort();
                    payload.dedup();
                    payload
                };
                Self { keys, payload, directory: DirectoryPolicy::DenseU32 { terminal_entries: 1 } }
            }).collect();
            (name.clone(), layouts)
        }).collect()
    }
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
/// Scalar locals reuse checked column identifiers, including their macro hygiene;
/// rebuilding them from column-name strings disconnects generated CQ variables.
pub struct Rows {
    pub width: usize,
    pub columns: BTreeMap<String, usize>,
    pub setup: TokenStream,
    pub build: TokenStream,
    /// Optional support objects as `(identifier, before setup, after setup)`.
    /// Each is emitted only when generated code outside its own declaration
    /// references the identifier; unused adapters would load device modules.
    pub support: Vec<(Ident, TokenStream, TokenStream)>,
    pub parameters: Vec<(Ident, Type, TokenStream)>,
    pub rust_functions: Vec<(Ident, TokenStream, Ident)>,
    pub output_columns: Vec<String>,
    pub dense: BTreeSet<usize>,
    sources: BTreeMap<String, Source>,
    providers: BTreeMap<String, Provider>,
    bounds: BTreeMap<String, TokenStream>,
    /// Run layouts have scalar type ranges, not observed column bounds.
    unmeasured_bounds: BTreeSet<String>,
    definitions: BTreeMap<String, Definition>,
    physical_rows: BTreeMap<String, Row>,
    resident_hosts: BTreeSet<String>,
    run_sources: BTreeMap<String, RunSource>,
    run_providers: BTreeSet<String>,
    expressions: BTreeMap<String, syn::Expr>,
    capture_count: usize,
    disabled_captures: BTreeMap<usize, TokenStream>,
    capture_uses: BTreeMap<String, BTreeSet<usize>>,
    provider_layouts: BTreeMap<String, Vec<ProviderLayout>>,
}

impl Rows {
    /// Consume recorded expression dependencies and explicit launch domains.
    /// Parameter indices refer to this argument vector, never a launch ABI.
    pub fn capture_arguments(&self, domains: &[(Vec<String>, TokenStream)]) -> Vec<TokenStream> {
        self.parameters.iter().enumerate().map(|(index, (_, _, value))| {
            if index >= self.capture_count { return value.clone(); }
            let conditions = domains.iter().filter_map(|(definitions, condition)| {
                definitions.iter().any(|name|
                    self.capture_uses.get(name).is_some_and(|uses| uses.contains(&index)))
                    .then_some(condition)
            });
            let disabled = self.disabled_captures.get(&index).cloned()
                .unwrap_or_else(|| quote!(::core::default::Default::default()));
            quote!(if false #(|| (#conditions))* { #value } else { #disabled })
        }).collect()
    }

    pub fn new(program: &Program, analysis: &Analysis) -> syn::Result<Self> {
        let physical = &program.structures.physical;
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
        let provider_layouts = ProviderLayout::check(program, analysis, &scanned);
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
        let codecs = crate::runs::Codecs::check(&program.structures)?;
        let mut captures = crate::capture::Captures::default();
        let mut expressions = BTreeMap::new();
        let mut capture_uses = BTreeMap::new();
        // DESIGN contract7-gpu-rows
        // Consumes checked identity/encoded read facts before making device Rust callables.
        // Contract 7, BE-RustRepresentation/T-Gpu; source: doc/PLANNING-REDESIGN.md Contract 7.
        // Source: doc/formal/CONFORMANCE.md BE-RustRepresentation and GPU-Capture.
        // DO NOT admit encoded row reads to opaque Rust or infer identity from a scalar Rust type.
        // Encoded predicates belong to `where`; unsupported Rust placement stays at host, not rewritten.
        for (name, definition) in &definitions {
            captures.used.clear();
            let expression = match &definition.operator {
                Operator::Guard(guard) if !analysis.rust_requires_host(&definition.operator) => {
                    let row = &analysis.structures.physical.rows[&symbol_name(&guard.input)];
                    captures.rust_expression(&guard.condition, &row.types, &row.bindings, &syn::parse_quote!(bool), &codecs)
                }
                Operator::Bind(bind) if !analysis.rust_requires_host(&definition.operator) => {
                    let row = &analysis.structures.physical.rows[&symbol_name(&bind.input)];
                    let names = cq_syntax::rust::bindings(&bind.pattern);
                    let output = &analysis.structures.physical.rows[name].types[&symbol_name(&names[0])];
                    captures.rust_expression(&bind.expression, &row.types, &row.bindings, output, &codecs)
                }
                Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                    let types = &analysis.structures.physical.rows[&symbol_name(&select.input)].types;
                    captures.relational(&select.condition, types, &codecs)?
                }
                _ => continue,
            };
            capture_uses.insert(name.clone(), captures.used.clone());
            expressions.insert(name.clone(), expression);
        }
        let mut host_bound_definitions = definitions.clone();
        for definition in host_bound_definitions.values_mut() {
            if let Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) = &mut definition.operator {
                let types = &analysis.structures.physical.rows[&symbol_name(&select.input)].types;
                captures.host_relational(&mut select.condition, types, &codecs);
            }
        }
        let mut physical_rows = analysis.structures.physical.rows.clone();
        for row in physical_rows.values_mut() {
            for ty in row.types.values_mut() {
                if codecs.get(ty).is_some() { *ty = syn::parse_quote!(u32); }
            }
        }
        let capture_count = captures.parameters.len();
        let rust_functions = captures.rust_functions;
        let disabled_captures = captures.disabled;
        let mut setup = captures.setup;
        let mut build = TokenStream::new();
        let mut parameters = captures.parameters;
        let mut hosts = BTreeMap::new();
        let mut raw = BTreeMap::new();
        let mut first = true;
        let mut run_hosts = BTreeMap::new();
        setup.extend(quote! {
            let mut __cq_run_owners: ::std::vec::Vec<::std::sync::Arc<dyn ::std::any::Any>> = ::std::vec::Vec::new();
        });
        for input in physical.inputs() {
            let name = symbol_name(&input.name);
            let mut run_binding = None;
            let mut resident = false;
            for entry in &program.structures.structures.entries {
                if analysis.structures.physical.hosts.get(&symbol_name(&entry.name)) != Some(&name) {
                    continue;
                }
                if let cq_lang_structures::Structure::Host { reader, keyed, .. } = &entry.structure {
                    if !matches!(reader, cq_lang_structures::ReaderRef::Scan) { continue; }
                    if let Some((types, keys)) = crate::runs::layout_types(&keyed.ty)? {
                        run_binding = Some((types, keys));
                        resident = true;
                    } else if crate::native_abi(&keyed.ty) == Some(crate::NativeAbi::Columns) {
                        resident = true;
                    }
                }
            }
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
                });
                first = false;
            }
            if let Some((types, keys)) = run_binding {
                let schema: Vec<_> = types.iter().map(crate::runs::scalar).collect();
                let view = format_ident!("__cq_sequence_runs_{name}");
                setup.extend(quote! {
                    if !::std::sync::Arc::ptr_eq(&__cq_pool, #id.pool()) {
                        return Err("T-Gpu: resident inputs must share a pool and stream".into());
                    }
                    #id.validate(&[#(#schema),*])?;
                    let #length = #id.len();
                    if ::gpu_datalog_runtime::trace_kernels() {
                        ::std::eprintln!("gpu_sequence_input\tname={}\tdescriptors={:p}\trows={}\truns={}",
                            #name, #id.view().runs, #length, #id.view().run_count);
                    }
                    __cq_run_owners.push(#id.owner());
                });
                parameters.push((view.clone(), syn::parse_quote!(::gpu_datalog_runtime::RunsView), quote!(#id.view())));
                run_hosts.insert(name.clone(), (view, types, keys));
                hosts.insert(name, (id, length));
                continue;
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
            setup.extend(quote!(let __cq_pool = ::gpu_datalog_runtime::thread_pool()?;));
        }
        let output_width = output_columns.len();
        let support = vec![
            (
                format_ident!("__cq_dictionary"),
                quote!(let mut __cq_dictionary = ::cq_gpu_runtime::Dictionary::default();),
                quote!(let __cq_dictionary = ::std::sync::Arc::new(__cq_dictionary);),
            ),
            (
                format_ident!("__cq_statuses"),
                quote!(let mut __cq_statuses = ::std::vec::Vec::<
                    ::std::sync::Arc<::cq_gpu_runtime::Buffer<::cq_gpu_sequence_runtime::OutputCount>>
                >::new();),
                TokenStream::new(),
            ),
            (
                format_ident!("__cq_range_builder"),
                TokenStream::new(),
                quote!(let __cq_range_builder = ::cq_gpu_sequence_runtime::RangeBuilder::new(&__cq_pool)?;),
            ),
            (
                format_ident!("__cq_output_adapter"),
                TokenStream::new(),
                quote! {
                    ::std::thread_local! {
                        static __CQ_OUTPUT_ADAPTER: ::std::cell::RefCell<Option<
                            ::std::rc::Rc<::std::cell::RefCell<::cq_gpu_sequence_runtime::canon::PackedColumnsAdapter>>
                        >> = const { ::std::cell::RefCell::new(None) };
                    }
                    ::cq_gpu_sequence_runtime::register_query_cleanup(|| {
                        __CQ_OUTPUT_ADAPTER.with(|cache| cache.borrow_mut().take());
                    });
                    let __cq_output_adapter = __CQ_OUTPUT_ADAPTER.with(|cache| -> ::cq_gpu_sequence_runtime::Result<_> {
                        let mut cache = cache.borrow_mut();
                        if cache.as_ref().is_none_or(|adapter|
                            !::std::sync::Arc::ptr_eq(adapter.borrow().pool(), &__cq_pool)) {
                            *cache = Some(::std::rc::Rc::new(::std::cell::RefCell::new(
                                ::cq_gpu_sequence_runtime::canon::PackedColumnsAdapter::new(__cq_pool.clone(), #output_width)?)));
                        }
                        Ok(cache.as_ref().unwrap().clone())
                    })?;
                },
            ),
        ];
        let mut sources = BTreeMap::new();
        let mut providers = BTreeMap::new();
        let mut run_sources = BTreeMap::new();
        let mut run_providers = BTreeSet::new();
        let lookup_demands = lookup_demands(program);
        let mut bound_sources: BTreeMap<String, BTreeSet<(String, usize)>> = BTreeMap::new();
        // Only checked device consumers require resident source adapters.
        // A host store can retain arbitrary Rust values (for example nullable
        // aggregates); it is not itself a host-to-device publication.
        let required_sources = device_source_results(analysis);
        let resident_hosts: BTreeSet<_> = hosts.keys().cloned().collect();
        for (name, definition) in &definitions {
            if !matches!(definition.operator, Operator::Rename(_))
                && (device_results.contains(name) || !required_sources.contains(name))
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
                if !required_sources.contains(name) {
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
                    .map(|column| &physical_rows[name].types[column]);
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
                    predicate: false,
                }
            } else {
                let Some(origin) = origin(name, &host_bound_definitions)? else {
                    continue;
                };
                // A predicate-filtered result is computed from its run, not
                // read as one: its provider is built from the compacted rows.
                if origin.predicate {
                    continue;
                }
                origin
            };
            let (host, host_length) = &hosts[&origin.host];
            if let Some((view, types, _key_count)) = run_hosts.get(&origin.host) {
                if origin.selection.is_some() {
                    // Packed runs have no interval-view ABI. The recorded
                    // Select remains row work over their unfiltered input.
                    continue;
                }
                let layout = &analysis.structures.host_layouts[&origin.host];
                let payload = layout.iter().map(|source| {
                    origin.columns.iter().find(|(_, original)| *original == source)
                        .map(|(column, _)| column.clone())
                        .ok_or_else(|| error("GPU-Runs: supplied layout lost a packed column"))
                }).collect::<syn::Result<Vec<_>>>()?;
                let reads = payload.iter().enumerate().map(|(index,column)| {
                    let slot = columns[column];
                    (slot,index)
                }).collect();
                run_sources.insert(name.clone(), RunSource {
                    view: view.clone(), reads, length: quote!(#host_length),
                });
                let mut native_provider = |keys: &[String], provider_name: &str| -> syn::Result<Provider> {
                    if keys.len() > payload.len() || keys.iter().any(|key| !payload.contains(key)) {
                        return Err(error("GPU-Runs: native lookup key is absent from the packed layout"));
                    }
                    let leading = payload[..keys.len()].iter().all(|key| keys.contains(key));
                    let (provider_view, provider_payload) = if leading {
                        (view.clone(), payload.clone())
                    } else {
                        // Realize the recorded lookup keys by a sorted
                        // provider permutation, not a new join plan.
                        let reordered: Vec<_> = keys.iter().cloned()
                            .chain(payload.iter().filter(|column| !keys.contains(column)).cloned())
                            .collect();
                        let order: Vec<_> = reordered.iter().map(|column|
                            payload.iter().position(|source| source == column).expect("checked provider column")).collect();
                        let owner = format_ident!("__cq_native_provider_{provider_name}");
                        let provider_view = format_ident!("__cq_native_view_{provider_name}");
                        build.extend(quote! {
                            let #owner = #host.permuted_provider(&[#(#order),*])?;
                            __cq_run_owners.push(#owner.owner());
                        });
                        parameters.push((provider_view.clone(), syn::parse_quote!(::gpu_datalog_runtime::RunsView), quote!(#owner.view())));
                        (provider_view, reordered)
                    };
                    Ok(Provider {
                        view: provider_view,
                        keys: provider_payload[..keys.len()].to_vec(),
                        payload: provider_payload,
                    })
                };
                if let Some(layouts) = analysis.structures.built.get(name) {
                    if layouts.len() != 1 {
                        return Err(error("GPU-Runs: name each supplied provider layout separately"));
                    }
                    let (levels, ty) = &layouts[0];
                    if crate::runs::layout_types(ty)?.is_none() {
                        // The run supplies rows; the recorded nonnative index
                        // is built only after its device store publishes them.
                        continue;
                    }
                    let keys: Vec<_> = levels.iter().flatten().cloned().collect();
                    providers.insert(name.clone(), native_provider(&keys, name)?);
                    run_providers.insert(name.clone());
                } else {
                    // Only Sequence Lookup consumes a native keyed provider.
                    // Select maps may record nonleading host-reader fallbacks,
                    // but execute their predicate over the run's row scan.
                    for ((result, reader, keys), ty) in &analysis.structures.fallbacks {
                        if result != name || matches!(reader, cq_lang_structures::ReaderRef::Scan) { continue; }
                        let mut lookup_keys = keys.clone();
                        lookup_keys.sort();
                        if !lookup_demands.contains(&(result.clone(), lookup_keys.clone())) {
                            continue;
                        }
                        if crate::runs::layout_types(ty)?.is_none() {
                            return Err(error("GPU-Runs: native lookup requires a declared run layout"));
                        }
                        let provider_name = format!("{name}__{}", lookup_keys.join("__"));
                        if providers.contains_key(&provider_name) { continue; }
                        providers.insert(provider_name.clone(), native_provider(keys, &provider_name)?);
                        run_providers.insert(provider_name);
                    }
                }
                let _ = types;
                continue;
            }
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
            if built.is_some() {
                if analysis.structures.built[name].len() != 1 {
                    return Err(syn::Error::new_spanned(
                        definition,
                        "T-Gpu: source has multiple recorded provider layouts; name each provider store separately",
                    ));
                }
                let layout = &provider_layouts[name][0];
                let keys = layout.keys.clone();
                let mut key_descriptors = Vec::new();
                for key in &keys {
                    let source = origin.columns.get(key).ok_or_else(|| {
                        syn::Error::new_spanned(
                            definition,
                            "T-Gpu: provider key has no resident origin",
                        )
                    })?;
                    let index = fields[source].0;
                    let domain_size = layout.directory.domain(quote!(#host.storage().bounds[#index]));
                    key_descriptors.push(quote! {
                        ::cq_gpu_sequence_runtime::KeyDescriptor {
                            column: unsafe { ::cq_gpu_sequence_runtime::ColumnDescriptor::new(
                                #host.storage().columns[#index].clone(), #host_length)? },
                            domain_size: #domain_size,
                        }
                    });
                }
                let payload = layout.payload.clone();
                let mut payload_descriptors = Vec::new();
                for column in &payload {
                    let source = &origin.columns[column];
                    let index = fields[source].0;
                    let ty = &physical_rows[name].types[column];
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
                let directory_entries = layout.directory.allocation();
                preparation.extend(quote! {
                    let __cq_provider_keys: Vec<::cq_gpu_sequence_runtime::KeyDescriptor> = vec![#(#key_descriptors),*];
                    let #descriptor = ::cq_gpu_sequence_runtime::RangeDescriptor {
                        directory_entries: #directory_entries,
                        key: __cq_provider_keys, selection: #selection,
                        payload: vec![#(#payload_descriptors),*],
                    };
                    ::std::thread_local! {
                        static #cache: ::std::cell::RefCell<Option<::std::rc::Rc<::std::cell::RefCell<
                            ::cq_gpu_sequence_runtime::RangeWorkspace
                        >>>> = const { ::std::cell::RefCell::new(None) };
                    }
                    ::cq_gpu_sequence_runtime::register_query_cleanup(|| {
                        #cache.with(|cache| cache.borrow_mut().take());
                    });
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
        let mut unmeasured_bounds = BTreeSet::new();
        for (name, source) in &run_sources {
            for &(slot, _) in &source.reads {
                let column = columns.iter().find(|(_, index)| **index == slot).unwrap().0;
                let ty = &physical_rows[name].types[column];
                unmeasured_bounds.insert(column.clone());
                bounds.insert(column.clone(), quote!(::cq_gpu_runtime::Bounds::Inclusive {
                    min: <#ty as ::gpu_datalog_runtime::Scalar>::MIN as i64,
                    max: <#ty as ::gpu_datalog_runtime::Scalar>::MAX as i64,
                }));
            }
        }
        for (column, origins) in bound_sources {
            unmeasured_bounds.remove(&column);
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
        // Native row publication obtains bounds from Scalar metadata. Rustc
        // checks the trait only when a publication consumes these bounds.
        for row in physical_rows.values() {
            for column in &row.computed {
                let ty = &row.types[column];
                bounds.entry(column.clone()).or_insert_with(|| {
                    quote! {
                        ::cq_gpu_runtime::Bounds::Inclusive {
                            min: <#ty as ::gpu_datalog_runtime::Scalar>::MIN as i64,
                            max: <#ty as ::gpu_datalog_runtime::Scalar>::MAX as i64,
                        }
                    }
                });
            }
        }
        build.extend(quote! {
            let __cq_timing_built = if __cq_timing_enabled {
                Some(__cq_pool.stream().record_event(
                    Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?)
            } else { None };
        });
        // Parent carriers are the recorded i64 row ABI, regardless of the
        // logical Rust type or the source column's compressed storage width.
        let dense = BTreeSet::new();
        Ok(Self {
            width: columns.len().max(1),
            columns,
            setup,
            build,
            support,
            parameters,
            rust_functions,
            output_columns,
            dense,
            sources,
            providers,
            bounds,
            unmeasured_bounds,
            definitions,
            physical_rows,
            resident_hosts,
            run_sources,
            run_providers,
            expressions,
            capture_count,
            disabled_captures,
            capture_uses,
            provider_layouts,
        })
    }

    pub fn has_source(&self, name: &str) -> bool {
        self.sources.contains_key(name) || self.run_sources.contains_key(name)
    }

    pub fn has_runs(&self) -> bool {
        !self.run_sources.is_empty()
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
        let layout = &analysis.structures.layouts[name];
        let rows = host_rows(
            program,
            analysis,
            name,
            &self.definitions,
            &self.resident_hosts,
            bindings,
            !layout.is_empty(),
        )?;
        let id = format_ident!("__cq_sequence_uploaded_{name}");
        let length = format_ident!("__cq_sequence_uploaded_length_{name}");
        let types = layout.iter().map(|column| &self.physical_rows[name].types[column]);
        let upload = if layout.is_empty() {
            quote!(::cq_gpu_runtime::DeviceColumns::from_rows(
                __cq_pool.clone(), &rows, &mut ::cq_gpu_runtime::Dictionary::default(), &[])?)
        } else {
            quote!(::cq_gpu_sequence_runtime::upload_numeric_columns::<(#(#types,)*), _>(
                __cq_pool.clone(), rows)?)
        };
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
                #upload
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
        if let Some(source) = self.run_sources.get(name) {
            let view = &source.view;
            // DECODE V: P6 run scans unpack every demanded field before relational consumers; retain narrow codes unless rust_reads requires identity.
            let reads = source.reads.iter().map(|(slot,column)| quote! {
                #row[#slot] = unsafe { ::gpu_datalog_runtime::device::decode(
                    #view, __run_index, __run_position, #column as u32) };
            });
            return Ok(quote! {
                let (__run_index, __run_position) = unsafe {
                    ::gpu_datalog_runtime::device::locate(#view, (#position) as u32)
                };
                #(#reads)*
            });
        }
        let source = self.sources.get(name).ok_or_else(|| {
            error(format!(
                "T-Gpu: {name} has no resident source; materialize its Sequence producer"
            ))
        })?;
        let position = match &source.positions {
            Some(positions) => quote!(unsafe { *#positions.add((#position) as usize) } as u64),
            None => quote!((#position) as u64),
        };
        // DECODE V: P6 source scans reconstruct FOR values into i64 even for code-only joins and predicates.
        let reads = source.reads.iter().map(|(slot, view)| {
            let decode = if self.dense.contains(slot) { format_ident!("decode_u32") } else { format_ident!("decode") };
            quote! {
                #row[#slot] = unsafe { ::cq_gpu_sequence_runtime::device::#decode(#view, #position) } as i64;
            }
        });
        Ok(quote!(#(#reads)*))
    }
    pub fn source_len(&self, name: &str) -> syn::Result<TokenStream> {
        if let Some(source) = self.run_sources.get(name) { return Ok(source.length.clone()); }
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
    pub fn run_provider(&self, name: &str) -> bool { self.run_providers.contains(name) }
    pub fn output_types(&self) -> syn::Result<Vec<Type>> {
        self.output_columns.iter().map(|column| {
            self.physical_rows.values().find_map(|row| row.types.get(column)).cloned()
                .ok_or_else(|| error(format!("GPU-Arena: output column {column} has no declared type")))
        }).collect()
    }
    /// Realize the recorded provider only after its device store has published
    /// its live count. Descriptors borrow the stored layout without transposing
    /// or copying it before the ordinary radix/lower-bound range build.
    pub fn publish_provider(
        &mut self,
        analysis: &Analysis,
        _program: &Program,
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
        for layout in self.provider_layouts[result].clone() {
            let keys = layout.keys;
            let payload = layout.payload;
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
                let measure = self.unmeasured_bounds.contains(key).then(|| quote! {
                    column.measure_bounds()?;
                    if column.rows() != 0 { ::gpu_datalog_runtime::record_readback(); }
                });
                let domain_size = layout.directory.domain(quote!(column.bounds()));
                key_descriptors.push(quote! {{
                    let mut column = #source;
                    #measure
                    let domain_size = #domain_size;
                    ::cq_gpu_sequence_runtime::KeyDescriptor { column, domain_size }
                }});
            }
            let mut payload_descriptors = Vec::new();
            for name in &payload {
                let source = column(name)?;
                let kind = payload_kind(&self.physical_rows[result].types[name])?;
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
            let directory_entries = layout.directory.allocation();
            output.extend(quote! {
                let __cq_provider_keys: Vec<::cq_gpu_sequence_runtime::KeyDescriptor> = vec![#(#key_descriptors),*];
                let #descriptor = ::cq_gpu_sequence_runtime::RangeDescriptor {
                    directory_entries: #directory_entries,
                    key: __cq_provider_keys,
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
    fn map_definitions(&self, name: &str, input_result: &str) -> syn::Result<Vec<&Definition>> {
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
        Ok(chain)
    }

    pub fn capture_definitions(&self, name: &str, input_result: &str) -> syn::Result<Vec<String>> {
        Ok(self.map_definitions(name, input_result)?.into_iter()
            .map(|definition| symbol_name(&definition.name)).collect())
    }

    pub fn map(
        &self,
        name: &str,
        input_result: &str,
        row: TokenStream,
        live: TokenStream,
    ) -> syn::Result<TokenStream> {
        let mut output = TokenStream::new();
        for definition in self.map_definitions(name, input_result)?.into_iter().rev() {
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
                    let input_row = &self.physical_rows[&input];
                    let bindings = input_row.types
                        .iter()
                        .filter(|(column, _)| uses.0.contains(*column))
                        .map(|(column, ty)| {
                            let id = &input_row.bindings[column];
                            let slot = self.columns[column];
                            // DECODE V: P6 relational Select/Point/Range reconstructs Scalar values; preserved-op predicates must compare codes.
                            quote!(let #id = &<#ty as ::gpu_datalog_runtime::Scalar>::decode(#row[#slot] as i128);)
                        });
                    let expression = self.expressions.get(&symbol_name(&definition.name))
                        .ok_or_else(|| error("GPU-Capture: selection is not an integer device predicate"))?;
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
                    let input_row = &self.physical_rows[&input];
                    let expression = self.expressions.get(&symbol_name(&definition.name))
                        .ok_or_else(|| error("T-Gpu: opaque Rust binding requires host evaluation"))?;
                    let dependencies = cq_syntax::rust::free_variables(&bind.expression)
                        .dependencies(&input_row.types.keys().cloned().collect());
                    let bindings = dependencies.iter().map(|column| {
                        let id = &input_row.bindings[column];
                        let ty = &input_row.types[column];
                        let slot = self.columns[column];
                        // DECODE R: Bind free-variable dependencies select these columns; checked identity representation permits the verbatim opaque Rust expression.
                        quote!(let #id = &<#ty as ::gpu_datalog_runtime::Scalar>::decode(#row[#slot] as i128);)
                    });
                    let names = cq_syntax::rust::bindings(&bind.pattern);
                    let slot = self.columns[&symbol_name(&names[0])];
                    let ty = &self.physical_rows[&symbol_name(&definition.name)].types[&symbol_name(&names[0])];
                    output.extend(quote! {
                        if #live { #(#bindings)* #row[#slot] = <#ty as ::gpu_datalog_runtime::Scalar>::encode(&(#expression)); }
                    });
                }
                Operator::Guard(guard) => {
                    let input = symbol_name(&guard.input);
                    let input_row = &self.physical_rows[&input];
                    let expression = self.expressions.get(&symbol_name(&definition.name))
                        .ok_or_else(|| error("T-Gpu: opaque Rust guard requires host evaluation"))?;
                    let dependencies = cq_syntax::rust::free_variables(&guard.condition)
                        .dependencies(&input_row.types.keys().cloned().collect());
                    let bindings = dependencies.iter().map(|column| {
                        let id = &input_row.bindings[column];
                        let ty = &input_row.types[column];
                        let slot = self.columns[column];
                        // DECODE R: Guard free-variable dependencies select these columns; checked identity representation permits the verbatim opaque Rust condition.
                        quote!(let #id = &<#ty as ::gpu_datalog_runtime::Scalar>::decode(#row[#slot] as i128);)
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

/// Sources admitted by a dispatch, a native provider, or a recorded scalar
/// host publication. Arbitrary host-only Rust rows do not acquire a device ABI.
pub(crate) fn device_source_results(analysis: &Analysis) -> BTreeSet<String> {
    let mut sources: BTreeSet<_> = analysis.dispatches.iter().flat_map(|dispatch| {
        ::core::iter::once(&dispatch.outer_store).chain(dispatch.inner_stores.iter())
    }).map(|name| analysis.results.get(name).unwrap_or(name).clone()).collect();
    sources.extend(analysis.structures.built.iter().filter_map(|(name, layouts)| {
        layouts.iter().any(|(_, ty)| crate::native_abi(ty) == Some(crate::NativeAbi::Range))
            .then(|| name.clone())
    }));
    sources.extend(analysis.stores.keys().filter_map(|store| {
        let result = analysis.results.get(store)?;
        let host = analysis.levels.get(store)?.last().is_some_and(|level| level == "host");
        let represented = analysis.structures.layouts[result].iter().all(|column| {
            analysis.representation.identity.contains(column)
                || analysis.representation.encoded.contains(column)
        });
        (host && represented).then(|| result.clone())
    }));
    sources
}

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
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
                crate::native_abi(ty) == Some(crate::NativeAbi::Range)
            }) {
                return Err(error(format!(
                    "T-Gpu: host publication {name} reads device-only provider {provider}; record a device grade or a host-capable provider"
                )));
            }
        }
    }
    let mut run_host_types = BTreeMap::new();
    for entry in &program.structures.structures.entries {
        if let cq_lang_structures::Structure::Host { keyed, .. } = &entry.structure
            && let Some((types, _)) = crate::runs::layout_types(&keyed.ty)?
            && let Some(host) = analysis.structures.physical.hosts.get(&symbol_name(&entry.name))
        {
            run_host_types.insert(host.clone(), types);
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
            && matches!(reader, cq_lang_structures::ReaderRef::Scan)
        {
            // Downloaded rows use the same scan adapter as default Structures;
            // the resident annotation describes only the original GPU view.
            keyed.ty = syn::parse_quote!(::core::iter::Map<_, _>);
            keyed.supports = None;
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
            let download = if let Some(types) = run_host_types.get(host) {
                // DECODE V: P6 host fallback reconstructs the complete run layout, not only opaque-Rust rust_reads columns.
                let values = types.iter().enumerate().map(|(index, ty)| {
                    quote!(<#ty as ::gpu_datalog_runtime::Scalar>::decode(__cq_codes[#index]))
                });
                quote!(#value.to_host_codes()?.into_iter().map(|__cq_codes| (#(#values,)*)).collect::<::std::vec::Vec<_>>())
            } else {
                // DECODE V: P6 host fallback materializes complete logical tuples before further relational processing, not final publication.
                quote!(#value.to_host()?)
            };
            context.statements.push(syn::parse_quote!(let #value = &#download;));
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
    let codecs = crate::runs::Codecs::check(&program.structures)?;
    let values: Vec<_> = analysis.structures.layouts[name]
        .iter()
        .map(|column| {
            let value = context.key_field(name, column, true);
            let ty = &analysis.structures.physical.rows[name].types[column];
            let (value, carrier) = match codecs.get(ty) {
                Some(codec) => (
                    quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::code(&(#value))),
                    syn::parse_quote!(u32),
                ),
                None => (value, ty.clone()),
            };
            if columnar {
                quote!(<#carrier as ::gpu_datalog_runtime::Scalar>::encode(&(#value)))
            } else {
                value
            }
        })
        .collect();
    let statements = &context.statements;
    if columnar {
        // The recorded Scalar ABI supplies each lane, including transparent
        // identity newtypes. Keep the direct column path: no tuple copy,
        // dynamic row encoding, or scratch transpose.
        let columns: Vec<_> = (0..values.len())
            .map(|index| format_ident!("__cq_host_column_{index}"))
            .collect();
        if let [value] = values.as_slice() {
            return Ok(quote!(|__cq_columns: &mut [::std::vec::Vec<i64>]| {
                #(#statements)*
                __cq_columns[0].extend(
                    ::core::iter::Iterator::map(#rows, |#pattern| #value));
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
                #(#columns.push(#values);)*
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

pub(crate) fn origin(name: &str, definitions: &BTreeMap<String, Definition>) -> syn::Result<Option<Origin>> {
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
                predicate: false,
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
                        quote!(::gpu_datalog_runtime::encode_scalar(&(#value))),
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
            // A noncontiguous selection stays a row predicate on the scan.
            if !contiguous(&select.condition) {
                result.predicate = true;
            } else {
                intervals(&select.condition, &result.columns, &mut result.selection)?;
            }
            Ok(Some(result))
        }
        Operator::Project(project) => origin(&symbol_name(&project.input), definitions),
        _ => Ok(None),
    }
}

/// Every constraint is an ordered comparison or equality against a host value
/// on one column, so the selection is one half-open interval.
fn contiguous(theta: &Theta) -> bool {
    let variable = |term: &Term| matches!(term, Term::Variable(_));
    theta.constraints.iter().all(|constraint| match constraint {
        Constraint::Compare { left, comparison, right } => {
            !matches!(comparison, Comparison::Ne(_)) && variable(left) && !variable(right)
        }
        Constraint::Between { term, low, high } => {
            variable(term) && !variable(low) && !variable(high)
        }
        _ => false,
    })
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
            quote!(::gpu_datalog_runtime::encode_scalar(&(#value))),
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
        Term::Literal(value) => Ok(quote!(::gpu_datalog_runtime::encode_scalar(&(#value)))),
        Term::Closed(value) => Ok(quote!(::gpu_datalog_runtime::encode_scalar(&(#value)))),
        _ => Err(error(
            "T-Gpu: provider bound must be a constant or host parameter",
        )),
    }
}

fn payload_kind(ty: &Type) -> syn::Result<TokenStream> {
    Ok(quote! {
        match <#ty as ::gpu_datalog_runtime::Scalar>::COLUMN {
            ::gpu_datalog_runtime::Column::U8
            | ::gpu_datalog_runtime::Column::U16
            | ::gpu_datalog_runtime::Column::U32 => ::cq_gpu_sequence_runtime::PayloadKind::U32,
            ::gpu_datalog_runtime::Column::I8
            | ::gpu_datalog_runtime::Column::I16
            | ::gpu_datalog_runtime::Column::I32 => ::cq_gpu_sequence_runtime::PayloadKind::I32,
            ::gpu_datalog_runtime::Column::I64 => ::cq_gpu_sequence_runtime::PayloadKind::I64,
            ::gpu_datalog_runtime::Column::U64 => return Err("T-Gpu: unsigned 64-bit payload exceeds the row ABI".into()),
        }
    })
}

fn lookup_demands(program: &Program) -> BTreeSet<(String, Vec<String>)> {
    fn collect(term: &cq_lang_sequence::Term, demands: &mut BTreeSet<(String, Vec<String>)>) {
        use cq_lang_sequence::Term;
        match term {
            Term::Lookup { provider, keys, inner } => {
                let mut keys: Vec<_> = keys.iter().map(symbol_name).collect();
                keys.sort();
                demands.insert((symbol_name(provider), keys));
                collect(inner, demands);
            }
            Term::Map(_, inner) | Term::Compact(_, inner) | Term::Flatten(inner)
            | Term::Fold(_, inner) | Term::Group(_, inner) | Term::Canon(inner)
            | Term::Store { inner, .. } | Term::Tile { inner, .. }
            | Term::PartitionK { inner, .. } => collect(inner, demands),
            Term::Union(arms) => {
                for arm in arms { collect(arm, demands); }
            }
            Term::Choose(arms) => {
                for (_, arm) in arms { collect(arm, demands); }
            }
            Term::Rows(_) | Term::Ref(_) => {}
        }
    }
    let mut demands = BTreeSet::new();
    for definition in &program.sequence.definitions {
        collect(&definition.term, &mut demands);
    }
    demands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_store_without_device_consumers_does_not_upload_nullable_values() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(id:i64) = (&data.r);
                relation H(value:Option<i64>) = (&data.h);
                r = pipe rename R@0 {id -> id};
                h = let rename H@1 {value -> value};
                joined = pipe probe first [] r,h;
                filtered = pipe guard joined if (value.is_some());
                result = pipe canon project filtered keep {id};
                return result as answer(id);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                h host scan = ::core::iter::Map<_,_> [];
                h = ::cq_storage::SortedRun<_,_> [];
                result canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                input = store rows r;
                host_only = store rows h;
                joined_rows = flatten lookup h [] input;
                selected = store canon map result map filtered joined_rows;
                result_rows = store selected;
                output count;
            }
            graded gpu {
                input at host;
                host_only at host;
                joined_rows at host;
                selected at host;
                result_rows at host.device;
            }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let rows = Rows::new(&program, &analysis).unwrap();
        assert!(!rows.has_source("h"));
        assert!(!rows.build.to_string().contains("__cq_sequence_uploaded_h"));
    }

    #[test]
    fn nonleading_selection_fallback_does_not_construct_a_lookup_provider() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i32,b:i32) = (&data.r);
                r = pipe rename R@0 {a -> a,b -> b};
                selected = pipe canon point select r if (b = 7);
                return selected as answer(a,b);
            }
            structures {
                r host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,i32),()> [];
                r host result(selected) = ::gpu_datalog_runtime::DeviceLayout<(i32,i32),()> [b];
                selected canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                input = store rows r;
                result = store canon map selected flatten tile 256 input;
                output count;
            }
            graded gpu { input at host; result at host.device; }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let rows = Rows::new(&program, &analysis).unwrap();
        assert!(rows.run_sources.contains_key("r"));
        assert!(!rows.run_providers.contains("r"));
        assert!(rows.provider_name("r", &["b".into()]).is_err());
        assert!(lookup_demands(&program).is_empty());

        let mut lookup = program;
        lookup.sequence.definitions[1].term =
            syn::parse_quote!(flatten lookup r [b] input);
        assert_eq!(
            lookup_demands(&lookup),
            BTreeSet::from([("r".into(), vec!["b".into()])])
        );
    }

    #[test]
    fn built_native_provider_realizes_nonleading_keys_without_changing_scan_order() {
        let program: Program = syn::parse_quote! {
            physical {
                relation Driver(b:i32) = (&data.driver);
                relation R(a:i32,b:i32) = (&data.r);
                d = pipe rename Driver@0 {b -> b};
                r = let rename R@1 {a -> a,b -> b};
                joined = pipe probe first [b] d,r;
                result = pipe canon project joined keep {a,b};
                return result as answer(a,b);
            }
            structures {
                d host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,),()> [];
                r host scan = ::gpu_datalog_runtime::DeviceLayout<(i32,i32),()> [];
                r = ::gpu_datalog_runtime::DeviceLayout<(i32,i32),()> [b];
                result canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                input = store rows d;
                index = store rows r;
                result_rows = store canon map result flatten lookup r [b] flatten tile 256 input;
                output count;
            }
            graded gpu { input at host; index at host; result_rows at host.device; }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let rows = Rows::new(&program, &analysis).unwrap();
        let provider = &rows.providers["r"];
        assert_eq!(provider.keys, ["b"]);
        assert_eq!(provider.payload, ["b", "a"]);
        assert_eq!(rows.run_sources["r"].reads[0].0, rows.columns["a"]);
        assert!(rows.build.to_string().contains("permuted_provider (& [1usize , 0usize])"));
    }

    #[test]
    fn translation_rejects_conflicting_logical_symbol_codecs() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i32,b:i32) = (&data.r);
                relation answer(x:i32,y:i32);
                r = pipe rename R@0 {a -> x,b -> y};
                result = pipe canon project r keep {x,y};
                return result as answer(x,y);
            }
            structures {
                r host scan = ::gpu_datalog_runtime::DeviceLayout<
                    (::gpu_datalog_runtime::Symbol<i32, FirstCodec>,),
                    (::gpu_datalog_runtime::Symbol<i32, SecondCodec>,)> [];
                result canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                input = store rows r;
                result_rows = store canon map result flatten tile 256 input;
                output count;
            }
            graded gpu {
                input at host;
                result_rows at host.device;
            }
        };
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let error = crate::translate(&program, &analysis).unwrap_err();
        assert_eq!(error.to_string(), "conflicting codecs for a logical symbol type");
    }

    #[test]
    fn rust_conditionals_preserve_native_operations_at_every_depth() {
        let expression: syn::Expr = syn::parse_quote!((if *x > 0 { *x * 2 } else { *x / -1 }) == 3);
        let tokens = quote!(#expression).to_string();
        assert!(!tokens.contains("cq_storage"), "{tokens}");
        assert!(!tokens.contains("wrapping"), "{tokens}");
        assert!(tokens.contains("* x * 2"), "{tokens}");
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

        let columns = host_rows(
            &program,
            &analysis,
            "result",
            &definitions,
            &resident_hosts,
            &bindings,
            true,
        )
        .unwrap()
        .to_string();
        assert!(columns.contains("Scalar > :: encode"), "{columns}");
        assert!(!columns.contains("as i64"), "{columns}");
    }

    #[test]
    fn host_interval_bounds_dispatch_through_the_scalar_layout() {
        let term = Term::Closed(syn::parse_quote!(cutoff));
        let encoded = host_value(&term).unwrap().to_string();
        assert_eq!(
            encoded,
            quote!(::gpu_datalog_runtime::encode_scalar(&(cutoff))).to_string(),
        );
    }
}
