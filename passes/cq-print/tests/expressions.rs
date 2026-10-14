use cq_print::expressions::relational_condition_with;
#[test]
fn relational_equalities_require_the_leibniz_operation() {
    let theta = syn::parse_quote!(x = y, z = -5);
    let printed = relational_condition_with(&theta, &Default::default()).to_string();
    assert!(printed.contains("relational_eq (x , y)"));
    assert!(printed.contains("relational_eq (z , & (- 5))"));
}
