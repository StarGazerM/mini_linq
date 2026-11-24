use cq_syntax::constraint::{Constraint, Term};
use quote::ToTokens;

#[test]
fn closed_operands_round_trip_without_expression_recognition() {
    for source in [
        "x = @(constants.limit)",
        "x < @(host::<A, B>())",
        "x >= @({ let local = source(); local })",
        "x between @(bounds[0]) and @(bounds[1])",
        "(x = @(host()) and y = 1)",
    ] {
        let constraint: Constraint = syn::parse_str(source).unwrap();
        constraint.check().unwrap();
        assert_eq!(
            syn::parse2::<Constraint>(constraint.to_token_stream()).unwrap(),
            constraint
        );
        assert!(constraint.parameters().next().is_some());
    }
}

#[test]
fn closed_terms_use_the_complete_expression_as_the_capture_key() {
    let expression: syn::Expr = syn::parse_quote!(limits.lookup(index));
    let term = Term::Closed(expression.clone());
    let parameters = [(
        expression.to_token_stream().to_string(),
        syn::parse_quote!(captured),
    )]
    .into_iter()
    .collect();
    assert_eq!(term.reference(&parameters).to_string(), "captured");
    assert_eq!(
        term.reference(&Default::default()).to_string(),
        "& (limits . lookup (index))"
    );
}
