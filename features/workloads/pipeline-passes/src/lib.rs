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

/// Compile an existing authored corpus query through its exact configured
/// pipeline, replacing only the terminal with a serialized Sequence program.
#[proc_macro]
pub fn conformance_program(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    use quote::{ToTokens, quote};
    use syn::parse::{Parse, ParseStream};
    struct Request {
        case: syn::Ident,
        positional: bool,
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
            let pipeline;
            syn::braced!(pipeline in input);
            Ok(Self {
                case,
                positional,
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
        let source = source.to_token_stream();
        let path = path.to_string_lossy();
        Ok(quote!({
            const _: &str = include_str!(#path);
            ::mini_linq::compile! {
                pipeline {
                    #pipeline
                    select = ::mini_linq::passes::select(#advice);
                    print = ::cq_workload_passes::sequence_literal;
                }
                #source
            }
        }))
    })();
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Preserve the complete selected construction for runtime target conformance.
#[proc_macro]
pub fn sequence_literal(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    use quote::ToTokens;
    cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(input.into(), |source, _| {
        let sequence = cq_sequence_lowering::compile(source)?;
        cq_sequence_lowering::check(source, &sequence)?;
        let source = sequence.to_token_stream().to_string();
        Ok(syn::parse_quote!(::syn::parse_str::<::cq_lang_sequence::Program>(#source).unwrap()))
    })
    .into()
}
