use crate::{print, storage};
use quote::{quote, ToTokens};

fn program() -> cq_lang_datalog_round::Program {
    program_with_symbols(syn::parse_quote!(i32), quote!())
}

fn program_with_symbols(ty: syn::Type, symbols: proc_macro2::TokenStream) -> cq_lang_datalog_round::Program {
    let host = cq_datalog_storage::compile(&syn::parse_quote! {
        struct Paths;
        relation Edge(x:#ty,y:#ty) = (edges);
        relation Reach(x:#ty,y:#ty);
        output Reach;
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Reach(x,y), Edge(y,z);
        strata { component [Reach]; }
        incremental { scheme seminaive_covering;
            delta Reach = ReachChanges;
            variant 0 seed { Reach(*x,*y) :- Edge(x,y); }
            variant 1 delta 0 { Reach(*x,*z) :- ReachChanges(x,y), Edge(y,z); }
        }
    }).unwrap();
    let device = storage::compile_with(&host, symbols).unwrap();
    let mut pipeline = cq_datalog_fork::default_pipeline();
    let terminal = pipeline.passes.last_mut().unwrap();
    terminal.path = syn::parse_quote!(::cq_gpu_sequence::translate_arena);
    // Printer contract tests splice tokens only; actual stage execution belongs
    // to the CUDA oracle using the fully configured Structures/Sequence queue.
    pipeline.passes.insert(pipeline.passes.len() - 1,
        syn::parse_quote!(::cq_gpu_datalog::bind_inputs));
    let binding = pipeline.passes.len() - 2;
    let round = cq_datalog_fork::compile_with(&device, quote!(
        cq = #pipeline;
        lower = 2;
        terminals = [::cq_gpu_sequence::translate_arena => batch];
    )).unwrap();
    crate::bindings::compile_with(&round, quote!(binding = #binding)).unwrap()
}

#[test]
fn tc_emits_resident_waves_admission_and_publication() {
    let program = program();
    let source = print::compile(&program).unwrap().to_token_stream().to_string();
    assert!(source.contains("concatenate_batches"));
    assert!(source.contains("permute_device"));
    assert!(source.contains("append_sorted"));
    assert!(source.contains("while"));
    assert!(source.contains("translate_arena"));
    assert_eq!(source.matches("to_host").count(), program.context.outputs.len());
    assert_eq!(source.matches("Rows :: rows").count(), 1);
    assert!(!source.contains("synchronize"));
    assert_eq!(source.matches("release_cached_scratch").count(), program.context.strata.len());
    assert_eq!(source.matches("release_unused").count(), program.context.strata.len());
    assert!(source.contains("std :: mem :: drop"));
}

#[test]
fn rejects_host_observers_and_timing() {
    let mut observed = program();
    observed.storage.incremental.strata.source.observer = Some(syn::parse_quote!(observer));
    observed.context.observer = Some(syn::parse_quote!(observer));
    assert!(print::compile(&observed).unwrap_err().to_string().contains("observer"));
    let mut timed = program();
    timed.policy.timing = syn::parse_quote!(phases);
    assert!(print::compile(&timed).unwrap_err().to_string().contains("timing"));
}

#[test]
fn rejects_missing_input_binding_stage() {
    let mut program = program();
    for hole in &mut program.holes {
        hole.binding = None;
    }
    assert!(print::compile(&program).unwrap_err().to_string().contains("binding invocation owner"));
}

#[test]
fn checked_invocations_survive_roundtrip_and_aliasing() {
    let mut program = program();
    for hole in &mut program.holes {
        let binding = hole.binding.unwrap();
        hole.pipeline.passes[binding].path = syn::parse_quote!(::consumer::resident_inputs);
    }
    let roundtrip = syn::parse2::<cq_lang_datalog_round::Program>(quote!(#program)).unwrap();
    assert_eq!(roundtrip, program);
    assert!(print::compile(&roundtrip).is_ok());
    let binding = program.holes[0].binding.unwrap();
    program.holes[0].pipeline.passes[binding].arguments = quote!();
    assert!(print::compile(&program).unwrap_err().to_string().contains("declared inputs"));
}

#[test]
fn rejects_unrecorded_batch_arena_arguments() {
    let mut program = program();
    program.holes[0].pipeline.passes.last_mut().unwrap().arguments = quote!();
    assert!(print::compile(&program).unwrap_err().to_string().contains("arena argument"));
}

#[test]
fn device_bindings_preserve_scan_and_keyed_readers() {
    let source = syn::parse_quote! {
        physical {
            relation Delta(x:i32,y:i32) = (&delta);
            relation Edge(y:i32,z:i32) = (&edge);
            consumer canonical;
            d = pipe rename Delta@0 {x -> x,y -> y};
            e = pipe rename Edge@1 {y -> y,z -> z};
            joined = pipe probe first [y] d,e;
            result = pipe project joined keep {x,z};
            return result as answer(x,z);
        }
        structures {
            d host scan = ::core::iter::Map<_,_> [];
            e host scan = ::core::iter::Map<_,_> [];
            e host result(joined) = ::cq_storage::HashIndex<'_,_,_> [y];
        }
    };
    let target = crate::bind_inputs(&source, quote! {
        Delta => ::gpu_datalog_runtime::DeviceLayout<(i32,),(i32,)> ;
        Edge => ::gpu_datalog_runtime::DeviceLayout<(i32,),(i32,)> ;
    }).unwrap();
    assert_eq!(target.structures.entries.len(), 3);
    for (before, after) in source.structures.entries.iter().zip(&target.structures.entries) {
        let cq_lang_structures::Structure::Host { reader: before, .. } = &before.structure else { panic!() };
        let cq_lang_structures::Structure::Host { reader: after, .. } = &after.structure else { panic!() };
        assert_eq!(before, after);
    }
}

#[test]
fn symbols_cross_only_host_boundaries_and_keep_logical_types() {
    let program = program_with_symbols(syn::parse_quote!(String), quote!(symbols(String => TextCodec)));
    let source = print::compile(&program).unwrap().to_token_stream().to_string();
    assert_eq!(source.matches(":: code").count(), 2);
    assert_eq!(source.matches(":: decode").count(), 2);
    assert!(source.contains("Symbol < String , TextCodec >"));
    assert!(source.contains("Column :: U32"));
    assert!(!source.contains("as String"));
    for relation in &program.context.relations {
        assert!(relation.declaration.columns.iter().all(|column| column.ty == syn::parse_quote!(String)));
    }
}
