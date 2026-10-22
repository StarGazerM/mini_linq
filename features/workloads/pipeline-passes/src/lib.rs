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

/// Select the corpus terminal at macro expansion time. Cargo tracks the switch
/// through this crate's build script, so changing it rebuilds consuming queries.
#[proc_macro]
pub fn terminal(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    use syn::parse::{Parse, ParseStream};
    struct Input {
        printer: cq_stage::Invocation,
        pipeline: cq_stage::Pipeline,
        program: proc_macro2::TokenStream,
    }
    impl Parse for Input {
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            let arguments: syn::Ident = input.parse()?;
            if arguments != "arguments" {
                return Err(syn::Error::new_spanned(
                    arguments,
                    "expected printer arguments",
                ));
            }
            let content;
            syn::braced!(content in input);
            let printer = content.parse()?;
            input.parse::<syn::Token![;]>()?;
            let pipeline = input.parse()?;
            input.parse::<syn::Token![;]>()?;
            Ok(Self {
                printer,
                pipeline,
                program: input.parse()?,
            })
        }
    }
    let result = (|| {
        let Input {
            printer,
            mut pipeline,
            program,
        } = syn::parse(input)?;
        let mut passes = if option_env!("MINI_LINQ_SEQUENCE") == Some("1") {
            vec![
                syn::parse_quote!(::mini_linq::passes::sequence),
                syn::parse_quote!(::mini_linq::passes::sequential),
                syn::parse_quote!(::mini_linq::passes::translate_pull),
            ]
        } else {
            vec![printer]
        };
        passes.append(&mut pipeline.passes);
        cq_stage::Pipeline { passes }.start(program)
    })();
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}
