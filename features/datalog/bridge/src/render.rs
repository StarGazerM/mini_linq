//! Rust realization of the checked Datalog round context. CQ holes have already
//! been expanded by the caller's continuation; this bridge alone owns loops.

use cq_datalog_fork::{ExpandedFork, KeyRepresentation};
use cq_syntax::rust;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

pub fn render(fork: ExpandedFork) -> TokenStream {
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
    for layout in &context.layouts {
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
        let emit = |rule: &cq_datalog_fork::RuleHole| {
            let relation = &context.relations[rule.head];
            insert_new(&relation.seen, &relation.next, &holes[rule.hole])
        };
        let seeds: Vec<_> = stratum.seeds.iter().map(emit).collect();
        let steps: Vec<_> = stratum.steps.iter().map(emit).collect();
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
                let updates =
                    (!updates.is_empty()).then(|| quote!(for row in &#next { #(#updates)* }));
                quote!(
                    #updates
                    #all.extend(#next.iter().cloned());
                    #delta = #next;
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
        // Keep borrowed lower-stratum key sets local: they cannot outlive this
        // component or delay moving the final output relation into its iterator.
        strata.push(quote!({
            #(#keys)*
            #(#allocate)* #(#seeds)* #(#publish)*
            let mut #round = 0usize;
            while #nonempty != 0 {
                #observe
                #(#allocate)* #(#steps)* #(#publish)*
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
