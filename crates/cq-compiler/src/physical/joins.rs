//! Natural-join realization from an existing RA definition and checked bindings.
//! Key and output positions are derived from the contextual source, never supplied
//! as a second positional join language.
use crate::RecipeOptions;
use cq_ir::{physical_context as context, rust_access_plan as access};
use quote::quote;

/// CP-Hash: realize a contextual NaturalJoin through exact witness buckets.
pub fn compile_hash_join(
    source: &context::Context,
    options: &RecipeOptions,
) -> syn::Result<syn::Expr> {
    emit(source, options, true)
}

/// CP-Nested: realize the same NaturalJoin by direct paired traversal.
pub fn compile_nested_loop_join(
    source: &context::Context,
    options: &RecipeOptions,
) -> syn::Result<syn::Expr> {
    emit(source, options, false)
}

fn emit(source: &context::Context, options: &RecipeOptions, hash: bool) -> syn::Result<syn::Expr> {
    context::contract::check_join(source)?;
    let analysis = context::analyze(source)?;
    let next = &options.access;
    let (lp, l) = (
        &analysis.left.provider.as_ref().unwrap().path,
        &analysis.left.value,
    );
    let (rp, r) = (
        &analysis.right.provider.as_ref().unwrap().path,
        &analysis.right.value,
    );
    let (lk, rk) = (&analysis.left_columns, &analysis.right_columns);
    let key_arity = lk.len();
    let width = analysis.output_layout.len();
    let work = options
        .work
        .as_ref()
        .map(|work| quote!(let __join_work = (#work);));
    let probes = options
        .work
        .as_ref()
        .map(|_| quote!(__join_work.add(|w| w.exact_probes += 1);));
    let index_rows = options
        .work
        .as_ref()
        .map(|_| quote!(__join_work.add(|w| w.index_rows += 1);));
    let fields: Vec<_> = analysis
        .output_layout
        .iter()
        .map(|column| {
            if let Some(position) = analysis.left_layout.iter().position(|name| name == column) {
                quote!(__left_row[#position])
            } else {
                let position = analysis
                    .right_layout
                    .iter()
                    .position(|name| name == column)
                    .unwrap();
                quote!(__right_row[#position])
            }
        })
        .collect();
    let mut clauses: Vec<access::Clause> = vec![syn::parse2(quote! {
        Left(__left_row) => for (__left_row,) in (
            #lp! { scan (__join_left) }.map(|row|(row,))
        )
    })?];
    let build = if hash {
        clauses.push(syn::parse2(quote! {
            Right(__right_row) => for (__right_row,) in ({
                #probes
                __join_index_ref.get(&[#(__left_row[#lk]),*]).into_iter().flatten().copied().map(|row|(row,))
            })
        })?);
        quote! {
            let mut __join_index = ::std::collections::HashMap::<[::std::primitive::i64;#key_arity],::std::vec::Vec<_>>::new();
            for row in #rp! { scan (__join_right) } {
                #index_rows
                __join_index.entry([#(row[#rk]),*]).or_default().push(row);
            }
            let __join_index_ref = &__join_index;
        }
    } else {
        clauses.push(syn::parse2(quote! {
            Right(__right_row) => for (__right_row,) in (
                #rp! { scan (__join_right) }.map(|row|(row,))
            )
        })?);
        clauses.push(syn::parse2(quote! {
            if [#(__left_row[#lk]),*] == [#(__right_row[#rk]),*] => if ({
                #probes
                let left:[::std::primitive::i64;#key_arity]=[#(__left_row[#lk]),*];
                let right:[::std::primitive::i64;#key_arity]=[#(__right_row[#rk]),*];
                left==right
            })
        })?);
        quote!()
    };
    let plan: access::Plan = syn::parse2(quote! {
        out(__left_row,__right_row) => ({ let __joined: [::std::primitive::i64; #width] = [#(#fields),*]; __joined }) :- #(#clauses),*;
    })?;
    access::contract::check(&plan)?;
    syn::parse2(quote! {{
        let __join_left = &(#l);
        let __join_right = &(#r);
        #work
        #build
        #next! {#plan}
    }})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstrumented_join_recipes_emit_no_counter_bindings_or_operations() {
        let source = syn::parse_quote! {
            joined = natural_join l with r; {
                l(a,b) => crate::left_rows { left };
                r(a,c) => crate::right_rows { right };
            }
        };
        let options = RecipeOptions {
            storage: syn::parse_quote!(crate::storage),
            access: syn::parse_quote!(crate::collect),
            keys: None,
            work: None,
        };
        for compile in [compile_hash_join, compile_nested_loop_join] {
            let result = compile(&source, &options).unwrap();
            let text = quote!(#result).to_string();
            assert!(!text.contains("__join_work"), "{text}");
            assert!(!text.contains(". add"), "{text}");
        }
    }
}
