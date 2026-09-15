macro_rules! check_native_provider {
    () => {
use super::{Options, compile};
use cq_ir::physical_provider::Operation;
use quote::quote;

#[test]
fn row_sets_reject_requested_block_access() {
    let options = Options::default();
    for operation in [
        quote!(blocks(&rows)),
        quote!(bounds(descriptor)),
        quote!(block_rows(descriptor)(&rows)),
    ] {
        let input = syn::parse2(operation).unwrap();
        assert!(
            compile(&input, &options)
                .unwrap_err()
                .to_string()
                .contains("no block descriptor lowering")
        );
    }
}

#[test]
fn operations_emit_declared_fragments_without_storage_configuration() {
    for operation in [
        quote!(type 0),
        quote!(input(&rows)),
        quote!(len(&value)),
        quote!(row(handle)),
        quote!(scan(&value)),
        quote!(rows(&value)),
        quote!(select(&value)(selected)),
        quote!(materialize owner value (rows)),
    ] {
        let input: Operation = syn::parse2(operation).unwrap();
        let output = compile(&input, &Options::default()).unwrap();
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
fn observers_are_evaluated_once_only_by_traversal() {
    let options = Options {
        work: Some(syn::parse_quote!(unique_work())),
    };
    for operation in [quote!(scan(&value)), quote!(rows(&value))] {
        let input = syn::parse2(operation).unwrap();
        let observed = compile(&input, &options).unwrap().to_string();
        assert_eq!(observed.matches("unique_work").count(), 1);
        let plain = compile(&input, &Options::default()).unwrap().to_string();
        assert!(!plain.contains("inspect"));
        assert!(!plain.contains("__cq_work"));
    }
    for operation in [
        quote!(type 3),
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
                .contains("unique_work")
        );
    }
}

    };
}
pub(crate) use check_native_provider;
