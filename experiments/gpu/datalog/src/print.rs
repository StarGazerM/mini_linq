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
fn canonical_type(context: &RoundContext, index: usize) -> Type {
    let types = types(context, index);
    crate::storage::layout_type(&types, types.len())
}
fn input_types(program: &Program) -> syn::Result<std::collections::BTreeMap<String, Type>> {
    let context = &program.context;
    let mut inputs = std::collections::BTreeMap::new();
    for (index, relation) in context.relations.iter().enumerate() {
        let ty = canonical_type(context, index);
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
            let columns = types(context, key.relation);
            let selected = key.columns.iter().map(|&i| columns[i]).collect::<Vec<_>>();
            inputs.insert(key.name.to_string(), crate::storage::layout_type(&selected, selected.len()));
        }
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            if let Some(scan) = &rule.scan {
                let ty = match scan.source {
                    InputSource::Delta(index) => canonical_type(context, index),
                    InputSource::Layout(index) => context.layouts[index].provider.clone(),
                };
                inputs.insert(scan.binding.to_string(), ty);
            }
        }
    }
    Ok(inputs)
}

/// Formal rule P-DL-Gpu: Interpret the recorded waves, admission owners and
/// publication operations using immutable packed runs on one device stream.
pub fn compile(program: &Program) -> syn::Result<syn::Expr> {
    crate::storage::check_device(&program.storage)?;
    round::contract::check(program)?;
    ensure(program.context.observer.is_none(), program, "GPU Datalog does not support observer callbacks")?;
    ensure(program.policy.timing == "none", program, "GPU Datalog does not support recorded phase timing")?;
    ensure(program.execution.is_empty(), program, "GPU Datalog does not support host execution certificates")?;
    for hole in &program.holes {
        ensure(hole.interface == HoleInterface::Batch, hole, "GPU Datalog requires batch holes")?;
    }
    let context = &program.context;
    // Check eligibility independently, including schemas not read by a hole.
    for index in 0..context.relations.len() { crate::storage::word_type(types(context, index))?; }
    let input_types = input_types(program)?;
    let bindings = input_types.iter().map(|(name, ty)| {
        let name = Ident::new(name, proc_macro2::Span::call_site());
        quote!(#name => #ty;)
    }).collect::<TokenStream>();
    let holes = cq_datalog_print::splice_with(program, |hole| {
        let mut pipeline = hole.pipeline.clone();
        let mut bound = false;
        for pass in &mut pipeline.passes {
            if pass.path.segments.last().is_some_and(|segment| segment.ident == "bind_inputs") {
                ensure(pass.arguments.is_empty(), hole, "device input bindings belong to the Round printer")?;
                pass.arguments = bindings.clone();
                bound = true;
            }
        }
        ensure(bound, hole, "GPU Datalog CQ pipeline requires cq_gpu_datalog::bind_inputs before bind_runs")?;
        let terminal = pipeline.passes.last_mut().expect("checked terminal");
        ensure(terminal.arguments.is_empty(), hole, "GPU Datalog owns the batch arena argument")?;
        terminal.arguments = quote!(&mut *__cq_gpu_arena);
        pipeline.start(hole.cq.to_token_stream())
    })?;
    let mut initialize = Vec::new();
    for (index, relation) in context.relations.iter().enumerate() {
        let state = local("state", index);
        let packed = local("packing", index);
        let arena = local("arena", index);
        let word = crate::storage::word_type(types(context, index))?;
        let schema = packing(&types(context, index))?;
        let all = &relation.all;
        let ty = canonical_type(context, index);
        initialize.push(quote!(
            let #packed = #schema;
            let mut #state = ::gpu_datalog_runtime::DeviceRuns::<#word>::new(__cq_gpu_pool.clone())?;
            let mut #arena = ::gpu_datalog_runtime::CandidateArena::new(__cq_gpu_pool.clone());
        ));
        if let Some(value) = &relation.declaration.value {
            let expression = &value.expression;
            let fields = (0..relation.declaration.arity()).map(syn::Index::from)
                .map(|field| quote!(*row.#field as i128));
            initialize.push(quote!({
                let host = &(#expression);
                let words = ::cq_storage::rows::Rows::rows(host)
                    .map(|row| #packed.pack::<#word>(&[#(#fields),*]))
                    .collect::<::gpu_datalog_runtime::Result<::std::vec::Vec<_>>>()?;
                let upload = ::gpu_datalog_runtime::Buffer::from_host_async(__cq_gpu_pool.clone(), &words)?;
                #state.admit(&upload, words.len())?;
            }));
        }
        initialize.push(quote!(let mut #all: #ty = <#ty>::from_runs(
            __cq_gpu_pool.clone(), #packed.clone(), #state.owned_runs())?;));
        if let Some(delta) = &relation.delta_host {
            initialize.push(quote!(let mut #delta: #ty = <#ty>::from_runs::<#word>(
                __cq_gpu_pool.clone(), #packed.clone(), ::std::vec::Vec::new())?;));
        }
    }
    for (index, layout) in context.layouts.iter().enumerate() {
        let state = local("layout", index);
        let relation_state = local("state", layout.relation);
        let packed = local("packing", layout.relation);
        let schema = local("layout_packing", index);
        let word = crate::storage::word_type(types(context, layout.relation))?;
        let order = &layout.columns;
        let ty = &layout.provider;
        let host = &layout.host;
        initialize.push(quote!(
            let #schema = #packed.permute(&[#(#order),*])?;
            let mut #state = ::gpu_datalog_runtime::DeviceRuns::<#word>::new(__cq_gpu_pool.clone())?;
            for run in #relation_state.owned_runs() {
                let permuted = #packed.permute_device(run.buffer(), run.len(), &[#(#order),*])?;
                let sorted = #state.sort_unique(&permuted, run.len())?;
                // A permutation is injective; canonical accepted runs are disjoint.
                unsafe { #state.append_sorted(sorted)?; }
            }
            let mut #host: #ty = <#ty>::from_runs(__cq_gpu_pool.clone(), #schema.clone(), #state.owned_runs())?;
        ));
    }
    for (index, layout) in context.delta_layouts.iter().enumerate() {
        let packed = local("packing", layout.relation);
        let schema = local("delta_packing", index);
        let word = crate::storage::word_type(types(context, layout.relation))?;
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
            let relation_types = types(context, key.relation);
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
                let state = ::gpu_datalog_runtime::DeviceRuns::<#word>::new(__cq_gpu_pool.clone())?;
                let run = state.sort_unique(&buffer, buffer.len())?;
                <#ty>::from_runs(__cq_gpu_pool.clone(), packing, vec![run])?
            };));
        }
        let seed = phase(program, index, round::schedule::SEED, &holes)?;
        let step = phase(program, index, round::schedule::STEP, &holes)?;
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
        strata.push(quote!({ #(#keys)* #seed #rounds }));
    }
    let outputs = context.outputs.iter().map(|output| {
        let index = output.relation;
        let state = local("state", index);
        let packed = local("packing", index);
        let fields = types(context, index).into_iter().enumerate().map(|(i, ty)| quote!(row[#i] as #ty));
        // Both recorded conversions return owned tuples; EDB borrow cloning is
        // already discharged by the one-time upload of supported Copy columns.
        quote!(#state.to_host()?.into_iter().map(|word| {
            let row = #packed.unpack(word)?;
            Ok((#(#fields,)*))
        }).collect::<::gpu_datalog_runtime::Result<::std::vec::Vec<_>>>()?.into_iter())
    }).collect::<Vec<_>>();
    let result = if outputs.len() == 1 { outputs[0].clone() } else { quote!((#(#outputs,)*)) };
    syn::parse2(quote!({
        (|| -> ::gpu_datalog_runtime::Result<_> {
            let __cq_gpu_pool = ::gpu_datalog_runtime::Pool::new(0, 0, 0)?;
            #(#initialize)*
            #(#strata)*
            Ok(#result)
        })().expect("GPU Datalog execution failed")
    }))
}

fn phase(program: &Program, index: usize, phase: &str, holes: &[TokenStream]) -> syn::Result<TokenStream> {
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
                let __cq_gpu_arena = &mut #arena;
                #next.push(#invocation);
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
        let word = crate::storage::word_type(types(context, i))?;
        Ok(quote!(let #fresh = {
            let candidates = ::gpu_datalog_runtime::concatenate_batches::<#word>(__cq_gpu_pool.clone(), &#packed, &#next)?;
            #state.admit(&candidates, candidates.len())?
        };))
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
                        let run = #layout_state.sort_unique(&permuted, #fresh.len())?;
                        unsafe { #layout_state.append_sorted(run)?; }
                        #host = <#ty>::from_runs(__cq_gpu_pool.clone(), #schema.clone(), #layout_state.owned_runs())?;
                    }));
                },
                "all_extend" => {
                    let all = &context.relations[i].all;
                    let ty = canonical_type(context, i);
                    publication.push(quote!(#all = <#ty>::from_runs(__cq_gpu_pool.clone(), #packed.clone(), #state.owned_runs())?;));
                }
                "delta_swap" => {
                    let delta = context.relations[i].delta_host.as_ref().expect("checked delta");
                    let ty = canonical_type(context, i);
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
                        let run = #state.sort_unique(&permuted, #fresh.len())?;
                        #host = <#ty>::from_runs(__cq_gpu_pool.clone(), #schema.clone(), vec![run])?;
                    }));
                },
                _ => unreachable!("checked publication operation"),
            }
        }
    }
    Ok(quote!(#(#batches)* #(#waves)* #(#admissions)* #(#publication)*))
}
