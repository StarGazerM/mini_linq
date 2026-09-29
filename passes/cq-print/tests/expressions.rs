use cq_print::expressions::window;
#[test]
fn comparison_windows_keep_tight_bounds_and_unsupported_residuals() {
    let expr = syn::parse_quote!(x >= 2 && x > 4 && 9 > x && x <= 20 && y != 3);
    let w = window(&expr, "x");
    assert_eq!((w.low, w.high, w.residual.len()), (Some(5), Some(8), 1));
    for expr in [
        syn::parse_quote!(x > 2147483647),
        syn::parse_quote!(x < -2147483648),
        syn::parse_quote!(x != 5),
        syn::parse_quote!(f(x)),
    ] {
        let w = window(&expr, "x");
        assert_eq!((w.low, w.high, w.residual.len()), (None, None, 1));
    }
}
