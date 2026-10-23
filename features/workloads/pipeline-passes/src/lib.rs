//! Corpus-owned Structures rewrite: replace HashIndex leaves while preserving
//! every written curried level and all other representation decisions.
use syn::visit_mut::VisitMut;

#[proc_macro]
pub fn sorted_payload(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, arguments| {
        if !arguments.is_empty() {
            return Err(syn::Error::new_spanned(
                arguments,
                "sorted_payload takes no arguments",
            ));
        }
        struct Sorted;
        impl VisitMut for Sorted {
            fn visit_type_mut(&mut self, ty: &mut syn::Type) {
                syn::visit_mut::visit_type_mut(self, ty);
                if let syn::Type::Path(path) = ty {
                    if path
                        .path
                        .segments
                        .last()
                        .is_some_and(|s| s.ident == "HashIndex")
                    {
                        *ty = syn::parse_quote!(::cq_workloads::SortedIndex<_, _>);
                    }
                }
            }
        }
        let mut target = source.clone();
        for entry in &mut target.structures.entries {
            match &mut entry.structure {
                cq_lang_structures::Structure::Built { keyed, .. }
                | cq_lang_structures::Structure::Host { keyed, .. } => {
                    Sorted.visit_type_mut(&mut keyed.ty)
                }
                _ => {}
            }
        }
        Ok(target)
    })
    .into()
}

/// Choose ordered state for every recorded deduplication boundary.
///
/// A query without such a boundary has no state to replace. Keeping that
/// decision here lets the visible test matrix compose the same pass everywhere,
/// without running compiler analysis in a build script first.
#[proc_macro]
pub fn ordered_dedup(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, arguments| {
        if !arguments.is_empty() {
            return Err(syn::Error::new_spanned(
                arguments,
                "ordered_dedup takes no arguments",
            ));
        }
        let mut target = source.clone();
        for entry in &mut target.structures.entries {
            if let cq_lang_structures::Structure::Dedup { ty, .. }
            | cq_lang_structures::Structure::HeadDedup { ty, .. } = &mut entry.structure
            {
                *ty = syn::parse_quote!(::std::collections::BTreeSet<_>);
            }
        }
        cq_lang_structures::check_rewrite(source, &target)?;
        Ok(target)
    })
    .into()
}

/// Compile both CPU targets for an authored corpus configuration into the test
/// binary. The returned functions bind the same caller-owned fixture inputs.
#[proc_macro]
pub fn conformance_targets(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    use quote::{ToTokens, quote};
    use syn::parse::{Parse, ParseStream};
    struct Request {
        case: syn::Ident,
        positional: bool,
        loader: syn::Ident,
        pipeline: proc_macro2::TokenStream,
    }
    impl Parse for Request {
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            let case = input.parse()?;
            input.parse::<syn::Token![,]>()?;
            let advice;
            syn::bracketed!(advice in input);
            let positional = !advice.is_empty();
            let _: proc_macro2::TokenStream = advice.parse()?;
            input.parse::<syn::Token![,]>()?;
            let loader = input.parse()?;
            input.parse::<syn::Token![,]>()?;
            let pipeline;
            syn::braced!(pipeline in input);
            Ok(Self {
                case,
                positional,
                loader,
                pipeline: pipeline.parse()?,
            })
        }
    }
    let result = (|| -> syn::Result<proc_macro2::TokenStream> {
        let request: Request = syn::parse(input)?;
        let name = request.case.to_string();
        let name = name.strip_prefix("run_").unwrap_or(&name);
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/queries");
        let path = ["classic", "retail"]
            .into_iter()
            .map(|suite| root.join(suite).join(format!("{name}.rs")))
            .find(|path| path.is_file())
            .ok_or_else(|| syn::Error::new_spanned(&request.case, "unknown corpus query"))?;
        let source = cq_workload_authored::program(&path)?;
        let names: Vec<_> = source.inputs().map(|input| &input.name).collect();
        let advice = request.positional.then(|| quote!(positional(#(#names),*)));
        let pipeline = request.pipeline;
        let loader = request.loader;
        let source = source.to_token_stream();
        let path = path.to_string_lossy();
        Ok(quote!({
            const _: &str = include_str!(#path);
            ::mini_linq::compile! {
                pipeline {
                    #pipeline
                    select = ::mini_linq::passes::select(#advice);
                    print = ::cq_workload_passes::sequence_targets(#loader);
                }
                #source
            }
        }))
    })();
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Plan and emit both target functions after the configured Structures pipeline.
#[proc_macro]
pub fn sequence_targets(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    use cq_testkit::conformance::{Target, translate_with_catalog};
    use quote::quote;
    cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(input.into(), |source, arguments| {
        let loader: syn::Ident = syn::parse2(arguments)?;
        let sequence = cq_sequence_lowering::compile(source)?;
        cq_sequence_lowering::check(source, &sequence)?;
        // This provider belongs to the workload, not to either target crate.
        let catalog = cq_lang_sequence::Catalog {
            providers: vec![cq_lang_sequence::ProviderToken {
                ty: syn::parse_quote!(::cq_workloads::SortedIndex<_, _>),
                elements: None,
                capabilities: vec![
                    cq_lang_sequence::Capability::Exact,
                    cq_lang_sequence::Capability::Membership,
                ],
                levels: vec![
                    vec!["host".into()],
                    vec!["host".into(), "pool".into()],
                    vec!["host".into(), "pool".into(), "task".into()],
                ],
            }],
            ..Default::default()
        };
        let (sequential, _) = translate_with_catalog(&sequence, Target::Sequential, &catalog)?;
        let (tasks, canonical) = translate_with_catalog(&sequence, Target::Tasks, &catalog)?;
        let bindings: Vec<_> = sequence
            .structures
            .physical
            .inputs()
            .map(|input| {
                let ident = &input.name;
                let name = ident.to_string();
                let types = input.columns.iter().map(|column| &column.ty);
                if loader == "take" {
                    quote!(let #ident = &inputs.#loader::<(#(#types,)*)>(#name)[..];)
                } else {
                    quote!(let #ident = &inputs.#loader::<(#(#types,)*)>(#name);)
                }
            })
            .collect();
        Ok(syn::parse_quote!({
            #[allow(non_snake_case, unused_variables)]
            fn sequential(inputs: &mut ::cq_workloads::schema::Inputs) -> Vec<String> {
                #(#bindings)*
                (#sequential).into_iter().map(|row| format!("{row:?}")).collect()
            }
            #[allow(non_snake_case, unused_variables)]
            fn tasks(inputs: &mut ::cq_workloads::schema::Inputs) -> Vec<String> {
                #(#bindings)*
                (#tasks).into_iter().map(|row| format!("{row:?}")).collect()
            }
            (sequential as fn(&mut ::cq_workloads::schema::Inputs) -> Vec<String>,
             tasks as fn(&mut ::cq_workloads::schema::Inputs) -> Vec<String>,
             #canonical)
        }))
    })
    .into()
}
