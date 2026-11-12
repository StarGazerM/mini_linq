//! Checked construction of complete GPU Round invocations before translation.
use cq_datalog_support::ensure;
use cq_lang_datalog_round::{self as round, Hole, HoleInterface, InputSource, Program, storage};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
use syn::{Ident, Type};

/// Produce input records with the shared Structures declaration constructor.
/// Statistics demand, when selected, belongs to Structures refinement; native
/// input bindings do not invent publication demands.
pub(crate) fn input_providers(
    program: &Program,
) -> syn::Result<BTreeMap<String, cq_lang_structures::Provider>> {
    let context = &program.context;
    let device = program.storage.device.as_ref()
        .ok_or_else(|| syn::Error::new_spanned(program, "missing device storage records"))?;
    let provider = |source: &storage::Provider,
                    input: &cq_lang_structures::provider::InputSchema,
                    word: &Type,
                    state: &storage::DeviceState| -> syn::Result<cq_lang_structures::Provider> {
        let declaration = device.providers.iter()
            .find(|declaration| declaration.token == source.token)
            .ok_or_else(|| syn::Error::new_spanned(&source.token, "undeclared device input provider"))?;
        let mut instance = declaration.instance();
        instance.input = Some(input.clone());
        instance.radix = Some(cq_lang_structures::provider::RadixAdvice {
            word: word.clone(),
            offset: state.offset.clone(),
            ratio: state.ratio,
            radix_bits: state.radix_bits,
            merge_tile: state.merge_tile_rows,
        });
        instance.ty = source.ty.clone();
        Ok(instance)
    };
    let mut inputs = BTreeMap::new();
    for (relation, state) in context.relations.iter().zip(&device.relations) {
        if let Some(delta) = &relation.delta {
            inputs.insert(
                delta.to_string(),
                provider(&relation.providers.delta, &state.input, &state.word,
                    &device.layouts[relation.providers.canonical])?,
            );
        }
    }
    for (layout, state) in context
        .layouts
        .iter()
        .zip(&device.layouts)
        .chain(context.delta_layouts.iter().zip(&device.delta_layouts))
    {
        inputs.insert(
            layout.name.to_string(),
            provider(&layout.provider, &state.input, &state.word, state)?,
        );
    }
    for (component, stratum) in context.strata.iter().enumerate() {
        for key in &stratum.keys {
            ensure(
                !key.columns.is_empty(),
                program,
                "GPU Datalog does not support unit-valued existential negation keys",
            )?;
            let state = &device.negations[crate::print::negation(program, component, key)?];
            inputs.insert(key.name.to_string(), provider(&key.provider, &state.input, &state.word, state)?);
        }
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            if let Some(scan) = &rule.scan {
                let input = match scan.source {
                    InputSource::Delta(index) => provider(
                        &context.relations[index].providers.delta,
                        &device.relations[index].input,
                        &device.relations[index].word,
                        &device.layouts[context.relations[index].providers.canonical],
                    ),
                    InputSource::Layout(index) => provider(
                        &context.layouts[index].provider,
                        &device.layouts[index].input,
                        &device.layouts[index].word,
                        &device.layouts[index],
                    ),
                }?;
                inputs.insert(scan.binding.to_string(), input);
            }
        }
    }
    Ok(inputs)
}

/// The recorded device provider of each component's negative key set, as a
/// `radix_relation` token instance (declared by device storage).
fn key_providers(program: &Program) -> syn::Result<Vec<Vec<round::storage::Provider>>> {
    let device = program
        .storage
        .device
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(program, "missing device storage records"))?;
    program
        .context
        .strata
        .iter()
        .enumerate()
        .map(|(component, stratum)| {
            stratum
                .keys
                .iter()
                .map(|key| {
                    let state = &device.negations[crate::print::negation(program, component, key)?];
                    Ok(round::storage::Provider {
                        token: state.token.clone(),
                        ty: state.provider.clone(),
                    })
                })
                .collect()
        })
        .collect()
}

fn domain_declarations(program: &Program) -> syn::Result<Vec<Option<Vec<cq_syntax::RelationDecl>>>> {
    if !crate::storage::uses_domains(&program.storage) {
        return Ok(vec![None; program.holes.len()]);
    }
    let domains = crate::domains::DomainComponents::infer(&program.storage.incremental.strata.source)?;
    let symbols = crate::storage::Symbols::from_program(&program.storage)?;
    let mut names = BTreeMap::new();
    for (index, relation) in program.context.relations.iter().enumerate() {
        if let Some(delta) = &relation.delta {
            names.insert(delta.to_string(), domains.columns[index].clone());
        }
    }
    for layout in program.context.layouts.iter().chain(&program.context.delta_layouts) {
        names.insert(layout.name.to_string(), layout.columns.iter()
            .map(|&column| domains.columns[layout.relation][column]).collect());
    }
    for stratum in &program.context.strata {
        for key in &stratum.keys {
            let ids: Vec<_> = key.columns.iter().map(|&column| domains.columns[key.relation][column]).collect();
            names.insert(key.name.to_string(), ids.clone());
            names.insert(program.holes[key.hole].cq.query.head.relation.to_string(), ids);
        }
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            names.insert(program.holes[rule.hole].cq.query.head.relation.to_string(), domains.columns[rule.head].clone());
            if let Some(scan) = &rule.scan {
                let ids = match scan.source {
                    InputSource::Delta(index) => domains.columns[index].clone(),
                    InputSource::Layout(index) => {
                        let layout = &program.context.layouts[index];
                        layout.columns.iter().map(|&column| domains.columns[layout.relation][column]).collect()
                    }
                };
                names.insert(scan.binding.to_string(), ids);
            }
        }
    }
    program.holes.iter().map(|hole| crate::encode::declarations(&hole.cq, &names, &symbols).map(Some)).collect()
}

/// Each input carries its Storage token, declared GPU facts and physical schema.
fn packet(
    hole: &Hole,
    inputs: &BTreeMap<String, cq_lang_structures::Provider>,
    providers: &[cq_lang_structures::Declaration],
    encoded: Option<&[cq_syntax::RelationDecl]>,
) -> TokenStream {
    let declared = hole
        .cq
        .declarations
        .iter()
        .map(|declaration| declaration.name.to_string())
        .collect::<BTreeSet<_>>();
    let inputs = inputs
        .iter()
        .filter(|(name, _)| declared.contains(*name))
        .collect::<Vec<_>>();
    let providers = providers.iter().filter(|declaration| {
        inputs
            .iter()
            .any(|(_, provider)| provider.token == declaration.token)
    });
    let bindings = inputs.iter().map(|(name, provider)| {
        let mut adapted;
        let provider = if let Some(declarations) = encoded {
            adapted = (**provider).clone();
            if let Some(input) = &mut adapted.input {
                let declaration = declarations.iter().find(|declaration| declaration.name == name.as_str())
                    .expect("checked hole input declaration");
                for (column, declared) in input.columns.iter_mut().zip(&declaration.columns) {
                    column.logical = declared.ty.clone();
                    column.logical_type_id = declared.logical_type_id;
                    column.codec_id = None;
                    column.codec = None;
                }
            }
            &adapted
        } else { provider };
        let name = Ident::new(name, proc_macro2::Span::call_site());
        quote!(#name => #provider;)
    });
    quote!(providers { #(#providers)* } #(#bindings)*)
}

/// Record the invocation owner by ordinal, independent of the invoked macro's name.
pub fn compile_with(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    struct Owner(usize);
    impl syn::parse::Parse for Owner {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let name: Ident = input.parse()?;
            if name != "binding" {
                return Err(syn::Error::new_spanned(
                    name,
                    "expected binding = invocation_index",
                ));
            }
            input.parse::<syn::Token![=]>()?;
            let owner = input.parse::<syn::LitInt>()?.base10_parse()?;
            Ok(Self(owner))
        }
    }
    let Owner(owner) = syn::parse2(arguments)?;
    round::contract::check(source)?;
    crate::storage::check_device(&source.storage)?;
    let mut target = source.clone();
    let inputs = input_providers(&target)?;
    let encoded = domain_declarations(&target)?;
    for (index, hole) in target.holes.iter_mut().enumerate() {
        ensure(
            hole.interface == HoleInterface::Batch,
            &*hole,
            "GPU Datalog requires batch holes",
        )?;
        ensure(
            owner > hole.lower && owner < hole.pipeline.passes.len().saturating_sub(1),
            &*hole,
            "binding owner must follow lowering and precede the terminal",
        )?;
        ensure(
            hole.binding.is_none() && hole.pipeline.passes[owner].arguments.is_empty(),
            &*hole,
            "binding invocation already has arguments or an owner",
        )?;
        if let Some(declarations) = &encoded[index] {
            let adapter = &mut hole.pipeline.passes[0];
            ensure(adapter.arguments.is_empty(), &*adapter, "domain surface adapter already has arguments")?;
            adapter.arguments = quote!(#(#declarations)*);
        }
        let bindings = packet(
            hole,
            &inputs,
            &target
                .storage
                .device
                .as_ref()
                .expect("checked device records")
                .providers,
            encoded[index].as_deref(),
        );
        hole.binding = Some(owner);
        hole.pipeline.passes[owner].arguments = bindings;
        let terminal = hole.pipeline.passes.last_mut().expect("checked terminal");
        ensure(
            terminal.arguments.is_empty(),
            &terminal.path,
            "GPU batch terminal already has arguments",
        )?;
        if encoded[index].is_some() {
            terminal.path = syn::parse_quote!(::cq_gpu_datalog::translate_domain_arena);
        }
        terminal.arguments = quote!(&mut *__cq_gpu_arena);
    }
    round::check_rewrite(source, &target)?;
    check(&target)?;
    Ok(target)
}

/// Formal rule DL-DeviceBindings: negative key sets use their recorded device
/// providers, the declared owner contains exactly the input-layout packet and
/// the terminal contains its complete arena argument.
pub fn check(program: &Program) -> syn::Result<()> {
    for (stratum, providers) in program.context.strata.iter().zip(key_providers(program)?) {
        for (key, provider) in stratum.keys.iter().zip(providers) {
            ensure(
                key.provider == provider && key.hasher == round::storage::Hasher::None,
                key,
                "GPU key set differs from its recorded device provider",
            )?;
        }
    }
    let inputs = input_providers(program)?;
    let encoded = domain_declarations(program)?;
    for (index, hole) in program.holes.iter().enumerate() {
        let owner = hole
            .binding
            .ok_or_else(|| syn::Error::new_spanned(hole, "missing GPU binding invocation owner"))?;
        let invocation = hole.pipeline.passes.get(owner).ok_or_else(|| {
            syn::Error::new_spanned(hole, "GPU binding invocation owner is out of bounds")
        })?;
        ensure(
            owner > hole.lower && owner < hole.pipeline.passes.len().saturating_sub(1),
            hole,
            "GPU binding invocation owner is outside the Structures pipeline",
        )?;
        if let Some(declarations) = &encoded[index] {
            ensure(hole.pipeline.passes[0].arguments.to_string() == quote!(#(#declarations)*).to_string(),
                hole, "domain surface adapter differs from checked component declarations")?;
        }
        ensure(
            invocation.arguments.to_string()
                == packet(
                    hole,
                    &inputs,
                    &program
                        .storage
                        .device
                        .as_ref()
                        .expect("checked device records")
                        .providers,
                    encoded[index].as_deref(),
                )
                .to_string(),
            hole,
            "GPU binding invocation differs from its declared inputs",
        )?;
        ensure(
            hole.pipeline.passes.last().is_some_and(|terminal| {
                terminal.arguments.to_string() == quote!(&mut *__cq_gpu_arena).to_string()
                    && (encoded[index].is_some()
                        == (terminal.path == syn::parse_quote!(::cq_gpu_datalog::translate_domain_arena)))
            }),
            hole,
            "GPU batch invocation differs from its recorded arena argument",
        )?;
    }
    Ok(())
}
