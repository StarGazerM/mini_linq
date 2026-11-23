//! GPU interpretation of checked Round/Schedule records on one ordered stream.
//! Providers, initializers, backing states and scratch lifetimes come from the
//! recorded device Storage; this printer composes their fixed operations.
//! Packed-word and layout aliases are defined once in the generated block,
//! before hole functions. Their schemas, collision checks and use checks stay
//! printer-local: no alias definitions travel through the whole-program IR.
use cq_datalog_support::ensure;
use crate::storage::FullStates;
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

/// A name must denote exactly one normalized column tuple, even if the stable
/// hash collides. Keep the tuple, rather than trusting the hash as identity.
#[derive(Default)]
struct PackedAliases(BTreeMap<String, (Type, Type)>);

impl PackedAliases {
    fn insert(
        &mut self,
        name: &Type,
        types: &[&Type],
        definition: impl FnOnce() -> syn::Result<Type>,
    ) -> syn::Result<()> {
        let key = name.to_token_stream().to_string();
        let tuple: Type = syn::parse_quote!((#(#types,)*));
        if let Some((previous, _)) = self.0.get(&key) {
            ensure(
                *previous == tuple,
                name,
                &format!("GPU packed-type alias collision for `{key}`: different column tuples"),
            )?;
        } else {
            self.0.insert(key, (tuple, definition()?));
        }
        Ok(())
    }

    fn check_uses(&self, tokens: TokenStream) -> syn::Result<()> {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Group(group) => self.check_uses(group.stream())?,
                proc_macro2::TokenTree::Ident(name) => {
                    let key = name.to_string();
                    if (key.starts_with("__cq_gpu_word_") || key.starts_with("__cq_gpu_layout_type_"))
                        && !self.0.contains_key(&key)
                    {
                        return Err(syn::Error::new_spanned(
                            name,
                            format!("undefined GPU packed-type alias `{key}`"),
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn definitions(program: &Program) -> syn::Result<TokenStream> {
        let device = program.storage.device.as_ref().ok_or_else(|| {
            syn::Error::new_spanned(program, "missing device storage advice")
        })?;
        let mut aliases = Self::default();
        for input in device.relations.iter().map(|record| &record.input).chain(
            device.layouts.iter().chain(&device.delta_layouts).chain(&device.negations)
                .map(|record| &record.input),
        ) {
            let types = input.columns.iter().map(|column| &column.storage).collect::<Vec<_>>();
            aliases.insert(&crate::storage::word_type(types.iter().copied())?, &types, || {
                crate::storage::word_definition(types.iter().copied())
            })?;
            aliases.insert(&crate::storage::layout_type(&types), &types, || {
                Ok(crate::storage::layout_definition(&types))
            })?;
        }
        aliases.check_uses(program.to_token_stream())?;
        let definitions = aliases.0.iter().map(|(name, (_, definition))| {
            let name = Ident::new(name, proc_macro2::Span::call_site());
            quote!(#[allow(non_camel_case_types)] type #name = #definition;)
        });
        Ok(quote!(#(#definitions)*))
    }
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
    full_states: &FullStates,
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
            .enumerate()
            .filter(|(index, _)| full_states.owner(*index) == *index)
            .map(|(_, layout)| layout)
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
            // SAFETY: the checked full-heading bijection preserves uniqueness
            // and the same declared active bit width, but not ordering.
            "unique_permutation" => quote!(unsafe {
                #state.reorder_unique(&#packing, &#fresh, &[#(#permutation),*], #threads)?
            }),
            _ => unreachable!("checked admitted input certificate"),
        };
        quote!(let #name = #build;)
    });
    quote!(#(#orders)*)
}

/// Formal rule DL-DeviceSharedOrder: append and publish once per checked
/// physical full state (the owner of each admitted order), then refresh all
/// logical prefix-read hosts with Arc-backed clones of that publication.
fn full_layouts(
    program: &Program,
    full_states: &FullStates,
    context: &RoundContext,
    index: usize,
    layouts: &[usize],
) -> syn::Result<TokenStream> {
    for &layout in layouts {
        ensure(
            layouts.contains(&full_states.owner(layout)), program,
            "full-state publication must include every referenced owner",
        )?;
    }
    let updates = layouts.iter().filter(|&&j| full_states.owner(j) == j).map(|&j| {
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
    let aliases = layouts.iter().filter(|&&j| full_states.owner(j) != j).map(|&j| {
        let source = &context.layouts[full_states.owner(j)].host;
        let host = &context.layouts[j].host;
        quote!(#host = #source.clone();)
    });
    Ok(quote!(#(#updates)* #(#aliases)*))
}

/// One member's admission and recorded publication transaction.
fn admit_publish(
    program: &Program,
    full_states: &FullStates,
    index_in_stratum: usize,
    transaction: &round::Publication,
    phase: &str,
) -> syn::Result<TokenStream> {
    let context = &program.context;
    let i = transaction.relation;
    let relation = &context.relations[i];
    let record = &device(program).relations[i];
    let packing = local("packing", i);
    let candidates = local("candidates", i);
    let fresh = local("fresh", i);
    let threads = device(program).layouts[relation.providers.canonical].permutation_threads;
    let order = &record.admitted_orders[0].columns;
    let permutation = match record.candidate_order.to_string().as_str() {
        "canonical" => quote!(::core::option::Option::None),
        "permutation" => quote!(::core::option::Option::Some((&[#(#order),*][..], #threads))),
        _ => unreachable!("checked candidate order certificate"),
    };
    let orders = admitted_orders(program, i, &fresh);
    let relation_name = relation.declaration.name.to_string();
    let canonical = local("layout", relation.providers.canonical);
    let mut operations = Vec::new();
    for operation in &transaction.operations {
        operations.push(match operation.kind.to_string().as_str() {
            "full_layouts" => full_layouts(program, full_states, context, i, &operation.layouts)?,
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
            let candidates = #candidates.iter().map(|batch| batch.len()).sum::<usize>();
            if ::gpu_datalog_runtime::profiling() {
                ::gpu_datalog_runtime::profile_enter(&__cq_gpu_pool, #index_in_stratum, "admit", #relation_name);
            }
            // SAFETY: every batch was packed with this relation's declared
            // packing, which initializes exactly the recorded codec bit window.
            // The bag is admitted one sort portion at a time, never copied whole.
            let #fresh = unsafe { #canonical.fresh_batches(&#packing, &#candidates, #permutation)? };
            ::std::mem::drop(#candidates);
            if ::gpu_datalog_runtime::trace_kernels() {
                ::std::eprintln!("gpu_datalog_admission\tstratum={}\tphase={}\tround={}\trelation={}\tcandidates={}\tadmitted={}",
                    #index_in_stratum, #phase, __cq_gpu_round, #relation_name, candidates, #fresh.len());
            }
            #fresh
        };
        if ::gpu_datalog_runtime::profiling() {
            ::gpu_datalog_runtime::profile_enter(&__cq_gpu_pool, #index_in_stratum, "orders", #relation_name);
        }
        #orders
        if ::gpu_datalog_runtime::profiling() {
            ::gpu_datalog_runtime::profile_enter(&__cq_gpu_pool, #index_in_stratum, "publish", #relation_name);
        }
        #(#operations)*
        if ::gpu_datalog_runtime::profiling() {
            ::gpu_datalog_runtime::profile_enter(&__cq_gpu_pool, #index_in_stratum, "holes", "");
        }
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

/// Formal rule P-DL-Gpu: interpret the recorded waves, admission owners,
/// publication placement and transactions over the recorded device states on
/// one device stream. Admission is fresh = unique(candidates) minus the
/// committed canonical layout; publication appends fresh rows to every layout.
/// Constructors consume recorded merge tiles and auxiliary geometry.
pub fn compile(program: &Program) -> syn::Result<syn::Expr> {
    let packed_aliases = PackedAliases::definitions(program)?;
    crate::storage::check_device(&program.storage)?;
    round::contract::check(program)?;
    crate::bindings::check(program)?;
    let full_states = FullStates::build(&program.storage)?;
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
    let holes = cq_datalog_print::splice_with(program, |hole| {
        let index = hole_index;
        hole_index += 1;
        let standalone = !reads_environment(&hole.cq);
        let mut cq = hole.cq.clone();
        let mut parameters = Vec::new();
        if standalone {
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
        let batch = quote!(::gpu_datalog_runtime::CandidateBatch);
        let result = if crate::bindings::head_argument(hole) {
            quote!(::core::option::Option<#batch>)
        } else {
            batch
        };
        hole_items.push(quote!(
            fn #function(
                __cq_gpu_arena: &mut #arena,
                #scan_parameter #(#declared,)*
            ) -> #result {
                #invocation
            }
        ));
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
        if full_states.owner(index) != index {
            continue;
        }
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
    // Owners are initialized first: a canonical owner may occur after its alias.
    for (index, layout) in context.layouts.iter().enumerate() {
        let owner = full_states.owner(index);
        if owner != index {
            let host = &layout.host;
            let source = &context.layouts[owner].host;
            initialize.push(quote!(let mut #host = #source.clone();));
        }
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
        let publish = full_layouts(program, &full_states, context, index, &layouts)?;
        let runner = program_runner();
        initialize.push(quote!(
            #runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> {
                let host = &(#expression);
                let words = ::cq_storage::rows::Rows::rows(host)
                    .map(|row| #packing.pack::<#word>(&[#(#fields),*]))
                    .collect::<::gpu_datalog_runtime::Result<::std::vec::Vec<_>>>()?;
                let #upload = ::gpu_datalog_runtime::Buffer::from_host_async(__cq_gpu_pool.clone(), &words)?;
                #admit
                #orders
                #publish
                Ok(())
            })?;
        ));
    }
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
            keys.push(quote!(let #name: #ty = {
                let mut arena: #arena_ty = #arena_init;
                let __cq_gpu_arena = &mut arena;
                let batch = #invocation;
                let packing = #packing;
                let buffer = ::gpu_datalog_runtime::concatenate_batches::<#word>(__cq_gpu_pool.clone(), &packing, &[batch])?;
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
        let (seed_groups, seed) = phase(program, &full_states, index, round::schedule::SEED, &holes)?;
        let seed = quote!(#seed_runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> { #seed Ok(()) })?;);
        let (step_groups, step) = phase(program, &full_states, index, round::schedule::STEP, &holes)?;
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
            &full_states,
            &stratum.members,
            &[],
            &format!("stratum={index}/released"),
        );
        strata.push(quote!({
            #stratum_runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> {
                #(#arenas)*
                #(#keys)*
                #seed_groups #step_groups
                let mut __cq_gpu_round = 0usize;
                #seed
                #rounds
                Ok(())
            })?;
            if ::gpu_datalog_runtime::release_at_stratum_boundary(&__cq_gpu_pool) {
                ::cq_gpu_sequence_runtime::release_cached_scratch(&__cq_gpu_pool);
                __cq_gpu_pool.release_unused()?;
            }
            #released
        }));
    }
    let outputs = context.outputs.iter().map(|output| {
        let relation = &context.relations[output.relation];
        let canonical = &context.layouts[relation.providers.canonical];
        let host = &canonical.host;
        let at = &device.relations[output.relation].output_permutation;
        let fields = relation.declaration.columns.iter().zip(at).map(|(declared, position)| {
            let ty = &declared.ty;
            match symbols.codec(declared.logical_type_id) {
                // DECODE P: generated SymbolCodec inverse resolves final datalog host output fields.
                Some(codec) => quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::decode(__cq_gpu_row.field(#position) as u32)),
                // DECODE P: generated numeric carrier inversion constructs final host output fields.
                None => quote!(__cq_gpu_row.field(#position) as #ty),
            }
        });
        // Canonical keys cross to the host only at the output boundary, where
        // each is written once as its logical host row.
        let runner = program_runner();
        quote!(#runner(#[inline] || -> ::gpu_datalog_runtime::Result<_> {
            // DECODE P: invert canonical radix key fields only at final output.
            Ok(#host.publish_rows(&mut __cq_gpu_output, |__cq_gpu_row| (#(#fields,)*))?.into_iter())
        })?)
    }).collect::<Vec<_>>();
    let result = if outputs.len() == 1 {
        outputs[0].clone()
    } else {
        quote!((#(#outputs,)*))
    };
    let relation_count = context.relations.len();
    let indices = (0..relation_count).collect::<Vec<_>>();
    let initialized = memory_snapshot(context, &full_states, &indices, &[], "initialized");
    let initialized_runner = program_runner();
    let initialized = quote!(#initialized_runner(#[inline] || { #initialized }););
    let census = context.layouts.iter().enumerate().map(|(index, layout)| {
        let host = &layout.host;
        let owner = full_states.owner(index);
        let relation = context.relations[layout.relation].declaration.name.to_string();
        quote!(#host.trace_full_state(#relation, #index, #owner);)
    });
    syn::parse2(quote!({
        #packed_aliases
        #(#hole_items)*
        (|| -> ::gpu_datalog_runtime::Result<_> {
            let __cq_gpu_pool = ::gpu_datalog_runtime::thread_pool()?;
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
            if ::gpu_datalog_runtime::profiling() {
                ::gpu_datalog_runtime::profile_enter(&__cq_gpu_pool, 0, "holes", "");
            }
            ::gpu_datalog_runtime::trace_work("setup");
            ::gpu_datalog_runtime::reset_work_counters();
            #(#strata)*
            if ::gpu_datalog_runtime::profiling() {
                ::gpu_datalog_runtime::profile_report(&__cq_gpu_pool);
            }
            if __cq_gpu_trace_memory { #(#census)* }
            ::gpu_datalog_runtime::phases::mark(&__cq_gpu_pool, "fixpoint")?;
            ::gpu_datalog_runtime::trace_work("fixpoint");
            let mut __cq_gpu_output = ::gpu_datalog_runtime::OutputStaging::new(&__cq_gpu_pool);
            let __cq_gpu_result = #result;
            ::std::mem::drop(__cq_gpu_output);
            ::gpu_datalog_runtime::phases::mark(&__cq_gpu_pool, "output")?;
            Ok(__cq_gpu_result)
        })().expect("GPU Datalog execution failed")
    }))
}

/// Validate all active heads against one stream snapshot. A capacity miss is
/// replayable; arithmetic/kernel failure is not. No publication takes place
/// between the first launch and the last successful attempt.
fn head_attempts(
    group: &Ident,
    slots: &BTreeMap<usize, usize>,
    phase: &str,
    functions: &[TokenStream],
    launches: &[TokenStream],
    replays: &[TokenStream],
) -> TokenStream {
    let heads = slots.len();
    let phase_index = usize::from(phase == round::schedule::STEP);
    let begin = slots.iter().map(|(&head, &slot)| {
        let arena = local("arena", head);
        let packing = local("packing", head);
        quote!(#arena.begin_head(&#group, #slot, #packing.clone(), #phase_index)?;)
    });
    let validate = slots.iter().map(|(&head, &slot)| {
        let arena = local("arena", head);
        let candidates = local("candidates", head);
        quote! {
            if __cq_gpu_active[#slot] {
                if __cq_gpu_words[2 * #slot + 1] != 0 {
                    return Err("Sequence device count or fold overflow".into());
                }
                let rows = __cq_gpu_words[2 * #slot] as usize;
                if rows > #arena.head_capacity() {
                    // Discard Some(batch) fallback results of this attempt too.
                    #candidates.clear();
                    #arena.retry_head(rows)?;
                    __cq_gpu_previous[#slot] = 0;
                } else {
                    // SAFETY: same-stream dispatches initialized the prefix,
                    // and both total count and failure have been checked.
                    let batch = unsafe { #arena.finish_head(rows)? };
                    if !batch.is_empty() { #candidates.push(batch); }
                    __cq_gpu_active[#slot] = false;
                }
            }
        }
    });
    quote!({
        #(#begin)*
        let mut __cq_gpu_active = [true; #heads];
        let mut __cq_gpu_previous = [0usize; #heads];
        #(#functions)*
        #(#launches)*
        loop {
            let __cq_gpu_words = #group.read()?;
            #(#validate)*
            if !__cq_gpu_active.contains(&true) { break; }
            #(#replays)*
        }
    })
}

fn phase(
    program: &Program,
    full_states: &FullStates,
    index: usize,
    phase: &str,
    holes: &[TokenStream],
) -> syn::Result<(TokenStream, TokenStream)> {
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
    let mut groups = TokenStream::new();
    let mut steps = Vec::new();
    for (point, epoch) in crate::bindings::epochs(stratum, schedule, phase).into_iter().enumerate() {
        let mut slots = BTreeMap::new();
        let mut functions = Vec::new();
        let mut launches = Vec::new();
        let mut replays = Vec::new();
        for rule_index in epoch.rules {
            let rule = &rules[rule_index];
            let candidates = local("candidates", rule.head);
            let head = crate::bindings::head_argument(&program.holes[rule.hole]);
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
                    let #rows = ::gpu_datalog_runtime::concatenate_batches::<#word>(
                        __cq_gpu_pool.clone(), &#packing, ::core::slice::from_ref(&__cq_gpu_batch))?;
                    #admit
                    __cq_expansion_capture.report(#rule_name, #variant, __cq_gpu_round, #fresh.len());
                })
            } else {
                None
            };
            // `GPU_DATALOG_PROFILE` names a hole by its rule, changing occurrence and index.
            let profile_label = {
                let variant = &program.storage.incremental.variants[rule.variant];
                let label = program.storage.incremental.strata.source.labels[variant.rule]
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| head_name.clone());
                let delta = variant
                    .delta
                    .map_or_else(|| "seed".to_owned(), |position| position.to_string());
                format!("{label}@{delta}#{hole}")
            };
            let trace_batch = quote! {
                if __cq_gpu_trace_memory {
                    ::std::eprintln!("gpu_datalog_hole_result\tstratum={}\tphase={}\thole={}\trows={}\tcapacity={}\tbytes={}",
                        #index, #phase, #hole, __cq_gpu_batch.len(), __cq_gpu_batch.reserved_capacity(), __cq_gpu_batch.allocated_bytes());
                }
            };
            let deliver = if head {
                let packing = local("packing", rule.head);
                quote! {
                    if let Some(__cq_gpu_batch) = __cq_gpu_batch {
                        #trace_batch
                        #candidates.push(__cq_gpu_batch);
                    } else if __cq_gpu_trace_memory || ::gpu_datalog_runtime::trace_kernels() {
                        let count = __cq_gpu_arena.trace_head_count()?;
                        let rows = count.checked_sub(*__cq_gpu_last).ok_or("Sequence device cursor wrapped")?;
                        *__cq_gpu_last = count;
                        let capacity = __cq_gpu_arena.head_capacity();
                        if __cq_gpu_trace_memory {
                            ::std::eprintln!("gpu_datalog_hole_result\tstratum={}\tphase={}\thole={}\trows={}\tcapacity={}\tbytes={}",
                                #index, #phase, #hole, rows, capacity, capacity * (#packing.word_bits() as usize / 8));
                        }
                        if ::gpu_datalog_runtime::trace_kernels() {
                            ::std::eprintln!("gpu_datalog_head_dispatch\tstratum={}\tphase={}\tround={}\thole={}\thead={}\tcapacity={}\tcandidates={}",
                                #index, #phase, __cq_gpu_round, #hole, #head_name, capacity, rows);
                        }
                    }
                }
            } else {
                quote!(#trace_batch #candidates.push(__cq_gpu_batch);)
            };
            let launch = quote!({
                if ::gpu_datalog_runtime::profiling() {
                    ::gpu_datalog_runtime::profile_enter(&__cq_gpu_pool, #index, "holes", #profile_label);
                }
                #scan
                if __cq_gpu_trace_memory {
                    let memory = __cq_gpu_pool.memory_stats();
                    ::std::eprintln!("gpu_datalog_hole\tstratum={}\tphase={}\thole={}\thead={}\tcurrent_bytes={}\treserved_bytes={}\tarena_bytes={}",
                        #index, #phase, #hole, #head_name, memory.current_bytes, memory.reserved_bytes, #arena.allocated_bytes());
                }
                // Sequence-ineligible fallback still owns its per-hole hint.
                let __cq_gpu_arena = #arena.hole(#hole);
                #counter_scope
                let __cq_gpu_batch = #invocation;
                #counter_report
                #deliver
            });
            if head {
                let next = slots.len();
                let slot = *slots.entry(rule.head).or_insert(next);
                // Share one body (and one expansion of its kernels) across
                // the original and retry call sites.
                let function = local("launch", hole);
                let arena_ty = &device.relations[rule.head].arena;
                let batches_ty = &device.relations[rule.head].candidates;
                functions.push(quote! {
                    let #function = |#arena: &mut #arena_ty, #candidates: &mut #batches_ty, __cq_gpu_last: &mut usize| -> ::gpu_datalog_runtime::Result<()> {
                        #launch
                        Ok(())
                    };
                });
                let call = quote!(#function(&mut #arena, &mut #candidates, &mut __cq_gpu_previous[#slot])?;);
                replays.push(quote!(if __cq_gpu_active[#slot] { #call }));
                launches.push(call);
            } else {
                launches.push(launch);
            }
        }
        if slots.is_empty() {
            steps.push(quote!({ #(#launches)* }));
        } else {
            let group = format_ident!("__cq_gpu_{}_group_{}", phase, point);
            let heads = slots.len();
            groups.extend(quote! {
                let #group = ::gpu_datalog_runtime::HeadCursors::new(__cq_gpu_pool.clone(), #heads)?;
            });
            steps.push(head_attempts(&group, &slots, phase, &functions, &launches, &replays));
        }
        for transaction in epoch.publications {
            steps.push(admit_publish(program, full_states, index, transaction, phase)?);
        }
    }
    let before = memory_snapshot(
        context,
        full_states,
        &[],
        &[],
        &format!("stratum={index}/{phase}/before"),
    );
    let after = memory_snapshot(
        context,
        full_states,
        &stratum.members,
        &stratum.members,
        &format!("stratum={index}/{phase}/after"),
    );
    Ok((groups, quote!(
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
    )))
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    #[test]
    fn colliding_packed_aliases_are_rejected() {
        let mut aliases = PackedAliases::default();
        let ty: Type = syn::parse_quote!(u32);
        let other: Type = syn::parse_quote!(u64);
        let name = crate::storage::word_type([&ty]).unwrap();
        aliases.insert(&name, &[&ty], || crate::storage::word_definition([&ty])).unwrap();
        aliases.insert(&name, &[&ty], || panic!("duplicate definition")).unwrap();
        let error = aliases.insert(&name, &[&other], || panic!("colliding definition")).unwrap_err();
        assert!(error.to_string().contains("alias collision"));
    }
}
