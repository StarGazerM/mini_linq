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
pub fn bind(physical: &cq_lang_structures::physical::Program, source: &Program) -> syn::Result<Program> {
    cq_lang_structures::contract::check(physical, source)?;
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
        .declare(&[cq_lang_structures::provider::base::declaration("group_map")])?;
    let analysis = cq_lang_structures::contract::check(physical, &target)?;
    check(physical, &analysis)?;
    Ok(target)
}

/// Validate the supplied accumulator facts without rerunning their selection.
pub fn check(physical: &cq_lang_structures::physical::Program, analysis: &Analysis) -> syn::Result<()> {
    let expected = provider();
    for definition in &physical.program.definitions {
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
        // Token identity with its declared facts; the spelling is printed and
        // rustc-checked.
        let declared = cq_lang_structures::provider::base::declaration("group_map").facts;
        if !recorded.same_token(&expected) || analysis.facts(recorded).clone() != declared {
            return Err(syn::Error::new_spanned(
                recorded,
                "GPU-Group: keyed accumulator must record the group_map token (SegmentedGroup); apply GPU grouping binding before translation",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn source(keyed: bool) -> (cq_lang_structures::physical::Program, Program) {
        let keys = keyed.then(|| quote::quote!(key));
        let output_key = keyed.then(|| quote::quote!(key,));
        let accumulator = keyed.then(|| {
            quote::quote!(
                grouped group = @group_map as ::std::collections::HashMap<_, (i64, usize)>;
            )
        });
        let group_map = keyed.then(|| {
            quote::quote!(group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed };)
        });
        syn::parse::Parser::parse2(|input: syn::parse::ParseStream<'_>| {
            let physical = input.parse()?;
            let structures = Program::parse(input, &physical)?;
            Ok((physical, structures))
        }, quote::quote! {
            physical {
                relation R(id:i64,key:i32,value:i64) = (&data.r);
                r = pipe rename R {id -> id,key -> key,value -> value};
                valuation = let canon project r keep {id,key,value};
                grouped = pipe group valuation by [#keys] {
                    total:i64=sum(value); rows:usize=count();
                };
                return grouped as answer(#output_key total,rows);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; #group_map }
                r host scan = @device_columns input { keys 0; columns [i64 => i64, i32 => i32, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i32,i64)> [];
                valuation rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                valuation canon = rows;
                #accumulator
            }
        })
        .unwrap()
    }

    #[test]
    fn checker_accepts_legal_records_and_binding_records_provider() {
        // The source already records the group_map token with its declared
        // facts; the spelling (HashMap) does not decide legality.
        let (physical, source) = source(true);
        let original = source.to_token_stream().to_string();
        let analysis = cq_lang_structures::contract::check(&physical, &source).unwrap();
        assert!(analysis.groups["grouped"].same_token(&provider()));
        check(&physical, &analysis).unwrap();
        let target = bind(&physical, &source).unwrap();
        assert_eq!(source.to_token_stream().to_string(), original);
        let analysis = cq_lang_structures::contract::check(&physical, &target).unwrap();
        assert_eq!(analysis.groups["grouped"], provider());
        check(&physical, &analysis).unwrap();
        assert_eq!(
            bind(&physical, &target).unwrap().to_token_stream().to_string(),
            target.to_token_stream().to_string()
        );
        let mut missing = analysis;
        missing.groups.remove("grouped");
        assert!(check(&physical, &missing).is_err());
    }

    #[test]
    fn checker_compares_accumulator_tokens_not_spellings() {
        let (physical, source) = source(true);
        let respelled = |ty: syn::Type| {
            let mut target = bind(&physical, &source).unwrap();
            for entry in &mut target.structures.entries {
                if let Structure::Group {
                    provider: recorded, ..
                } = &mut entry.structure
                {
                    recorded.ty = ty.clone();
                }
            }
            target
        };
        let target = respelled(syn::parse_quote!(
            ::cq_gpu_sequence_runtime::group::SegmentedGroup<u32>
        ));
        let analysis = cq_lang_structures::contract::check(&physical, &target).unwrap();
        check(&physical, &analysis).unwrap();
        let mut foreign = bind(&physical, &source).unwrap();
        foreign
            .structures
            .declare(&[cq_lang_structures::provider::base::declaration(
                "sorted_run",
            )])
            .unwrap();
        for entry in &mut foreign.structures.entries {
            if let Structure::Group {
                provider: recorded, ..
            } = &mut entry.structure
            {
                *recorded = cq_lang_structures::provider::base::instance(
                    "sorted_run",
                    syn::parse_quote!(::cq_gpu_sequence_runtime::group::SegmentedGroup),
                );
            }
        }
        let rejected = cq_lang_structures::contract::check(&physical, &foreign)
            .map_or(true, |analysis| check(&physical, &analysis).is_err());
        assert!(rejected);
    }

    #[test]
    fn global_fold_has_no_keyed_accumulator_record() {
        let (physical, source) = source(false);
        let target = bind(&physical, &source).unwrap();
        let analysis = cq_lang_structures::contract::check(&physical, &target).unwrap();
        assert!(analysis.groups.is_empty());
        check(&physical, &analysis).unwrap();
        assert_eq!(
            target.to_token_stream().to_string(),
            source.to_token_stream().to_string()
        );
    }
}
