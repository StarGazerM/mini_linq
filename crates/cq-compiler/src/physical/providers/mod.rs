//! Open physical-provider expansion entry points.
//!
//! Providers own concrete types, construction and access fragments. Unsupported
//! operations fail at macro expansion; Rust checks the resulting composition.

mod block;
mod native;

pub use block::compile_block_provider;
pub use native::{compile_hash_provider, compile_sorted_provider};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderOptions;
    use cq_ir::physical_provider::Operation;
    use quote::quote;

    #[test]
    fn row_sets_reject_requested_block_access() {
        let options = ProviderOptions {
            storage: syn::parse_quote!(crate::data),
            work: None,
        };
        for compile in [compile_hash_provider, compile_sorted_provider] {
            let input = syn::parse2(quote!(blocks(&rows))).unwrap();
            let error = compile(&input, &options).unwrap_err();
            assert!(error.to_string().contains("no block descriptor lowering"));
        }
    }

    #[test]
    fn typed_operations_emit_their_declared_rust_fragment_category() {
        let operations = [
            quote!(type 0),
            quote!(input(&rows)),
            quote!(len(&value)),
            quote!(row(handle)),
            quote!(scan(&value)),
            quote!(rows(&value)),
            quote!(select(&value)(selected)),
            quote!(materialize owner value (rows) (size)),
            quote!(blocks(&value)),
            quote!(bounds(descriptor)),
            quote!(block_rows(descriptor)(&value)),
        ];
        for operation in operations {
            let input: Operation = syn::parse2(operation).unwrap();
            let options = ProviderOptions {
                storage: syn::parse_quote!(crate::data),
                work: None,
            };
            for (family, compile) in [
                compile_block_provider,
                compile_hash_provider,
                compile_sorted_provider,
            ]
            .into_iter()
            .enumerate()
            {
                let output = compile(&input, &options);
                if matches!(
                    input,
                    Operation::Blocks { .. }
                        | Operation::Bounds { .. }
                        | Operation::BlockRows { .. }
                ) && family > 0
                {
                    // Native sets intentionally reject descriptor operations.
                    assert!(
                        output
                            .unwrap_err()
                            .to_string()
                            .contains("no block descriptor lowering")
                    );
                    continue;
                }
                let output = output.unwrap();
                match input {
                    Operation::Type { .. } => {
                        syn::parse2::<syn::Type>(output).unwrap();
                    }
                    Operation::Materialize { .. } => {
                        syn::parse2::<syn::Block>(quote!({ #output })).unwrap();
                    }
                    _ => {
                        syn::parse2::<syn::Expr>(output).unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn instrumentation_is_absent_unless_requested_and_evaluated_once_when_used() {
        let operations = [
            syn::parse_quote!(scan(&value)),
            syn::parse_quote!(rows(&value)),
            syn::parse_quote!(blocks(&value)),
            syn::parse_quote!(block_rows(descriptor)(&value)),
        ];
        for compile in [
            compile_hash_provider,
            compile_sorted_provider,
            compile_block_provider,
        ] {
            for input in &operations {
                let options = ProviderOptions {
                    storage: syn::parse_quote!(crate::data),
                    work: None,
                };
                let Ok(plain) = compile(input, &options) else {
                    continue;
                };
                let plain = plain.to_string();
                for absent in [
                    "__cq_work",
                    "__cq_opened",
                    "inspect",
                    "rows_visited",
                    "blocks_opened",
                    "blocks_considered",
                ] {
                    assert!(!plain.contains(absent), "unexpected {absent} in {plain}");
                }
                let options = ProviderOptions {
                    storage: syn::parse_quote!(crate::data),
                    work: Some(syn::parse_quote!(unique_work_expression())),
                };
                let observed = compile(input, &options).unwrap().to_string();
                assert_eq!(
                    observed.matches("unique_work_expression").count(),
                    1,
                    "{observed}"
                );
                assert!(observed.contains("__cq_work"));
            }
        }
    }

    #[test]
    fn operations_without_work_do_not_evaluate_the_instrumentation_expression() {
        let options = ProviderOptions {
            storage: syn::parse_quote!(crate::data),
            work: Some(syn::parse_quote!(compile_error!("unused instrumentation"))),
        };
        let operations = [
            syn::parse_quote!(type 3),
            syn::parse_quote!(input(&rows)),
            syn::parse_quote!(row(handle)),
            syn::parse_quote!(len(&value)),
            syn::parse_quote!(select(&value)(selected)),
            syn::parse_quote!(materialize owner value (rows) (size)),
        ];
        for compile in [
            compile_hash_provider,
            compile_sorted_provider,
            compile_block_provider,
        ] {
            for input in &operations {
                let output = compile(input, &options).unwrap().to_string();
                assert!(!output.contains("unused instrumentation"), "{output}");
            }
        }
    }
}
