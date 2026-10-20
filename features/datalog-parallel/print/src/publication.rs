//! Hybrid publication with exclusive ownership of each original native layout.
use cq_lang_datalog_round::{Layout, RoundContext, Stratum};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Ident;

/// Formal rule DL-LayoutPublication: Publish each native layout through one exclusive owner and join before evaluation.
pub fn publication(
    context: &RoundContext,
    stratum: &Stratum,
    threshold: usize,
) -> Vec<TokenStream> {
    let mut full_updates = Vec::new();
    let mut relation_updates = Vec::new();
    let mut delta_updates = Vec::new();
    // Extract the recorded operations rather than reconstructing layout coverage.
    // The checked dependencies permit all extension to move past full-layout
    // updates, but every full update must finish before any delta is replaced.
    for publication in &stratum.publication {
        let relation = &context.relations[publication.relation];
        let all = &relation.all;
        let next = &relation.next;
        let delta = relation
            .delta_host
            .as_ref()
            .expect("checked IDB delta host");
        for operation in &publication.operations {
            match operation.kind.to_string().as_str() {
                "full_layouts" => {
                    full_updates.extend(
                        operation
                            .layouts
                            .iter()
                            .map(|&index| publish_layout(&context.layouts[index], next, false)),
                    );
                }
                "all_extend" => {
                    relation_updates.push(quote!(#all.extend(#next.iter().cloned());));
                }
                "delta_swap" => relation_updates.push(quote!(#delta = #next;)),
                "delta_rebuild" => {
                    delta_updates.extend(
                        operation.layouts.iter().map(|&index| {
                            publish_layout(&context.delta_layouts[index], delta, true)
                        }),
                    );
                }
                _ => unreachable!("checked publication operation"),
            }
        }
    }

    let inline = Ident::new("__cq_publication_inline", Span::mixed_site());
    let lengths = stratum.members.iter().map(|&index| {
        let next = &context.relations[index].next;
        quote!(#next.len())
    });
    let full_phase = layout_phase(&full_updates, &inline);
    let delta_phase = layout_phase(&delta_updates, &inline);
    // Both scopes join before proceeding: no next vector moves while a full
    // layout reads it, and no observer or next evaluation sees partial maps.
    vec![quote!({
        let #inline = (0usize #(+ #lengths)*) < #threshold
            || ::rayon::current_num_threads() == 1;
        #full_phase
        #(#relation_updates)*
        #delta_phase
    })]
}

fn publish_layout(layout: &Layout, rows: &Ident, rebuild: bool) -> TokenStream {
    let host = &layout.host;
    let clear = rebuild.then(|| quote!(#host.clear();));
    let insert = cq_datalog_print::insert_layout(layout, false);
    quote! {
        #clear
        for row in &#rows { #insert }
    }
}

fn layout_phase(updates: &[TokenStream], inline: &Ident) -> TokenStream {
    if updates.is_empty() {
        return TokenStream::new();
    }
    let scope = Ident::new("__cq_publication_scope", Span::mixed_site());
    let tasks: Vec<_> = (0..updates.len())
        .map(|index| format_ident!("__cq_publish_layout_{}", index, span = Span::mixed_site()))
        .collect();
    let closures = tasks
        .iter()
        .zip(updates)
        .map(|(task, update)| quote!(let mut #task = || { #update };));
    let spawned = tasks
        .iter()
        .map(|task| quote!(#scope.spawn(move |_| { #task(); });));
    // Each closure borrows a distinct map exclusively and its relation's rows
    // immutably. The phase block releases these borrows before relation swaps.
    quote!({
        #(#closures)*
        if #inline {
            #(#tasks();)*
        } else {
            ::rayon::scope(|#scope| { #(#spawned)* });
        }
    })
}
