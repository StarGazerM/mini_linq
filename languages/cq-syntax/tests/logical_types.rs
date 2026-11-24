use cq_syntax::{LogicalTypeId, RelationDecl, intern_logical_types};
use quote::ToTokens;

#[test]
fn unnumbered_spellings_are_interned_once() {
    let mut declarations: Vec<RelationDecl> = vec![
        syn::parse_quote!(relation A(a: HostType, b: OtherType);),
        syn::parse_quote!(relation B(c: HostType);),
    ];
    intern_logical_types(&mut declarations).unwrap();
    assert_eq!(
        declarations[0].columns[0].logical_type_id,
        Some(LogicalTypeId(0))
    );
    assert_eq!(
        declarations[0].columns[1].logical_type_id,
        Some(LogicalTypeId(1))
    );
    assert_eq!(
        declarations[1].columns[0].logical_type_id,
        Some(LogicalTypeId(0))
    );
    let recorded = declarations.clone();
    intern_logical_types(&mut declarations).unwrap();
    assert_eq!(declarations, recorded);
}

#[test]
fn explicit_identities_survive_equal_spellings_and_payload_mutation() {
    let mut declarations: Vec<RelationDecl> = vec![syn::parse_quote!(
        relation A(a: HostType [7], b: HostType [9], c: HostType);
    )];
    intern_logical_types(&mut declarations).unwrap();
    assert_eq!(
        declarations[0].columns[0].logical_type_id,
        Some(LogicalTypeId(7))
    );
    assert_eq!(
        declarations[0].columns[1].logical_type_id,
        Some(LogicalTypeId(9))
    );
    assert_eq!(
        declarations[0].columns[2].logical_type_id,
        Some(LogicalTypeId(9))
    );
    declarations[0].columns[0].ty = syn::parse_quote!(EntirelyDifferentPayload);
    intern_logical_types(&mut declarations).unwrap();
    assert_eq!(
        declarations[0].columns[0].logical_type_id,
        Some(LogicalTypeId(7))
    );
    let printed = declarations[0].to_token_stream();
    assert_eq!(
        syn::parse2::<RelationDecl>(printed).unwrap(),
        declarations[0]
    );
}

#[test]
fn logical_identity_and_domain_identity_round_trip_independently() {
    let declaration: RelationDecl = syn::parse_quote!(
        relation A(a: [HostType; 4] [11] in [3] (domain));
    );
    assert_eq!(
        declaration.columns[0].logical_type_id,
        Some(LogicalTypeId(11))
    );
    assert_eq!(
        declaration.columns[0].domain.as_ref().unwrap().id,
        Some(cq_syntax::DomainId(3))
    );
    assert_eq!(
        syn::parse2::<RelationDecl>(declaration.to_token_stream()).unwrap(),
        declaration
    );
}

#[test]
fn fully_numbered_maximum_identity_needs_no_allocation() {
    let mut declarations: Vec<RelationDecl> = vec![syn::parse_quote!(
        relation A(a: HostType [4294967295]);
    )];
    intern_logical_types(&mut declarations).unwrap();
    assert_eq!(
        declarations[0].columns[0].logical_type_id,
        Some(LogicalTypeId(u32::MAX))
    );
}

#[test]
fn identity_exhaustion_is_reported_when_interning_is_needed() {
    let mut declarations: Vec<RelationDecl> = vec![syn::parse_quote!(
        relation A(a: HostType [4294967295], b: OtherType);
    )];
    assert!(intern_logical_types(&mut declarations).is_err());
}

#[test]
fn malformed_identity_is_rejected() {
    for source in [
        "relation A(a: HostType [1 2]);",
        "relation A(a: HostType [4294967296]);",
        "relation A(a: HostType []);",
    ] {
        assert!(syn::parse_str::<RelationDecl>(source).is_err());
    }
}
