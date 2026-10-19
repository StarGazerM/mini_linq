//! Rust realization of the checked Datalog round context. CQ holes have already
//! been expanded by the caller's continuation; this bridge alone owns loops.

use cq_datalog_fork::{ExpandedFork, KeyRepresentation, PartitionSource, RoundContext, RuleHole};
use cq_syntax::rust;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Ident;

pub fn render(fork: ExpandedFork, parallel: bool) -> TokenStream {
    let ExpandedFork { context, holes } = fork;
    let mut bindings = Vec::new();
    for relation in &context.relations {
        let declaration = &relation.declaration;
        let all = &relation.all;
        let delta = &relation.delta_host;
        let types: Vec<_> = declaration
            .columns
            .iter()
            .map(|column| &column.ty)
            .collect();
        if let Some(value) = &declaration.value {
            let expression = &value.expression;
            // Keep the original host alive, including temporary and grouped hosts.
            // Rows already supplies Copy tuples of borrowed columns: hash those
            // tuples once, without cloning values or adding another reference layer.
            bindings.push(quote!(
                let #all = &(#expression);
                let #all = &{
                    struct Canonical<R>(::cq_storage::HashSet<R>);
                    impl<R: ::core::marker::Copy> ::cq_storage::rows::Rows for &Canonical<R> {
                        type Row = R;
                        fn rows(self) -> impl ::core::iter::Iterator<Item = R> {
                            self.0.iter().copied()
                        }
                    }
                    Canonical(::cq_storage::rows::Rows::rows(#all).collect())
                };
            ));
        } else {
            let seen = &relation.seen;
            bindings.push(quote!(
                let mut #all: ::std::vec::Vec<(#(#types,)*)> = ::std::vec::Vec::new();
                let mut #seen: ::cq_storage::HashSet<(#(#types,)*)> = ::core::default::Default::default();
            ));
        }
        if let Some(delta) = delta {
            bindings.push(
                quote!(let mut #delta: ::std::vec::Vec<(#(#types,)*)> = ::std::vec::Vec::new();),
            );
        }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        let relation = &context.relations[layout.relation];
        let host = &layout.host;
        let types: Vec<_> = layout
            .columns
            .iter()
            .map(|&i| &relation.declaration.columns[i].ty)
            .collect();
        let (keys, values) = types.split_at(layout.key);
        bindings.push(quote!(
            let mut #host: ::std::collections::HashMap<(#(#keys,)*), ::std::vec::Vec<(#(#values,)*)>, ::cq_storage::HashState>
                = ::core::default::Default::default();
        ));
        // Delta layouts name IDBs, so both IDB host kinds start empty.
        if relation.declaration.value.is_some() {
            let all = &relation.all;
            let insert = insert_layout(layout, true);
            bindings.push(quote!(for row in ::cq_storage::rows::Rows::rows(#all) { #insert }));
        }
    }
    let observer = &context.observer_name;
    let round = &context.round_name;
    let bind_observer = context.observer.as_ref().map(|expression| {
        let expression = rust::unparen(expression);
        quote!(#[allow(unused_mut)] let mut #observer = #expression;)
    });
    let mut strata = Vec::new();
    for stratum in &context.strata {
        let keys = stratum.keys.iter().map(|key| {
            let name = &key.name;
            let invocation = &holes[key.hole];
            match key.representation {
                KeyRepresentation::HashSetBorrowed => {
                    quote!(let #name = &(#invocation).collect::<::cq_storage::HashSet<_>>();)
                }
            }
        });
        let seeds = evaluate(&stratum.seeds, &context, &holes, parallel);
        let steps = evaluate(&stratum.steps, &context, &holes, parallel);
        let allocate: Vec<_> = stratum
            .members
            .iter()
            .map(|&i| {
                let relation = &context.relations[i];
                let next = &relation.next;
                let types = relation.declaration.columns.iter().map(|column| &column.ty);
                quote!(let mut #next: ::std::vec::Vec<(#(#types,)*)> = ::std::vec::Vec::new();)
            })
            .collect();
        let publish: Vec<_> = stratum
            .members
            .iter()
            .map(|&i| {
                let relation = &context.relations[i];
                let all = &relation.all;
                let next = &relation.next;
                let delta = relation
                    .delta_host
                    .as_ref()
                    .expect("checked IDB delta host");
                let updates: Vec<_> = context
                    .layouts
                    .iter()
                    .filter(|layout| layout.relation == i)
                    .map(|layout| insert_layout(layout, false))
                    .collect();
                // Keep admitted-row maintenance separate from delta-run builds.
                let updates =
                    (!updates.is_empty()).then(|| quote!({ for row in &#next { #(#updates)* } }));
                let delta_runs = context
                    .delta_layouts
                    .iter()
                    .filter(|layout| layout.relation == i)
                    .map(|layout| {
                        let host = &layout.host;
                        let insert = insert_layout(layout, false);
                        // Replace even an empty run: another SCC member may
                        // continue, but must never see this relation's old delta.
                        quote!({
                            #host = ::core::default::Default::default();
                            for row in &#delta { #insert }
                        })
                    });
                quote!(
                    #updates
                    #all.extend(#next.iter().cloned());
                    #delta = #next;
                    #(#delta_runs)*
                )
            })
            .collect();
        let lengths = stratum.members.iter().map(|&i| {
            let delta = context.relations[i]
                .delta_host
                .as_ref()
                .expect("checked IDB delta host");
            quote!(#delta.len())
        });
        let nonempty = quote!(0usize #(+ #lengths)*);
        let observe = context
            .observer
            .as_ref()
            .map(|_| quote!((#observer)(#round, #nonempty);));
        let seed_round = round_body(&allocate, seeds, &publish);
        let step_round = round_body(&allocate, steps, &publish);
        // Keep borrowed lower-stratum key sets local: they cannot outlive this
        // component or delay moving the final output relation into its iterator.
        strata.push(quote!({
            #(#keys)*
            #seed_round
            let mut #round = 0usize;
            while #nonempty != 0 {
                #observe
                #step_round
                #round += 1;
            }
        }));
    }
    let outputs: Vec<_> = context
        .outputs
        .iter()
        .map(|&index| {
            let relation = &context.relations[index];
            let output = &relation.all;
            if relation.declaration.value.is_some() {
                // Layouts, negative projections, and outputs share the same
                // canonical borrowed source. Clone only distinct output values.
                let columns = (0..relation.declaration.arity()).map(|column| {
                    let column = syn::Index::from(column);
                    quote!(::core::clone::Clone::clone(row.#column))
                });
                quote!(::cq_storage::rows::Rows::rows(#output)
                .map(|row| (#(#columns,)*))
                .collect::<::std::vec::Vec<_>>()
                .into_iter())
            } else {
                quote!(#output.into_iter())
            }
        })
        .collect();
    let result = if outputs.len() == 1 {
        outputs[0].clone()
    } else {
        quote!((#(#outputs,)*))
    };
    quote!({ #(#bindings)* #bind_observer #(#strata)* #result })
}

/// Evaluate every hole against published immutable hosts, then admit in written
/// hole and chunk order. Indexed parallel collection preserves the driving scan.
fn evaluate(
    rules: &[RuleHole],
    context: &RoundContext,
    holes: &[TokenStream],
    parallel: bool,
) -> (TokenStream, TokenStream) {
    let mut declarations = Vec::new();
    let mut evaluations = Vec::new();
    let mut admissions = Vec::new();
    let scope = Ident::new("__cq_scope", Span::mixed_site());
    let source_name = Ident::new("__cq_source", Span::mixed_site());
    let evaluator = Ident::new("__cq_evaluate", Span::mixed_site());
    let grain = Ident::new("__cq_grain", Span::mixed_site());
    let bucket = Ident::new("__cq_bucket", Span::mixed_site());
    let admitted_chunk = Ident::new("__cq_admitted_chunk", Span::mixed_site());
    // Timing builds materialize sequential candidates too, so the two phases
    // can be measured independently. Ordinary sequential builds still stream.
    let materialize = parallel || cfg!(feature = "phase-timing");
    for rule in rules {
        let candidates = format_ident!("__cq_candidates_{}", rule.hole, span = Span::mixed_site());
        let invocation = &holes[rule.hole];
        let head = &context.relations[rule.head];
        let types: Vec<_> = head
            .declaration
            .columns
            .iter()
            .map(|column| &column.ty)
            .collect();
        let row = quote!((#(#types,)*));
        let consume = if materialize {
            quote!((#invocation).collect::<::std::vec::Vec<#row>>())
        } else {
            insert_new(&head.seen, &head.next, invocation)
        };
        let evaluation = if let Some(partition) = &rule.partition {
            let chunk = &partition.chunk;
            let (source, whole, relation, columns) = match partition.source {
                PartitionSource::DeltaHost(index) => {
                    let relation = &context.relations[index];
                    let host = relation.delta_host.as_ref().expect("checked delta host");
                    (
                        quote!(#host.as_slice()),
                        quote!(&#host),
                        relation,
                        (0..relation.declaration.arity()).collect::<Vec<_>>(),
                    )
                }
                PartitionSource::LayoutBucket(index) => {
                    let layout = &context.layouts[index];
                    let host = &layout.host;
                    (
                        quote!(#host.get(&()).map_or(&[][..], |#bucket| #bucket.as_slice())),
                        quote!(&#host),
                        &context.relations[layout.relation],
                        layout.columns.clone(),
                    )
                }
            };
            let input_types = columns.iter().map(|&i| &relation.declaration.columns[i].ty);
            if parallel {
                quote!({
                    let #source_name = #source;
                    let #evaluator = |#chunk: &[(#(#input_types,)*)]| { #consume };
                    let #grain = ::core::cmp::max(1024, #source_name.len() / (4 * ::rayon::current_num_threads()));
                    if #source_name.len() <= #grain {
                        ::std::vec![#evaluator(#source_name)]
                    } else {
                        use ::rayon::prelude::*;
                        #source_name.par_chunks(#grain).map(#evaluator).collect::<::std::vec::Vec<_>>()
                    }
                })
            } else {
                // The single whole-source chunk retains its original host kind,
                // so ordinary sequential builds keep the same Rows realization.
                let consume = if materialize {
                    quote!(::std::vec![#consume])
                } else {
                    consume
                };
                quote!({ let #chunk = #whole; #consume })
            }
        } else if materialize {
            quote!(::std::vec![#consume])
        } else {
            quote!({ #consume })
        };
        if parallel {
            declarations.push(quote!(let mut #candidates = ::std::vec::Vec::new();));
            evaluations.push(quote!(#scope.spawn(|_| { #candidates = #evaluation; });));
        } else if materialize {
            evaluations.push(quote!(let #candidates = #evaluation;));
        } else {
            evaluations.push(evaluation);
        }
        if materialize {
            let insert = insert_new(&head.seen, &head.next, &quote!(#admitted_chunk));
            admissions.push(quote!(for #admitted_chunk in #candidates { #insert }));
        }
    }
    let evaluation = if parallel && !rules.is_empty() {
        quote!(#(#declarations)* ::rayon::scope(|#scope| { #(#evaluations)* });)
    } else {
        quote!(#(#evaluations)*)
    };
    (evaluation, quote!(#(#admissions)*))
}

fn round_body(
    allocate: &[TokenStream],
    (evaluate, admit): (TokenStream, TokenStream),
    publish: &[TokenStream],
) -> TokenStream {
    if cfg!(feature = "phase-timing") {
        let phase_start = Ident::new("__cq_phase_start", Span::mixed_site());
        let evaluation_time = Ident::new("__cq_evaluation_time", Span::mixed_site());
        let admission_time = Ident::new("__cq_admission_time", Span::mixed_site());
        quote! {
            #(#allocate)*
            let #phase_start = ::std::time::Instant::now();
            #evaluate
            let #evaluation_time = #phase_start.elapsed();
            let #phase_start = ::std::time::Instant::now();
            #admit
            let #admission_time = #phase_start.elapsed();
            let #phase_start = ::std::time::Instant::now();
            #(#publish)*
            ::std::eprintln!("datalog-phase evaluation_ns={} admission_ns={} publication_ns={}",
                #evaluation_time.as_nanos(), #admission_time.as_nanos(), #phase_start.elapsed().as_nanos());
        }
    } else {
        quote!(#(#allocate)* #evaluate #admit #(#publish)*)
    }
}

// Every full layout is immutable while the round's holes run. Only admitted
// rows reach publication, so no repeated whole-IDB scans or rebuilds occur.
fn insert_layout(layout: &cq_datalog_fork::Layout, borrowed_columns: bool) -> TokenStream {
    let host = &layout.host;
    let columns: Vec<_> = layout
        .columns
        .iter()
        .map(|&i| {
            let index = syn::Index::from(i);
            if borrowed_columns {
                quote!(::core::clone::Clone::clone(row.#index))
            } else {
                quote!(::core::clone::Clone::clone(&row.#index))
            }
        })
        .collect();
    let (keys, values) = columns.split_at(layout.key);
    quote!(#host.entry((#(#keys,)*)).or_default().push((#(#values,)*));)
}

/// Formal rule DL-InsertNew: Canonical all-column membership admits each row with one insertion lookup.
fn insert_new(seen: &Ident, next: &Ident, invocation: &TokenStream) -> TokenStream {
    quote!(for row in #invocation {
        if #seen.insert(::core::clone::Clone::clone(&row)) {
            #next.push(row);
        }
    })
}
