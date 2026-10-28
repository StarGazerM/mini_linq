//! Test-only bridge from checked execution certificates to the base Round oracle.
use quote::quote;

/// Check the extension language, then evaluate its recorded Round meaning.
/// The base interpreter reads certified whole-input bindings as complete sets;
/// shard ownership and barriers are validated by the extension contract.
#[proc_macro]
pub fn eval_round(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_datalog_partition::ShardedProgram, syn::Expr>(
        input.into(),
        |program, arguments| {
            if !arguments.is_empty() {
                return Err(syn::Error::new_spanned(
                    arguments,
                    "reference terminal takes no arguments",
                ));
            }
            let round = &program.round;
            syn::parse2(quote!(
                ::cq_datalog_test_passes::eval_round! { pipeline {}; #round }
            ))
        },
    )
    .into()
}

/// Exercise per-hole advice without adding a production policy parser.
#[proc_macro]
pub fn hole_policy(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, cq_datalog_partition::ShardedProgram>(
        input.into(),
        |program, _| {
            let mut program = program.clone();
            program.schedule = program.schedule.with_policy(
                0,
                syn::parse_quote!(phase { seed } index { 0 } grain { [1] } inline_below { [0] }),
            );
            cq_datalog_partition::compile_sharded(&program)
        },
    )
    .into()
}

/// Supply a legal nondefault publication order through the public Round pipeline.
#[proc_macro]
pub fn reorder_publication(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(input.into(), |program, _| {
        let mut target = program.clone();
        for stratum in &mut target.context.strata {
            for publication in &mut stratum.publication {
                let mut full = Vec::new();
                let mut delta = Vec::new();
                for operation in &publication.operations {
                    if operation.kind == "full_layouts" {
                        full.extend(operation.layouts.iter().copied());
                    } else if operation.kind == "delta_rebuild" {
                        delta.extend(operation.layouts.iter().copied());
                    }
                }
                let mut operations = vec![syn::parse_quote!(
                    kind { all_extend } layouts { [] }
                )];
                operations.extend(full.into_iter().rev().map(|index| {
                    syn::parse_quote!(
                        kind { full_layouts } layouts { [{#index}] }
                    )
                }));
                operations.push(syn::parse_quote!(
                    kind { delta_swap } layouts { [] }
                ));
                operations.extend(delta.into_iter().rev().map(|index| {
                    syn::parse_quote!(
                        kind { delta_rebuild } layouts { [{#index}] }
                    )
                }));
                publication.operations = operations;
            }
        }
        cq_lang_datalog_round::check_rewrite(program, &target)?;
        Ok(target)
    })
    .into()
}

/// Instrument native full-layout hashing without changing publication records.
#[proc_macro]
pub fn trace_publication(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::storage::Program>(
        input.into(),
        |program, arguments| {
            let alias = if arguments.is_empty() {
                false
            } else {
                let name: syn::Ident = syn::parse2(arguments)?;
                if name != "alias" {
                    return Err(syn::Error::new_spanned(name, "expected alias"));
                }
                true
            };
            let mut target = program.clone();
            let mut ordinal = 0usize;
            for layout in &mut target.layouts {
                let declaration = target
                    .incremental
                    .strata
                    .source
                    .declarations
                    .iter()
                    .find(|declaration| declaration.name == layout.relation)
                    .expect("checked layout relation");
                if declaration.value.is_some() {
                    continue;
                }
                let types: Vec<_> = layout
                    .columns
                    .iter()
                    .map(|&index| &declaration.columns[index].ty)
                    .collect();
                let (keys, values) = types.split_at(layout.key);
                layout.provider = syn::parse_quote!(
                    ::std::collections::HashMap<
                        (#(#keys,)*),
                        ::std::vec::Vec<(#(#values,)*)>,
                        PublicationHasher<#ordinal>
                    >
                );
                // DL-LayoutProtocol: the replacement declares its operations, not its spelling.
                layout.protocol = cq_lang_datalog_round::storage::LayoutProtocol::GroupedMap;
                ordinal += 1;
            }
            if alias {
                for layout in target.layouts.iter_mut().chain(&mut target.delta_layouts) {
                    let provider = &layout.provider;
                    layout.provider = syn::parse_quote!(RunLayout<#provider>);
                }
            }
            Ok(target)
        },
    )
    .into()
}
