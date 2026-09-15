//! Shared native-set operation shape. The selected module supplies its type.
use cq_ir::physical_provider::Operation;
use proc_macro2::TokenStream;
use quote::quote;

pub(super) fn compile(
    operation: &Operation,
    set: TokenStream,
    work: Option<&syn::Expr>,
) -> syn::Result<TokenStream> {
    let output = match operation {
        Operation::Type { arity, .. } => {
            quote!(#set<[::core::primitive::i64; #arity]>)
        }
        Operation::Input { value, .. } | Operation::Row { value, .. } => {
            quote!((#value))
        }
        Operation::Len { value, .. } => {
            quote!((#value).len())
        }
        Operation::Scan { value, .. } | Operation::Rows { value, .. } => {
            if let Some(work) = work {
                quote!({
                    let __cq_source = (#value);
                    let __cq_work = (#work);
                    __cq_source.iter().inspect(move |_| {
                        __cq_work.add(|w| w.rows_visited += 1);
                    })
                })
            } else {
                quote!((#value).iter())
            }
        }
        Operation::Select { value, rows, .. } => {
            quote!({
                let __cq_source = (#value);
                let __cq_selected = (#rows);
                let mut __cq_result = #set::new();
                for __cq_row in __cq_selected {
                    assert!(__cq_source.contains(__cq_row),
                        "selected row is absent from the current input");
                    __cq_result.insert(*__cq_row);
                }
                __cq_result
            })
        }
        Operation::Materialize {
            owner, value, rows, ..
        } => {
            quote! {
                let #owner: #set<_> = (#rows).into_iter().collect();
                let #value = &#owner;
            }
        }
        Operation::Blocks { .. } | Operation::Bounds { .. } | Operation::BlockRows { .. } => {
            return Err(syn::Error::new_spanned(
                operation,
                "this native row-set provider has no block descriptor lowering; select a block provider for this binding or ordinary row access",
            ));
        }
        _ => {
            return Err(syn::Error::new_spanned(
                operation,
                "unsupported native row-set operation",
            ));
        }
    };
    Ok(output)
}
