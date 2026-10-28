//! Hybrid publication with exclusive ownership of each original native layout.
use cq_datalog_partition::{LayoutOwnership, PublicationBarrier, ShardCertificate};
use cq_lang_datalog_round::{Publication, RoundContext};
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
    publication(context, barrier, certificate, &early)
}

/// Publish a completed member in one task, after its last reader has joined.
pub fn member(
    context: &RoundContext,
    publication: &Publication,
    certificate: &ShardCertificate,
) -> TokenStream {
    let next = &context.relations[publication.relation].next;
    let threshold = certificate.threshold;
    let inline = Ident::new("__cq_publication_inline", Span::mixed_site());
    let operations = operations(context, publication, certificate, &inline);
    quote!({
        let #inline = #next.len() < #threshold || ::rayon::current_num_threads() == 1;
        #operations
    })
}

pub fn publication(
    context: &RoundContext,
    barrier: &PublicationBarrier,
    certificate: &ShardCertificate,
    excluded: &[usize],
) -> Vec<TokenStream> {
    context.strata[barrier.stratum]
        .publication
        .iter()
        .filter(|publication| {
            barrier.relations.contains(&publication.relation)
                && !excluded.contains(&publication.relation)
        })
        .map(|publication| member(context, publication, certificate))
        .collect()
}

/// Formal rule P-DL-PublicationOrder: Execute the recorded operations in order, joining each layout operation before the next.
fn operations(
    context: &RoundContext,
    publication: &Publication,
    certificate: &ShardCertificate,
    inline: &Ident,
) -> TokenStream {
    let relation = &context.relations[publication.relation];
    let operations =
        publication
            .operations
            .iter()
            .map(|operation| match operation.kind.to_string().as_str() {
                "full_layouts" => layout_phase(
                    operation
                        .layouts
                        .iter()
                        .map(|&index| {
                            (
                                owner(&certificate.layouts, index),
                                cq_datalog_print::full_insert(
                                    std::iter::once(&context.layouts[index]),
                                    &relation.next,
                                ),
                            )
                        })
                        .collect(),
                    inline,
                ),
                "all_extend" => cq_datalog_print::all_extend(relation),
                "delta_swap" => cq_datalog_print::delta_swap(relation),
                "delta_rebuild" => {
                    let delta = relation
                        .delta_host
                        .as_ref()
                        .expect("checked IDB delta host");
                    layout_phase(
                        operation
                            .layouts
                            .iter()
                            .map(|&index| {
                                (
                                    owner(&certificate.delta_layouts, index),
                                    cq_datalog_print::delta_rebuild(
                                        &context.delta_layouts[index],
                                        delta,
                                    ),
                                )
                            })
                            .collect(),
                        inline,
                    )
                }
                _ => unreachable!("checked publication operation"),
            });
    quote!(#(#operations)*)
}

fn owner(ownership: &[LayoutOwnership], index: usize) -> usize {
    ownership
        .iter()
        .find(|entry| entry.index == index)
        .expect("checked complete layout ownership")
        .owner
}

/// Formal rule DL-LayoutPublication: Give each recorded layout one exclusive owner and join before the next operation.
fn layout_phase(updates: Vec<(usize, TokenStream)>, inline: &Ident) -> TokenStream {
    if updates.is_empty() {
        return TokenStream::new();
    }
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
            protocol: cq_lang_datalog_round::storage::LayoutProtocol::GroupedMap,
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
        assert_bulk_publication(syn::parse_quote!(
            ::cq_datalog_parallel_runtime::ShardedLayout<(u32,), (u32, u32)>
        ));
    }

    #[test]
    fn run_publication_bulk_fills_outside_shared_hashmap_loop() {
        assert_bulk_publication(syn::parse_quote!(
            ::cq_datalog_parallel_runtime::RunLayout<(u32,), (u32, u32)>
        ));
    }

    fn assert_bulk_publication(provider: syn::Type) {
        let map = layout(
            syn::parse_quote!(::std::collections::HashMap<(u32,), Vec<(u32, u32)>>),
            syn::parse_quote!(map),
        );
        let mut sharded = layout(provider, syn::parse_quote!(sharded));
        sharded.protocol = cq_lang_datalog_round::storage::LayoutProtocol::Bulk;
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
        assert_bulk_delta(syn::parse_quote!(ShardedLayout<(u32, u32, u32), ()>));
    }

    #[test]
    fn run_delta_clears_and_preserves_empty_payload_tuple() {
        assert_bulk_delta(syn::parse_quote!(RunLayout<(u32, u32, u32), ()>));
    }

    fn assert_bulk_delta(provider: syn::Type) {
        let mut layout = layout(provider, syn::parse_quote!(sharded));
        layout.protocol = cq_lang_datalog_round::storage::LayoutProtocol::Bulk;
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
    fn grouped_protocol_ignores_provider_spelling() {
        for provider in [
            syn::parse_quote!(ShardedLayout::HashMap<(u32,), Vec<(u32, u32)>>),
            syn::parse_quote!(RunLayout::HashMap<(u32,), Vec<(u32, u32)>>),
            syn::parse_quote!(HashMap<(u32,), Vec<RunLayout>>),
            syn::parse_quote!(ShardedLayout<(u32,), (u32, u32)>),
            syn::parse_quote!(RunLayout<(u32,), (u32, u32)>),
        ] {
            let layout = layout(provider, syn::parse_quote!(map));
            let printed =
                cq_datalog_print::full_insert([&layout], &syn::parse_quote!(next)).to_string();
            assert!(printed.contains("entry"));
            assert!(!printed.contains("extend_from"));
            let delta =
                cq_datalog_print::delta_rebuild(&layout, &syn::parse_quote!(delta)).to_string();
            assert!(delta.contains("Default"));
            assert!(!delta.contains("clear"));
        }
    }

    #[test]
    fn certified_owners_name_tasks_without_reordering_them() {
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
            printed.find("__cq_publish_layout_9").unwrap()
                < printed.find("__cq_publish_layout_3").unwrap()
        );
        assert!(printed.find("second_map").unwrap() < printed.find("first_map").unwrap());
        assert_eq!(printed.matches("first_map").count(), 1);
        assert_eq!(printed.matches("second_map").count(), 1);
    }
}
