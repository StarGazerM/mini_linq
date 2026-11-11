//! GPU interpretation of checked Round/Schedule records on one ordered stream.
//! Providers, initializers, backing states and scratch lifetimes come from the
//! recorded device Storage; this printer composes their fixed operations.
use cq_datalog_support::ensure;
use cq_lang_datalog_round::{
    self as round, HoleInterface, InputSource, KeySet, Program, RoundContext, storage,
};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeMap;
use syn::{Ident, Type};

fn local(kind: &str, index: usize) -> Ident {
    format_ident!("__cq_gpu_{}_{}", kind, index)
}
fn device(program: &Program) -> &storage::DeviceStorage {
    program
        .storage
        .device
        .as_ref()
        .expect("checked device records")
}
fn admitted_name(relation: usize, order: usize) -> Ident {
    format_ident!("__cq_gpu_admitted_{}_{}", relation, order)
}

/// The Storage negative projection a component's key set realizes.
pub(crate) fn negation(program: &Program, component: usize, key: &KeySet) -> syn::Result<usize> {
    program
        .storage
        .negations
        .iter()
        .position(|negative| {
            negative.component == component
                && negative.relation == key.relation
                && negative.columns == key.columns
                && negative.filters == key.filters
                && negative.equalities == key.equalities
        })
        .ok_or_else(|| syn::Error::new_spanned(key, "negative key set has no Storage projection"))
}

fn memory_snapshot(
    context: &RoundContext,
    indices: &[usize],
    arenas: &[usize],
    stage: &str,
) -> TokenStream {
    let entries = indices.iter().map(|&index| {
        let relation = &context.relations[index];
        let name = relation.declaration.name.to_string();
        let canonical = local("layout", relation.providers.canonical);
        let arena = local("arena", index);
        let arena_bytes = if arenas.contains(&index) {
            quote!(#arena.allocated_bytes())
        } else {
            quote!(0usize)
        };
        let layouts = context
            .layouts
            .iter()
            .chain(&context.delta_layouts)
            .filter(|layout| layout.relation == index)
            .map(|layout| &layout.host)
            .collect::<Vec<_>>();
        quote!(__cq_gpu_memory_usage[#index] = ::gpu_datalog_runtime::MemoryUse {
            name: #name, rows: #canonical.len(), accepted_live_bytes: #canonical.live_bytes(),
            accepted_bytes: #canonical.allocated_bytes(),
            layout_live_bytes: 0usize #(+ #layouts.live_bytes())*,
            layout_bytes: 0usize #(+ #layouts.allocated_bytes())*,
            arena_bytes: #arena_bytes,
        };)
    });
    quote!(if __cq_gpu_trace_memory {
        #(#entries)*
        ::gpu_datalog_runtime::trace_memory(&__cq_gpu_pool, #stage, &__cq_gpu_memory_usage);
    })
}

/// Bind `fresh` to the sorted unique rows of `candidates` (relation order)
/// absent from the committed canonical layout, in canonical column order.
/// The canonical state is read, not changed: publication appends separately.
fn admission(
    program: &Program,
    context: &RoundContext,
    index: usize,
    candidates: &Ident,
    fresh: &Ident,
) -> TokenStream {
    let canonical = context.relations[index].providers.canonical;
    let threads = device(program).layouts[canonical].permutation_threads;
    let state = local("layout", canonical);
    let packing = local("packing", index);
    let record = &device(program).relations[index];
    let order = &record.admitted_orders[0].columns;
    let ordered = match record.candidate_order.to_string().as_str() {
        "canonical" => quote!(),
        "permutation" => quote!(
            let #candidates = #packing.permute_device(&#candidates, #candidates.len(), &[#(#order),*], #threads)?;
        ),
        _ => unreachable!("checked candidate order certificate"),
    };
    quote!(#ordered
        // SAFETY: declared packing initializes exactly the recorded codec bit window.
        let #fresh = unsafe { #state.fresh(&#candidates, #candidates.len())? };
    )
}

/// Materialize each recorded admitted order once; full and delta views share it.
fn admitted_orders(program: &Program, index: usize, fresh: &Ident) -> TokenStream {
    let canonical = program.context.relations[index].providers.canonical;
    let threads = device(program).layouts[canonical].permutation_threads;
    let state = local("layout", canonical);
    let packing = local("layout_packing", canonical);
    let orders = device(program).relations[index].admitted_orders.iter().enumerate().map(|(i, order)| {
        let name = admitted_name(index, i);
        let permutation = &order.permutation;
        let build = match order.certificate.to_string().as_str() {
            "sorted_unique" => quote!(#fresh.clone()),
            "unique_permutation" => quote!({
                let permuted = #packing.permute_device(
                    #fresh.buffer(), #fresh.len(), &[#(#permutation),*], #threads)?;
                // SAFETY: the checked full-heading bijection preserves uniqueness
                // and the same declared active bit width, but not ordering.
                unsafe { #state.sort_known_unique(&permuted, #fresh.len())? }
            }),
            _ => unreachable!("checked admitted input certificate"),
        };
        quote!(let #name = #build;)
    });
    quote!(#(#orders)*)
}

/// Bulk `full_layouts` maintenance: append the fresh rows to each layout's
/// recorded radix state in its own order, then republish its immutable view.
/// Unchanged snapshots and field descriptors are shared.
fn full_layouts(
    program: &Program,
    context: &RoundContext,
    index: usize,
    layouts: &[usize],
) -> TokenStream {
    let updates = layouts.iter().map(|&j| {
        let layout = &context.layouts[j];
        let state = local("layout", j);
        let host = &layout.host;
        let run = admitted_name(index, device(program).layouts[j].admitted_order.expect("checked admitted order"));
        quote!({
            let run = #run.clone();
            // SAFETY: fresh rows are absent from the canonical layout, and every
            // full layout holds exactly the canonical rows in its own order.
            unsafe { #state.append_sorted(run)?; }
            #host = #host.republish(&#state)?;
        })
    });
    quote!(#(#updates)*)
}

fn concatenate(
    program: &Program,
    relation: usize,
    columns: Option<&[usize]>,
    word: &Type,
    packing: TokenStream,
    batches: TokenStream,
) -> syn::Result<TokenStream> {
    if !crate::storage::uses_domains(&program.storage) {
        return Ok(quote!(::gpu_datalog_runtime::concatenate_batches::<#word>(
            __cq_gpu_pool.clone(), #packing, #batches)?));
    }
    let domains = crate::domains::DomainComponents::infer(&program.storage.incremental.strata.source)?;
    let input = &device(program).relations[relation].input;
    let columns: Vec<_> = columns.map_or_else(|| (0..input.columns.len()).collect(), |columns| columns.to_vec());
    let ids: Vec<_> = columns.iter().map(|&column| domains.columns[relation][column]).collect();
    let types = columns.iter().map(|&column| &input.columns[column].storage);
    let fields = types.map(|ty| quote!(<#ty as ::gpu_datalog_runtime::Scalar>::COLUMN));
    Ok(quote!({
        let source_packing = ::gpu_datalog_runtime::Packing::with_columns(&[#(#fields),*])?;
        let codes = ::gpu_datalog_runtime::concatenate_batches::<#word>(
            __cq_gpu_pool.clone(), &source_packing, #batches)?;
        __cq_gpu_domains.repack_codes(&codes, &source_packing, &[#(#ids),*])?
    }))
}

/// One member's admission and recorded publication transaction.
fn admit_publish(
    program: &Program,
    index_in_stratum: usize,
    transaction: &round::Publication,
    phase: &str,
) -> syn::Result<TokenStream> {
    let context = &program.context;
    let i = transaction.relation;
    let relation = &context.relations[i];
    let record = &device(program).relations[i];
    let word = &record.word;
    let packing = local("packing", i);
    let candidates = local("candidates", i);
    let fresh = local("fresh", i);
    let buffer = format_ident!("__cq_gpu_candidate_rows");
    let concatenate = concatenate(program, i, None, word, quote!(&#packing), quote!(&#candidates))?;
    let admit = admission(program, context, i, &buffer, &fresh);
    let orders = admitted_orders(program, i, &fresh);
    let relation_name = relation.declaration.name.to_string();
    let canonical = local("layout", relation.providers.canonical);
    let mut operations = Vec::new();
    for operation in &transaction.operations {
        operations.push(match operation.kind.to_string().as_str() {
            "full_layouts" => full_layouts(program, context, i, &operation.layouts),
            "delta_swap" => {
                let delta = relation.delta_host.as_ref().expect("checked delta");
                let next = &relation.next;
                let ty = &relation.providers.next.ty;
                let run = admitted_name(i, record.relation_order);
                // Recorded `fresh_run` NEW: the fresh rows in relation order,
                // built directly; publishing it as the next delta is a move.
                quote!(
                    let #next: #ty = #delta.with_sorted(#run.clone(), #canonical.params())?;
                    #delta = #next;
                )
            }
            "delta_rebuild" => {
                let rebuilds = operation.layouts.iter().map(|&j| {
                    let layout = &context.delta_layouts[j];
                    let state = local("delta_layout", j);
                    let host = &layout.host;
                    let run = admitted_name(i, device(program).delta_layouts[j].admitted_order.expect("checked admitted order"));
                    quote!(#host = #host.with_sorted(#run.clone(), #state.params())?;)
                });
                quote!(#(#rebuilds)*)
            }
            _ => unreachable!("checked publication operation"),
        });
    }
    Ok(quote!({
        let #fresh = {
            let #buffer = #concatenate;
            ::std::mem::drop(#candidates);
            let candidates = #buffer.len();
            #admit
            if ::gpu_datalog_runtime::trace_kernels() {
                ::std::eprintln!("gpu_datalog_admission\tstratum={}\tphase={}\tround={}\trelation={}\tcandidates={}\tadmitted={}",
                    #index_in_stratum, #phase, __cq_gpu_round, #relation_name, candidates, #fresh.len());
            }
            #fresh
        };
        #orders
        #(#operations)*
    }))
}

/// Replace each host identifier read by a hole with a dereferenced reference
/// parameter, recording `(parameter, host, type)` once per distinct host.
fn substitute(
    tokens: TokenStream,
    hosts: &std::collections::BTreeMap<String, Type>,
    parameters: &mut Vec<(Ident, Ident, Type)>,
) -> TokenStream {
    use proc_macro2::{Group, TokenTree};
    tokens
        .into_iter()
        .map(|token| match token {
            TokenTree::Ident(name) if hosts.contains_key(&name.to_string()) => {
                let position = parameters
                    .iter()
                    .position(|(_, host, _)| *host == name)
                    .unwrap_or_else(|| {
                        let parameter = format_ident!("__cq_gpu_input_{}", parameters.len());
                        parameters.push((
                            parameter,
                            name.clone(),
                            hosts[&name.to_string()].clone(),
                        ));
                        parameters.len() - 1
                    });
                let parameter = &parameters[position].0;
                quote!((*#parameter))
            }
            TokenTree::Group(group) => {
                let mut replacement = Group::new(
                    group.delimiter(),
                    substitute(group.stream(), hosts, parameters),
                );
                replacement.set_span(group.span());
                TokenStream::from(TokenTree::Group(replacement))
            }
            token => TokenStream::from(token),
        })
        .collect()
}

/// Whether a hole's query names anything bound outside the query, such as a
/// local of the function containing `datalog!`. Host declaration values are
/// substituted separately. Callees (`sym("…")`) are items visible to nested
/// functions; macros are opaque and conservatively count as environment reads.
pub(crate) fn reads_environment(cq: &round::cq::Program) -> bool {
    use round::cq::BodyItem;
    use syn::visit::Visit;
    struct Callees(std::collections::BTreeSet<String>);
    impl<'ast> Visit<'ast> for Callees {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = &*call.func {
                if let Some(name) = path.path.get_ident() {
                    self.0.insert(cq_syntax::symbol_name(name));
                }
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    let mut bound = std::collections::BTreeSet::new();
    let mut expressions: Vec<&syn::Expr> = cq.query.head.args.iter().collect();
    expressions.extend(
        cq.query
            .captures
            .items
            .iter()
            .flat_map(cq_syntax::Capture::expressions),
    );
    for item in &cq.query.body {
        match item {
            BodyItem::Positive { atom } => {
                for argument in &atom.args {
                    match cq_syntax::rust::variable(argument) {
                        Some(variable) => {
                            bound.insert(cq_syntax::symbol_name(variable));
                        }
                        None => expressions.push(argument),
                    }
                }
            }
            BodyItem::Negation(negation) => expressions.extend(&negation.atom.args),
            BodyItem::Filter(filter) => expressions.push(&filter.condition),
            BodyItem::Let(binding) => {
                bound.extend(
                    cq_syntax::rust::bindings(&binding.pattern)
                        .iter()
                        .map(cq_syntax::symbol_name),
                );
                expressions.push(&binding.expression);
            }
            BodyItem::For(binding) => {
                bound.extend(
                    cq_syntax::rust::bindings(&binding.pattern)
                        .iter()
                        .map(cq_syntax::symbol_name),
                );
                expressions.push(&binding.expression);
            }
            _ => return true,
        }
    }
    expressions.into_iter().any(|expression| {
        let free = cq_syntax::rust::free_variables(expression);
        let mut callees = Callees(std::collections::BTreeSet::new());
        callees.visit_expr(expression);
        free.opaque
            || free
                .names
                .iter()
                .any(|name| !bound.contains(name) && !callees.0.contains(name))
    })
}

/// A standalone hole's function item and the hosts it reads, in parameter order.
struct HoleCall {
    function: Ident,
    hosts: Vec<Ident>,
}

/// Formal rule P-DL-Gpu: interpret the recorded waves, admission owners,
/// publication placement and transactions over the recorded device states on
/// one device stream. Admission is fresh = unique(candidates) minus the
/// committed canonical layout; publication appends fresh rows to every layout.
/// Constructors consume recorded merge tiles and auxiliary geometry; worker
/// arenas use the same recorded initializers and scratch release as the caller.
pub fn compile(program: &Program) -> syn::Result<syn::Expr> {
    crate::storage::check_device(&program.storage)?;
    round::contract::check(program)?;
    crate::bindings::check(program)?;
    crate::lanes::check(program)?;
    ensure(
        program.context.observer.is_none(),
        program,
        "GPU Datalog does not support observer callbacks",
    )?;
    ensure(
        program.policy.timing == "none",
        program,
        "GPU Datalog does not support recorded phase timing",
    )?;
    ensure(
        program.execution.is_empty(),
        program,
        "GPU Datalog does not support host execution certificates",
    )?;
    for hole in &program.holes {
        ensure(
            hole.interface == HoleInterface::Batch,
            hole,
            "GPU Datalog requires batch holes",
        )?;
    }
    for rule in program
        .context
        .strata
        .iter()
        .flat_map(|stratum| stratum.seeds.iter().chain(&stratum.steps))
    {
        ensure(
            rule.known == round::Admission::Admit,
            rule,
            "GPU Datalog admission is at admit only (ADM-InPlan GPU follow-up)",
        )?;
    }
    ensure(
        program
            .schedule
            .strata
            .iter()
            .all(|stratum| stratum.policy.is_empty()),
        &program.schedule,
        "GPU Datalog runs each hole as one device invocation; hole grain and inline advice are unsupported",
    )?;
    let context = &program.context;
    let device = device(program);
    ensure(
        device.scratch.arenas == "component" && device.scratch.release == "component",
        program,
        "GPU Datalog supports only component-scoped candidate arenas and scratch release",
    )?;
    // NEW is built from admission's fresh rows; its declared empty `next_init`
    // is checked by the device refinement and never printed.
    for relation in &device.relations {
        ensure(
            relation.next == "fresh_run",
            &relation.next,
            "GPU Datalog builds NEW only from fresh rows",
        )?;
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        ensure(
            layout.protocol == storage::LayoutProtocol::Bulk,
            &layout.name,
            "GPU Datalog maintains bulk device layouts only",
        )?;
    }
    for output in &context.outputs {
        ensure(
            output.conversion == "borrowed_cloned_into_iter",
            output,
            "GPU Datalog outputs read borrowed canonical rows",
        )?;
    }
    let symbols = crate::storage::Symbols::from_program(&program.storage)?;
    let domains = crate::storage::uses_domains(&program.storage)
        .then(|| crate::domains::DomainComponents::infer(&program.storage.incremental.strata.source)).transpose()?;
    // Hosts a hole may read through its CQ declarations, with their providers.
    let mut hosts = BTreeMap::<String, Type>::new();
    for relation in &context.relations {
        if let Some(delta) = &relation.delta_host {
            hosts.insert(delta.to_string(), relation.providers.delta.ty.clone());
        }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        hosts.insert(layout.host.to_string(), layout.provider.ty.clone());
    }
    // The relation whose candidate arena each hole's terminal fills.
    let mut owners = vec![0usize; program.holes.len()];
    let mut scans = BTreeMap::new();
    for stratum in &context.strata {
        for key in &stratum.keys {
            hosts.insert(key.name.to_string(), key.provider.ty.clone());
            owners[key.hole] = key.relation;
        }
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            owners[rule.hole] = rule.head;
            if let Some(scan) = &rule.scan {
                let ty = match scan.source {
                    InputSource::Delta(relation) => &context.relations[relation].providers.delta.ty,
                    InputSource::Layout(layout) => &context.layouts[layout].provider.ty,
                };
                scans.insert(rule.hole, (&scan.binding, ty));
            }
        }
    }
    // Each hole is a separate function item: rustc type-checks a closure with
    // its enclosing body, so closures would keep the whole program one query.
    // A hole whose query reads a local of the code around `datalog!` keeps the
    // closure form, because a function item cannot capture locals.
    let mut hole_items = Vec::new();
    let mut hole_index = 0usize;
    // Standalone hole items with the hosts they read, for DL-WaveLanes workers.
    let mut calls: Vec<Option<HoleCall>> = Vec::new();
    let holes = cq_datalog_print::splice_with(program, |hole| {
        let index = hole_index;
        hole_index += 1;
        let standalone = !reads_environment(&hole.cq);
        calls.push(None);
        let mut cq = hole.cq.clone();
        let mut parameters = Vec::new();
        if standalone {
            if domains.is_some() {
                parameters.push((
                    format_ident!("__cq_gpu_domains"), format_ident!("__cq_gpu_domains"),
                    syn::parse_quote!(::std::sync::Arc<::gpu_datalog_runtime::DomainCatalog>),
                ));
            }
            for declaration in &mut cq.declarations {
                if let Some(value) = &mut declaration.value {
                    let expression = value.expression.to_token_stream();
                    value.expression =
                        syn::parse2(substitute(expression, &hosts, &mut parameters))?;
                }
            }
        }
        let invocation = hole.invocation_pipeline()?.start(cq.to_token_stream())?;
        if !standalone {
            return Ok(invocation);
        }
        let function = format_ident!("__cq_gpu_hole_{}", index);
        let (scan_parameter, scan_argument) = match scans.get(&index) {
            Some((binding, ty)) => {
                (quote!(#binding: &#ty,), quote!(#binding,))
            }
            None => (quote!(), quote!()),
        };
        let arena = &device.relations[owners[index]].arena;
        let declared = parameters
            .iter()
            .map(|(parameter, _, ty)| quote!(#parameter: &#ty));
        let arguments = parameters.iter().map(|(_, host, _)| quote!(&#host));
        hole_items.push(quote!(
            fn #function(
                __cq_gpu_arena: &mut #arena,
                #scan_parameter #(#declared,)*
            ) -> ::gpu_datalog_runtime::CandidateBatch {
                #invocation
            }
        ));
        calls[index] = Some(HoleCall {
            function: function.clone(),
            hosts: parameters.iter().map(|(_, host, _)| host.clone()).collect(),
        });
        Ok(quote!(#function(&mut *__cq_gpu_arena, #scan_argument #(#arguments,)*)))
    })?;
    // Setup and output run through the same partitioned runners as strata, so
    // the program body keeps only the owning locals rather than every
    // relation's construction, upload, permutation and error path.
    let mut slot = 0usize;
    let mut program_runner = || {
        let partition = format_ident!("partition_{}", slot % 16);
        slot += 1;
        quote!(::cq_gpu_sequence_runtime::__codegen::#partition::run)
    };
    let mut initialize = Vec::new();
    for (index, (layout, record)) in context.layouts.iter().zip(&device.layouts).enumerate() {
        let (state, packing, host) = (
            local("layout", index),
            local("layout_packing", index),
            &layout.host,
        );
        let (state_ty, state_init, packing_init, ty, init) = (
            &record.state,
            &record.state_init,
            &record.packing,
            &layout.provider.ty,
            &record.init,
        );
        let runner = program_runner();
        initialize.push(quote!(
            let (#packing, mut #state, mut #host) = #runner(#[inline] || -> ::gpu_datalog_runtime::Result<(_, #state_ty, #ty)> {
                let __cq_gpu_packing = #packing_init;
                let __cq_gpu_state: #state_ty = #state_init;
                let __cq_gpu_view: #ty = #init;
                Ok((__cq_gpu_packing, __cq_gpu_state, __cq_gpu_view))
            })?;
        ));
    }
    for (index, (layout, record)) in context
        .delta_layouts
        .iter()
        .zip(&device.delta_layouts)
        .enumerate()
    {
        let (state, packing, host) = (
            local("delta_layout", index),
            local("delta_packing", index),
            &layout.host,
        );
        let (state_ty, state_init, packing_init, ty, init) = (
            &record.state,
            &record.state_init,
            &record.packing,
            &layout.provider.ty,
            &record.init,
        );
        let runner = program_runner();
        initialize.push(quote!(
            let (#packing, #state, mut #host) = #runner(#[inline] || -> ::gpu_datalog_runtime::Result<(_, #state_ty, #ty)> {
                let __cq_gpu_packing = #packing_init;
                let __cq_gpu_state: #state_ty = #state_init;
                let __cq_gpu_view: #ty = #init;
                Ok((__cq_gpu_packing, __cq_gpu_state, __cq_gpu_view))
            })?;
        ));
    }
    for (index, (relation, record)) in context.relations.iter().zip(&device.relations).enumerate() {
        let packing = local("packing", index);
        let packing_init = &record.packing;
        let runner = program_runner();
        initialize.push(quote!(
            let #packing = #runner(#[inline] || -> ::gpu_datalog_runtime::Result<::gpu_datalog_runtime::Packing> {
                Ok(#packing_init)
            })?;
        ));
        if let Some(delta) = &relation.delta_host {
            let (ty, init) = (&relation.providers.delta.ty, &relation.providers.delta_init);
            let active_bits = &device.layouts[relation.providers.canonical].active_bits;
            let runner = program_runner();
            initialize.push(quote!(
                let mut #delta: #ty = #runner(#[inline] || -> ::gpu_datalog_runtime::Result<#ty> {
                    let __cq_gpu_active_bits = #active_bits;
                    Ok(#init)
                })?;
            ));
        }
    }
    // EDB loading is one admission against the empty canonical layout followed
    // by publication into every full layout of the relation.
    for (index, relation) in context.relations.iter().enumerate() {
        let Some(value) = &relation.declaration.value else {
            continue;
        };
        let expression = &value.expression;
        let packing = local("packing", index);
        let word = &device.relations[index].word;
        let fields = relation.declaration.columns.iter().enumerate().map(|(column, declared)| {
            let field = syn::Index::from(column);
            let ty = &declared.ty;
            match symbols.codec(declared.logical_type_id) {
                Some(codec) => quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::code(row.#field) as i128),
                None => quote!(*row.#field as i128),
            }
        });
        let (upload_packing, remap) = if let Some(domains) = &domains {
            let types = device.relations[index].input.columns.iter().map(|column| &column.storage);
            let columns = types.map(|ty| quote!(<#ty as ::gpu_datalog_runtime::Scalar>::COLUMN));
            let ids = &domains.columns[index];
            (
                quote!(let __cq_gpu_source_packing = ::gpu_datalog_runtime::Packing::with_columns(&[#(#columns),*])?;),
                quote!(let __cq_gpu_upload = __cq_gpu_domains.remap(&__cq_gpu_upload, &__cq_gpu_source_packing, &[#(#ids),*])?;),
            )
        } else { (quote!(), quote!()) };
        let source_packing = if domains.is_some() { quote!(__cq_gpu_source_packing) } else { quote!(#packing) };
        let upload = format_ident!("__cq_gpu_upload");
        let fresh = local("fresh", index);
        let admit = admission(program, context, index, &upload, &fresh);
        let orders = admitted_orders(program, index, &fresh);
        let layouts = context
            .layouts
            .iter()
            .enumerate()
            .filter(|(_, layout)| layout.relation == index)
            .map(|(j, _)| j)
            .collect::<Vec<_>>();
        let publish = full_layouts(program, context, index, &layouts);
        let runner = program_runner();
        initialize.push(quote!(
            #runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> {
                let host = &(#expression);
                #upload_packing
                let words = ::cq_storage::rows::Rows::rows(host)
                    .map(|row| #source_packing.pack::<#word>(&[#(#fields),*]))
                    .collect::<::gpu_datalog_runtime::Result<::std::vec::Vec<_>>>()?;
                let #upload = ::gpu_datalog_runtime::Buffer::from_host_async(__cq_gpu_pool.clone(), &words)?;
                #remap
                #admit
                #orders
                #publish
                Ok(())
            })?;
        ));
    }
    // DL-WaveLanes: recorded streams beyond the program's own run persistent
    // lane workers; their arenas are component-scoped like the main arenas.
    let streams = program.schedule.streams;
    let (create_lanes, release_lanes) = if streams > 1 {
        let initializers = device
            .relations
            .iter()
            .enumerate()
            .map(|(relation, record)| {
                let (ty, init) = (&record.arena, &record.arena_init);
                quote!(#relation => { let arena: #ty = #init; arena })
            });
        (
            quote!(let __cq_gpu_lanes = ::gpu_datalog_runtime::Lanes::new(
                &__cq_gpu_pool, #streams,
                |relation, __cq_gpu_pool| match relation {
                    #(#initializers,)*
                    _ => unreachable!("checked lane head relation"),
                },
                ::cq_gpu_sequence_runtime::release_cached_scratch,
            )?;),
            quote!(__cq_gpu_lanes.release()?;),
        )
    } else {
        (quote!(), quote!())
    };
    let mut strata = Vec::new();
    for (index, stratum) in context.strata.iter().enumerate() {
        let mut keys = Vec::new();
        for key in &stratum.keys {
            let name = &key.name;
            let ty = &key.provider.ty;
            let record = &device.negations[negation(program, index, key)?];
            let (word, packing, state_ty, state_init) = (
                &record.word,
                &record.packing,
                &record.state,
                &record.state_init,
            );
            let relation = &device.relations[key.relation];
            let (arena_ty, arena_init) = (&relation.arena, &relation.arena_init);
            let invocation = &holes[key.hole];
            let concatenate = concatenate(program, key.relation, Some(&key.columns),
                word, quote!(&packing), quote!(&[batch]))?;
            keys.push(quote!(let #name: #ty = {
                let mut arena: #arena_ty = #arena_init;
                let __cq_gpu_arena = &mut arena;
                let batch = #invocation;
                let packing = #packing;
                let buffer = #concatenate;
                let state: #state_ty = #state_init;
                // SAFETY: candidate concatenation used this checked projected packing.
                let run = unsafe { state.sort_unique(&buffer, buffer.len())? };
                <#ty>::from_sorted(__cq_gpu_pool.clone(), packing, state.params(), run, state.active_bits())?
            };));
        }
        // Each stratum and phase runs as its own non-inlined function. Relation
        // states stay in the program body and are captured by reference, so
        // neither LLVM nor their error paths see one program-sized function.
        let runner = |offset: usize| {
            let partition = format_ident!("partition_{}", (3 * index + offset) % 16);
            quote!(::cq_gpu_sequence_runtime::__codegen::#partition::run)
        };
        let (stratum_runner, seed_runner, step_runner) = (runner(0), runner(1), runner(2));
        let seed = phase(program, index, round::schedule::SEED, &holes, &calls)?;
        let seed = quote!(#seed_runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> { #seed Ok(()) })?;);
        let step = phase(program, index, round::schedule::STEP, &holes, &calls)?;
        let step = quote!(#step_runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> { #step Ok(()) })?;);
        let lengths = stratum
            .termination
            .iter()
            .map(|&i| {
                let delta = context.relations[i]
                    .delta_host
                    .as_ref()
                    .expect("checked delta");
                quote!(#delta.len())
            })
            .collect::<Vec<_>>();
        let condition = match stratum.termination_test.to_string().as_str() {
            "sum_delta_lengths" => quote!(0usize #(+ #lengths)* != 0),
            "any_delta" => quote!(false #(|| #lengths != 0)*),
            _ => unreachable!("checked termination"),
        };
        let rounds = match stratum.loop_form.to_string().as_str() {
            "while_nonempty" => quote!(while #condition { #step }),
            "loop_until_empty" => quote!(loop { if !(#condition) { break; } #step }),
            _ => unreachable!("checked loop"),
        };
        // Recorded scratch policy: candidate arenas live for the component, and
        // cached scratch and unused pool storage are released after it.
        let arenas = stratum.members.iter().map(|&i| {
            let arena = local("arena", i);
            let (ty, init) = (&device.relations[i].arena, &device.relations[i].arena_init);
            quote!(let mut #arena: #ty = #init;)
        });
        let released = memory_snapshot(
            context,
            &stratum.members,
            &[],
            &format!("stratum={index}/released"),
        );
        strata.push(quote!({
            #stratum_runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> {
                #(#arenas)*
                #(#keys)*
                let mut __cq_gpu_round = 0usize;
                #seed
                #rounds
                Ok(())
            })?;
            #release_lanes
            ::cq_gpu_sequence_runtime::release_cached_scratch(&__cq_gpu_pool);
            __cq_gpu_pool.release_unused()?;
            #released
        }));
    }
    let outputs = context.outputs.iter().map(|output| {
        let relation = &context.relations[output.relation];
        let canonical = &context.layouts[relation.providers.canonical];
        let host = &canonical.host;
        let at = &device.relations[output.relation].output_permutation;
        let fields = relation.declaration.columns.iter().zip(at).enumerate().map(|(column, (declared, position))| {
            let ty = &declared.ty;
            match symbols.codec(declared.logical_type_id) {
                Some(codec) if domains.is_some() => {
                    let id = domains.as_ref().unwrap().columns[output.relation][column];
                    // DECODE P: final DOOP output uses its field's recorded component inverse.
                    quote!(__cq_gpu_domains.decode::<#ty, #codec>(#id, row[#position] as u32)
                        .expect("invalid dense symbol code in final output"))
                }
                // DECODE P: generated SymbolCodec inverse resolves final datalog host output fields.
                Some(codec) => quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::decode(row[#position] as u32)),
                // DECODE P: generated numeric carrier inversion constructs final host output fields.
                None => quote!(row[#position] as #ty),
            }
        });
        // Borrowed canonical rows cross to the host only at the output boundary,
        // where logical host symbols are restored.
        let runner = program_runner();
        quote!(#runner(#[inline] || -> ::gpu_datalog_runtime::Result<_> {
            // DECODE P: invert canonical radix key fields only at final output.
            Ok(#host.to_rows()?.into_iter()
                .map(|row| (#(#fields,)*))
                .collect::<::std::vec::Vec<_>>()
                .into_iter())
        })?)
    }).collect::<Vec<_>>();
    let result = if outputs.len() == 1 {
        outputs[0].clone()
    } else {
        quote!((#(#outputs,)*))
    };
    let relation_count = context.relations.len();
    let indices = (0..relation_count).collect::<Vec<_>>();
    let initialized = memory_snapshot(context, &indices, &[], "initialized");
    let initialized_runner = program_runner();
    let initialized = quote!(#initialized_runner(#[inline] || { #initialized }););
    syn::parse2(quote!({
        #(#hole_items)*
        (|| -> ::gpu_datalog_runtime::Result<_> {
            let __cq_gpu_pool = ::gpu_datalog_runtime::thread_pool()?;
            #create_lanes
            ::gpu_datalog_runtime::phases::start();
            let __cq_gpu_trace_rounds = ::std::env::var_os("GPU_DATALOG_TRACE_ROUNDS").is_some();
            let __cq_gpu_trace_memory = ::std::env::var_os("GPU_DATALOG_TRACE_MEMORY").is_some();
            ::gpu_datalog_runtime::reset_work_counters();
            let mut __cq_gpu_memory_usage = if __cq_gpu_trace_memory {
                ::std::vec![::gpu_datalog_runtime::MemoryUse::default(); #relation_count]
            } else { ::std::vec::Vec::new() };
            #(#initialize)*
            #initialized
            ::gpu_datalog_runtime::phases::mark(&__cq_gpu_pool, "setup")?;
            ::gpu_datalog_runtime::trace_work("setup");
            ::gpu_datalog_runtime::reset_work_counters();
            #(#strata)*
            ::gpu_datalog_runtime::phases::mark(&__cq_gpu_pool, "fixpoint")?;
            ::gpu_datalog_runtime::trace_work("fixpoint");
            let __cq_gpu_result = #result;
            ::gpu_datalog_runtime::phases::mark(&__cq_gpu_pool, "output")?;
            Ok(__cq_gpu_result)
        })().expect("GPU Datalog execution failed")
    }))
}

fn phase(
    program: &Program,
    index: usize,
    phase: &str,
    holes: &[TokenStream],
    calls: &[Option<HoleCall>],
) -> syn::Result<TokenStream> {
    let context = &program.context;
    let device = device(program);
    let stratum = &context.strata[index];
    let schedule = &program.schedule.strata[index];
    let rules = if phase == round::schedule::SEED {
        &stratum.seeds
    } else {
        &stratum.steps
    };
    let batches = stratum.members.iter().map(|&i| {
        let candidates = local("candidates", i);
        let (ty, init) = (
            &device.relations[i].candidates,
            &device.relations[i].candidates_init,
        );
        quote!(let mut #candidates: #ty = #init;)
    });
    // Recorded earliest safe publication wave per member in this phase; a
    // phase without waves admits and publishes every member after its holes.
    let after = |relation: usize| {
        schedule
            .publication
            .iter()
            .find(|entry| entry.phase == phase && entry.relation == relation)
            .map(|entry| entry.wave)
    };
    let mut published = std::collections::BTreeSet::new();
    let mut steps = Vec::new();
    for (global, wave) in schedule.waves.iter().enumerate() {
        if wave.phase != phase {
            continue;
        }
        // P-DL-Gpu prints recorded lanes only (DL-WaveLanes): lane 0 runs on
        // this thread and stream, later lanes on the program's lane workers.
        let laned = schedule
            .lanes
            .iter()
            .find(|entry| entry.wave == global && entry.lanes.len() > 1);
        let position = |hole: usize| {
            wave.holes
                .iter()
                .position(|&h| h == hole)
                .expect("checked lane partition")
        };
        let mut launches = Vec::new();
        let main_lane: &[usize] = laned.map_or(&wave.holes, |entry| &entry.lanes[0]);
        for &rule_index in main_lane {
            let rule = &rules[rule_index];
            let candidates = local("candidates", rule.head);
            let deliver = match laned {
                Some(_) => {
                    let at = position(rule_index);
                    quote!(__cq_gpu_wave[#at] = Some(__cq_gpu_batch);)
                }
                None => quote!(#candidates.push(__cq_gpu_batch);),
            };
            let arena = local("arena", rule.head);
            let invocation = &holes[rule.hole];
            let hole = rule.hole;
            let head_name = context.relations[rule.head].declaration.name.to_string();
            let scan = rule.scan.as_ref().map(|scan| {
                let binding = &scan.binding;
                let source = match scan.source {
                    InputSource::Delta(i) => context.relations[i]
                        .delta_host
                        .as_ref()
                        .expect("checked delta"),
                    InputSource::Layout(i) => &context.layouts[i].host,
                };
                quote!(let #binding = &#source;)
            });
            let counter_scope = std::env::var_os("CQ_EXPANSION_COUNTERS").is_some().then(|| quote! {
                let __cq_expansion_capture = ::cq_gpu_runtime::expansions::Capture::begin();
                let __cq_expansion_previous_timing = ::cq_gpu_runtime::arena_timing_enabled();
                if ::gpu_datalog_runtime::trace_kernels() {
                    ::cq_gpu_runtime::enable_arena_timing(true);
                }
            });
            let counter_report = if std::env::var_os("CQ_EXPANSION_COUNTERS").is_some() {
                let variant = rule.variant;
                let packing = local("packing", rule.head);
                let word = &device.relations[rule.head].word;
                let source_rule = program.storage.incremental.variants[variant].rule;
                let delta = program.storage.incremental.variants[variant]
                    .delta
                    .map_or_else(|| "seed".to_owned(), |position| position.to_string());
                let rule_name = program.storage.incremental.strata.source.labels[source_rule]
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("{}/{}", head_name, program.holes[hole].cq.name));
                let rows = format_ident!("__cq_expansion_candidates");
                let fresh = format_ident!("__cq_expansion_fresh");
                let admit = admission(program, context, rule.head, &rows, &fresh);
                let concatenate = concatenate(program, rule.head, None, word, quote!(&#packing),
                    quote!(::core::slice::from_ref(&__cq_gpu_batch)))?;
                Some(quote! {
                    if ::gpu_datalog_runtime::trace_kernels() {
                        if let Some(timing) = ::cq_gpu_runtime::take_last_timings() {
                            ::std::eprintln!("gpu_datalog_hole_timing\trule={}\tvariant={}\tdelta={}\titeration={}\thole={}\tkernel_ms={:.6}\ttotal_ms={:.6}\twall_ms={:.6}",
                                #rule_name, #variant, #delta, __cq_gpu_round, #hole,
                                timing.kernel_ms, timing.total_ms, timing.wall_ms);
                        }
                        ::cq_gpu_runtime::enable_arena_timing(__cq_expansion_previous_timing);
                    }
                    // SQ-ExpansionCounters: this rule's fresh rows against the
                    // committed canonical layout, without publishing them.
                    let #rows = #concatenate;
                    #admit
                    __cq_expansion_capture.report(#rule_name, #variant, __cq_gpu_round, #fresh.len());
                })
            } else {
                None
            };
            launches.push(quote!({
                #scan
                if __cq_gpu_trace_memory {
                    let memory = __cq_gpu_pool.memory_stats();
                    ::std::eprintln!("gpu_datalog_hole\tstratum={}\tphase={}\thole={}\thead={}\tcurrent_bytes={}\treserved_bytes={}\tarena_bytes={}",
                        #index, #phase, #hole, #head_name, memory.current_bytes, memory.reserved_bytes, #arena.allocated_bytes());
                }
                // GPU-ReservePrevious: the capacity hint is this Round hole's own.
                let __cq_gpu_arena = #arena.hole(#hole);
                #counter_scope
                let __cq_gpu_batch = #invocation;
                #counter_report
                if __cq_gpu_trace_memory {
                    ::std::eprintln!("gpu_datalog_hole_result\tstratum={}\tphase={}\thole={}\trows={}\tcapacity={}\tbytes={}",
                        #index, #phase, #hole, __cq_gpu_batch.len(), __cq_gpu_batch.reserved_capacity(), __cq_gpu_batch.allocated_bytes());
                }
                #deliver
            }));
        }
        match laned {
            // A single stream supplies both intra-wave order and inter-wave barriers.
            None => steps.push(quote!({ #(#launches)* })),
            Some(entry) => {
                let mut submits = Vec::new();
                for (lane, holes) in entry.lanes.iter().enumerate().skip(1) {
                    for &rule_index in holes {
                        let rule = &rules[rule_index];
                        let call = calls[rule.hole].as_ref().ok_or_else(|| {
                            syn::Error::new_spanned(
                                entry,
                                "a worker-lane hole must be a standalone function item",
                            )
                        })?;
                        let function = &call.function;
                        let at = position(rule_index);
                        let (head, hole) = (rule.head, rule.hole);
                        let (scan_clone, scan_argument) = match &rule.scan {
                            Some(scan) => {
                                let source = match scan.source {
                                    InputSource::Delta(i) => context.relations[i]
                                        .delta_host
                                        .as_ref()
                                        .expect("checked delta"),
                                    InputSource::Layout(i) => &context.layouts[i].host,
                                };
                                (
                                    quote!(let __cq_lane_scan = #source.clone();),
                                    quote!(&__cq_lane_scan,),
                                )
                            }
                            None => (quote!(), quote!()),
                        };
                        let names = (0..call.hosts.len())
                            .map(|i| format_ident!("__cq_lane_host_{i}"))
                            .collect::<Vec<_>>();
                        let hosts = &call.hosts;
                        submits.push(quote!({
                            #scan_clone
                            #(let #names = #hosts.clone();)*
                            __cq_gpu_lanes.submit(#lane, #at, ::std::boxed::Box::new(
                                move |__cq_lane: &mut ::gpu_datalog_runtime::Lane| {
                                    #function(__cq_lane.arena(#head, #hole), #scan_argument #(&#names,)*)
                                }))?;
                        }));
                    }
                }
                let count = wave.holes.len();
                let delivered = wave.holes.iter().enumerate().map(|(at, &rule_index)| {
                    let candidates = local("candidates", rules[rule_index].head);
                    quote!(#candidates.push(__cq_gpu_wave[#at].take().expect("joined wave hole"));)
                });
                steps.push(quote!({
                    let mut __cq_gpu_wave: ::std::vec::Vec<::std::option::Option<::gpu_datalog_runtime::CandidateBatch>> =
                        (0..#count).map(|_| None).collect();
                    __cq_gpu_lanes.fork()?;
                    #(#submits)*
                    #(#launches)*
                    for (at, batch) in __cq_gpu_lanes.join()? { __cq_gpu_wave[at] = Some(batch); }
                    #(#delivered)*
                }));
            }
        }
        for transaction in &stratum.publication {
            if after(transaction.relation) == Some(global) {
                published.insert(transaction.relation);
                steps.push(admit_publish(program, index, transaction, phase)?);
            }
        }
    }
    for transaction in &stratum.publication {
        if !published.contains(&transaction.relation) {
            ensure(
                after(transaction.relation).is_none(),
                schedule,
                "publication placement names a wave outside its phase",
            )?;
            steps.push(admit_publish(program, index, transaction, phase)?);
        }
    }
    let before = memory_snapshot(
        context,
        &[],
        &[],
        &format!("stratum={index}/{phase}/before"),
    );
    let after = memory_snapshot(
        context,
        &stratum.members,
        &stratum.members,
        &format!("stratum={index}/{phase}/after"),
    );
    Ok(quote!(
        #before
        let __cq_gpu_readbacks_before = if __cq_gpu_trace_rounds {
            ::gpu_datalog_runtime::readback_count()
        } else { 0 };
        #(#batches)* #(#steps)*
        #after
        if __cq_gpu_trace_rounds {
            ::std::eprintln!(
                "gpu_datalog_round\tstratum={}\tphase={}\tround={}\treadbacks={}",
                #index, #phase, __cq_gpu_round,
                ::gpu_datalog_runtime::readback_count() - __cq_gpu_readbacks_before,
            );
        }
        __cq_gpu_round += 1;
    ))
}
