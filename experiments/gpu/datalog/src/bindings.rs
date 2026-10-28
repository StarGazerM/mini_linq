//! Checked construction of complete GPU Round invocations before translation.
use cq_datalog_support::ensure;
use cq_lang_datalog_round::{self as round, Hole, HoleInterface, Program};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
use syn::{Ident, Type};

fn inputs(program: &Program) -> syn::Result<BTreeMap<String, Type>> {
    let symbols = crate::storage::Symbols::from_program(&program.storage)?;
    let schemas = program.context.relations.iter().map(|relation|
        relation.declaration.columns.iter().map(|column| symbols.device_type(&column.ty)).collect()
    ).collect::<Vec<_>>();
    crate::print::input_types(program, &schemas)
}

fn packet(hole: &Hole, inputs: &BTreeMap<String, Type>) -> TokenStream {
    let declared = hole.cq.declarations.iter().map(|declaration| declaration.name.to_string())
        .collect::<BTreeSet<_>>();
    inputs.iter().filter(|(name, _)| declared.contains(*name)).map(|(name, ty)| {
        let name = Ident::new(name, proc_macro2::Span::call_site());
        quote!(#name => #ty;)
    }).collect()
}

/// Record the invocation owner by ordinal, independent of the invoked macro's name.
pub fn compile_with(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    struct Owner(usize);
    impl syn::parse::Parse for Owner {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let name: Ident = input.parse()?;
            if name != "binding" { return Err(syn::Error::new_spanned(name, "expected binding = invocation_index")); }
            input.parse::<syn::Token![=]>()?;
            let owner = input.parse::<syn::LitInt>()?.base10_parse()?;
            Ok(Self(owner))
        }
    }
    let Owner(owner) = syn::parse2(arguments)?;
    round::contract::check(source)?;
    crate::storage::check_device(&source.storage)?;
    let inputs = inputs(source)?;
    let mut target = source.clone();
    for hole in &mut target.holes {
        ensure(hole.interface == HoleInterface::Batch, &*hole, "GPU Datalog requires batch holes")?;
        ensure(owner > hole.lower && owner < hole.pipeline.passes.len().saturating_sub(1),
            &*hole, "binding owner must follow lowering and precede the terminal")?;
        ensure(hole.binding.is_none() && hole.pipeline.passes[owner].arguments.is_empty(),
            &*hole, "binding invocation already has arguments or an owner")?;
        let bindings = packet(hole, &inputs);
        hole.binding = Some(owner);
        hole.pipeline.passes[owner].arguments = bindings;
        let terminal = hole.pipeline.passes.last_mut().expect("checked terminal");
        ensure(terminal.arguments.is_empty(), &terminal.path, "GPU batch terminal already has arguments")?;
        terminal.arguments = quote!(&mut *__cq_gpu_arena);
    }
    round::check_rewrite(source, &target)?;
    check(&target)?;
    Ok(target)
}

/// Formal rule DL-DeviceBindings: the declared owner contains exactly the
/// input-layout packet and the terminal contains its complete arena argument.
pub fn check(program: &Program) -> syn::Result<()> {
    let inputs = inputs(program)?;
    for hole in &program.holes {
        let owner = hole.binding.ok_or_else(|| syn::Error::new_spanned(hole, "missing GPU binding invocation owner"))?;
        let invocation = hole.pipeline.passes.get(owner).ok_or_else(||
            syn::Error::new_spanned(hole, "GPU binding invocation owner is out of bounds"))?;
        ensure(owner > hole.lower && owner < hole.pipeline.passes.len().saturating_sub(1),
            hole, "GPU binding invocation owner is outside the Structures pipeline")?;
        ensure(invocation.arguments.to_string() == packet(hole, &inputs).to_string(),
            hole, "GPU binding invocation differs from its declared inputs")?;
        ensure(hole.pipeline.passes.last().is_some_and(|terminal|
            terminal.arguments.to_string() == quote!(&mut *__cq_gpu_arena).to_string()),
            hole, "GPU batch invocation differs from its recorded arena argument")?;
    }
    Ok(())
}
