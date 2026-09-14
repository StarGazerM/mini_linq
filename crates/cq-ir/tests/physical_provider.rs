use cq_ir::physical_provider::Operation;
use quote::{ToTokens, quote};

#[test]
fn all_provider_operations_round_trip_through_declared_syntax() {
    let operations = [
        quote!(type 0),
        quote!(type 3),
        quote!(input(&rows)),
        quote!(len(value)),
        quote!(row(handle)),
        quote!(scan(choose(&rows))),
        quote!(rows((&rows, mask))),
        quote!(select(&value)(selected.into_iter())),
        quote!(materialize r#owner r#value (rows) (block_size())),
        quote!(blocks(&value)),
        quote!(bounds(descriptor)),
        quote!(block_rows(descriptor)(&value)),
        quote!(build { out(x) => ([x]) :- Rows(x) => for (x,) in (rows); }),
        quote!(contains(&resource)([key])),
        quote!(may_match(&resource)(bounds)(&[0, 2])),
        quote!(probe(&resource)([key])),
        quote!(candidates crate::rows { &value } (|bounds| keep(bounds))),
    ];
    for source in operations {
        let first: Operation = syn::parse2(source.clone()).unwrap();
        let encoded = first.to_token_stream();
        let second: Operation = syn::parse2(encoded.clone()).unwrap();
        assert_eq!(first, second);
        assert_eq!(encoded.to_string(), source.to_string());
    }
}

#[test]
fn argument_parentheses_preserve_complete_rust_expressions() {
    let input: Operation = syn::parse_quote! {
        block_rows
            ({ let index = choose(); descriptors[index] })
            ((&rows, make_mask::<(u32, u64)>(1, 2)))
    };
    let Operation::BlockRows {
        descriptor, value, ..
    } = &input
    else {
        panic!("expected block_rows");
    };
    assert!(matches!(descriptor.as_ref(), syn::Expr::Block(_)));
    assert!(matches!(value.as_ref(), syn::Expr::Tuple(_)));
    assert_eq!(
        input,
        syn::parse2::<Operation>(input.to_token_stream()).unwrap()
    );
}

#[test]
fn configuration_and_malformed_argument_shapes_are_rejected() {
    let invalid = [
        quote!(storage ::data; type 2),
        quote!(work (&work); rows (rows)),
        quote!(lookup(rows)),
        quote!(type -1),
        quote!(type width),
        quote!(type 3usize),
        quote!(type 1 2),
        quote!(input rows),
        quote!(input()),
        quote!(input (rows other)),
        quote!(scan(rows)(work)),
        quote!(rows(rows)(work)),
        quote!(blocks(rows)(work)),
        quote!(select (rows) selected),
        quote!(materialize owner (rows) (size)),
        quote!(materialize owner value (rows) (size) extra),
        quote!(bounds(descriptor, other)),
        quote!(block_rows(descriptor)),
        quote!(block_rows(descriptor)(rows)(work)),
        quote!(build(rows)),
        quote!(build { rows }),
        quote!(contains(&resource)),
        quote!(may_match(&resource)(bounds)),
        quote!(probe(&resource)(key)(extra)),
        quote!(candidates crate::rows (&value) (predicate)),
        quote!(candidates crate::rows { &value }),
    ];
    for source in invalid {
        assert!(
            syn::parse2::<Operation>(source.clone()).is_err(),
            "unexpectedly parsed {source}"
        );
    }
}

#[test]
fn zero_width_is_a_typed_column_and_not_an_empty_operation() {
    let input: Operation = syn::parse_quote!(type 0);
    let Operation::Type { arity, .. } = input else {
        panic!("expected type");
    };
    let _: cq_ir::Column = arity;
    assert_eq!(arity.index, 0);
}
