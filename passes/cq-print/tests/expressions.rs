use cq_print::expressions::relational_condition;
#[test]
fn relational_equalities_require_the_leibniz_operation() {
    let theta = syn::parse_quote!(x = y, z = -5);
    let printed = relational_condition(&theta).to_string();
    assert!(printed.contains("relational_eq (x , y)"));
    assert!(printed.contains("relational_eq (z , & (- 5))"));
}
