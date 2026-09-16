//! Whole equal-key pair producers. Each implementation owns both traversals.
//! The enclosing row provider owns the output sink and its representation.

#[cfg(feature = "pairs-hash")]
pub mod hash;
#[cfg(feature = "pairs-merge")]
pub mod merge;
#[cfg(feature = "pairs-nested")]
pub mod nested;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub work: Option<syn::Expr>,
}

#[cfg(any(
    feature = "pairs-hash",
    feature = "pairs-nested",
    feature = "pairs-merge"
))]
fn operands(
    source: &cq_ir::native::Operation,
) -> syn::Result<(
    &cq_ir::native::PairInput,
    &cq_ir::native::PairInput,
    &syn::Block,
)> {
    let cq_ir::native::Operation::Pairs {
        left, right, body, ..
    } = source
    else {
        return Err(syn::Error::new_spanned(
            source,
            "pair algorithms require `pairs`",
        ));
    };
    if cq_ir::symbol_name(&left.item) == cq_ir::symbol_name(&right.item) {
        return Err(syn::Error::new_spanned(
            &right.item,
            "pair item bindings must be distinct",
        ));
    }
    Ok((left, right, body))
}
