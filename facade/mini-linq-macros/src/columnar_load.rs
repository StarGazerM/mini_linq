use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{ext::IdentExt, parse::Parse, punctuated::Punctuated, Token};

mod kw {
    syn::custom_keyword!(columnar);
}

struct Field {
    name: syn::Ident,
    ty: syn::Type,
    family: syn::Ident,
}

impl Parse for Field {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty = input.parse()?;
        input.parse::<Token![@]>()?;
        let family: syn::Ident = input.parse()?;
        if family != "bytes" && family != "plain" {
            return Err(syn::Error::new_spanned(
                family,
                "expected an explicit scan family: `bytes` or `plain`",
            ));
        }
        Ok(Self { name, ty, family })
    }
}

struct Declaration {
    name: syn::Ident,
    fields: Punctuated<Field, Token![,]>,
}

impl Parse for Declaration {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<cq_lang_cq::kw::relation>()?;
        let name = input.parse()?;
        let fields;
        syn::parenthesized!(fields in input);
        let fields = fields.parse_terminated(Field::parse, Token![,])?;
        input.parse::<Token![;]>()?;
        Ok(Self { name, fields })
    }
}

struct Declarations {
    interned: bool,
    relations: Vec<Declaration>,
}

impl Parse for Declarations {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let interned = input.peek(kw::columnar);
        if interned {
            input.parse::<kw::columnar>()?;
            input.parse::<Token![;]>()?;
        }
        if input.is_empty() {
            return Err(input.error("expected a columnar relation declaration"));
        }
        let mut relations = Vec::new();
        while !input.is_empty() {
            relations.push(input.parse()?);
        }
        Ok(Self { interned, relations })
    }
}

pub(crate) fn expand(input: TokenStream, interned: bool) -> TokenStream {
    syn::parse::<Declarations>(input)
        .and_then(|declarations| {
            if declarations.interned != interned {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "use `columnar;` only with `interned_relation_loaders!`",
                ));
            }
            generate(declarations)
        })
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn generate(declarations: Declarations) -> syn::Result<proc_macro2::TokenStream> {
    // Reuse declaration validation without extending the query language with
    // loader-only representation annotations.
    let schemas: Vec<cq_lang_cq::RelationDecl> = declarations
        .relations
        .iter()
        .map(|declaration| {
            let name = &declaration.name;
            let names = declaration.fields.iter().map(|field| &field.name);
            let types = declaration.fields.iter().map(|field| &field.ty);
            syn::parse2(quote!(relation #name(#(#names: #types),*);))
        })
        .collect::<syn::Result<_>>()?;
    cq_lang_cq::check_input_declarations(&schemas)?;

    let mut output = proc_macro2::TokenStream::new();
    let mut selectors = Vec::new();
    for declaration in declarations.relations {
        let relation = &declaration.name;
        let name = relation.unraw().to_string();
        if declaration.fields.len() > 12 {
            return Err(syn::Error::new_spanned(
                relation,
                "scan tables support at most 12 columns",
            ));
        }
        let marker = format_ident!("__MiniLinqColumnarLoad{}", name);
        let loader = format_ident!("{}Loader", name);
        selectors.push(quote!((#relation) => { #loader };));
        let types: Vec<_> = declaration.fields.iter().map(|field| &field.ty).collect();
        let names: Vec<_> = declaration.fields.iter()
            .map(|field| field.name.unraw().to_string()).collect();
        let families: Vec<_> = declaration.fields.iter().map(|field| {
            if field.family == "bytes" {
                quote!(::cq_storage::scan::Bytes)
            } else {
                quote!(::cq_storage::scan::Plain)
            }
        }).collect();
        let parameters: Vec<_> = (0..types.len()).map(|i| format_ident!("Column{i}")).collect();
        let indices: Vec<_> = (0..types.len()).collect();
        let tuple_indices: Vec<_> = indices.iter().map(|&i| syn::Index::from(i)).collect();
        let (bounds, builders, initialize, append, finish) = if declarations.interned {
            (
                quote!(#(#parameters: ::cq_load::ColumnarField<#families>),*),
                quote!((#(<#parameters as ::cq_load::ColumnarField<#families>>::Builder,)*)),
                quote!((#(<#parameters as ::cq_load::ColumnarField<#families>>::builder(block_rows, summary),)*)),
                quote!(#(
                    <#parameters as ::cq_load::ColumnarField<#families>>::append(
                        batch, #indices, &mut builders.#tuple_indices,
                    )?;
                )*),
                quote!((#(<#parameters as ::cq_load::ColumnarField<#families>>::finish(
                    #names, dictionaries, builders.#tuple_indices,
                ),)*)),
            )
        } else {
            (
                quote!(#(#parameters: ::cq_load::Load + ::cq_storage::scan::OrdinalDecode),*),
                quote!((#(::cq_storage::scan::ScanColumnBuilder<#parameters, #families>,)*)),
                quote!((#(::cq_storage::scan::ScanColumnBuilder::<#parameters, #families>::new(block_rows, summary),)*)),
                quote!(#(
                    batch.append_column::<#parameters, #families>(#indices, &mut builders.#tuple_indices)?;
                )*),
                quote!((#(builders.#tuple_indices.finish(),)*)),
            )
        };
        output.extend(quote! {
            #[allow(non_camel_case_types)]
            pub struct #marker<#(#parameters),*>(::core::marker::PhantomData<fn() -> (#(#parameters,)*)>);
            impl<#bounds> ::cq_load::ColumnarRelation for #marker<#(#parameters),*> {
                const COLUMNS: &'static [&'static str] = &[#(#names,)*];
                type Table = ::cq_storage::scan::ScanTable<(
                    #(::cq_storage::scan::ScanColumn<#parameters, #families>,)*
                )>;
                type Builders = #builders;

                fn builders(block_rows: ::cq_storage::scan::BlockRows, summary: bool) -> Self::Builders {
                    #initialize
                }

                fn append(batch: &::cq_load::Batch<'_>, builders: &mut Self::Builders) -> ::cq_load::Result<()> {
                    #append
                    Ok(())
                }

                fn finish(builders: Self::Builders, dictionaries: &mut ::cq_load::Dictionaries) -> Self::Table {
                    let _ = &dictionaries;
                    ::cq_storage::scan::ScanTable::new(#finish)
                }
            }
            #[allow(non_camel_case_types, private_interfaces)]
            pub type #loader = ::cq_load::ColumnarLoader<#marker<#(#types),*>>;
        });
    }
    output.extend(quote! {
        #[allow(unused_macros)]
        macro_rules! relation_loader { #(#selectors)* }
    });
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{Declarations, generate};

    #[test]
    fn columnar_schemas_require_known_explicit_families() {
        for source in [
            "", "relation R(x: i64);", "relation R(x: i64 @ packed);",
            "relation R(x: i64 @ unknown);",
        ] {
            assert!(syn::parse_str::<Declarations>(source).is_err(), "{source}");
        }
        for source in [
            "relation R();", "relation R(x: i64 @ bytes, x: i32 @ plain);",
            "relation R(x: i64 @ bytes); relation R(x: i64 @ plain);",
            "relation R(a:i64 @ bytes,b:i64 @ bytes,c:i64 @ bytes,d:i64 @ bytes,e:i64 @ bytes,f:i64 @ bytes,g:i64 @ bytes,h:i64 @ bytes,i:i64 @ bytes,j:i64 @ bytes,k:i64 @ bytes,l:i64 @ bytes,m:i64 @ bytes);",
        ] {
            assert!(generate(syn::parse_str(source).unwrap()).is_err(), "{source}");
        }
    }

    #[test]
    fn both_modes_generate_column_builders_without_row_decoders() {
        for prefix in ["", "columnar;"] {
            let source = format!("{prefix} relation R(id: i64 @ plain, value: CodeAlias @ bytes);");
            let tokens = generate(syn::parse_str(&source).unwrap()).unwrap();
            syn::parse2::<syn::File>(tokens.clone()).unwrap();
            let text = tokens.to_string();
            assert!(text.contains("ScanTable"));
            assert!(text.contains("Plain"));
            assert!(text.contains("Bytes"));
            assert!(!text.contains("Vec"));
            assert!(!text.contains("fn decode"));
        }
    }
}
