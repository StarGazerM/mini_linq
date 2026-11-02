//! Named pipeline adapters for the checked GPU Sequence compiler.

use proc_macro::TokenStream;

fn no_arguments(arguments: proc_macro2::TokenStream) -> syn::Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            arguments,
            "GPU Sequence passes accept no arguments; record decisions in decompose",
        ))
    }
}

/// Explicit catalog capabilities, using the Sequence `CodecToken` fields.
struct CodecDeclarations(Vec<cq_lang_sequence::CodecToken>);

impl syn::parse::Parse for CodecDeclarations {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        fn name(input: syn::parse::ParseStream<'_>, expected: &str) -> syn::Result<()> {
            let actual: syn::Ident = input.parse()?;
            if actual == expected {
                Ok(())
            } else {
                Err(syn::Error::new(
                    actual.span(),
                    format!("expected `{expected}`"),
                ))
            }
        }
        let mut tokens = Vec::new();
        if input.is_empty() {
            return Ok(Self(tokens));
        }
        name(input, "codecs")?;
        let declarations;
        syn::bracketed!(declarations in input);
        while !declarations.is_empty() {
            name(&declarations, "CodecToken")?;
            let fields;
            syn::braced!(fields in declarations);
            name(&fields, "codec")?;
            fields.parse::<syn::Token![:]>()?;
            let variant: syn::Ident = fields.parse()?;
            let codec = if variant == "Some" {
                let ty;
                syn::parenthesized!(ty in fields);
                let codec = ty.parse::<syn::Type>()?;
                if !ty.is_empty() {
                    return Err(ty.error("expected one codec type"));
                }
                Some(codec)
            } else if variant == "None" {
                None
            } else {
                return Err(syn::Error::new(
                    variant.span(),
                    "expected `Some(Type)` or `None`",
                ));
            };
            fields.parse::<syn::Token![,]>()?;
            name(&fields, "ordered")?;
            fields.parse::<syn::Token![:]>()?;
            let ordered = fields.parse::<syn::LitBool>()?.value;
            fields.parse::<syn::Token![,]>()?;
            name(&fields, "levels")?;
            fields.parse::<syn::Token![:]>()?;
            let paths;
            syn::bracketed!(paths in fields);
            let mut levels = Vec::new();
            while !paths.is_empty() {
                let path;
                syn::bracketed!(path in paths);
                levels.push(
                    path.parse_terminated(|input| input.parse::<syn::Ident>(), syn::Token![,])?
                        .into_iter()
                        .map(|part| part.to_string())
                        .collect(),
                );
                if !paths.is_empty() {
                    paths.parse::<syn::Token![,]>()?;
                }
            }
            if !fields.is_empty() {
                fields.parse::<syn::Token![,]>()?;
            }
            if !fields.is_empty() {
                return Err(fields.error("unexpected CodecToken field"));
            }
            tokens.push(cq_lang_sequence::CodecToken {
                codec,
                ordered,
                levels,
            });
            if !declarations.is_empty() {
                declarations.parse::<syn::Token![,]>()?;
            }
        }
        Ok(Self(tokens))
    }
}

/// Keep GPU scheduling in Sequence rather than inheriting CPU batch decisions.
#[proc_macro]
pub fn stage(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        let target = cq_gpu_sequence_core::bind_structures(source)?;
        let analysis = cq_lang_structures::contract::check(&target)?;
        cq_gpu_sequence_core::check_structures(&target, &analysis)?;
        Ok(target)
    })
    .into()
}

/// Decompose a checked authored traversal without automatic relational replanning.
#[proc_macro]
pub fn decompose(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, arguments| {
        let declarations = syn::parse2::<CodecDeclarations>(arguments)?;
        let mut catalog = cq_gpu_sequence_core::catalog();
        catalog.codecs.extend(declarations.0);
        let source = cq_gpu_sequence_core::runs::record_representation(source)?;
        Ok(
            cq_sequence_plan::decompose(&source, &cq_gpu_sequence_core::machine(), &catalog)?
                .program,
        )
    })
    .into()
}

/// Check the authored grades against the GPU machine without choosing grades.
#[proc_macro]
pub fn machine_grade(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        let source = cq_gpu_sequence_core::runs::record_representation(source)?;
        cq_lang_sequence::check_grading(&source, &cq_gpu_sequence_core::machine())?;
        Ok(source)
    })
    .into()
}

/// Translate a checked, explicitly graded Sequence program to CUDA.
#[proc_macro]
pub fn translate(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        let source = cq_gpu_sequence_core::runs::record_representation(source)?;
        let analysis = cq_lang_sequence::check_grading(&source, &cq_gpu_sequence_core::machine())?;
        cq_gpu_sequence_core::translate(&source, &analysis)
    })
    .into()
}

/// Publish raw candidate rows into the explicitly supplied mutable arena.
///
/// Use `translate_arena(&mut arena)` as the terminal pipeline pass. The
/// recorded output protocol still controls counting and capacity retries.
#[proc_macro]
pub fn translate_arena(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(input.into(), |source, arguments| {
        if arguments.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "GPU Sequence translate_arena requires an explicit arena expression",
            ));
        }
        let arena = syn::parse2::<syn::Expr>(arguments)?;
        let source = cq_gpu_sequence_core::runs::record_representation(source)?;
        let analysis = cq_lang_sequence::check_grading(&source, &cq_gpu_sequence_core::machine())?;
        cq_gpu_sequence_core::translate_arena(&source, &analysis, &arena)
    })
    .into()
}

/// Bind declared native run inputs before lowering Structures to Sequence.
#[proc_macro]
pub fn bind_runs(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        cq_gpu_sequence_core::runs::bind(source)
    })
    .into()
}

/// Select device providers from the GPU catalog for every computed structure
/// (filtered supports, built indexes, key sets) that `bind_runs` did not bind
/// to a declared run input. Bound runs already match a catalog token.
#[proc_macro]
pub fn select_catalog(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        cq_sequence_plan::select_from_catalog(source, &cq_gpu_sequence_core::catalog())
    })
    .into()
}

/// Expose literal atom arguments as checked guards before provider selection.
#[proc_macro]
pub fn capture_constants(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::physical::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        cq_gpu_sequence_core::constants::compile(source)
    })
    .into()
}
