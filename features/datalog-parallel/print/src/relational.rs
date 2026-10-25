//! Event-driven step execution; seeds and the single-worker path stay synchronous.
//! The scheduler excludes every conflicting reader before dispatching a commit.
//! Host guards therefore never block a Rayon worker that could be running nested
//! chunk/admission work. Pending buffers are locked only to append or take;
//! canonical owners are touched only by commits, and task hashers are cloned
//! once before dispatch so tasks never borrow an owner undergoing admission.
use cq_lang_datalog_round::{self as round, Stratum};
use proc_macro2::{Group, Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use std::collections::BTreeMap;
use syn::Ident;

// Replace storage references, including those inside split evaluator arguments.
// Fresh local names must survive the enclosing component's storage splitting.
fn rename(tokens: TokenStream, names: &BTreeMap<String, TokenStream>) -> TokenStream {
    tokens
        .into_iter()
        .flat_map(|token| match token {
            TokenTree::Ident(ident) if names.contains_key(&ident.to_string()) => {
                names[&ident.to_string()].clone()
            }
            TokenTree::Group(group) => {
                let mut replaced = Group::new(group.delimiter(), rename(group.stream(), names));
                replaced.set_span(group.span());
                TokenStream::from(TokenTree::Group(replaced))
            }
            token => TokenStream::from(token),
        })
        .collect()
}

/// Formal rule P-DL-Relational:
/// Ordered reader cuts and exclusive publication realize tiled member batches.
pub(super) fn step_loop(
    program: &cq_datalog_partition::ShardedProgram,
    holes: &[TokenStream],
    index: usize,
    stratum: &Stratum,
    default: TokenStream,
) -> syn::Result<TokenStream> {
    if program.round.schedule.progress != round::schedule::RELATIONAL || stratum.steps.is_empty() {
        return Ok(default);
    }
    let context = &program.round.context;
    assert!(context.observer.is_none());
    assert_eq!(program.round.policy.timing, "none");
    let schedule = &program.round.schedule.strata[index];
    let threshold = program.certificate.threshold;
    let locks: Vec<_> = (0..stratum.members.len())
        .map(|m| format_ident!("__cq_rel_lock_{m}", span = Span::mixed_site()))
        .collect();
    let admits: Vec<_> = (0..stratum.members.len())
        .map(|m| format_ident!("__cq_rel_admit_{m}", span = Span::mixed_site()))
        .collect();
    let seen_locks: Vec<_> = (0..stratum.members.len())
        .map(|m| format_ident!("__cq_rel_seen_{m}", span = Span::mixed_site()))
        .collect();
    let local: BTreeMap<_, _> = stratum
        .members
        .iter()
        .enumerate()
        .map(|(m, &relation)| (relation, m))
        .collect();
    let hosts: Vec<Vec<Ident>> = stratum
        .members
        .iter()
        .map(|&member| {
            let relation = &context.relations[member];
            let mut hosts = vec![
                relation.all.clone(),
                relation.delta_host.clone().expect("checked delta host"),
            ];
            let publication = stratum
                .publication
                .iter()
                .find(|p| p.relation == member)
                .expect("checked publication");
            for op in &publication.operations {
                let layouts = if op.kind == "delta_rebuild" {
                    &context.delta_layouts
                } else {
                    &context.layouts
                };
                hosts.extend(op.layouts.iter().map(|&i| layouts[i].host.clone()));
            }
            hosts
        })
        .collect();
    let initial = hosts.iter().map(|hosts| {
        let delta = &hosts[1];
        quote!(!#delta.is_empty())
    });
    let declarations = stratum.members.iter().enumerate().map(|(m, &member)| {
        let lock = &locks[m];
        let admit = &admits[m];
        let seen_lock = &seen_locks[m];
        let hosts = &hosts[m];
        let relation = &context.relations[member];
        let seen = &relation.seen;
        let types = relation.declaration.columns.iter().map(|column| &column.ty);
        let hasher = format_ident!("__cq_rel_hasher_{member}", span = Span::mixed_site());
        let hasher = program.certificate.self_dedup.is_some().then(|| quote!(let #hasher = #seen.hasher().clone();));
        quote!(
            #hasher
            let #lock = ::std::sync::RwLock::new((#(&mut #hosts,)*));
            let #seen_lock = ::std::sync::Mutex::new(&mut #seen);
            let #admit = ::std::sync::Mutex::new(::std::vec::Vec::<::std::vec::Vec<(#(#types,)*)>>::new());
        )
    });
    let mut hole_reads = Vec::new();
    let mut hole_arms = Vec::new();
    for (h, rule) in stratum.steps.iter().enumerate() {
        let reads = &schedule
            .reads
            .iter()
            .find(|r| r.phase == round::schedule::STEP && r.index == h)
            .expect("checked hole reads")
            .reads;
        let deltas: Vec<_> = reads.iter().filter(|r| r.version == "delta").collect();
        assert_eq!(deltas.len(), 1, "step hole has exactly one delta read");
        let delta = *local
            .get(&deltas[0].relation)
            .expect("delta is a component member");
        let members: Vec<_> = stratum
            .members
            .iter()
            .enumerate()
            .filter(|(_, member)| reads.iter().any(|read| read.relation == **member))
            .map(|(m, _)| m)
            .collect();
        hole_reads.push(quote!((#delta, ::std::vec![#(#members,)*])));
        let mut bindings = Vec::new();
        let mut names = BTreeMap::new();
        for (m, &member) in stratum.members.iter().enumerate() {
            if !reads.iter().any(|read| read.relation == member) {
                continue;
            }
            let lock = &locks[m];
            let guard = format_ident!("__cq_rel_guard_{m}", span = Span::mixed_site());
            bindings.push(quote!(let #guard = #lock.try_read().expect("P-DL-Relational: scheduler isolation");));
            for (k, host) in hosts[m].iter().enumerate() {
                let field = syn::Index::from(k);
                let ro = format_ident!("__cq_rel_ro_{}", host, span = Span::mixed_site());
                bindings.push(quote!(let #ro = &*#guard.#field;));
                names.insert(host.to_string(), quote!(#ro));
            }
        }
        let head = local[&rule.head];
        let admit = &admits[head];
        let (evaluation, candidates) = super::evaluate(
            std::slice::from_ref(rule),
            context,
            holes,
            &program.round,
            &program.certificate,
            index,
            round::schedule::STEP,
            true,
            #[cfg(feature = "hole-timing")]
            &round::erase(&program.round)?,
        )?;
        let evaluation = rename(evaluation, &names);
        hole_arms.push(quote!(#h => {
            #(#bindings)*
            #evaluation
            let __cq_rel_chunks = #candidates;
            let __cq_rel_produced = __cq_rel_chunks.iter().any(|chunk| !chunk.is_empty());
            if __cq_rel_produced { #admit.lock().unwrap().extend(__cq_rel_chunks); }
            ::cq_datalog_parallel_runtime::relational::Completion::Hole {
                produced: if __cq_rel_produced { ::std::vec![#head] } else { ::std::vec::Vec::new() },
            }
        }));
    }
    // Reuse provider-aware publication under each exclusive member guard:
    // sharded layouts bulk-fill without introducing a separate commit path.
    let publications = cq_datalog_print::publication(context, stratum);
    let commit_arms = stratum.members.iter().enumerate().map(|(m, &member)| {
        let lock = &locks[m];
        let admit = &admits[m];
        let seen_lock = &seen_locks[m];
        let next = &context.relations[member].next;
        let mut names = BTreeMap::new();
        for (k, host) in hosts[m].iter().enumerate() {
            let field = syn::Index::from(k);
            names.insert(host.to_string(), quote!((*__cq_rel_write.#field)));
        }
        let p = stratum.publication.iter().position(|p| p.relation == member).expect("checked publication");
        let publication = rename(publications[p].clone(), &names);
        quote!(#m => {
            let #next = {
                let chunks = ::core::mem::take(&mut *#admit.lock().unwrap());
                #seen_lock.try_lock().expect("P-DL-Relational: exclusive admission").admit(chunks, #threshold)
            };
            let fresh = !#next.is_empty();
            if fresh {
                let mut __cq_rel_write_guard = #lock.try_write().expect("P-DL-Relational: scheduler isolation");
                // Reborrow the tuple once so Rust can split its disjoint fields.
                let __cq_rel_write = &mut *__cq_rel_write_guard;
                #publication
            }
            ::cq_datalog_parallel_runtime::relational::Completion::Commit { fresh }
        })
    });
    Ok(quote!(if ::rayon::current_num_threads() > 1 {
        let __cq_rel_initial = [#(#initial,)*];
        #(#declarations)*
        let mut __cq_rel_scheduler = ::cq_datalog_parallel_runtime::relational::RelationalScheduler::new(::std::vec![#(#hole_reads,)*], &__cq_rel_initial);
        ::cq_datalog_parallel_runtime::relational::drive(&mut __cq_rel_scheduler, &|action| match action {
            ::cq_datalog_parallel_runtime::relational::Action::Hole { hole, .. } => match hole { #(#hole_arms,)* _ => unreachable!("checked hole index") },
            ::cq_datalog_parallel_runtime::relational::Action::Commit { member } => match member { #(#commit_arms,)* _ => unreachable!("checked member index") },
        });
    } else { #default }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharded_publication_reborrows_guarded_layouts_and_delta() {
        assert_bulk_publication_reborrows(syn::parse_quote!(
            ::cq_datalog_parallel_runtime::ShardedLayout<(u32,), (u32,)>
        ));
    }

    #[test]
    fn run_publication_reborrows_guarded_layouts_and_delta() {
        assert_bulk_publication_reborrows(syn::parse_quote!(
            ::cq_datalog_parallel_runtime::RunLayout<(u32,), (u32,)>
        ));
    }

    fn assert_bulk_publication_reborrows(provider: syn::Type) {
        let layout = round::Layout {
            relation: 0,
            columns: vec![0, 1],
            key: 1,
            name: syn::parse_quote!(layout),
            host: syn::parse_quote!(full),
            provider,
            maintenance: syn::parse_quote!(insert),
        };
        let mut delta_layout = layout.clone();
        delta_layout.host = syn::parse_quote!(delta_layout);
        let full = cq_datalog_print::full_insert([&layout], &syn::parse_quote!(next));
        let delta = cq_datalog_print::delta_rebuild(&delta_layout, &syn::parse_quote!(delta));
        let names = BTreeMap::from([
            ("full".into(), quote!((*__cq_rel_write.2))),
            ("delta_layout".into(), quote!((*__cq_rel_write.3))),
            ("delta".into(), quote!((*__cq_rel_write.1))),
        ]);
        assert_eq!(
            rename(quote!(#full #delta), &names).to_string(),
            quote!(
                {
                    (*__cq_rel_write.2).extend_from(
                        &next,
                        |row| (::core::clone::Clone::clone(&row.0),),
                        |row| (::core::clone::Clone::clone(&row.1),)
                    );
                }
                {
                    (*__cq_rel_write.3).clear();
                    (*__cq_rel_write.3).extend_from(
                        &(*__cq_rel_write.1),
                        |row| (::core::clone::Clone::clone(&row.0),),
                        |row| (::core::clone::Clone::clone(&row.1),)
                    );
                }
            )
            .to_string(),
        );
    }
}
