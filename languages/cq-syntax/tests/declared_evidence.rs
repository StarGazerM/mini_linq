use cq_syntax::declared_evidence::{DeclaredEvidence, EvidenceView};
use quote::ToTokens;

const DOCUMENT: &str = include_str!("../../../runtime/cq-evidence/tests/fixtures/planner-evidence.json");

#[test]
fn canonical_attachment_roundtrip_and_projection_preserve_sources() {
    let input = format!("declared_evidence {:?} {{ R = \"R\" full [a=\"a\",b=\"b\"]; }}", DOCUMENT);
    let evidence: DeclaredEvidence = syn::parse_str(&input).unwrap();
    let declaration: cq_syntax::RelationDecl = syn::parse_quote!(relation R(a:i32,b:i32););
    evidence.check(&[declaration]).unwrap();
    let target: cq_syntax::RelationDecl = syn::parse_quote!(relation Reverse(b:i32,a:i32););
    let projected = evidence.project_relation(&syn::parse_quote!(R), &target, EvidenceView::Delta)
        .unwrap().unwrap();
    assert_eq!(projected.columns[0].source, "b");
    assert_eq!(projected.columns[1].source, "a");
    assert_eq!(projected.view, EvidenceView::Delta);
    assert_eq!(syn::parse2::<DeclaredEvidence>(evidence.to_token_stream()).unwrap(), evidence);
}

#[test]
fn projection_does_not_guess_a_missing_column_or_upgrade_a_subset() {
    let input = format!("declared_evidence {:?} {{ R = \"R\" subset [a=\"a\",b=\"b\"]; }}", DOCUMENT);
    let evidence: DeclaredEvidence = syn::parse_str(&input).unwrap();
    let missing: cq_syntax::RelationDecl = syn::parse_quote!(relation S(c:i32););
    assert!(evidence.project_relation(&syn::parse_quote!(R), &missing, EvidenceView::Full).is_err());
    let renamed: cq_syntax::RelationDecl = syn::parse_quote!(relation S(a:i32,b:i32););
    assert_eq!(evidence.project_relation(&syn::parse_quote!(R), &renamed, EvidenceView::Full)
        .unwrap().unwrap().view, EvidenceView::Subset);
    assert!(evidence.project_relation(&syn::parse_quote!(Absent), &renamed, EvidenceView::Full)
        .unwrap().is_none());
}

