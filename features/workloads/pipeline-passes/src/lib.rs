//! Corpus-owned Structures rewrite: replace `hash_index` instances, including
//! inner instances of composite tokens, by `sorted_index`, preserving every
//! written curried level and all other representation decisions.

/// Attach the workload's checked SF1 document before the ordinary public
/// pipeline runs. Query bodies remain opaque to this boundary adapter.
#[proc_macro]
pub fn tpch_evidence_query(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    use quote::quote;
    use syn::parse::{Parse, ParseStream};
    struct Request {
        pipeline: proc_macro2::TokenStream,
        visibility: syn::Visibility,
        name: syn::Ident,
        declarations: Vec<cq_syntax::RelationDecl>,
        body: proc_macro2::TokenStream,
    }
    impl Parse for Request {
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            let pipeline;
            syn::braced!(pipeline in input);
            let pipeline = pipeline.parse()?;
            let visibility = input.parse()?;
            input.parse::<syn::Token![struct]>()?;
            let name = input.parse()?;
            input.parse::<syn::Token![;]>()?;
            let declarations = cq_syntax::parse_declarations(input)?;
            Ok(Self { pipeline, visibility, name, declarations, body: input.parse()? })
        }
    }
    let result = (|| -> syn::Result<proc_macro2::TokenStream> {
        let Request { pipeline, visibility, name, declarations, body } = syn::parse(input)?;
        let evidence = cq_workload_authored::tpch_evidence::load_sf1(&declarations)?;
        Ok(quote!({
            ::mini_linq::compile! {
                pipeline { #pipeline }
                #visibility struct #name;
                #evidence
                #(#declarations)*
                #body
            }
        }))
    })();
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// The workload-owned sorted bucket index token (Rc-backed, host only). It
/// enumerates distinct keys, so replaced entries keep their `keys` support.
fn sorted_index() -> cq_lang_structures::Declaration {
    syn::parse_quote!(
        sorted_index = ::cq_workloads::SortedIndex<_, _> {
            reads [exact, membership], supports [keys], build typed
        };
    )
}

/// `provider` with every `hash_index` instance replaced by `sorted_index`.
/// A composite token is re-spelled from its declaration over the new inner.
fn sorted(
    providers: &cq_lang_structures::Providers,
    provider: &cq_lang_structures::Provider,
) -> syn::Result<cq_lang_structures::Provider> {
    if provider.token == "hash_index" {
        return Ok(sorted_index().instance());
    }
    let Some(inner) = &provider.inner else {
        return Ok(provider.clone());
    };
    let replaced = sorted(providers, inner)?;
    if replaced == **inner {
        return Ok(provider.clone());
    }
    providers
        .get(&provider.token)
        .ok_or_else(|| syn::Error::new_spanned(&provider.token, "undeclared provider token"))?
        .compose(replaced)
}

#[proc_macro]
pub fn sorted_payload(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::Structures::<cq_lang_structures::physical::Program, cq_lang_structures::Program>::rewrite(input.into(), |physical, source, arguments| {
        if !arguments.is_empty() {
            return Err(syn::Error::new_spanned(
                arguments,
                "sorted_payload takes no arguments",
            ));
        }
        let mut target = source.clone();
        for entry in &mut target.structures.entries {
            match &mut entry.structure {
                cq_lang_structures::Structure::Built { keyed, .. }
                | cq_lang_structures::Structure::Host { keyed, .. } => {
                    keyed.provider = sorted(&source.structures.providers, &keyed.provider)?;
                }
                _ => {}
            }
        }
        target.structures.declare(&[sorted_index()])?;
        Ok((physical.clone(), target))
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
                    print_input = structures;
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
    use cq_sequence_conformance::{Target, translate_with_catalog};
    use quote::quote;
    cq_stage::Structures::<cq_lang_structures::physical::Program, cq_lang_structures::Program>::pass::<syn::Expr>(input.into(), |physical, source, arguments| {
        let loader: syn::Ident = syn::parse2(arguments)?;
        let sequence = cq_sequence_lowering::compile(physical, source)?;
        cq_sequence_lowering::check(physical, source, &sequence)?;
        // This provider belongs to the workload, not to either target crate.
        // SortedIndex uses Rc-backed buckets and is a host-only provider.
        let mut catalog = cq_lang_sequence::Catalog::default();
        catalog.register(sorted_index(), vec![vec!["host".into()]]);
        let (sequential, _) = translate_with_catalog(physical, source, &sequence, Target::Sequential, &catalog)?;
        let (tasks, canonical) = translate_with_catalog(physical, source, &sequence, Target::Tasks, &catalog)?;
        let bindings: Vec<_> = physical
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
