//! IO schemas preserve authored column order and opaque Rust field types.
//! They deliberately do not accept CQ programs or compiler annotations.
use std::collections::BTreeSet;

use syn::{Token, ext::IdentExt, parse::Parse, punctuated::Punctuated};

mod kw {
    syn::custom_keyword!(relation);
}

pub(crate) struct Column {
    pub(crate) name: syn::Ident,
    pub(crate) ty: syn::Type,
    pub(crate) family: Option<syn::Ident>,
}

impl Parse for Column {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty = input.parse()?;
        let family = if input.peek(Token![@]) {
            input.parse::<Token![@]>()?;
            let family: syn::Ident = input.parse()?;
            if family != "bytes" && family != "plain" {
                return Err(syn::Error::new_spanned(
                    family,
                    "expected an explicit scan family: `bytes` or `plain`",
                ));
            }
            Some(family)
        } else {
            None
        };
        Ok(Self { name, ty, family })
    }
}

pub(crate) struct Declaration {
    pub(crate) name: syn::Ident,
    pub(crate) columns: Punctuated<Column, Token![,]>,
}

impl Parse for Declaration {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::relation>()?;
        let name = input.parse()?;
        let columns;
        syn::parenthesized!(columns in input);
        let columns = columns.parse_terminated(Column::parse, Token![,])?;
        input.parse::<Token![;]>()?;
        Ok(Self { name, columns })
    }
}

pub(crate) struct Declarations(pub(crate) Vec<Declaration>);

impl Parse for Declarations {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Err(input.error("expected an IO relation declaration"));
        }
        let mut declarations = Vec::new();
        while !input.is_empty() {
            declarations.push(input.parse()?);
        }
        Ok(Self(declarations))
    }
}

pub(crate) fn check_declarations(
    declarations: &[Declaration],
    columnar: bool,
) -> syn::Result<()> {
    let mut relation_names = BTreeSet::new();
    for declaration in declarations {
        if declaration.columns.is_empty() {
            return Err(syn::Error::new_spanned(
                &declaration.name,
                format!("input relation `{}` must have positive arity", declaration.name),
            ));
        }
        let mut column_names = BTreeSet::new();
        for column in &declaration.columns {
            let name = column.name.unraw().to_string();
            if !column_names.insert(name.clone()) {
                return Err(syn::Error::new_spanned(
                    &column.name,
                    format!(
                        "input relation `{}` declares column `{name}` more than once",
                        declaration.name,
                    ),
                ));
            }
            if columnar != column.family.is_some() {
                return Err(syn::Error::new_spanned(
                    &column.name,
                    if columnar {
                        "expected an explicit scan family: `@ bytes` or `@ plain`"
                    } else {
                        "scan family annotations require a columnar loader"
                    },
                ));
            }
        }
        if !relation_names.insert(declaration.name.unraw().to_string()) {
            return Err(syn::Error::new_spanned(
                &declaration.name,
                format!("input relation `{}` is declared more than once", declaration.name),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Declarations, check_declarations};
    use quote::quote;
    use syn::ext::IdentExt;

    #[test]
    fn preserves_order_raw_names_and_opaque_rust_types() {
        let declarations = syn::parse_str::<Declarations>(
            "relation r#type(z: crate::Alias, r#match: std::vec::Vec<(i64, String)>, a: [u8; 8],);",
        ).unwrap();
        check_declarations(&declarations.0, false).unwrap();
        let declaration = &declarations.0[0];
        assert_eq!(declaration.name.unraw().to_string(), "type");
        let names: Vec<_> = declaration.columns.iter()
            .map(|column| column.name.unraw().to_string()).collect();
        assert_eq!(names, ["z", "match", "a"]);
        let types: Vec<_> = declaration.columns.iter().map(|column| {
            let ty = &column.ty;
            quote!(#ty).to_string()
        }).collect();
        assert_eq!(types, ["crate :: Alias", "std :: vec :: Vec < (i64 , String) >", "[u8 ; 8]"]);
    }

    #[test]
    fn refuses_full_cq_programs_and_non_schema_syntax() {
        for source in [
            "",
            "struct Query; relation R(x: i64); relation answer(x: i64); answer(x) :- R(x);",
            "relation R(x: i64); relation answer(x: i64); answer(x) :- R(x);",
            "relation R(x: i64); struct Query;",
            "relation R(x:);",
            "relation R(x: i64)",
            "relation R(x i64);",
            "relation R(x: i64 @ packed);",
        ] {
            assert!(syn::parse_str::<Declarations>(source).is_err(), "{source}");
        }
    }

    #[test]
    fn rejects_empty_duplicate_and_wrong_family_schemas() {
        for (source, columnar) in [
            ("relation R();", false),
            ("relation R(x: i64, r#x: i32);", false),
            ("relation R(x: i64); relation r#R(y: i32);", false),
            ("relation R(x: i64 @ bytes);", false),
            ("relation R(x: i64);", true),
        ] {
            let declarations = syn::parse_str::<Declarations>(source).unwrap();
            assert!(check_declarations(&declarations.0, columnar).is_err(), "{source}");
        }
    }
}
