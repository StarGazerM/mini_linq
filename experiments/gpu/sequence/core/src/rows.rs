use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::gpu::bindings::{
    DirectoryPolicy, ProviderAbi, ProviderLayout, UploadAdapter, UploadBounds, UploadCodec,
};
use cq_lang_sequence::gpu::realization::{
    BaseBounds, BoundSource, DescriptorBounds, HostEvaluation, Interval, IntervalBound, Phase,
    ProviderBuild, Side, SourceAccess, SourceOrigin, TransferSource,
};
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
    /// Parameter identities supplying these reads, recorded with the adapter.
    uses: BTreeMap<usize, String>,
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
    radix: Option<RadixKernel>,
}

/// Kernel ABI of a recorded radix provider: the key view is `Provider::view`.
#[derive(Clone)]
pub struct RadixKernel {
    pub items: Ident,
    /// One item lane per payload column, in payload order.
    pub lanes: Vec<Type>,
}

trait DirectoryPrint {
    fn domain(&self, bounds: TokenStream) -> TokenStream;
    fn allocation(&self) -> TokenStream;
}
impl DirectoryPrint for DirectoryPolicy {
    fn domain(&self, bounds: TokenStream) -> TokenStream {
        match self {
            Self::DenseU32 { .. } => quote! {
                match #bounds {
                    ::cq_gpu_runtime::Bounds::Empty => 0usize,
                    ::cq_gpu_runtime::Bounds::Inclusive { min, max } => {
                        ::cq_gpu_sequence_runtime::dense_key_domain(
                            ::cq_gpu_runtime::Bounds::Inclusive { min, max })?
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

/// Formal rule GPU-ProviderDomain: print recorded descriptor bounds, the base
/// domain then pairwise upload merges; a recorded measured key is reduced once.
fn descriptor_bounds(bounds: &DescriptorBounds) -> syn::Result<TokenStream> {
    let source = |source: &BoundSource| match source {
        BoundSource::Resident { host, index } => {
            let host = format_ident!("__cq_sequence_host_{}", host);
            quote!(#host.storage().bounds[#index])
        }
        BoundSource::Upload { publication, index } => {
            let host = format_ident!("__cq_sequence_uploaded_{publication}");
            quote!(#host.storage().bounds[#index])
        }
    };
    let mut merges = bounds.merges.iter();
    let mut output = match &bounds.base {
        BaseBounds::Union(sources) => {
            let values = sources.iter().map(source);
            quote! {
                [#(#values),*].into_iter().fold(::cq_gpu_runtime::Bounds::Empty, |a,b| match (a,b) {
                    (::cq_gpu_runtime::Bounds::Empty, x) | (x, ::cq_gpu_runtime::Bounds::Empty) => x,
                    (::cq_gpu_runtime::Bounds::Inclusive { min:a, max:b }, ::cq_gpu_runtime::Bounds::Inclusive { min:c, max:d }) =>
                        ::cq_gpu_runtime::Bounds::Inclusive { min:a.min(c), max:b.max(d) },
                })
            }
        }
        BaseBounds::Scalar(ty) => quote! {
            ::cq_gpu_runtime::Bounds::Inclusive {
                min: <#ty as ::gpu_datalog_runtime::Scalar>::MIN as i64,
                max: <#ty as ::gpu_datalog_runtime::Scalar>::MAX as i64,
            }
        },
        BaseBounds::None => source(
            merges
                .next()
                .ok_or_else(|| error("T-Gpu: descriptor column has no recorded bounds"))?,
        ),
    };
    for merge in merges {
        let bound = source(merge);
        output = quote! {
            match (#output, #bound) {
                (::cq_gpu_runtime::Bounds::Empty, x) | (x, ::cq_gpu_runtime::Bounds::Empty) => x,
                (::cq_gpu_runtime::Bounds::Inclusive {min:a,max:b}, ::cq_gpu_runtime::Bounds::Inclusive {min:c,max:d}) =>
                    ::cq_gpu_runtime::Bounds::Inclusive {min:a.min(c),max:b.max(d)},
            }
        };
    }
    Ok(output)
}

fn upload_expression(adapter: &UploadAdapter) -> TokenStream {
    match adapter {
        UploadAdapter::EmptyRows => quote!(::cq_gpu_runtime::DeviceColumns::from_rows(
            __cq_pool.clone(),
            &rows,
            &mut ::cq_gpu_runtime::Dictionary::default(),
            &[]
        )?),
        UploadAdapter::NumericColumns { types, columns } => {
            let columns = columns.iter().map(|column| {
                let UploadCodec::For = column.codec;
                let bounds = match column.bounds {
                    UploadBounds::Measure => quote!(Measure),
                    UploadBounds::Scalar => quote!(Scalar),
                };
                quote!(::cq_gpu_sequence_runtime::UploadColumn {
                    codec: ::cq_gpu_sequence_runtime::UploadCodec::For,
                    bounds: ::cq_gpu_sequence_runtime::UploadBounds::#bounds,
                })
            });
            quote!(::cq_gpu_sequence_runtime::upload_numeric_columns::<(#(#types,)*), _>(
                __cq_pool.clone(), rows, &[#(#columns),*], __cq_auxiliary.bounds)?)
        }
    }
}

/// Device bindings are scalar codes or direct reads through a parent's owner
/// index. Every live read and write must have a recorded binding.
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
        self.rewrite_with_dead(tokens, &BTreeSet::new())
    }

    /// Only writes explicitly proven dead by the caller's liveness facts may
    /// be discarded. An absent binding alone never proves that a column is dead.
    pub fn rewrite_with_dead(
        &self,
        tokens: TokenStream,
        dead: &BTreeSet<usize>,
    ) -> syn::Result<TokenStream> {
        struct Rewrite<'a> {
            row: &'a DeviceRow,
            dead: &'a BTreeSet<usize>,
            error: Option<syn::Error>,
        }
        impl Rewrite<'_> {
            fn missing(&mut self, expression: &syn::Expr, slot: usize, access: &str) {
                let error = syn::Error::new_spanned(
                    expression,
                    format!("T-Gpu: unmapped live __row[{slot}] {access}"),
                );
                if let Some(previous) = &mut self.error {
                    previous.combine(error);
                } else {
                    self.error = Some(error);
                }
            }
        }
        impl VisitMut for Rewrite<'_> {
            fn visit_expr_mut(&mut self, expression: &mut syn::Expr) {
                if let syn::Expr::Assign(assign) = expression
                    && let Some(slot) = row_slot(&assign.left)
                {
                    if let Some(value) = self.row.0.get(&slot) {
                        self.visit_expr_mut(&mut assign.right);
                        let right = &assign.right;
                        *expression = syn::parse_quote!(#value = (#right) as _);
                    } else if self.dead.contains(&slot) {
                        *expression = syn::parse_quote!(());
                    } else {
                        self.missing(expression, slot, "write");
                    }
                    return;
                }
                if let Some(slot) = row_slot(expression) {
                    if let Some(value) = self.row.0.get(&slot) {
                        *expression = syn::parse_quote!(#value);
                    } else {
                        self.missing(expression, slot, "read");
                    }
                    return;
                }
                syn::visit_mut::visit_expr_mut(self, expression);
            }
        }
        let mut block: syn::Block = syn::parse2(quote!({#tokens}))?;
        let mut rewrite = Rewrite {
            row: self,
            dead,
            error: None,
        };
        rewrite.visit_block_mut(&mut block);
        if let Some(error) = rewrite.error {
            return Err(error);
        }
        let statements = block.stmts;
        Ok(quote!(#(#statements)*))
    }
}

/// The kernel scalar publishing a capture answer column (IF-Capture).
pub(crate) fn head_capture(column: &str) -> Ident {
    format_ident!("__cq_head_capture_{column}")
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
    pub whole_lookups: BTreeMap<String, String>,
    whole_lookup_names: BTreeMap<(String, Vec<String>), String>,
    pub setup: TokenStream,
    pub build: TokenStream,
    /// Optional support objects as `(identifier, before setup, after setup)`.
    /// Each is emitted only when generated code outside its own declaration
    /// references the identifier; unused adapters would load device modules.
    pub support: Vec<(Ident, TokenStream, TokenStream)>,
    pub parameters: Vec<(Ident, Type, TokenStream)>,
    /// Canonical bounds and host witnesses shared by checked kernel records.
    pub type_witnesses: Vec<std::sync::Arc<(Ident, TokenStream, Ident)>>,
    /// Parameter identity → generic identity, recorded at ABI construction.
    /// Nested native view types are opaque to the kernel ABI printer.
    pub parameter_witnesses: BTreeMap<String, Ident>,
    pub output_columns: Vec<String>,
    /// The query's host captures (A-Capture), bound once before input setup.
    pub query_captures: cq_lang_structures::syntax::Captures,
    /// Carrier types of capture answer columns, published as kernel scalars.
    capture_types: BTreeMap<String, Type>,
    sources: BTreeMap<String, Source>,
    providers: BTreeMap<String, Provider>,
    definitions: BTreeMap<String, Definition>,
    physical_rows: BTreeMap<String, Row>,
    run_sources: BTreeMap<String, RunSource>,
    run_providers: BTreeSet<String>,
    expressions: BTreeMap<String, syn::Expr>,
    capture_count: usize,
    disabled_captures: BTreeMap<usize, TokenStream>,
    capture_uses: BTreeMap<String, BTreeSet<usize>>,
    provider_layouts: BTreeMap<String, Vec<ProviderLayout>>,
    /// Checked Physical Rust-read facts of each Bind/Guard definition.
    rust_reads: BTreeMap<String, BTreeSet<String>>,
    abi_uses: std::cell::RefCell<BTreeSet<String>>,
}

impl Rows {
    pub(crate) fn begin_abi_uses(&self) {
        self.abi_uses.borrow_mut().clear();
    }

    pub(crate) fn abi_uses(&self) -> BTreeSet<String> {
        self.abi_uses.borrow().clone()
    }

    pub(crate) fn use_parameter(&self, parameter: &Ident) {
        self.abi_uses.borrow_mut().insert(parameter.to_string());
    }

    /// Consume recorded expression dependencies and explicit launch domains.
    /// Parameter indices refer to this argument vector, never a launch ABI.
    pub fn capture_arguments(&self, domains: &[(Vec<String>, TokenStream)]) -> Vec<(Ident, TokenStream)> {
        self.parameters
            .iter()
            .enumerate()
            .map(|(index, (name, _, value))| {
                if index >= self.capture_count {
                    return (name.clone(), value.clone());
                }
                let conditions = domains.iter().filter_map(|(definitions, condition)| {
                    definitions
                        .iter()
                        .any(|name| {
                            self.capture_uses
                                .get(name)
                                .is_some_and(|uses| uses.contains(&index))
                        })
                        .then_some(condition)
                });
                let disabled = self
                    .disabled_captures
                    .get(&index)
                    .cloned()
                    .unwrap_or_else(|| quote!(::core::default::Default::default()));
                (name.clone(), quote!(if false #(|| (#conditions))* { #value } else { #disabled }))
            })
            .collect()
    }

    pub fn new(program: &Program, analysis: &Analysis) -> syn::Result<Self> {
        let physical = &program.structures.physical;
        crate::bindings::check(program, analysis, &analysis.gpu_bindings)?;
        let provider_layouts = analysis.gpu_bindings.providers.clone();
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
        let codecs =
            crate::runs::Codecs::check(&program.structures, &analysis.structures.physical.rows)?;
        let realization = &analysis.gpu_bindings.realization;
        if let Some(rejection) = &realization.rejection {
            return Err(error(rejection));
        }
        crate::realize::check(program, analysis, realization)?;
        let expressions = realization.captures.device.clone();
        let capture_uses = realization.captures.uses.clone();
        let mut physical_rows = analysis.structures.physical.rows.clone();
        for row in physical_rows.values_mut() {
            for (column, ty) in &mut row.types {
                if codecs
                    .get(row.logical_type_ids.get(column).copied())
                    .is_some()
                {
                    *ty = syn::parse_quote!(u32);
                }
            }
        }
        // DESIGN contract7-gpu-rows
        // Consumes recorded captures built from checked identity/encoded read facts.
        // Contract 7, BE-RustRepresentation/T-Gpu; source: doc/PLANNING-REDESIGN.md Contract 7.
        // Source: doc/formal/CONFORMANCE.md BE-RustRepresentation and GPU-Capture.
        // DO NOT admit encoded row reads to opaque Rust or infer identity from a scalar Rust type.
        // Encoded predicates belong to `where`; unsupported Rust placement stays at host, not rewritten.
        let captures = crate::capture::print(&realization.captures.captures);
        let capture_count = realization.captures.captures.len();
        let mut type_witnesses = captures.rust_functions;
        let mut parameter_witnesses: BTreeMap<_, _> = type_witnesses.iter()
            .map(|(generic, _, parameter)| (parameter.to_string(), generic.clone()))
            .collect();
        let disabled_captures = captures.disabled;
        let mut setup = captures.setup;
        let mut build = TokenStream::new();
        let mut parameters = captures.parameters;
        let mut capture_types = BTreeMap::new();
        // IF-Capture on GPU: an answer column naming a capture is published from
        // one host-encoded kernel scalar, never a device row slot.
        for item in &physical.program.captures.items {
            for (binder, ty) in item.binding_types() {
                let column = symbol_name(&binder);
                if !output_columns.contains(&column) {
                    continue;
                }
                if matches!(ty, syn::Type::Infer(_)) {
                    return Err(syn::Error::new_spanned(
                        &binder,
                        "T-Gpu: a capture answer column needs a recorded type",
                    ));
                }
                let (value, carrier): (TokenStream, Type) = match codecs.column(&column) {
                    Some(codec) => (
                        quote!((<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::code(#binder) as i64)),
                        syn::parse_quote!(u32),
                    ),
                    None => (
                        quote!(::gpu_datalog_runtime::encode_scalar::<#ty>(#binder)),
                        ty.clone(),
                    ),
                };
                capture_types.insert(column.clone(), carrier);
                parameters.push((head_capture(&column), syn::parse_quote!(i64), value));
            }
        }
        let mut hosts = BTreeMap::new();
        let mut raw = BTreeMap::new();
        let mut first = true;
        let mut run_hosts = BTreeMap::new();
        // Inputs below extend their own setup; the owner list's declaration is
        // placed between captures and inputs once run bindings are known.
        let captured_setup = std::mem::take(&mut setup);
        for input in physical.inputs() {
            let name = symbol_name(&input.name);
            let mut run_binding = None;
            let mut resident = false;
            for entry in &program.structures.structures.entries {
                if analysis
                    .structures
                    .physical
                    .hosts
                    .get(&symbol_name(&entry.name))
                    != Some(&name)
                {
                    continue;
                }
                if let cq_lang_structures::Structure::Host { reader, keyed, .. } = &entry.structure
                {
                    if !matches!(reader, cq_lang_structures::ReaderRef::Scan) {
                        continue;
                    }
                    if analysis.structures.facts(&keyed.provider).abi
                        == Some(cq_lang_structures::provider::NativeAbi::RadixRelation)
                    {
                        run_binding = Some(analysis.gpu_bindings.relation_inputs.get(&name)
                            .ok_or_else(|| error("GPU-RadixRelation: missing recorded input read"))?);
                        resident = true;
                    } else if analysis.structures.facts(&keyed.provider).abi
                        == Some(cq_lang_structures::provider::NativeAbi::Columns)
                    {
                        resident = true;
                    }
                }
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
            if !resident {
                continue;
            }
            if first {
                setup.extend(quote! {
                    let __cq_pool = #id.pool().clone();
                });
                first = false;
            }
            if let Some(read) = run_binding {
                let types = read.key_types.clone();
                let keys = types.len();
                let cq_lang_sequence::gpu::bindings::RadixWordCapture::InputView { host } = &read.word_capture;
                let word = format_ident!("__CqRadixWord_{host}");
                let witness = format_ident!("__cq_radix_word_{host}");
                type_witnesses.push((word.clone(), quote!(::gpu_datalog_runtime::Word), witness.clone()));
                let offset = &read.offset;
                let ty = &read.provider.ty;
                let schema: Vec<_> = types.iter().map(crate::runs::scalar).collect();
                let view = format_ident!("__cq_sequence_runs_{name}");
                parameter_witnesses.insert(view.to_string(), word.clone());
                setup.extend(quote! {
                    let #id: &#ty = #id;
                    if !__cq_pool.shares_memory(#id.pool()) {
                        return Err("T-Gpu: resident inputs must share a pool's memory".into());
                    }
                    #id.validate(&[#(#schema),*])?;
                    let #witness = {
                        fn infer<W: ::gpu_datalog_runtime::Word>(
                            _: &::gpu_datalog_runtime::radix_run::RadixLayoutView<W, u64>,
                        ) -> W { W::zero() }
                        infer(&#id.view())
                    };
                    let #length = #id.len();
                    if ::gpu_datalog_runtime::trace_kernels() {
                        ::std::eprintln!("gpu_sequence_input\tname={}\tdescriptors={:p}\trows={}\truns={}",
                            #name, #id.view().keys.runs, #length, #id.view().keys.run_count);
                    }
                    __cq_run_owners.push(#id.owner());
                });
                parameters.push((
                    view.clone(),
                    syn::parse_quote!(::gpu_datalog_runtime::radix_run::RadixLayoutView<#word, #offset>),
                    quote!(#id.view_on(&__cq_pool)?),
                ));
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
        // DL-WaveLanes: run-input owners travel with candidate batches across
        // lane threads, so a program with run bindings retains Send + Sync owners.
        let owner = if run_hosts.is_empty() {
            quote!(dyn ::std::any::Any)
        } else {
            quote!(dyn ::std::any::Any + Send + Sync)
        };
        setup = {
            let mut ordered = captured_setup;
            ordered.extend(quote! {
                let mut __cq_run_owners: ::std::vec::Vec<::std::sync::Arc<#owner>> = ::std::vec::Vec::new();
            });
            ordered.extend(setup);
            ordered
        };
        if first {
            setup.extend(quote!(let __cq_pool = ::gpu_datalog_runtime::thread_pool()?;));
        }
        let output_width = output_columns.len();
        let auxiliary = crate::reduce::auxiliary_tokens(
            realization
                .auxiliary
                .as_ref()
                .ok_or_else(|| error("GPU-Realization: missing auxiliary kernel geometry"))?,
        );
        // The auxiliary geometry precedes every support object that reads it.
        let mut support = vec![
            (
                format_ident!("__cq_auxiliary"),
                TokenStream::new(),
                quote!(let __cq_auxiliary = #auxiliary;),
            ),
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
                                ::cq_gpu_sequence_runtime::canon::PackedColumnsAdapter::new(__cq_pool.clone(), #output_width, __cq_auxiliary.canon)?)));
                        }
                        Ok(cache.as_ref().unwrap().clone())
                    })?;
                },
            ),
        ];
        if let Some(layout) = provider_layouts
            .values()
            .flatten()
            .find(|layout| layout.abi == ProviderAbi::Range)
        {
            let ty = &layout.provider.ty;
            support.push((
                format_ident!("__cq_range_builder"),
                TokenStream::new(),
                quote!(let __cq_range_builder = <#ty>::builder(&__cq_pool, &__cq_auxiliary)?;),
            ));
        }
        let mut sources = BTreeMap::new();
        let mut providers = BTreeMap::new();
        let mut run_sources = BTreeMap::new();
        let mut run_providers = BTreeSet::new();
        // Formal rule GPU-Realization: every device source, its origin, access
        // and construction phase are recorded before printing.
        for (name, realization) in &realization.sources {
            let definition = &definitions[name];
            let needs_host = realization.phase == Phase::Build;
            if let SourceOrigin::Uploaded { evaluation } = &realization.origin {
                let layout = &analysis.structures.layouts[name];
                let adapter = analysis
                    .gpu_bindings
                    .uploads
                    .get(name)
                    .ok_or_else(|| error("GPU-Upload: missing source upload adapter"))?;
                let rows = host_rows(
                    program,
                    analysis,
                    name,
                    &definitions,
                    evaluation,
                    &BTreeMap::new(),
                    matches!(adapter, UploadAdapter::NumericColumns { .. }),
                )?;
                let id = format_ident!("__cq_sequence_uploaded_{name}");
                let length = format_ident!("__cq_sequence_uploaded_length_{name}");
                let upload = upload_expression(adapter);
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
            }
            let origin_host = match &realization.origin {
                SourceOrigin::Resident { host } | SourceOrigin::RadixRelation { host, .. } => host.clone(),
                SourceOrigin::Uploaded { .. } => name.clone(),
            };
            let (host, host_length) = &hosts[&origin_host];
            if let SourceOrigin::RadixRelation {
                providers: native, ..
            } = &realization.origin
            {
                let (view, _types, _key_count) = run_hosts
                    .get(&origin_host)
                    .ok_or_else(|| error("GPU-RadixRelation: run source names a nonrun input"))?;
                let layout = &analysis.structures.host_layouts[&origin_host];
                let payload = layout
                    .iter()
                    .map(|source| {
                        realization
                            .columns
                            .iter()
                            .find(|(_, original)| *original == source)
                            .map(|(column, _)| column.clone())
                            .ok_or_else(|| error("GPU-RadixRelation: supplied layout lost a packed column"))
                    })
                    .collect::<syn::Result<Vec<_>>>()?;
                let reads = payload
                    .iter()
                    .enumerate()
                    .map(|(index, column)| {
                        let slot = columns[column];
                        (slot, index)
                    })
                    .collect();
                run_sources.insert(
                    name.clone(),
                    RunSource {
                        view: view.clone(),
                        reads,
                        length: quote!(#host_length),
                    },
                );
                for provider_name in native {
                    if analysis
                        .structures
                        .built
                        .get(name)
                        .is_some_and(|layouts| layouts.len() != 1)
                    {
                        return Err(error(
                            "GPU-RadixRelation: name each supplied provider layout separately",
                        ));
                    }
                    let recorded = analysis
                        .gpu_bindings
                        .native
                        .get(provider_name)
                        .ok_or_else(|| error("GPU-NativeAbi: missing recorded native lookup"))?;
                    if recorded.source != payload || recorded.host != origin_host {
                        return Err(error("GPU-NativeAbi: stale native lookup"));
                    }
                    let (provider_view, provider_payload) = if let Some(materialization) =
                        &recorded.materialization
                    {
                        let order = &materialization.permutation;
                        let cq_lang_sequence::gpu::bindings::RadixWordCapture::InputView { host: capture_host } = &recorded.radix.word_capture;
                        let word = format_ident!("__CqRadixWord_{capture_host}");
                        let offset = &recorded.radix.offset;
                        let provider = &materialization.provider.ty;
                        let types = materialization.provider.input.as_ref()
                            .expect("checked permutation schema").columns.iter().map(|column| &column.storage);
                        let cq_lang_sequence::gpu::bindings::RadixParams {
                            ratio, radix_bits, merge_tile_rows, block_threads,
                        } = materialization.params;
                        let threads = materialization.permutation_threads;
                        let active_bits = &materialization.active_bits;
                        let owner = format_ident!("__cq_native_provider_{provider_name}");
                        let provider_view = format_ident!("__cq_native_view_{provider_name}");
                        parameter_witnesses.insert(provider_view.to_string(), word.clone());
                        build.extend(quote! {
                                let #owner: #provider = #host.permuted_provider::<(#(#types,)*)>(
                                    &__cq_pool,
                                    &[#(#order),*],
                                    ::gpu_datalog_runtime::radix_run::RadixRunParams {
                                        ratio: #ratio, radix_bits: #radix_bits, merge_tile_rows: #merge_tile_rows, block_threads: #block_threads,
                                    },
                                    #threads,
                                    #active_bits,
                                )?;
                                __cq_run_owners.push(#owner.owner());
                            });
                        parameters.push((
                            provider_view.clone(),
                            syn::parse_quote!(::gpu_datalog_runtime::radix_run::RadixLayoutView<#word, #offset>),
                            quote!(#owner.view_on(&__cq_pool)?),
                        ));
                        (provider_view, recorded.payload.clone())
                    } else {
                        (view.clone(), recorded.payload.clone())
                    };
                    providers.insert(
                        provider_name.clone(),
                        Provider {
                            view: provider_view,
                            keys: provider_payload[..recorded.keys.len()].to_vec(),
                            payload: provider_payload,
                            radix: None,
                        },
                    );
                    run_providers.insert(provider_name.clone());
                }
                continue;
            }
            let fields = &raw[&origin_host];
            let interval = match &realization.access {
                SourceAccess::Scan => None,
                SourceAccess::Positions(interval) => Some(interval),
                SourceAccess::Provider { selection } => selection.as_ref(),
            };
            let selection = if let Some(interval) = interval {
                let index = fields[&interval.column].0;
                let (start, end) = interval_tokens(interval);
                quote!(Some(::cq_gpu_sequence_runtime::RangeSelection {
                    column: unsafe { ::cq_gpu_sequence_runtime::ColumnDescriptor::new(
                        #host.storage().columns[#index].clone(), #host_length)? },
                    start: #start, end: #end,
                }))
            } else {
                quote!(None)
            };
            if let SourceAccess::Provider { .. } = &realization.access {
                if analysis.structures.built[name].len() != 1 {
                    return Err(syn::Error::new_spanned(
                        definition,
                        "T-Gpu: source has multiple recorded provider layouts; name each provider store separately",
                    ));
                }
                let layout = &provider_layouts[name][0];
                if let Some(radix) = &layout.radix {
                    // Formal rule GPU-RadixSourceBuild: the recorded K/V lanes,
                    // W/O and params; the provider spelling is rustc-checked.
                    let descriptor = |column: &String| -> syn::Result<TokenStream> {
                        let source = realization.columns.get(column).ok_or_else(|| {
                            syn::Error::new_spanned(
                                definition,
                                "T-Gpu: radix provider column has no resident origin",
                            )
                        })?;
                        let index = fields[source].0;
                        Ok(
                            quote!(unsafe { ::cq_gpu_sequence_runtime::ColumnDescriptor::new(
                            #host.storage().columns[#index].clone(), #host_length)? }),
                        )
                    };
                    let keys = layout
                        .keys
                        .iter()
                        .map(descriptor)
                        .collect::<syn::Result<Vec<_>>>()?;
                    let values = layout
                        .payload
                        .iter()
                        .map(descriptor)
                        .collect::<syn::Result<Vec<_>>>()?;
                    let ty = &layout.provider.ty;
                    let key_types = &radix.key_types;
                    let lanes = &radix.lanes;
                    let word = &radix.word;
                    let offset = &radix.offset;
                    let ratio = radix.params.ratio;
                    let radix_bits = radix.params.radix_bits;
                    let merge_tile_rows = radix.params.merge_tile_rows;
                    let block_threads = radix.params.block_threads;
                    let owner = format_ident!("__cq_sequence_provider_{}", name);
                    let view = format_ident!("__cq_sequence_provider_view_{}", name);
                    let items = format_ident!("__cq_sequence_provider_items_{}", name);
                    build.extend(quote! {
                        let #owner: #ty = ::cq_gpu_sequence_runtime::radix_layout::<
                            (#(#key_types,)*), (#(#lanes,)*), #word, #offset,
                            (#(::gpu_datalog_runtime::Buffer<#lanes>,)*)
                        >(
                            &__cq_pool,
                            ::gpu_datalog_runtime::RadixRunParams {
                                ratio: #ratio, radix_bits: #radix_bits, merge_tile_rows: #merge_tile_rows,
                                block_threads: #block_threads,
                            },
                            &[#(#keys),*],
                            &[#(#values),*],
                            (#selection).as_ref(),
                            __cq_auxiliary.selection,
                        )?;
                    });
                    // GPU-Smallest ChoiceCarrier::PositionTag: a side's key
                    // slots and candidate ordinals fit the tag's low bits.
                    let tag_bits = analysis
                        .gpu_bindings
                        .realization
                        .smallest
                        .values()
                        .filter(|record| record.sides.iter().any(|side| side.provider == *name))
                        .map(|record| {
                            let cq_lang_sequence::gpu::realization::ChoiceCarrier::PositionTag {
                                choice_bits,
                            } = record.carrier;
                            31 - choice_bits
                        })
                        .min();
                    if let Some(bits) = tag_bits {
                        build.extend(quote! {
                            if #owner.len() >= 1usize << #bits {
                                return Err("GPU-Smallest: a candidate side exceeds its position tag".into());
                            }
                        });
                    }
                    parameters.push((
                        view.clone(),
                        syn::parse_quote!(::gpu_datalog_runtime::RadixLayoutView<#word, #offset>),
                        quote!(#owner.view()),
                    ));
                    parameters.push((
                        items.clone(),
                        syn::parse_quote!(::gpu_datalog_runtime::RadixItemsView),
                        quote!(#owner.items_view()),
                    ));
                    // Scans read the resident source, never the grouped items.
                    let reads = realization
                        .columns
                        .iter()
                        .filter_map(|(column, source)| {
                            columns.get(column).map(|&slot| {
                                let param = &fields[source].1;
                                (slot, quote!(#param))
                            })
                        })
                        .collect();
                    let (positions, length) = if interval.is_some() {
                        let selected = format_ident!("__cq_sequence_selected_{}", name);
                        let param = format_ident!("__cq_sequence_selected_rows_{}", name);
                        build.extend(quote! {
                            let #selected = ::cq_gpu_sequence_runtime::select(
                                __cq_pool.clone(), &(#selection).expect("recorded selection"),
                                __cq_auxiliary.selection)?;
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
                            uses: realization.columns.iter()
                                .filter_map(|(column, source)| columns.get(column)
                                    .map(|slot| (*slot, fields[source].1.to_string()))).collect(),
                        },
                    );
                    providers.insert(
                        name.clone(),
                        Provider {
                            view,
                            payload: layout.payload.clone(),
                            keys: layout.keys.clone(),
                            radix: Some(RadixKernel {
                                items,
                                lanes: lanes.clone(),
                            }),
                        },
                    );
                    continue;
                }
                if layout.abi != ProviderAbi::Range {
                    return Err(error(
                        "GPU-ProviderLayout: recorded provider cannot build resident columns",
                    ));
                }
                let directory = layout
                    .directory
                    .as_ref()
                    .ok_or_else(|| error("GPU-ProviderLayout: missing directory"))?;
                let keys = layout.keys.clone();
                let mut key_descriptors = Vec::new();
                for key in &keys {
                    let source = realization.columns.get(key).ok_or_else(|| {
                        syn::Error::new_spanned(
                            definition,
                            "T-Gpu: provider key has no resident origin",
                        )
                    })?;
                    let index = fields[source].0;
                    let domain_size = directory.domain(quote!(#host.storage().bounds[#index]));
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
                    let source = &realization.columns[column];
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
                let directory_entries = directory.allocation();
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
                        uses: payload.iter().map(|column| (columns[column], view.to_string())).collect(),
                    },
                );
                providers.insert(
                    name.clone(),
                    Provider {
                        view,
                        payload,
                        keys,
                        radix: None,
                    },
                );
            } else {
                let reads = realization
                    .columns
                    .iter()
                    .filter_map(|(column, source)| {
                        columns.get(column).map(|&slot| {
                            let param = &fields[source].1;
                            (slot, quote!(#param))
                        })
                    })
                    .collect();
                let (positions, length) =
                    if matches!(realization.access, SourceAccess::Positions(_)) {
                        let selected = format_ident!("__cq_sequence_selected_{}", name);
                        let param = format_ident!("__cq_sequence_selected_rows_{}", name);
                        build.extend(quote! {
                            let #selected = ::cq_gpu_sequence_runtime::select(
                                __cq_pool.clone(), &(#selection).expect("recorded selection"),
                                __cq_auxiliary.selection)?;
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
                        uses: realization.columns.iter()
                            .filter_map(|(column, source)| columns.get(column)
                                .map(|slot| (*slot, fields[source].1.to_string()))).collect(),
                    },
                );
            }
        }
        build.extend(quote! {
            let __cq_timing_built = if __cq_timing_enabled {
                Some(__cq_pool.stream().record_event(
                    Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?)
            } else { None };
        });
        let rust_reads = definitions
            .iter()
            .filter_map(|(name, definition)| {
                analysis
                    .structures
                    .physical
                    .rust_reads(&definition.operator)
                    .map(|reads| (name.clone(), reads))
            })
            .collect();
        Ok(Self {
            width: columns.len().max(1),
            whole_lookups: analysis
                .gpu_bindings
                .realization
                .whole_lookups
                .iter()
                .map(|name| (format!("__cq_whole_{name}"), name.clone()))
                .collect(),
            whole_lookup_names: analysis
                .gpu_bindings
                .realization
                .whole_lookups
                .iter()
                .map(|name| ((name.clone(), Vec::new()), format!("__cq_whole_{name}")))
                .collect(),
            columns,
            setup,
            build,
            support,
            parameters,
            type_witnesses: type_witnesses.into_iter().map(std::sync::Arc::new).collect(),
            parameter_witnesses,
            output_columns,
            query_captures: physical.program.captures.clone(),
            capture_types,
            sources,
            providers,
            definitions,
            physical_rows,
            run_sources,
            run_providers,
            expressions,
            capture_count,
            disabled_captures,
            capture_uses,
            provider_layouts,
            rust_reads,
            abi_uses: Default::default(),
        })
    }

    pub fn has_runs(&self) -> bool {
        !self.run_sources.is_empty()
    }

    /// Realize the recorded host boundary after its device operands publish.
    /// Bindings are decoded complete publications, never re-evaluated producers.
    pub fn publish_host(
        &mut self,
        program: &Program,
        analysis: &Analysis,
        name: &str,
        evaluation: &HostEvaluation,
        bindings: &BTreeMap<String, Ident>,
    ) -> syn::Result<TokenStream> {
        let layout = &analysis.structures.layouts[name];
        let adapter = analysis
            .gpu_bindings
            .uploads
            .get(name)
            .ok_or_else(|| error("GPU-Upload: missing publication upload adapter"))?;
        let rows = host_rows(
            program,
            analysis,
            name,
            &self.definitions,
            evaluation,
            bindings,
            matches!(adapter, UploadAdapter::NumericColumns { .. }),
        )?;
        let id = format_ident!("__cq_sequence_uploaded_{name}");
        let length = format_ident!("__cq_sequence_uploaded_length_{name}");
        let upload = upload_expression(adapter);
        let mut reads = Vec::new();
        for (index, column) in layout.iter().enumerate() {
            let param = format_ident!("__cq_sequence_uploaded_column_{name}_{index}");
            self.parameters.push((
                param.clone(),
                syn::parse_quote!(::cq_gpu_runtime::ColumnView),
                quote!(unsafe { #id.storage().columns[#index].view() }),
            ));
            reads.push((self.columns[column], quote!(#param)));
        }
        self.sources.insert(
            name.to_owned(),
            Source {
                reads,
                length: quote!(#length),
                positions: None,
                uses: layout.iter().enumerate()
                    .map(|(index, column)| (self.columns[column], format!("__cq_sequence_uploaded_column_{name}_{index}")))
                    .collect(),
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
        self.source_columns(name, position, row, None)
    }

    /// Demand comes from checked row operations, never from printed identifiers.
    pub(crate) fn source_columns(
        &self,
        name: &str,
        position: TokenStream,
        row: TokenStream,
        live: Option<&BTreeSet<usize>>,
    ) -> syn::Result<TokenStream> {
        if let Some(source) = self.run_sources.get(name) {
            let view = &source.view;
            self.use_parameter(view);
            // DECODE V: P6 run scans unpack every demanded field before relational consumers; retain narrow codes unless rust_reads requires identity.
            let reads = source.reads.iter().filter(|(slot, _)| live.is_none_or(|live| live.contains(slot))).map(|(slot, column)| {
                quote! {
                    #row[#slot] = unsafe { ::gpu_datalog_runtime::radix_run::device::decode_key(
                        #view, __run_index, __run_position, #column as u32) };
                }
            });
            return Ok(quote! {
                let (__run_index, __run_position) = unsafe {
                    ::gpu_datalog_runtime::radix_run::device::locate_key(#view.keys, (#position) as u64)
                };
                #(#reads)*
            });
        }
        let source = self.sources.get(name).ok_or_else(|| {
            error(format!(
                "T-Gpu: {name} has no resident source; materialize its Sequence producer"
            ))
        })?;
        let mut uses = source.uses.iter()
            .filter(|(slot, _)| live.is_none_or(|live| live.contains(slot)))
            .map(|(_, parameter)| parameter.clone()).peekable();
        if uses.peek().is_some() {
            self.abi_uses.borrow_mut().extend(uses);
            if let Some(positions) = &source.positions {
                self.use_parameter(positions);
            }
        }
        let position = match &source.positions {
            Some(positions) => quote!(unsafe { *#positions.add((#position) as usize) } as u64),
            None => quote!((#position) as u64),
        };
        // DECODE V: P6 source scans reconstruct FOR values into i64 even for code-only joins and predicates.
        let reads = source.reads.iter().filter(|(slot, _)| live.is_none_or(|live| live.contains(slot))).map(|(slot, view)| {
            quote! {
                #row[#slot] = unsafe { ::cq_gpu_sequence_runtime::device::decode(#view, #position) } as i64;
            }
        });
        Ok(quote!(#(#reads)*))
    }
    pub fn source_len(&self, name: &str) -> syn::Result<TokenStream> {
        if let Some(source) = self.run_sources.get(name) {
            return Ok(source.length.clone());
        }
        self.sources
            .get(name)
            .map(|s| s.length.clone())
            .ok_or_else(|| error(format!("T-Gpu: {name} has no recorded source length")))
    }
    pub fn provider_name(&self, name: &str, keys: &[String]) -> syn::Result<String> {
        if let Some(alias) = self
            .whole_lookup_names
            .get(&(name.to_owned(), keys.to_vec()))
        {
            return Ok(alias.clone());
        }
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
            .map(|p| {
                self.use_parameter(&p.view);
                (p.view.clone(), p.payload.clone(), p.keys.clone())
            })
            .ok_or_else(|| {
                error(format!(
                    "T-Gpu: {name} has no recorded resident range layout"
                ))
            })
    }
    pub fn run_provider(&self, name: &str) -> bool {
        self.run_providers.contains(name)
    }
    /// The kernel ABI of a recorded radix provider.
    pub fn radix_provider(&self, name: &str) -> Option<&RadixKernel> {
        self.providers
            .get(name)
            .and_then(|provider| provider.radix.as_ref())
            .inspect(|radix| self.use_parameter(&radix.items))
    }
    /// The device value of an answer column: its row slot, or the kernel
    /// scalar of a capture answer column (IF-Capture).
    pub fn output_value(&self, column: &str) -> syn::Result<TokenStream> {
        if self.capture_types.contains_key(column) {
            let parameter = head_capture(column);
            self.use_parameter(&parameter);
            return Ok(quote!(#parameter));
        }
        let slot = self
            .columns
            .get(column)
            .ok_or_else(|| error(format!("missing device column {column}")))?;
        Ok(quote!(__row[#slot]))
    }
    /// Whether an answer column is a capture published from a kernel scalar.
    pub fn is_capture_output(&self, column: &str) -> bool {
        self.capture_types.contains_key(column)
    }
    /// Realize the recorded provider only after its device store has published
    /// its live count. Descriptors borrow the stored layout without transposing
    /// or copying it before the ordinary radix/lower-bound range build.
    pub fn publish_provider(
        &mut self,
        analysis: &Analysis,
        result: &str,
        publication: &str,
        index: usize,
        packed: Option<&[String]>,
        build: Option<&ProviderBuild>,
    ) -> syn::Result<TokenStream> {
        let Some(build) = build else {
            return Ok(TokenStream::new());
        };
        let layouts =
            analysis.structures.built.get(result).ok_or_else(|| {
                error("GPU-Realization: provider publication lacks built layouts")
            })?;
        let rows = format_ident!("__cq_store_rows_{index}");
        let mut output = TokenStream::new();
        if packed.is_none() {
            output.extend(quote! { let #rows = ::std::sync::Arc::new(#rows); });
        }
        for &layout_index in &build.layouts {
            let layout = self.provider_layouts[result][layout_index].clone();
            if layout.abi != ProviderAbi::Range {
                return Err(error(
                    "GPU-ProviderLayout: recorded provider cannot build a row publication",
                ));
            }
            let directory = layout
                .directory
                .as_ref()
                .ok_or_else(|| error("GPU-ProviderLayout: missing directory"))?;
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
                let bounds = descriptor_bounds(build.columns.get(column).ok_or_else(|| {
                    error(format!("T-Gpu: {column} has no recorded codec bounds"))
                })?)?;
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
                let measure = build.columns[key].measure.then(|| {
                    quote! {
                        column.measure_bounds(__cq_auxiliary.bounds)?;
                        if column.rows() != 0 { ::gpu_datalog_runtime::record_readback(); }
                    }
                });
                let domain_size = directory.domain(quote!(column.bounds()));
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
                        radix: None,
                    },
                );
            }
            let directory_entries = directory.allocation();
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
        Ok(self
            .map_definitions(name, input_result)?
            .into_iter()
            .map(|definition| symbol_name(&definition.name))
            .collect())
    }

    /// Backward demand over the checked Physical operators. Bind/Guard reads
    /// are the recorded Rust dependencies; relational reads are structured
    /// Theta columns. Renames/projects do not execute a row assignment.
    pub(crate) fn map_live_columns(
        &self,
        name: &str,
        input_result: &str,
        live: &mut BTreeSet<usize>,
    ) -> syn::Result<()> {
        for definition in self.map_definitions(name, input_result)? {
            match &definition.operator {
                Operator::Bind(bind) => {
                    for binding in cq_syntax::rust::bindings(&bind.pattern) {
                        if let Some(slot) = self.columns.get(&symbol_name(&binding)) {
                            live.remove(slot);
                        }
                    }
                    live.extend(analysis_reads(&self.rust_reads, definition)?.iter()
                        .filter_map(|column| self.columns.get(column)).copied());
                }
                Operator::Guard(_) => {
                    live.extend(analysis_reads(&self.rust_reads, definition)?.iter()
                        .filter_map(|column| self.columns.get(column)).copied());
                }
                Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                    live.extend(select.condition.columns().iter()
                        .filter_map(|column| self.columns.get(column)).copied());
                }
                Operator::Rename(_) | Operator::Project(_) | Operator::Unit(_)
                | Operator::Semi(_, _) | Operator::Anti(_, _) | Operator::Probe(_, _)
                | Operator::Merge(_, _) => {}
                Operator::Group(_) | Operator::Union(_) => {
                    return Err(error("GPU-KernelAbi: row demand requires a flat checked map"));
                }
            }
        }
        Ok(())
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
            if let Some(uses) = self.capture_uses.get(&symbol_name(&definition.name)) {
                for &index in uses {
                    self.use_parameter(&self.parameters[index].0);
                }
            }
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
                    let expression = self
                        .expressions
                        .get(&symbol_name(&definition.name))
                        .ok_or_else(|| {
                            error("GPU-Capture: selection is not an integer device predicate")
                        })?;
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
                    let expression = self
                        .expressions
                        .get(&symbol_name(&definition.name))
                        .ok_or_else(|| {
                            error("T-Gpu: opaque Rust binding requires host evaluation")
                        })?;
                    let dependencies = analysis_reads(&self.rust_reads, definition)?;
                    let bindings = dependencies.iter().map(|column| {
                        let id = &input_row.bindings[column];
                        let ty = &input_row.types[column];
                        let slot = self.columns[column];
                        // DECODE R: Bind free-variable dependencies select these columns; checked identity representation permits the verbatim opaque Rust expression.
                        quote!(let #id = &<#ty as ::gpu_datalog_runtime::Scalar>::decode(#row[#slot] as i128);)
                    });
                    let names = cq_syntax::rust::bindings(&bind.pattern);
                    let slot = self.columns[&symbol_name(&names[0])];
                    let ty = &self.physical_rows[&symbol_name(&definition.name)].types
                        [&symbol_name(&names[0])];
                    output.extend(quote! {
                        if #live { #(#bindings)* #row[#slot] = <#ty as ::gpu_datalog_runtime::Scalar>::encode(&(#expression)); }
                    });
                }
                Operator::Guard(guard) => {
                    let input = symbol_name(&guard.input);
                    let input_row = &self.physical_rows[&input];
                    let expression = self
                        .expressions
                        .get(&symbol_name(&definition.name))
                        .ok_or_else(|| {
                            error("T-Gpu: opaque Rust guard requires host evaluation")
                        })?;
                    let dependencies = analysis_reads(&self.rust_reads, definition)?;
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

/// Sources admitted by a dispatch, a native provider, a whole-relation
/// lookup, or a recorded scalar host publication. Arbitrary host-only Rust
/// rows do not acquire a device ABI.
pub(crate) fn device_source_results(program: &Program, analysis: &Analysis) -> BTreeSet<String> {
    let mut sources: BTreeSet<_> = analysis
        .dispatches
        .iter()
        .flat_map(|dispatch| {
            ::core::iter::once(&dispatch.outer_store).chain(dispatch.inner_stores.iter())
        })
        .map(|name| analysis.results.get(name).unwrap_or(name).clone())
        .collect();
    sources.extend(
        device_lookup_demands(program, analysis)
            .into_iter()
            .filter_map(|(name, keys)| keys.is_empty().then_some(name)),
    );
    sources.extend(
        analysis
            .structures
            .built
            .iter()
            .filter_map(|(name, layouts)| {
                layouts
                    .iter()
                    .any(|(_, provider)| {
                        analysis.structures.facts(provider).abi
                            == Some(cq_lang_structures::provider::NativeAbi::Range)
                    })
                    .then(|| name.clone())
            }),
    );
    sources.extend(analysis.stores.keys().filter_map(|store| {
        let result = analysis.results.get(store)?;
        let host = analysis
            .levels
            .get(store)?
            .last()
            .is_some_and(|level| level == "host");
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

/// The checked Physical Rust reads of a row function; never re-derived here.
fn analysis_reads<'a>(
    reads: &'a BTreeMap<String, BTreeSet<String>>,
    definition: &Definition,
) -> syn::Result<&'a BTreeSet<String>> {
    reads
        .get(&symbol_name(&definition.name))
        .ok_or_else(|| error("T-Gpu: Rust row function lacks its checked read facts"))
}

/// Realize a host subgraph with the shared Rust row printer before uploading a
/// device-representable boundary. String guards execute before any upload.
fn host_rows(
    program: &Program,
    analysis: &Analysis,
    name: &str,
    definitions: &BTreeMap<String, Definition>,
    evaluation: &HostEvaluation,
    bindings: &BTreeMap<String, Ident>,
    columnar: bool,
) -> syn::Result<TokenStream> {
    let dependencies = host_dependencies_from(name, definitions, bindings);
    let mut run_host_types = BTreeMap::new();
    for entry in &program.structures.structures.entries {
        if let cq_lang_structures::Structure::Host { keyed, .. } = &entry.structure
            && analysis.structures.facts(&keyed.provider).abi
                == Some(cq_lang_structures::provider::NativeAbi::RadixRelation)
            && let Some(input) = &keyed.provider.input
            && let Some(host) = analysis
                .structures
                .physical
                .hosts
                .get(&symbol_name(&entry.name))
        {
            run_host_types.insert(
                host.clone(),
                input
                    .columns
                    .iter()
                    .map(|column| column.storage.clone())
                    .collect::<Vec<_>>(),
            );
        }
    }
    let host_structures = crate::bindings::host_structures(program, &analysis.gpu_bindings)
        .expect("checked host materialization adapters");
    let host_analysis = cq_lang_structures::contract::check(&host_structures)
        .expect("checked host Structures contract");
    let inputs = host_structures
        .physical
        .inputs()
        .map(|input| {
            let name = symbol_name(&input.name);
            let binding = format_ident!("__cq_sequence_host_{}", name);
            (name, binding)
        })
        .collect();
    let mut context =
        cq_print::Context::with_bound_captures(&host_structures, &host_analysis, &inputs);
    // This host/device boundary publishes its complete checked layout, even
    // when the final answer projects columns away. Seed that demand before
    // RowEmitter prunes carriers through the producer's probes and guards.
    context.demand_columns(&host_structures, name, &analysis.structures.layouts[name])?;
    // Formal rule GPU-Transfer: resident inputs read by host Rust download
    // the recorded complete rows through their recorded download adapter.
    for transfer in &evaluation.transfers {
        let TransferSource::Resident(host) = &transfer.source else {
            continue;
        };
        let cq_lang_sequence::gpu::realization::Granularity::CompleteRows = transfer.granularity;
        let value = &context.hosts[host].value;
        let download = if let Some(types) = run_host_types.get(host) {
            // DECODE V: P6 host fallback reconstructs the complete run layout, not only opaque-Rust rust_reads columns.
            let values = types.iter().enumerate().map(|(index, ty)| {
                quote!(<#ty as ::gpu_datalog_runtime::Scalar>::decode(__cq_codes[#index]))
            });
            quote!(#value.to_rows()?.into_iter().map(|__cq_codes| (#(#values,)*)).collect::<::std::vec::Vec<_>>())
        } else {
            // DECODE V: P6 host fallback materializes complete logical tuples before further relational processing, not final publication.
            quote!(#value.to_host()?)
        };
        context
            .statements
            .push(syn::parse_quote!(let #value = &#download;));
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
        if host_analysis.retained.contains_key(&result)
            && !host_analysis.rows_held(&result)
            && host_analysis.built.get(&result).is_none_or(Vec::is_empty)
        {
            // STR-RowsByValue, as in the row printer's `let`: a single-consumer
            // rows record is moved into its reader. A canonical rows record is
            // realized by its Sequence canonical publication (the recorded
            // device sort/unique build), never rebuilt or collected on host.
            let rows = emitter.context().rows_by_value(&result, rows);
            emitter
                .context_mut()
                .reprs
                .insert(result, cq_print::Repr::Pipe(Some(rows)));
        } else if host_analysis.built.contains_key(&result)
            || host_analysis.retained.contains_key(&result)
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
    let codecs =
        crate::runs::Codecs::check(&program.structures, &analysis.structures.physical.rows)?;
    let values: Vec<_> = analysis.structures.layouts[name]
        .iter()
        .map(|column| {
            let value = context.key_field(name, column, true);
            let ty = &analysis.structures.physical.rows[name].types[column];
            let value = match codecs.column(column) {
                Some(codec) => {
                    quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::code(&(#value)))
                }
                None => value,
            };
            if columnar {
                let carrier = &analysis.gpu_bindings.carriers[column];
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

pub(crate) fn host_dependencies(
    name: &str,
    definitions: &BTreeMap<String, Definition>,
) -> BTreeSet<String> {
    host_dependencies_from(name, definitions, &BTreeMap::new())
}

pub(crate) fn host_dependencies_from(
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

pub(crate) fn origin(
    name: &str,
    definitions: &BTreeMap<String, Definition>,
) -> syn::Result<Option<Origin>> {
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
                if constants
                    .equalities
                    .iter()
                    .map(|equality| symbol_name(&equality.left))
                    .collect::<BTreeSet<_>>()
                    .len()
                    > 1
                {
                    result.predicate = true;
                    return Ok(Some(result));
                }
                for equality in &constants.equalities {
                    let EqualityValue::Literal(value) = &equality.right else {
                        return Err(error(
                            "T-Gpu: source selection must compare a code to a constant",
                        ));
                    };
                    interval(
                        &mut result.selection,
                        name,
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
            if !contiguous(
                &select.condition,
                &result.columns,
                result.selection.as_ref(),
            ) {
                result.predicate = true;
            } else {
                intervals(
                    &select.condition,
                    &result.columns,
                    &mut result.selection,
                    name,
                )?;
            }
            Ok(Some(result))
        }
        Operator::Project(project) => origin(&symbol_name(&project.input), definitions),
        _ => Ok(None),
    }
}

/// Every constraint is an ordered comparison or equality against a host value
/// on one column, so the selection is one half-open interval.
fn contiguous(theta: &Theta, columns: &BTreeMap<String, String>, prior: Option<&Interval>) -> bool {
    let mut source = prior.map(|interval| interval.column.clone());
    let mut same_column = |column: &str| {
        let Some(column) = columns.get(column) else {
            return false;
        };
        match &source {
            Some(source) => source == column,
            None => {
                source = Some(column.clone());
                true
            }
        }
    };
    if !theta.equalities.iter().all(|equality| {
        matches!(&equality.right, EqualityValue::Literal(_))
            && same_column(&symbol_name(&equality.left))
    }) {
        return false;
    }
    let variable = |term: &Term| matches!(term, Term::Variable(_));
    theta.constraints.iter().all(|constraint| match constraint {
        Constraint::Compare {
            left: Term::Variable(column),
            comparison,
            right,
        } => {
            !matches!(comparison, Comparison::Ne(_))
                && !variable(right)
                && same_column(&symbol_name(column))
        }
        Constraint::Between {
            term: Term::Variable(column),
            low,
            high,
        } => !variable(low) && !variable(high) && same_column(&symbol_name(column)),
        _ => false,
    })
}

fn interval(
    target: &mut Option<Interval>,
    definition: &str,
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
        bounds: Vec::new(),
    });
    let value: syn::Expr = syn::parse2(value)?;
    let mut bound = |side, inclusive| {
        selection.bounds.push(IntervalBound {
            definition: definition.to_owned(),
            side,
            inclusive,
            value: value.clone(),
        });
    };
    match op {
        "eq" => {
            bound(Side::Lower, true);
            bound(Side::Upper, true);
        }
        "ge" => bound(Side::Lower, true),
        "gt" => bound(Side::Lower, false),
        "lt" => bound(Side::Upper, false),
        "le" => bound(Side::Upper, true),
        _ => return Err(error("T-Gpu: provider selection is not a contiguous range")),
    }
    Ok(())
}

/// The half-open `[start, end)` of a recorded interval: its bounds folded in
/// recorded order; inclusive upper and exclusive lower bounds step by one.
pub(crate) fn interval_tokens(interval: &Interval) -> (TokenStream, TokenStream) {
    let mut start = quote!(i64::MIN);
    let mut end = quote!(i64::MAX);
    for bound in &interval.bounds {
        let value = &bound.value;
        let next = quote!((#value).checked_add(1).ok_or("T-Gpu: selection bound overflow")?);
        match (bound.side, bound.inclusive) {
            (Side::Lower, true) => start = quote!((#start).max(#value)),
            (Side::Lower, false) => start = quote!((#start).max(#next)),
            (Side::Upper, true) => end = quote!((#end).min(#next)),
            (Side::Upper, false) => end = quote!((#end).min(#value)),
        }
    }
    (start, end)
}

fn intervals(
    theta: &Theta,
    columns: &BTreeMap<String, String>,
    target: &mut Option<Interval>,
    definition: &str,
) -> syn::Result<()> {
    for equality in &theta.equalities {
        let EqualityValue::Literal(value) = &equality.right else {
            return Err(error("T-Gpu: provider equality requires a constant code"));
        };
        interval(
            target,
            definition,
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
                let column = columns[&symbol_name(column)].clone();
                interval(target, definition, column, value, op)?;
            }
            Constraint::Between {
                term: Term::Variable(column),
                low,
                high,
            } => {
                let column = columns[&symbol_name(column)].clone();
                interval(target, definition, column.clone(), host_value(low)?, "ge")?;
                interval(target, definition, column, host_value(high)?, "le")?;
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

/// Every lookup of `term` as (provider, sorted keys), not following Refs.
fn collect_lookups(term: &cq_lang_sequence::Term, demands: &mut BTreeSet<(String, Vec<String>)>) {
    use cq_lang_sequence::Term;
    match term {
        Term::Lookup {
            provider,
            keys,
            inner,
        } => {
            let mut keys: Vec<_> = keys.iter().map(symbol_name).collect();
            keys.sort();
            demands.insert((symbol_name(provider), keys));
            collect_lookups(inner, demands);
        }
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. }
        | Term::Tile { inner, .. }
        | Term::PartitionK { inner, .. } => collect_lookups(inner, demands),
        Term::Union(arms) => {
            for arm in arms {
                collect_lookups(arm, demands);
            }
        }
        Term::Choose(arms) => {
            for (_, arm) in arms {
                collect_lookups(arm, demands);
            }
        }
        Term::Rows(_) | Term::Ref(_) => {}
    }
}

/// The providers `term` itself looks up.
pub(crate) fn lookup_providers(term: &cq_lang_sequence::Term) -> BTreeSet<String> {
    let mut demands = BTreeSet::new();
    collect_lookups(term, &mut demands);
    demands.into_iter().map(|(provider, _)| provider).collect()
}

pub(crate) fn lookup_demands(program: &Program) -> BTreeSet<(String, Vec<String>)> {
    let mut demands = BTreeSet::new();
    for definition in &program.sequence.definitions {
        collect_lookups(&definition.term, &mut demands);
    }
    demands
}

/// Only reads below host require device source adapters and counts.
pub(crate) fn device_lookup_demands(
    program: &Program,
    analysis: &Analysis,
) -> BTreeSet<(String, Vec<String>)> {
    let mut demands = BTreeSet::new();
    for definition in &program.sequence.definitions {
        if analysis
            .levels
            .get(&symbol_name(&definition.name))
            .and_then(|level| level.last())
            .is_some_and(|level| level != "host")
        {
            collect_lookups(&definition.term, &mut demands);
        }
    }
    demands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_regression_equality_intersects_existing_interval() {
        let mut selection = None;
        interval(&mut selection, "s", "x".into(), quote!(2i64), "lt").unwrap();
        interval(&mut selection, "s", "x".into(), quote!(3i64), "eq").unwrap();
        let (_, end) = interval_tokens(&selection.unwrap());
        assert_eq!(
            end.to_string(),
            quote!(
                ((i64::MAX).min(2i64)).min(
                    (3i64)
                        .checked_add(1)
                        .ok_or("T-Gpu: selection bound overflow")?
                )
            )
            .to_string(),
        );
    }

    #[test]
    fn row_regression_missing_read_is_a_named_error() {
        let error = DeviceRow::default()
            .rewrite(quote!(consume(__row[7]);))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("T-Gpu: unmapped live __row[7] read"),
            "{error}"
        );
    }

    #[test]
    fn row_regression_missing_write_is_a_named_error() {
        let error = DeviceRow::default()
            .rewrite(quote!(__row[7] = 3i64;))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("T-Gpu: unmapped live __row[7] write"),
            "{error}"
        );
    }

    #[test]
    fn row_regression_assignment_rewrites_its_live_inputs() {
        let row = DeviceRow(BTreeMap::from([(0, quote!(target)), (1, quote!(source))]));
        let rewritten = row.rewrite(quote!(__row[0] = __row[1] + 1;)).unwrap();
        assert_eq!(
            rewritten.to_string(),
            quote!(target = (source + 1) as _;).to_string()
        );
    }

    #[test]
    fn row_regression_assignment_reports_missing_live_input() {
        let row = DeviceRow(BTreeMap::from([(0, quote!(target))]));
        let error = row.rewrite(quote!(__row[0] = __row[7];)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("T-Gpu: unmapped live __row[7] read"),
            "{error}"
        );
    }

    #[test]
    fn row_regression_only_explicitly_dead_writes_are_discarded() {
        let row = DeviceRow::default();
        let dead = BTreeSet::from([7]);
        assert_eq!(
            row.rewrite_with_dead(quote!(__row[7] = 3i64;), &dead)
                .unwrap()
                .to_string(),
            quote!(() ;).to_string(),
        );
        let error = row
            .rewrite_with_dead(quote!(consume(__row[7]);), &dead)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("T-Gpu: unmapped live __row[7] read"),
            "{error}"
        );
        let error = row
            .rewrite_with_dead(quote!(__row[8] = 3i64;), &dead)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("T-Gpu: unmapped live __row[8] write"),
            "{error}"
        );
    }

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
                result = let canon project filtered keep {id};
                return result as answer(id);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @device_columns input { keys 0; columns [i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                h host scan = @host_scan as ::core::iter::Map<_,_> [];
                h rows = @vec as ::std::vec::Vec<_>;
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
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
        assert!(!analysis.gpu_bindings.realization.sources.contains_key("h"));
        assert!(!rows.build.to_string().contains("__cq_sequence_uploaded_h"));
    }

    #[test]
    fn nonleading_selection_fallback_does_not_construct_a_lookup_provider() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i32,b:i32) = (&data.r);
                r = pipe rename R@0 {a -> a,b -> b};
                selected = let canon point select r if (b = 7);
                return selected as answer(a,b);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [];
                r host result(selected) = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [b];
                selected rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                selected canon = rows;
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
        lookup.sequence.definitions[1].term = syn::parse_quote!(flatten lookup r [b] input);
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
                result = let canon project joined keep {a,b};
                return result as answer(a,b);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                d host scan = @radix_relation input { keys 1; columns [i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,),(),u64,u64,()> [];
                r host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u128; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u128,u64,()> [];
                r = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u128; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u128,u64,()> [b];
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
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
        let recorded = &analysis.gpu_bindings.native["r"];
        let materialization = recorded.materialization.as_ref().unwrap();
        assert_eq!(materialization.permutation, vec![1, 0]);
        assert_eq!(recorded.source, ["a", "b"]);
        assert_eq!(recorded.radix.fields, [1, 0]);
        assert_eq!(recorded.radix.key_types, vec![syn::parse_quote!(i32), syn::parse_quote!(i32)]);
        let mut stale = analysis.gpu_bindings.clone();
        stale.native.get_mut("r").unwrap().radix.fields.swap(0, 1);
        assert!(crate::bindings::check(&program, &analysis, &stale).is_err());
        let mut missing = analysis.gpu_bindings.clone();
        missing.native.remove("r");
        assert!(crate::bindings::check(&program, &analysis, &missing).is_err());
        let mut wrong_offset = analysis.gpu_bindings.clone();
        wrong_offset.relation_inputs.values_mut().next().unwrap().offset = syn::parse_quote!(u32);
        assert!(crate::bindings::check(&program, &analysis, &wrong_offset).is_err());
        let mut wrong_capture = analysis.gpu_bindings.clone();
        wrong_capture.relation_inputs.values_mut().next().unwrap().word_capture =
            cq_lang_sequence::gpu::bindings::RadixWordCapture::InputView { host: "absent".into() };
        assert!(crate::bindings::check(&program, &analysis, &wrong_capture).is_err());
        let mut wrong_permutation_capture = analysis.gpu_bindings.clone();
        wrong_permutation_capture.native.get_mut("r").unwrap().radix.word_capture =
            cq_lang_sequence::gpu::bindings::RadixWordCapture::InputView { host: "absent".into() };
        assert!(crate::bindings::check(&program, &analysis, &wrong_permutation_capture).is_err());
        struct Permutations(Vec<Vec<usize>>, Vec<syn::Expr>);
        impl VisitMut for Permutations {
            fn visit_expr_method_call_mut(&mut self, call: &mut syn::ExprMethodCall) {
                if call.method == "permuted_provider" {
                    self.1.push(call.args.iter().nth(4).expect("recorded active window").clone());
                    let Some(syn::Expr::Reference(reference)) = call.args.iter().nth(1) else {
                        panic!("permutation argument must borrow its recorded array")
                    };
                    let syn::Expr::Array(array) = reference.expr.as_ref() else {
                        panic!("permutation must be explicit")
                    };
                    self.0.push(
                        array
                            .elems
                            .iter()
                            .map(|element| {
                                let syn::Expr::Lit(syn::ExprLit {
                                    lit: syn::Lit::Int(value),
                                    ..
                                }) = element
                                else {
                                    panic!("permutation index must be literal")
                                };
                                value.base10_parse().unwrap()
                            })
                            .collect(),
                    );
                }
                syn::visit_mut::visit_expr_method_call_mut(self, call);
            }
        }
        let build = &rows.build;
        let mut block: syn::Block = syn::parse2(quote!({ #build })).unwrap();
        let mut printed = Permutations(Vec::new(), Vec::new());
        printed.visit_block_mut(&mut block);
        assert_eq!(printed.0, vec![materialization.permutation.clone()]);
        assert_eq!(printed.1, vec![materialization.active_bits.clone()]);
        let mut full_width = analysis.clone();
        let full_bits: syn::Expr = syn::parse_quote!(<u128 as ::gpu_datalog_runtime::Word>::BITS);
        full_width.gpu_bindings.native.get_mut("r").unwrap()
            .materialization.as_mut().unwrap().active_bits = full_bits.clone();
        crate::bindings::check(&program, &full_width, &full_width.gpu_bindings).unwrap();
        let full_rows = Rows::new(&program, &full_width).unwrap();
        let build = &full_rows.build;
        let mut block: syn::Block = syn::parse2(quote!({ #build })).unwrap();
        let mut printed_full = Permutations(Vec::new(), Vec::new());
        printed_full.visit_block_mut(&mut block);
        assert_eq!(printed_full.0, printed.0);
        assert_eq!(printed_full.1, vec![full_bits]);
        let mut narrow = analysis.clone();
        narrow.gpu_bindings.native.get_mut("r").unwrap()
            .materialization.as_mut().unwrap().active_bits = syn::parse_quote!(32u32);
        assert!(Rows::new(&program, &narrow).is_err());
        let mut invalid = analysis.clone();
        invalid
            .gpu_bindings
            .native
            .get_mut("r")
            .unwrap()
            .materialization
            .as_mut()
            .unwrap()
            .permutation = vec![0, 0];
        assert!(Rows::new(&program, &invalid).is_err());
        let mut missing = analysis.clone();
        missing.gpu_bindings.native.remove("r");
        assert!(Rows::new(&program, &missing).is_err());
        let mut wrong_provider = analysis.clone();
        wrong_provider.gpu_bindings.providers.get_mut("r").unwrap()[0].provider =
            crate::providers::declaration("range_provider").instance();
        assert!(Rows::new(&program, &wrong_provider).is_err());
        let mut wrong_materialization = analysis.clone();
        wrong_materialization
            .gpu_bindings
            .native
            .get_mut("r")
            .unwrap()
            .materialization
            .as_mut()
            .unwrap()
            .provider = crate::providers::declaration("radix_relation").instance();
        assert!(Rows::new(&program, &wrong_materialization).is_err());
        let mut wide_permutation = analysis.clone();
        wide_permutation
            .gpu_bindings
            .native
            .get_mut("r")
            .unwrap()
            .materialization
            .as_mut()
            .unwrap()
            .permutation_threads = 512;
        assert!(Rows::new(&program, &wide_permutation).is_err());
        let mut missing_download = analysis.clone();
        missing_download.gpu_bindings.downloads.clear();
        assert!(Rows::new(&program, &missing_download).is_err());
        let mut missing_host_adapter = analysis.clone();
        assert!(!missing_host_adapter.gpu_bindings.host_materializations.is_empty());
        missing_host_adapter.gpu_bindings.host_materializations.clear();
        assert!(Rows::new(&program, &missing_host_adapter).is_err());
        let mut device_host_adapter = analysis.clone();
        *device_host_adapter.gpu_bindings.host_materializations.values_mut().next().unwrap() =
            crate::providers::declaration("range_provider").instance();
        assert!(Rows::new(&program, &device_host_adapter).is_err());
        let mut incomplete_payload = analysis.clone();
        incomplete_payload
            .gpu_bindings
            .providers
            .get_mut("r")
            .unwrap()[0]
            .payload
            .pop();
        assert!(Rows::new(&program, &incomplete_payload).is_err());
        let mut missing_upload = analysis.clone();
        missing_upload.gpu_bindings.uploads.clear();
        assert!(Rows::new(&program, &missing_upload).is_err());
        let mut wrong_bounds = analysis.clone();
        wrong_bounds.gpu_bindings.bounds.insert(
            "a".into(),
            cq_lang_sequence::gpu::bindings::ColumnBounds::Scalar {
                ty: syn::parse_quote!(u8),
            },
        );
        assert!(Rows::new(&program, &wrong_bounds).is_err());
    }

    #[test]
    fn binding_rejects_same_spelling_with_conflicting_codec_ids() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i32,b:i32) = (&data.r);
                relation answer(x:i32,y:i32);
                r = pipe rename R@0 {a -> x,b -> y};
                result = let canon project r keep {x,y};
                return result as answer(x,y);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @radix_relation input { keys 2; columns [i32 => ::gpu_datalog_runtime::Symbol<i32, Codec> codec [7] Codec, i32 => ::gpu_datalog_runtime::Symbol<i32, Codec> codec [8] Codec] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<
                    (::gpu_datalog_runtime::Symbol<i32, Codec>, ::gpu_datalog_runtime::Symbol<i32, Codec>), (), u64, u64, ()> [];
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
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
        assert!(crate::runs::Codecs::check(&program.structures, &BTreeMap::new()).is_err());
        assert!(cq_lang_sequence::check_grading(&program, &crate::machine()).is_err());
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
                result = let canon project joined keep {order_key,part_key,supplier_nation};
                return result as answer(order_key,part_key,supplier_nation);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sorted_run = ::cq_storage::SortedRun<_, _> { reads [exact, range, membership], supports [retain, expand, keys], build typed, join_filter, run_filter, shared, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact run_finalize stat_rows host required build single, keys: keys distinct_keys exact run_finalize stat_keys host required build single, max_group: max_group max_bucket_occurrences exact run_finalize stat_max_group host required build single, runs: runs runs exact run_finalize stat_runs host required build single ] }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                driver host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64, i64 => i64, i64 => i64, i64 => i64, i64 => i64, i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64,i64,i64,i64,i64,i64,i64)> [];
                nation host scan = @host_scan as ::core::iter::Map<_,_> [];
                nation = @sorted_run as ::cq_storage::SortedRun<_,_> [supplier_nation];
                filtered rows = @vec as ::std::vec::Vec<_>;
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
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
        // The bound publication covers Driver: no resident transfer.
        let evaluation = HostEvaluation {
            bound: Vec::new(),
            transfers: Vec::new(),
        };
        let publication = host_rows(
            &program,
            &analysis,
            "joined",
            &definitions,
            &evaluation,
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
            &evaluation,
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
            &evaluation,
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
