//! GPU keyed accumulator selection belongs to Structures refinement, not printing.

use cq_lang_structures::{
    Analysis, Program, Structure,
    physical::{Operator, symbol_name},
};

pub(crate) fn provider() -> cq_lang_structures::Provider {
    cq_lang_structures::provider::base::instance(
        "group_map",
        syn::parse_quote!(::cq_gpu_sequence_runtime::group::SegmentedGroup),
    )
}

/// Select the GPU segmented accumulator before Sequence scheduling/printing.
/// Global (keyless) folds have no accumulator-table record under STR-Group.
/// Formal rule GPU-GroupProvider: bind the keyed accumulator in Structures before GPU translation.
pub fn bind(source: &Program) -> syn::Result<Program> {
    cq_lang_structures::contract::check(source)?;
    let mut target = source.clone();
    for entry in &mut target.structures.entries {
        if let Structure::Group {
            provider: recorded, ..
        } = &mut entry.structure
        {
            *recorded = provider();
        }
    }
    target
        .structures
        .declare(&[cq_lang_structures::provider::base::declaration("group_map")]);
    let analysis = cq_lang_structures::contract::check(&target)?;
    check(&target, &analysis)?;
    Ok(target)
}

/// Validate the supplied accumulator facts without rerunning their selection.
pub fn check(source: &Program, analysis: &Analysis) -> syn::Result<()> {
    let expected = provider();
    for definition in &source.physical.program.definitions {
        let Operator::Group(group) = &definition.operator else {
            continue;
        };
        if group.keys.is_empty() {
            continue;
        }
        let recorded = analysis
            .groups
            .get(&symbol_name(&definition.name))
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    &definition.name,
                    "GPU-Group: missing recorded keyed accumulator provider",
                )
            })?;
        if recorded != &expected {
            return Err(syn::Error::new_spanned(
                recorded,
                "GPU-Group: keyed accumulator must record ::cq_gpu_sequence_runtime::group::SegmentedGroup; apply GPU grouping binding before translation",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn source(keyed: bool) -> Program {
        let keys = keyed.then(|| quote::quote!(key));
        let output_key = keyed.then(|| quote::quote!(key,));
        let accumulator = keyed.then(|| {
            quote::quote!(
                grouped group = @group_map as ::std::collections::HashMap<_, (i64, usize)>;
            )
        });
        let group_map = keyed.then(|| {
            quote::quote!(group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build macro };)
        });
        syn::parse2(quote::quote! {
            physical {
                relation R(id:i64,key:i32,value:i64) = (&data.r);
                r = pipe rename R@0 {id -> id,key -> key,value -> value};
                valuation = let canon project r keep {id,key,value};
                grouped = pipe group valuation by [#keys] {
                    total:i64=sum(value); rows:usize=count();
                };
                return grouped as answer(#output_key total,rows);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build macro, abi columns, bounds resident_columns }; packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon { reads [distinct], build inferred, packed_bits 128 }; #group_map }
                r host scan = @device_columns input { keys 0; columns [i64 => i64, i32 => i32, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i32,i64)> [];
                valuation rows = @packed_canon as ::cq_gpu_sequence_runtime::canon::PackedCanon;
                valuation canon = rows;
                #accumulator
            }
        })
        .unwrap()
    }

    #[test]
    fn binding_records_provider_and_checker_does_not_select_it() {
        let source = source(true);
        let original = source.to_token_stream().to_string();
        let analysis = cq_lang_structures::contract::check(&source).unwrap();
        assert!(
            check(&source, &analysis)
                .unwrap_err()
                .to_string()
                .contains("GPU-Group")
        );
        assert_eq!(source.to_token_stream().to_string(), original);
        let target = bind(&source).unwrap();
        let analysis = cq_lang_structures::contract::check(&target).unwrap();
        assert_eq!(analysis.groups["grouped"], provider());
        check(&target, &analysis).unwrap();
        assert_eq!(
            bind(&target).unwrap().to_token_stream().to_string(),
            target.to_token_stream().to_string()
        );
        assert_eq!(source.to_token_stream().to_string(), original);
        let mut missing = analysis;
        missing.groups.remove("grouped");
        assert!(
            check(&target, &missing)
                .unwrap_err()
                .to_string()
                .contains("missing recorded")
        );
    }

    #[test]
    fn checker_rejects_foreign_and_malformed_accumulators() {
        let providers: [syn::Type; 3] = [
            syn::parse_quote!(::other::SegmentedGroup),
            syn::parse_quote!(SegmentedGroup),
            syn::parse_quote!(::cq_gpu_sequence_runtime::group::SegmentedGroup<u32>),
        ];
        for ty in providers {
            let mut target = bind(&source(true)).unwrap();
            for entry in &mut target.structures.entries {
                if let Structure::Group {
                    provider: recorded, ..
                } = &mut entry.structure
                {
                    recorded.ty = ty.clone();
                }
            }
            let analysis = cq_lang_structures::contract::check(&target).unwrap();
            assert!(
                check(&target, &analysis)
                    .unwrap_err()
                    .to_string()
                    .contains("GPU-Group")
            );
        }
    }

    #[test]
    fn global_fold_has_no_keyed_accumulator_record() {
        let source = source(false);
        let target = bind(&source).unwrap();
        let analysis = cq_lang_structures::contract::check(&target).unwrap();
        assert!(analysis.groups.is_empty());
        check(&target, &analysis).unwrap();
        assert_eq!(
            target.to_token_stream().to_string(),
            source.to_token_stream().to_string()
        );
    }
}
