//! Hybrid publication with exclusive ownership of each original native layout.
use cq_datalog_partition::{LayoutOwnership, PublicationBarrier, ShardCertificate};
use cq_lang_datalog_round::RoundContext;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Ident;

pub fn early_members(
    schedule: &cq_lang_datalog_round::schedule::StratumSchedule,
    phase: &str,
) -> Vec<usize> {
    let last = schedule.waves.iter().rposition(|wave| wave.phase == phase);
    schedule
        .publication
        .iter()
        .filter(|entry| entry.phase == phase && Some(entry.wave) < last)
        .map(|entry| entry.relation)
        .collect()
}

pub fn phase_publication(
    context: &RoundContext,
    barrier: &PublicationBarrier,
    certificate: &ShardCertificate,
    schedule: &cq_lang_datalog_round::schedule::StratumSchedule,
    phase: &str,
) -> Vec<TokenStream> {
    let early = early_members(schedule, phase);
    let mut barrier = barrier.clone();
    barrier.relations.retain(|index| !early.contains(index));
    barrier
        .layouts
        .retain(|&index| !early.contains(&context.layouts[index].relation));
    barrier
        .delta_layouts
        .retain(|&index| !early.contains(&context.delta_layouts[index].relation));
    publication(context, &barrier, certificate)
}

/// Publish a completed member in one task, after its last reader has joined.
pub fn member(context: &RoundContext, index: usize, certificate: &ShardCertificate) -> TokenStream {
    let relation = &context.relations[index];
    let mut layouts: Vec<_> = certificate
        .layouts
        .iter()
        .filter(|entry| context.layouts[entry.index].relation == index)
        .collect();
    layouts.sort_unstable_by_key(|entry| owner(&certificate.layouts, entry.index));
    let full = layouts.iter().map(|entry| {
        cq_datalog_print::full_insert(
            std::iter::once(&context.layouts[entry.index]),
            &relation.next,
        )
    });
    let extend = cq_datalog_print::all_extend(relation);
    let swap = cq_datalog_print::delta_swap(relation);
    let mut layouts: Vec<_> = certificate
        .delta_layouts
        .iter()
        .filter(|entry| context.delta_layouts[entry.index].relation == index)
        .collect();
    layouts.sort_unstable_by_key(|entry| owner(&certificate.delta_layouts, entry.index));
    let delta = relation
        .delta_host
        .as_ref()
        .expect("checked IDB delta host");
    let rebuild = layouts
        .iter()
        .map(|entry| cq_datalog_print::delta_rebuild(&context.delta_layouts[entry.index], delta));
    quote!(#(#full)* #extend #swap #(#rebuild)*)
}

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
    // Each closure borrows a distinct layout exclusively and its relation's rows
    // immutably. Sharded layouts bulk-fill internally; the phase block releases
    // all borrows before relation swaps.
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

    fn layout(provider: syn::Type, host: Ident) -> cq_datalog_print::Layout {
        cq_datalog_print::Layout {
            relation: 0,
            columns: vec![1, 0, 2],
            key: 1,
            name: syn::parse_quote!(layout),
            host,
            provider,
            maintenance: syn::parse_quote!(insert),
        }
    }

    #[test]
    fn hashmap_publication_tokens_stay_unchanged() {
        let layout = layout(
            syn::parse_quote!(::std::collections::HashMap<(u32,), Vec<(u32, u32)>>),
            syn::parse_quote!(map),
        );
        let insert = quote!(
            map.entry((::core::clone::Clone::clone(&row.1),))
                .or_default()
                .push((::core::clone::Clone::clone(&row.0), ::core::clone::Clone::clone(&row.2),));
        );
        assert_eq!(
            cq_datalog_print::insert_layout(&layout, false).to_string(),
            insert.to_string(),
        );
        assert_eq!(
            cq_datalog_print::full_insert([&layout], &syn::parse_quote!(next)).to_string(),
            quote!({ for row in &next { #insert } }).to_string(),
        );
        assert_eq!(
            cq_datalog_print::delta_rebuild(&layout, &syn::parse_quote!(delta)).to_string(),
            quote!({
                map = ::core::default::Default::default();
                for row in &delta { #insert }
            })
            .to_string(),
        );
        assert_eq!(
            cq_datalog_print::insert_layout(&layout, true).to_string(),
            quote!(
                map.entry((::core::clone::Clone::clone(row.1),))
                    .or_default()
                    .push((::core::clone::Clone::clone(row.0), ::core::clone::Clone::clone(row.2),));
            ).to_string(),
        );
    }

    #[test]
    fn sharded_publication_bulk_fills_outside_shared_hashmap_loop() {
        let map = layout(
            syn::parse_quote!(::std::collections::HashMap<(u32,), Vec<(u32, u32)>>),
            syn::parse_quote!(map),
        );
        let sharded = layout(
            syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedLayout<(u32,), (u32, u32)>),
            syn::parse_quote!(sharded),
        );
        let insert = cq_datalog_print::insert_layout(&map, false);
        let bulk = quote!(
            sharded.extend_from(
                &next,
                |row| (::core::clone::Clone::clone(&row.1),),
                |row| (::core::clone::Clone::clone(&row.0), ::core::clone::Clone::clone(&row.2),)
            );
        );
        let printed = cq_datalog_print::full_insert([&sharded, &map], &syn::parse_quote!(next));
        assert_eq!(
            printed.to_string(),
            quote!({ for row in &next { #insert } #bulk }).to_string()
        );
        assert_eq!(
            cq_datalog_print::full_insert([&sharded], &syn::parse_quote!(next)).to_string(),
            quote!({ #bulk }).to_string(),
        );
        let phase = layout_phase(vec![(7, printed)], &syn::parse_quote!(inline)).to_string();
        assert_eq!(phase.matches("extend_from").count(), 1);
        assert_eq!(phase.matches("for row").count(), 1);
    }

    #[test]
    fn sharded_delta_clears_and_preserves_empty_payload_tuple() {
        let mut layout = layout(
            syn::parse_quote!(ShardedLayout<(u32, u32, u32), ()>),
            syn::parse_quote!(sharded),
        );
        layout.key = 3;
        assert_eq!(
            cq_datalog_print::delta_rebuild(&layout, &syn::parse_quote!(delta)).to_string(),
            quote!({
                sharded.clear();
                sharded.extend_from(
                    &delta,
                    |row| {
                        (
                            ::core::clone::Clone::clone(&row.1),
                            ::core::clone::Clone::clone(&row.0),
                            ::core::clone::Clone::clone(&row.2),
                        )
                    },
                    |row| (),
                );
            })
            .to_string(),
        );
        assert_eq!(
            cq_datalog_print::insert_layout(&layout, false).to_string(),
            quote!(
                sharded.push(
                    (::core::clone::Clone::clone(&row.1), ::core::clone::Clone::clone(&row.0), ::core::clone::Clone::clone(&row.2),),
                    ()
                );
            ).to_string(),
        );
    }

    #[test]
    fn provider_detection_uses_only_last_path_segment() {
        let layout = layout(
            syn::parse_quote!(ShardedLayout::HashMap<(u32,), Vec<(u32, u32)>>),
            syn::parse_quote!(map),
        );
        let printed =
            cq_datalog_print::full_insert([&layout], &syn::parse_quote!(next)).to_string();
        assert!(printed.contains("entry"));
        assert!(!printed.contains("extend_from"));
    }

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
