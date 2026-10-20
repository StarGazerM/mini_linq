//! Hybrid publication with exclusive ownership of each original native layout.
use cq_datalog_partition::{LayoutOwnership, PublicationBarrier, ShardCertificate};
use cq_lang_datalog_round::RoundContext;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Ident;

/// Formal rule DL-LayoutPublication: Publish each native layout through one exclusive owner and join before evaluation.
pub fn publication(
    context: &RoundContext,
    barrier: &PublicationBarrier,
    certificate: &ShardCertificate,
) -> Vec<TokenStream> {
    // Coverage comes from the checked barrier; owner labels determine task
    // identity and scheduling order, independently of the producer's ordering.
    let full_updates = barrier
        .layouts
        .iter()
        .map(|&index| {
            let layout = &context.layouts[index];
            let next = &context.relations[layout.relation].next;
            (
                owner(&certificate.layouts, index),
                cq_datalog_print::full_insert(std::iter::once(layout), next),
            )
        })
        .collect();
    let delta_updates = barrier
        .delta_layouts
        .iter()
        .map(|&index| {
            let layout = &context.delta_layouts[index];
            let delta = context.relations[layout.relation]
                .delta_host
                .as_ref()
                .expect("checked IDB delta host");
            (
                owner(&certificate.delta_layouts, index),
                cq_datalog_print::delta_rebuild(layout, delta),
            )
        })
        .collect();
    let relation_updates = barrier.relations.iter().map(|&index| {
        let relation = &context.relations[index];
        let extend = cq_datalog_print::all_extend(relation);
        let swap = cq_datalog_print::delta_swap(relation);
        quote!(#extend #swap)
    });
    let threshold = certificate.threshold;

    let inline = Ident::new("__cq_publication_inline", Span::mixed_site());
    let lengths = barrier.relations.iter().map(|&index| {
        let next = &context.relations[index].next;
        quote!(#next.len())
    });
    let full_phase = layout_phase(full_updates, &inline);
    let delta_phase = layout_phase(delta_updates, &inline);
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

fn owner(ownership: &[LayoutOwnership], index: usize) -> usize {
    ownership
        .iter()
        .find(|entry| entry.index == index)
        .expect("checked complete layout ownership")
        .owner
}

fn layout_phase(mut updates: Vec<(usize, TokenStream)>, inline: &Ident) -> TokenStream {
    if updates.is_empty() {
        return TokenStream::new();
    }
    updates.sort_unstable_by_key(|(owner, _)| *owner);
    let scope = Ident::new("__cq_publication_scope", Span::mixed_site());
    let tasks: Vec<_> = updates
        .iter()
        .map(|(owner, _)| format_ident!("__cq_publish_layout_{}", owner, span = Span::mixed_site()))
        .collect();
    let closures = tasks
        .iter()
        .zip(updates)
        .map(|(task, (_, update))| quote!(let mut #task = || { #update };));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certified_owners_drive_task_identity_and_order() {
        let ownership = [
            LayoutOwnership { index: 0, owner: 9 },
            LayoutOwnership { index: 1, owner: 3 },
        ];
        let updates = vec![
            (owner(&ownership, 0), quote!(second_map())),
            (owner(&ownership, 1), quote!(first_map())),
        ];
        let printed = layout_phase(updates, &syn::parse_quote!(inline)).to_string();
        assert!(
            printed.find("__cq_publish_layout_3").unwrap()
                < printed.find("__cq_publish_layout_9").unwrap()
        );
        assert!(printed.find("first_map").unwrap() < printed.find("second_map").unwrap());
        assert_eq!(printed.matches("first_map").count(), 1);
        assert_eq!(printed.matches("second_map").count(), 1);
    }
}
