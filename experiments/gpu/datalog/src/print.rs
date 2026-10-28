//! GPU interpretation of checked Round/Schedule records on one ordered stream.
use cq_datalog_support::ensure;
use cq_lang_datalog_round::{self as round, HoleInterface, InputSource, Program, RoundContext};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{Ident, Type};

fn local(kind: &str, index: usize) -> Ident { format_ident!("__cq_gpu_{}_{}", kind, index) }
fn types(context: &RoundContext, index: usize) -> Vec<&Type> {
    context.relations[index].declaration.columns.iter().map(|column| &column.ty).collect()
}
fn packing(types: &[&Type]) -> syn::Result<TokenStream> {
    let columns = types.iter().map(|ty| crate::storage::scalar(ty).map(|(_, column)| column))
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(quote!(::gpu_datalog_runtime::Packing::with_columns(&[#(#columns),*])?))
}
fn canonical_type(schemas: &[Vec<Type>], index: usize) -> Type {
    let types = schemas[index].iter().collect::<Vec<_>>();
    crate::storage::layout_type(&types, types.len())
}
pub(crate) fn input_types(program: &Program, schemas: &[Vec<Type>]) -> syn::Result<std::collections::BTreeMap<String, Type>> {
    let context = &program.context;
    let mut inputs = std::collections::BTreeMap::new();
    for (index, relation) in context.relations.iter().enumerate() {
        let ty = canonical_type(schemas, index);
        inputs.insert(relation.declaration.name.to_string(), ty.clone());
        if let Some(delta) = &relation.delta { inputs.insert(delta.to_string(), ty); }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        inputs.insert(layout.name.to_string(), layout.provider.clone());
    }
    for stratum in &context.strata {
        for key in &stratum.keys {
            ensure(!key.columns.is_empty(), program,
                "GPU Datalog does not support unit-valued existential negation keys")?;
            let columns = schemas[key.relation].iter().collect::<Vec<_>>();
            let selected = key.columns.iter().map(|&i| columns[i]).collect::<Vec<_>>();
            inputs.insert(key.name.to_string(), crate::storage::layout_type(&selected, selected.len()));
        }
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            if let Some(scan) = &rule.scan {
                let ty = match scan.source {
                    InputSource::Delta(index) => canonical_type(schemas, index),
                    InputSource::Layout(index) => context.layouts[index].provider.clone(),
                };
                inputs.insert(scan.binding.to_string(), ty);
            }
        }
    }
    Ok(inputs)
}

fn memory_snapshot(context: &RoundContext, indices: &[usize], arenas: &[usize], stage: &str) -> TokenStream {
    let entries = indices.iter().map(|&index| {
        let relation = &context.relations[index];
        let name = relation.declaration.name.to_string();
        let state = local("state", index);
        let arena = local("arena", index);
        let arena_bytes = if arenas.contains(&index) { quote!(#arena.allocated_bytes()) } else { quote!(0usize) };
        let layouts = context.layouts.iter().chain(&context.delta_layouts)
            .filter(|layout| layout.relation == index).map(|layout| &layout.host).collect::<Vec<_>>();
        quote!(__cq_gpu_memory_usage[#index] = ::gpu_datalog_runtime::MemoryUse {
            name: #name, rows: #state.len(), accepted_live_bytes: #state.live_bytes(),
            accepted_bytes: #state.allocated_bytes(),
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

/// Replace each host identifier read by a hole with a dereferenced reference
/// parameter, recording `(parameter, host, type)` once per distinct host.
fn substitute(
    tokens: TokenStream,
    hosts: &std::collections::BTreeMap<String, Type>,
    parameters: &mut Vec<(Ident, Ident, Type)>,
) -> TokenStream {
    use proc_macro2::{Group, TokenTree};
    tokens.into_iter().map(|token| match token {
        TokenTree::Ident(name) if hosts.contains_key(&name.to_string()) => {
            let position = parameters.iter().position(|(_, host, _)| *host == name).unwrap_or_else(|| {
                let parameter = format_ident!("__cq_gpu_input_{}", parameters.len());
                parameters.push((parameter, name.clone(), hosts[&name.to_string()].clone()));
                parameters.len() - 1
            });
            let parameter = &parameters[position].0;
            quote!((*#parameter))
        }
        TokenTree::Group(group) => {
            let mut replacement = Group::new(group.delimiter(), substitute(group.stream(), hosts, parameters));
            replacement.set_span(group.span());
            TokenStream::from(TokenTree::Group(replacement))
        }
        token => TokenStream::from(token),
    }).collect()
}

/// Whether a hole's query names anything bound outside the query, such as a
/// local of the function containing `datalog!`. Host declaration values are
/// substituted separately. Callees (`sym("…")`) are items visible to nested
/// functions; macros are opaque and conservatively count as environment reads.
fn reads_environment(cq: &round::cq::Program) -> bool {
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
    for item in &cq.query.body {
        match item {
            BodyItem::Positive { atom } => for argument in &atom.args {
                match cq_syntax::rust::variable(argument) {
                    Some(variable) => { bound.insert(cq_syntax::symbol_name(variable)); }
                    None => expressions.push(argument),
                }
            },
            BodyItem::Negation(negation) => expressions.extend(&negation.atom.args),
            BodyItem::Filter(filter) => expressions.push(&filter.condition),
            BodyItem::Let(binding) => {
                bound.extend(cq_syntax::rust::bindings(&binding.pattern).iter().map(cq_syntax::symbol_name));
                expressions.push(&binding.expression);
            }
            BodyItem::For(binding) => {
                bound.extend(cq_syntax::rust::bindings(&binding.pattern).iter().map(cq_syntax::symbol_name));
                expressions.push(&binding.expression);
            }
            _ => return true,
        }
    }
    expressions.into_iter().any(|expression| {
        let free = cq_syntax::rust::free_variables(expression);
        let mut callees = Callees(std::collections::BTreeSet::new());
        callees.visit_expr(expression);
        free.opaque || free.names.iter().any(|name| !bound.contains(name) && !callees.0.contains(name))
    })
}

/// Formal rule P-DL-Gpu: Interpret the recorded waves, admission owners and
/// publication operations using immutable packed runs on one device stream.
pub fn compile(program: &Program) -> syn::Result<syn::Expr> {
    crate::storage::check_device(&program.storage)?;
    round::contract::check(program)?;
    crate::bindings::check(program)?;
    ensure(program.context.observer.is_none(), program, "GPU Datalog does not support observer callbacks")?;
    ensure(program.policy.timing == "none", program, "GPU Datalog does not support recorded phase timing")?;
    ensure(program.execution.is_empty(), program, "GPU Datalog does not support host execution certificates")?;
    for hole in &program.holes {
        ensure(hole.interface == HoleInterface::Batch, hole, "GPU Datalog requires batch holes")?;
    }
    let context = &program.context;
    let symbols = crate::storage::Symbols::from_program(&program.storage)?;
    let ratio = program.storage.device.as_ref().expect("checked device advice").ratio;
    let schemas = (0..context.relations.len()).map(|index|
        types(context, index).into_iter().map(|ty| symbols.device_type(ty)).collect::<Vec<_>>()
    ).collect::<Vec<_>>();
    // Check eligibility independently, including schemas not read by a hole.
    for schema in &schemas { crate::storage::word_type(schema)?; }
    let input_types = input_types(program, &schemas)?;
    // Hosts a hole may read through its CQ declarations, with their local types.
    let mut hosts = std::collections::BTreeMap::<String, Type>::new();
    for (index, relation) in context.relations.iter().enumerate() {
        let ty = canonical_type(&schemas, index);
        hosts.insert(relation.all.to_string(), ty.clone());
        if let Some(delta) = &relation.delta_host { hosts.insert(delta.to_string(), ty); }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        hosts.insert(layout.host.to_string(), layout.provider.clone());
    }
    let mut scans = std::collections::BTreeMap::new();
    for stratum in &context.strata {
        for key in &stratum.keys {
            hosts.insert(key.name.to_string(), input_types[&key.name.to_string()].clone());
        }
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            if let Some(scan) = &rule.scan { scans.insert(rule.hole, scan.binding.clone()); }
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
                    value.expression = syn::parse2(substitute(expression, &hosts, &mut parameters))?;
                }
            }
        }
        let invocation = hole.pipeline.clone().start(cq.to_token_stream())?;
        if !standalone {
            return Ok(invocation);
        }
        let function = format_ident!("__cq_gpu_hole_{}", index);
        let (scan_parameter, scan_argument) = match scans.get(&index) {
            Some(binding) => {
                let ty = &input_types[&binding.to_string()];
                (quote!(#binding: &#ty,), quote!(#binding,))
            }
            None => (quote!(), quote!()),
        };
        let declared = parameters.iter().map(|(parameter, _, ty)| quote!(#parameter: &#ty));
        let arguments = parameters.iter().map(|(_, host, _)| quote!(&#host));
        hole_items.push(quote!(
            fn #function(
                __cq_gpu_arena: &mut ::gpu_datalog_runtime::CandidateArena,
                #scan_parameter #(#declared,)*
            ) -> ::gpu_datalog_runtime::CandidateBatch {
                #invocation
            }
        ));
        Ok(quote!(#function(&mut *__cq_gpu_arena, #scan_argument #(#arguments,)*)))
    })?;
    // Setup and output run through the same partitioned runners as strata, so
    // the program body keeps only the owning locals rather than every
    // relation's construction, upload, permutation and error path.
    let program_runner = |slot: usize| {
        let partition = format_ident!("partition_{}", slot % 16);
        quote!(::cq_gpu_sequence_runtime::__codegen::#partition::run)
    };
    let mut initialize = Vec::new();
    for (index, relation) in context.relations.iter().enumerate() {
        let state = local("state", index);
        let packed = local("packing", index);
        let word = crate::storage::word_type(&schemas[index])?;
        let schema = packing(&schemas[index].iter().collect::<Vec<_>>())?;
        let all = &relation.all;
        let ty = canonical_type(&schemas, index);
        let upload = relation.declaration.value.as_ref().map(|value| {
            let expression = &value.expression;
            let fields = types(context, index).into_iter().enumerate().map(|(index, ty)| {
                let field = syn::Index::from(index);
                match symbols.codec(ty) {
                    Some(codec) => quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::code(row.#field) as i128),
                    None => quote!(*row.#field as i128),
                }
            });
            quote!({
                let host = &(#expression);
                let words = ::cq_storage::rows::Rows::rows(host)
                    .map(|row| #packed.pack::<#word>(&[#(#fields),*]))
                    .collect::<::gpu_datalog_runtime::Result<::std::vec::Vec<_>>>()?;
                let upload = ::gpu_datalog_runtime::Buffer::from_host_async(__cq_gpu_pool.clone(), &words)?;
                #state.admit(&upload, words.len())?;
            })
        });
        let (delta_binding, delta_value, delta_result) = match &relation.delta_host {
            Some(delta) => (
                quote!(mut #delta,),
                quote!(let #delta: #ty = <#ty>::from_runs::<#word>(
                    __cq_gpu_pool.clone(), #packed.clone(), ::std::vec::Vec::new())?;),
                quote!(#delta,),
            ),
            None => (quote!(), quote!(), quote!()),
        };
        let runner = program_runner(index);
        initialize.push(quote!(
            let (#packed, mut #state, mut #all, #delta_binding) =
                #runner(#[inline] || -> ::gpu_datalog_runtime::Result<_> {
                    let #packed = #schema;
                    let mut #state = ::gpu_datalog_runtime::DeviceRuns::<#word>::new(__cq_gpu_pool.clone(), #ratio)?;
                    #upload
                    let #all: #ty = <#ty>::from_runs(__cq_gpu_pool.clone(), #packed.clone(), #state.owned_runs())?;
                    #delta_value
                    Ok((#packed, #state, #all, #delta_result))
                })?;
        ));
    }
    for (index, layout) in context.layouts.iter().enumerate() {
        let state = local("layout", index);
        let relation_state = local("state", layout.relation);
        let packed = local("packing", layout.relation);
        let schema = local("layout_packing", index);
        let word = crate::storage::word_type(&schemas[layout.relation])?;
        let order = &layout.columns;
        let ty = &layout.provider;
        let host = &layout.host;
        let runner = program_runner(context.relations.len() + index);
        initialize.push(quote!(
            let (#schema, mut #state, mut #host) = #runner(#[inline] || -> ::gpu_datalog_runtime::Result<_> {
                let #schema = #packed.permute(&[#(#order),*])?;
                let mut #state = ::gpu_datalog_runtime::DeviceRuns::<#word>::new(__cq_gpu_pool.clone(), #ratio)?;
                for run in #relation_state.owned_runs() {
                    let permuted = #packed.permute_device(run.buffer(), run.len(), &[#(#order),*])?;
                    // SAFETY: a column permutation preserves uniqueness of the run.
                    let sorted = unsafe { #state.sort_known_unique(&permuted, run.len())? };
                    // A permutation is injective; canonical accepted runs are disjoint.
                    unsafe { #state.append_sorted(sorted)?; }
                }
                let #host: #ty = <#ty>::from_runs(__cq_gpu_pool.clone(), #schema.clone(), #state.owned_runs())?;
                Ok((#schema, #state, #host))
            })?;
        ));
    }
    for (index, layout) in context.delta_layouts.iter().enumerate() {
        let packed = local("packing", layout.relation);
        let schema = local("delta_packing", index);
        let word = crate::storage::word_type(&schemas[layout.relation])?;
        let order = &layout.columns;
        let ty = &layout.provider;
        let host = &layout.host;
        initialize.push(quote!(
            let #schema = #packed.permute(&[#(#order),*])?;
            let mut #host: #ty = <#ty>::from_runs::<#word>(__cq_gpu_pool.clone(), #schema.clone(), ::std::vec::Vec::new())?;
        ));
    }
    let mut strata = Vec::new();
    for (index, stratum) in context.strata.iter().enumerate() {
        let mut keys = Vec::new();
        for key in &stratum.keys {
            let name = &key.name;
            let ty = &input_types[&name.to_string()];
            let relation_types = schemas[key.relation].iter().collect::<Vec<_>>();
            let types = key.columns.iter().map(|&i| relation_types[i]).collect::<Vec<_>>();
            let word = crate::storage::word_type(types.iter().copied())?;
            let schema = packing(&types)?;
            let invocation = &holes[key.hole];
            keys.push(quote!(let #name: #ty = {
                let mut arena = ::gpu_datalog_runtime::CandidateArena::new(__cq_gpu_pool.clone());
                let __cq_gpu_arena = &mut arena;
                let batch = #invocation;
                let packing = #schema;
                let buffer = ::gpu_datalog_runtime::concatenate_batches::<#word>(__cq_gpu_pool.clone(), &packing, &[batch])?;
                let state = ::gpu_datalog_runtime::DeviceRuns::<#word>::new(__cq_gpu_pool.clone(), #ratio)?;
                let run = state.sort_unique(&buffer, buffer.len())?;
                <#ty>::from_runs(__cq_gpu_pool.clone(), packing, vec![run])?
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
        let seed = phase(program, index, round::schedule::SEED, &holes, &schemas)?;
        let seed = quote!(#seed_runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> { #seed Ok(()) })?;);
        let step = phase(program, index, round::schedule::STEP, &holes, &schemas)?;
        let step = quote!(#step_runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> { #step Ok(()) })?;);
        let lengths = stratum.termination.iter().map(|&i| {
            let delta = context.relations[i].delta_host.as_ref().expect("checked delta");
            quote!(#delta.len())
        }).collect::<Vec<_>>();
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
        let arenas = stratum.members.iter().map(|&i| {
            let arena = local("arena", i);
            quote!(let mut #arena = ::gpu_datalog_runtime::CandidateArena::new(__cq_gpu_pool.clone());)
        });
        let released = memory_snapshot(context, &stratum.members, &[], &format!("stratum={index}/released"));
        strata.push(quote!({
            #stratum_runner(#[inline] || -> ::gpu_datalog_runtime::Result<()> {
                #(#arenas)*
                #(#keys)*
                let mut __cq_gpu_round = 0usize;
                #seed
                #rounds
                Ok(())
            })?;
            ::cq_gpu_sequence_runtime::release_cached_scratch(&__cq_gpu_pool);
            __cq_gpu_pool.release_unused()?;
            #released
        }));
    }
    let outputs = context.outputs.iter().map(|output| {
        let index = output.relation;
        let state = local("state", index);
        let packed = local("packing", index);
        let fields = types(context, index).into_iter().enumerate().map(|(i, ty)| {
            match symbols.codec(ty) {
                Some(codec) => quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::decode(row[#i] as u32)),
                None => quote!(row[#i] as #ty),
            }
        });
        // Restore logical host symbols only at the explicit output boundary.
        let runner = program_runner(index);
        quote!(#runner(#[inline] || -> ::gpu_datalog_runtime::Result<_> {
            Ok(#state.to_host()?.into_iter().map(|word| {
                let row = #packed.unpack(word)?;
                Ok((#(#fields,)*))
            }).collect::<::gpu_datalog_runtime::Result<::std::vec::Vec<_>>>()?.into_iter())
        })?)
    }).collect::<Vec<_>>();
    let result = if outputs.len() == 1 { outputs[0].clone() } else { quote!((#(#outputs,)*)) };
    let relation_count = context.relations.len();
    let indices = (0..relation_count).collect::<Vec<_>>();
    let initialized = memory_snapshot(context, &indices, &[], "initialized");
    let initialized_runner = program_runner(relation_count);
    let initialized = quote!(#initialized_runner(#[inline] || { #initialized }););
    syn::parse2(quote!({
        #(#hole_items)*
        (|| -> ::gpu_datalog_runtime::Result<_> {
            let __cq_gpu_pool = ::gpu_datalog_runtime::thread_pool()?;
            ::gpu_datalog_runtime::phases::start();
            let __cq_gpu_trace_rounds = ::std::env::var_os("GPU_DATALOG_TRACE_ROUNDS").is_some();
            let __cq_gpu_trace_memory = ::std::env::var_os("GPU_DATALOG_TRACE_MEMORY").is_some();
            let mut __cq_gpu_memory_usage = if __cq_gpu_trace_memory {
                ::std::vec![::gpu_datalog_runtime::MemoryUse::default(); #relation_count]
            } else { ::std::vec::Vec::new() };
            #(#initialize)*
            #initialized
            ::gpu_datalog_runtime::phases::mark(&__cq_gpu_pool, "setup")?;
            #(#strata)*
            ::gpu_datalog_runtime::phases::mark(&__cq_gpu_pool, "fixpoint")?;
            let __cq_gpu_result = #result;
            ::gpu_datalog_runtime::phases::mark(&__cq_gpu_pool, "output")?;
            Ok(__cq_gpu_result)
        })().expect("GPU Datalog execution failed")
    }))
}

fn phase(program: &Program, index: usize, phase: &str, holes: &[TokenStream], schemas: &[Vec<Type>]) -> syn::Result<TokenStream> {
    let context = &program.context;
    let stratum = &context.strata[index];
    let rules = if phase == round::schedule::SEED { &stratum.seeds } else { &stratum.steps };
    let batches = stratum.members.iter().map(|&i| {
        let name = &context.relations[i].next;
        quote!(let mut #name = ::std::vec::Vec::<::gpu_datalog_runtime::CandidateBatch>::new();)
    });
    let mut waves = Vec::new();
    for wave in &program.schedule.strata[index].waves {
        if wave.phase != phase { continue; }
        let mut launches = Vec::new();
        for &rule_index in &wave.holes {
            let rule = &rules[rule_index];
            let next = &context.relations[rule.head].next;
            let arena = local("arena", rule.head);
            let invocation = &holes[rule.hole];
            let hole = rule.hole;
            let head_name = context.relations[rule.head].declaration.name.to_string();
            let scan = rule.scan.as_ref().map(|scan| {
                let binding = &scan.binding;
                let source = match scan.source {
                    InputSource::Delta(i) => context.relations[i].delta_host.as_ref().expect("checked delta"),
                    InputSource::Layout(i) => &context.layouts[i].host,
                };
                quote!(let #binding = &#source;)
            });
            launches.push(quote!({
                #scan
                if __cq_gpu_trace_memory {
                    let memory = __cq_gpu_pool.memory_stats();
                    ::std::eprintln!("gpu_datalog_hole\tstratum={}\tphase={}\thole={}\thead={}\tcurrent_bytes={}\treserved_bytes={}\tarena_bytes={}",
                        #index, #phase, #hole, #head_name, memory.current_bytes, memory.reserved_bytes, #arena.allocated_bytes());
                }
                let __cq_gpu_arena = &mut #arena;
                let __cq_gpu_batch = #invocation;
                if __cq_gpu_trace_memory {
                    ::std::eprintln!("gpu_datalog_hole_result\tstratum={}\tphase={}\thole={}\trows={}\tcapacity={}\tbytes={}",
                        #index, #phase, #hole, __cq_gpu_batch.len(), __cq_gpu_batch.reserved_capacity(), __cq_gpu_batch.allocated_bytes());
                }
                #next.push(__cq_gpu_batch);
            }));
        }
        // A single stream supplies both intra-wave order and inter-wave barriers.
        waves.push(quote!({ #(#launches)* }));
    }
    let admissions = stratum.members.iter().map(|&i| {
        let state = local("state", i);
        let packed = local("packing", i);
        let fresh = local("fresh", i);
        let next = &context.relations[i].next;
        let word = crate::storage::word_type(&schemas[i])?;
        let relation_name = context.relations[i].declaration.name.to_string();
        Ok(quote!(let #fresh = {
            let candidates = ::gpu_datalog_runtime::concatenate_batches::<#word>(__cq_gpu_pool.clone(), &#packed, &#next)?;
            let fresh = #state.admit(&candidates, candidates.len())?;
            if ::gpu_datalog_runtime::trace_kernels() {
                ::std::eprintln!("gpu_datalog_admission\tstratum={}\tphase={}\tround={}\trelation={}\tcandidates={}\tadmitted={}",
                    #index, #phase, __cq_gpu_round, #relation_name, candidates.len(), fresh.len());
            }
            fresh
        }; ::std::mem::drop(#next);))
    }).collect::<syn::Result<Vec<_>>>()?;
    let mut publication = Vec::new();
    for transaction in &stratum.publication {
        let i = transaction.relation;
        let fresh = local("fresh", i);
        let packed = local("packing", i);
        let state = local("state", i);
        for operation in &transaction.operations {
            match operation.kind.to_string().as_str() {
                "full_layouts" => for &j in &operation.layouts {
                    let layout = &context.layouts[j];
                    let layout_state = local("layout", j);
                    let schema = local("layout_packing", j);
                    let order = &layout.columns;
                    let host = &layout.host;
                    let ty = &layout.provider;
                    publication.push(quote!({
                        let permuted = #packed.permute_device(#fresh.buffer(), #fresh.len(), &[#(#order),*])?;
                        // SAFETY: a column permutation preserves uniqueness of the admitted delta.
                        let run = unsafe { #layout_state.sort_known_unique(&permuted, #fresh.len())? };
                        unsafe { #layout_state.append_sorted(run)?; }
                        #host = <#ty>::from_runs(__cq_gpu_pool.clone(), #schema.clone(), #layout_state.owned_runs())?;
                    }));
                },
                "all_extend" => {
                    let all = &context.relations[i].all;
                    let ty = canonical_type(schemas, i);
                    publication.push(quote!(#all = <#ty>::from_runs(__cq_gpu_pool.clone(), #packed.clone(), #state.owned_runs())?;));
                }
                "delta_swap" => {
                    let delta = context.relations[i].delta_host.as_ref().expect("checked delta");
                    let ty = canonical_type(schemas, i);
                    publication.push(quote!(#delta = <#ty>::from_runs(__cq_gpu_pool.clone(), #packed.clone(), vec![#fresh.clone()])?;));
                }
                "delta_rebuild" => for &j in &operation.layouts {
                    let layout = &context.delta_layouts[j];
                    let schema = local("delta_packing", j);
                    let order = &layout.columns;
                    let host = &layout.host;
                    let ty = &layout.provider;
                    publication.push(quote!({
                        let permuted = #packed.permute_device(#fresh.buffer(), #fresh.len(), &[#(#order),*])?;
                        // SAFETY: a column permutation preserves uniqueness of the admitted delta.
                        let run = unsafe { #state.sort_known_unique(&permuted, #fresh.len())? };
                        #host = <#ty>::from_runs(__cq_gpu_pool.clone(), #schema.clone(), vec![run])?;
                    }));
                },
                _ => unreachable!("checked publication operation"),
            }
        }
    }
    let release_fresh = stratum.members.iter().map(|&i| {
        let fresh = local("fresh", i);
        quote!(::std::mem::drop(#fresh);)
    });
    let before = memory_snapshot(context, &[], &[], &format!("stratum={index}/{phase}/before"));
    let after = memory_snapshot(context, &stratum.members, &stratum.members, &format!("stratum={index}/{phase}/after"));
    Ok(quote!(
        #before
        let __cq_gpu_readbacks_before = if __cq_gpu_trace_rounds {
            ::gpu_datalog_runtime::readback_count()
        } else { 0 };
        #(#batches)* #(#waves)* #(#admissions)* #(#publication)*
        #(#release_fresh)*
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
