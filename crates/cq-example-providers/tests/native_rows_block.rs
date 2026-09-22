//! Recovered tests from crates/cq-native-lowering/src/rows/block.rs.
use cq_native_lowering::rows::block::*;

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    fn options() -> Options {
        Options {
            storage: syn::parse_quote!(crate::data),
            packing: None,
            work: None,
        }
    }

    #[test]
    fn packing_belongs_only_to_materialization_and_evaluates_once() {
        let mut options = options();
        let operation = syn::parse_quote!(materialize owner value (unique_rows()));
        assert!(
            compile(&operation, &options)
                .unwrap_err()
                .to_string()
                .contains("packing")
        );
        options.packing = Some(syn::parse_quote!(unique_size()));
        let output = compile(&operation, &options).unwrap().to_string();
        assert_eq!(output.matches("unique_rows").count(), 1);
        assert_eq!(output.matches("unique_size").count(), 1);
        let operation = syn::parse_quote!(rows(&rows));
        assert!(
            !compile(&operation, &options)
                .unwrap()
                .to_string()
                .contains("unique_size")
        );
        options.packing = None;
        compile(&operation, &options).unwrap();
    }

    #[test]
    fn operations_emit_their_declared_rust_fragment_category() {
        let mut options = options();
        options.packing = Some(syn::parse_quote!(size));
        for operation in [
            quote!(type ()),
            quote!(input(&rows)),
            quote!(len(&value)),
            quote!(row(handle)),
            quote!(scan(&value)),
            quote!(rows(&value)),
            quote!(select(&value)(selected)),
            quote!(materialize owner value (rows)),
            quote!(blocks(&value)),
            quote!(bounds(descriptor)),
            quote!(block_rows(descriptor)(&value)),
        ] {
            let input: Operation = syn::parse2(operation).unwrap();
            let output = compile(&input, &options).unwrap();
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

    #[test]
    fn observers_are_absent_by_default_and_evaluated_once_by_traversal() {
        let mut options = options();
        for operation in [
            quote!(scan(&value)),
            quote!(rows(&value)),
            quote!(blocks(&value)),
            quote!(block_rows(descriptor)(&value)),
        ] {
            let input = syn::parse2(operation).unwrap();
            let plain = compile(&input, &options).unwrap().to_string();
            assert!(!plain.contains("__cq_work"));
            assert!(!plain.contains("__cq_opened"));
            options.work = Some(syn::parse_quote!(unique_work()));
            assert_eq!(
                compile(&input, &options)
                    .unwrap()
                    .to_string()
                    .matches("unique_work")
                    .count(),
                1
            );
            options.work = None;
        }
        options.work = Some(syn::parse_quote!(compile_error!("unused work")));
        options.packing = Some(syn::parse_quote!(size));
        for operation in [
            quote!(type (i64, i64, i64,)),
            quote!(input(&rows)),
            quote!(row(handle)),
            quote!(len(&value)),
            quote!(select(&value)(selected)),
            quote!(materialize owner value (rows)),
        ] {
            let input = syn::parse2(operation).unwrap();
            assert!(
                !compile(&input, &options)
                    .unwrap()
                    .to_string()
                    .contains("unused work")
            );
        }
    }
}
