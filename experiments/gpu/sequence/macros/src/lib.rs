//! Named pipeline adapters for the checked GPU Sequence compiler.

use proc_macro::TokenStream;
use cq_lang_structures::{Program as Structures, physical::Program as Physical};
use cq_lang_sequence::Program as Sequence;

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
            name(&fields, "codec_id")?;
            fields.parse::<syn::Token![:]>()?;
            let variant: syn::Ident = fields.parse()?;
            let codec_id = if variant == "Some" {
                let id;
                syn::parenthesized!(id in fields);
                let value = id.parse::<syn::LitInt>()?.base10_parse::<u32>()?;
                if !id.is_empty() {
                    return Err(id.error("expected one codec id"));
                }
                Some(cq_lang_structures::provider::CodecId(value))
            } else if variant == "None" {
                None
            } else {
                return Err(syn::Error::new(
                    variant.span(),
                    "expected `Some(integer)` or `None`",
                ));
            };
            fields.parse::<syn::Token![,]>()?;
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
                codec_id,
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
    cq_stage::Structures::<Physical, Structures>::rewrite(input.into(), |physical: &Physical, source: &Structures, arguments| {
        no_arguments(arguments)?;
        let target = cq_gpu_sequence_core::bind_structures(physical, source)?;
        let analysis = cq_lang_structures::contract::check(physical, &target)?;
        cq_gpu_sequence_core::check_structures(physical, &target, &analysis)?;
        Ok((physical.clone(), target))
    })
    .into()
}

/// Decompose a checked authored traversal without automatic relational replanning.
#[proc_macro]
pub fn decompose(input: TokenStream) -> TokenStream {
    expand_decompose(input.into()).into()
}

fn expand_decompose(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    cq_stage::Sequence::<Physical, Structures, Sequence>::rewrite(input, |_physical: &Physical, _structures: &Structures, source: &Sequence, arguments| {
        let declarations = syn::parse2::<CodecDeclarations>(arguments)?;
        let mut catalog = cq_gpu_sequence_core::catalog();
        catalog.codecs.extend(declarations.0);
        let source = cq_gpu_sequence_core::runs::record_representation(source)?;
        let target =
            cq_sequence_plan::decompose(&source, &cq_gpu_sequence_core::machine(), &catalog)?
                .program;
        Ok((target.structures.physical.clone(), target.structures.clone(), target))
    })
}

/// Check the authored grades against the GPU machine without choosing grades.
#[proc_macro]
pub fn machine_grade(input: TokenStream) -> TokenStream {
    cq_stage::Sequence::<Physical, Structures, Sequence>::rewrite(input.into(), |_physical: &Physical, _structures: &Structures, source: &Sequence, arguments| {
        no_arguments(arguments)?;
        let source = cq_gpu_sequence_core::runs::record_representation(source)?;
        cq_lang_sequence::check_grading(&source, &cq_gpu_sequence_core::machine())?;
        Ok((source.structures.physical.clone(), source.structures.clone(), source))
    })
    .into()
}

/// Translate a checked, explicitly graded Sequence program to CUDA.
#[proc_macro]
pub fn translate(input: TokenStream) -> TokenStream {
    cq_stage::Sequence::<Physical, Structures, Sequence>::pass::<syn::Expr>(input.into(), |_physical: &Physical, _structures: &Structures, source: &Sequence, arguments| {
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
    cq_stage::Sequence::<Physical, Structures, Sequence>::pass::<syn::Expr>(input.into(), |_physical: &Physical, _structures: &Structures, source: &Sequence, arguments| {
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
    cq_stage::Structures::<Physical, Structures>::rewrite(input.into(), |physical: &Physical, source: &Structures, arguments| {
        no_arguments(arguments)?;
        let target = cq_gpu_sequence_core::runs::bind(physical, source)?;
        Ok((physical.clone(), target))
    })
    .into()
}

/// Select device providers from the GPU catalog for every computed structure
/// (filtered supports, built indexes, key sets) that `bind_runs` did not bind
/// to a declared run input. Bound runs already match a catalog token.
/// Catalog selection checks the retention correspondence itself: unsupported
/// host indexes may become explicit retained builds, rather than preserving
/// the Physical tree required by an ordinary Structures rewrite.
#[proc_macro]
pub fn select_catalog(input: TokenStream) -> TokenStream {
    cq_stage::Structures::<Physical, Structures>::pass::<cq_stage::Structures<Physical, Structures>>(
        input.into(),
        |physical: &Physical, source: &Structures, arguments| {
            no_arguments(arguments)?;
            let (physical, structures) = cq_sequence_plan::select_from_catalog(physical, source, &cq_gpu_sequence_core::catalog())?;
            Ok(cq_stage::Structures::new(physical, structures))
        },
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use cq_lang_structures::provider::CodecId;
    use quote::quote;

    #[test]
    fn grouped_answer_gate_is_one_checked_diagnostic_expression() {
        let mut program: cq_lang_sequence::Program = syn::parse_quote! {
            physical {
                relation R(k:u32) = (data);
                r = let canon rename R@0 {k -> k};
                grouped = let group r by [k] { n:usize=count(); };
                return grouped as answer(k,n);
            }
            structures {
                providers {
                    device_columns = ::cq_gpu_runtime::DeviceColumns<_> {
                        reads [], build typed, abi columns, bounds resident_columns
                    };
                    radix_set = ::gpu_datalog_runtime::RadixLayout<
                        ::gpu_datalog_runtime::radix_run::EncodedRow, (),
                        ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()
                    > { reads [distinct, resident], build inferred };
                    grouped_rows = ::cq_gpu_sequence_runtime::group::GroupedRows<_> {
                        reads [exact, range, membership], build typed
                    };
                    group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> {
                        reads [], build typed
                    };
                }
                r host scan = @device_columns input { keys 0; columns [u32 => u32] }
                    as ::cq_gpu_runtime::DeviceColumns<'static,(u32,)> [];
                r rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<
                    ::gpu_datalog_runtime::radix_run::EncodedRow, (),
                    ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()
                >;
                r canon = rows;
                grouped rows = @grouped_rows as ::cq_gpu_sequence_runtime::group::GroupedRows<_>;
                grouped group = @group_map as ::std::collections::HashMap<_, (usize,)>;
            }
            sequence {
                source = store rows r;
                feed = canon flatten tile 256 source;
                valuations = store feed;
                result = store group grouped valuations;
                output count;
            }
            graded gpu {
                source at host; feed at device.block; valuations at host; result at host;
            }
        };
        program.structures = cq_gpu_sequence_core::bind_structures(&program.structures).unwrap();
        program = cq_gpu_sequence_core::runs::record_representation(&program).unwrap();
        let machine = cq_gpu_sequence_core::machine();
        cq_lang_sequence::check_grading(&program, &machine).unwrap();

        // The same grouped publication reaches its continuation without a gate.
        let accepted = expand_decompose(quote!(pipeline { continuation }; #program));
        let accepted: syn::ExprMacro = syn::parse2(accepted).unwrap();
        assert!(accepted.mac.path.is_ident("continuation"));

        program
            .structures
            .physical
            .program
            .captures
            .items
            .push(syn::parse_quote!(if (limit != 3)));
        assert!(cq_lang_sequence::check_grading(&program, &machine).is_err());

        // Exercise the actual decompose ABI adapter, not a printer or a copy of
        // the checker. Candidate failures must not escape as adjacent macros.
        let rejected = expand_decompose(quote!(pipeline { continuation }; #program));
        let expression: syn::ExprBlock = syn::parse2(rejected.clone()).unwrap();
        assert_eq!(expression.block.stmts.len(), 1);
        let syn::Stmt::Macro(diagnostic) = &expression.block.stmts[0] else {
            panic!("expected one checked diagnostic");
        };
        assert_eq!(diagnostic.mac.path.segments.last().unwrap().ident, "compile_error");
        let message: syn::LitStr = syn::parse2(diagnostic.mac.tokens.clone()).unwrap();
        assert!(!message.value().is_empty());
        syn::parse2::<syn::Expr>(quote!(Some(#rejected))).unwrap();
    }

    #[test]
    fn codec_declarations_require_explicit_identity() {
        let declarations = syn::parse2::<CodecDeclarations>(quote!(codecs [
            CodecToken {
                codec_id: Some(7),
                codec: Some(Codec),
                ordered: true,
                levels: [[host, device]],
            },
            CodecToken {
                codec_id: None,
                codec: None,
                ordered: true,
                levels: [[host]],
            },
        ]))
        .unwrap();
        assert_eq!(declarations.0[0].codec_id, Some(CodecId(7)));
        assert_eq!(declarations.0[0].codec, Some(syn::parse_quote!(Codec)));
        assert_eq!(declarations.0[1].codec_id, None);
        assert_eq!(declarations.0[1].codec, None);

        for declaration in [
            quote!(codec: Some(Codec), ordered: true, levels: [[host]]),
            quote!(codec_id: Some(4294967296), codec: Some(Codec), ordered: true, levels: [[host]]),
            quote!(codec_id: Some(7, 8), codec: Some(Codec), ordered: true, levels: [[host]]),
            quote!(codec_id: Some(CodecId(7)), codec: Some(Codec), ordered: true, levels: [[host]]),
        ] {
            assert!(
                syn::parse2::<CodecDeclarations>(quote!(
                    codecs [CodecToken { #declaration }]
                ))
                .is_err()
            );
        }
    }
}
