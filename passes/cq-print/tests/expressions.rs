use cq_print::expressions::relational_condition_with;
#[test]
fn relational_equalities_require_the_leibniz_operation() {
    let theta = syn::parse_quote!(x = y, z = -5);
    let printed = relational_condition_with(&theta, &Default::default()).to_string();
    assert!(printed.contains("relational_eq (x , y)"));
    assert!(printed.contains("relational_eq (z , & (- 5))"));
}

#[test]
fn column_dependencies_cover_equalities_constraints_and_opaque_rust() {
    let theta = syn::parse_quote!(x = y, z >= @floor, w in {1, 2});
    assert_eq!(
        cq_print::columns_used(&theta),
        ["w", "x", "y", "z"].map(String::from).into()
    );
    let heading = ["x", "y", "z"].map(String::from);
    let expression = syn::parse_quote!({
        let y = 1;
        x + y
    });
    assert_eq!(
        cq_print::columns_used_rust(&expression, &heading, None),
        ["x"].map(String::from).into()
    );
    let expression = syn::parse_quote!(opaque!(x));
    assert_eq!(
        cq_print::columns_used_rust(&expression, &heading, None),
        heading.into()
    );
}
