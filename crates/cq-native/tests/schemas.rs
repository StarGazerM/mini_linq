#[cfg(feature = "rows-hash")]
#[test]
fn array_provider_rejects_a_heterogeneous_schema_instead_of_ignoring_types() {
    let operation = syn::parse_quote!(type (String, i32,));
    let error = cq_native::rows::hash::compile(&operation, &Default::default()).unwrap_err();
    assert!(error.to_string().contains("requires i64 columns"));
}

#[cfg(feature = "rows-tuple-hash")]
#[test]
fn tuple_provider_preserves_each_declared_field_type() {
    let operation = syn::parse_quote!(type (String, i32,));
    let output = cq_native::rows::tuple_hash::compile(&operation, &Default::default()).unwrap();
    let ty: syn::Type = syn::parse2(output).unwrap();
    assert_eq!(
        ty,
        syn::parse_quote!(::std::collections::HashSet<(String, i32,)>)
    );
}

#[cfg(feature = "rows-block")]
#[test]
fn block_provider_checks_its_own_payload_schema() {
    let options = cq_native::rows::block::Options {
        storage: syn::parse_quote!(cq_storage),
        packing: None,
        work: None,
    };
    assert!(cq_native::rows::block::compile(&syn::parse_quote!(type (String,)), &options).is_err());
    let output =
        cq_native::rows::block::compile(&syn::parse_quote!(type (i64, i64,)), &options).unwrap();
    assert_eq!(
        syn::parse2::<syn::Type>(output).unwrap(),
        syn::parse_quote!(cq_storage::BlockStore<2usize>)
    );
}
