//! Independently selected refinements of native Access operations.
//! Candidate pruning is justified together with its enclosing exact residual.

pub mod candidates;
pub mod keys;
pub mod witnesses;

#[cfg(any(
    feature = "candidate-prune",
    feature = "keys-exact",
    feature = "keys-zone",
    feature = "witness-hash",
    feature = "witness-nested"
))]
fn work_binding(work: Option<&syn::Expr>) -> Option<proc_macro2::TokenStream> {
    use quote::quote;
    work.map(|work| quote!(let __cq_work = (#work);))
}

#[cfg(any(
    feature = "keys-exact",
    feature = "keys-zone",
    feature = "witness-hash",
    feature = "witness-nested"
))]
fn required<'a>(
    value: Option<&'a syn::Path>,
    operation: &cq_ir::physical_provider::Operation,
    name: &str,
) -> syn::Result<&'a syn::Path> {
    value.ok_or_else(|| {
        syn::Error::new_spanned(
            operation,
            format!("missing configuration `{name}` for this operation"),
        )
    })
}
