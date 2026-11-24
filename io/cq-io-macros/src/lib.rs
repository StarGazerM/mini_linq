//! Typed source-reader macros for `cq-load`, independent of query compilation.
//!
//! Inputs are ordered `relation Name(column: RustType, ...);` schemas, not CQ
//! programs. Columnar schemas additionally annotate every field with `@ bytes`
//! or `@ plain`. Field traits, not type spelling, determine verified-key metadata.
use proc_macro::TokenStream;

mod columnar_load;
mod load;
mod schema;

/// Generate typed source readers from IO relation declarations.
///
/// Accepts one or more `relation Name(column: RustType, ...);` declarations.
/// Full CQ programs, rules and compiler metadata are not IO schemas.
#[proc_macro]
pub fn relation_loaders(input: TokenStream) -> TokenStream {
    load::expand(input, false)
}

/// Generate scan-table readers with an explicit `@ bytes` or `@ plain` family
/// on every field. This does not change query planning or ordinary loaders.
#[proc_macro]
pub fn columnar_relation_loaders(input: TokenStream) -> TokenStream {
    columnar_load::expand(input, false)
}

/// Generate typed readers with explicit load-time string interning support.
///
/// Accepts the same IO relation declarations as `relation_loaders`. Adds an
/// `InternedRelation` implementation selected through `InternedField` traits,
/// enabling `Loader::intern` and `Loader::parquet_interned`. Prefix the schemas
/// with `columnar;` and annotate every field's family for columnar readers.
/// This does not change query syntax or inspect Rust field type spellings.
#[proc_macro]
pub fn interned_relation_loaders(input: TokenStream) -> TokenStream {
    if matches!(input.clone().into_iter().next(), Some(proc_macro::TokenTree::Ident(ident)) if ident.to_string() == "columnar")
    {
        columnar_load::expand(input, true)
    } else {
        load::expand(input, true)
    }
}
