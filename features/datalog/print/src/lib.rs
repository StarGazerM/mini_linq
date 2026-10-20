//! Reusable realization of checked storage, rounds, admission and outputs.
//! Alternative terminals supply only the phase evaluator, never a copy of the
//! storage or publication printer. CQ joins remain inside the recorded queues.
use cq_datalog_support::ensure;
pub use cq_lang_datalog_round as round;
use cq_syntax::rust;
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
pub use round::{
    Execution, Hole, InputSource, KeyRepresentation, Layout, Program, RoundContext, RuleHole,
    Stratum,
};
use syn::Ident;

pub fn splice(program: &Program) -> syn::Result<Vec<TokenStream>> {
    splice_with(program, |hole| {
        hole.pipeline.clone().start(hole.cq.to_token_stream())
    })
}
/// Splice one caller-owned expansion per checked hole, in recorded order.
pub fn splice_with(
    program: &Program,
    mut expand: impl FnMut(&Hole) -> syn::Result<TokenStream>,
) -> syn::Result<Vec<TokenStream>> {
    round::contract::check(program)?;
    program.holes.iter().map(&mut expand).collect()
}
pub fn compile(program: &Program) -> syn::Result<syn::Expr> {
    ensure(
        program.execution.is_empty(),
        program,
        "base printer does not interpret execution certificates",
    )?;
    let timed = program.policy.timing == "phases";
    compile_with(program, |rules, context, holes, _| {
        Ok(evaluate(rules, context, holes, timed))
    })
}
pub fn compile_with(
    program: &Program,
    evaluator: impl FnMut(
        &[RuleHole],
        &RoundContext,
        &[TokenStream],
        &[Execution],
    ) -> syn::Result<(TokenStream, TokenStream)>,
) -> syn::Result<syn::Expr> {
    let holes = splice(program)?;
    render_with(program, &holes, evaluator)
}
/// Realize already-spliced holes. The length and complete Round contract are checked.
pub fn render_with(
    program: &Program,
    holes: &[TokenStream],
    mut evaluator: impl FnMut(
        &[RuleHole],
        &RoundContext,
        &[TokenStream],
        &[Execution],
    ) -> syn::Result<(TokenStream, TokenStream)>,
) -> syn::Result<syn::Expr> {
    round::contract::check(program)?;
    ensure(
        holes.len() == program.holes.len(),
        program,
        "hole splice coverage changed",
    )?;
    let context = &program.context;
    let bindings = bindings(context);
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
        let seeds = evaluator(&stratum.seeds, context, holes, &program.execution)?;
        let steps = evaluator(&stratum.steps, context, holes, &program.execution)?;
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
        let publish = publication(context, stratum);
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
        let seed_round = round_body(
            &allocate,
            seeds,
            &publish,
            program.policy.timing == "phases",
        );
        let step_round = round_body(
            &allocate,
            steps,
            &publish,
            program.policy.timing == "phases",
        );
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
    let result = outputs(context);
    syn::parse2(quote!({ #(#bindings)* #bind_observer #(#strata)* #result }))
}
pub fn bindings(context: &RoundContext) -> Vec<TokenStream> {
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
    bindings
}
pub fn publication(context: &RoundContext, stratum: &Stratum) -> Vec<TokenStream> {
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
    publish
}
pub fn outputs(context: &RoundContext) -> TokenStream {
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
    if outputs.len() == 1 {
        outputs[0].clone()
    } else {
        quote!((#(#outputs,)*))
    }
}
/// Sequential evaluator; whole-host aliases retain the original Rows realization.
pub fn evaluate(
    rules: &[RuleHole],
    context: &RoundContext,
    holes: &[TokenStream],
    materialize: bool,
) -> (TokenStream, TokenStream) {
    let mut evaluations = Vec::new();
    let mut admissions = Vec::new();
    let admitted = Ident::new("__cq_admitted_rows", Span::mixed_site());
    for rule in rules {
        let invocation = &holes[rule.hole];
        let head = &context.relations[rule.head];
        let candidates = format_ident!("__cq_candidates_{}", rule.hole, span = Span::mixed_site());
        let types = head.declaration.columns.iter().map(|c| &c.ty);
        let consume = if materialize {
            quote!((#invocation).collect::<::std::vec::Vec<(#(#types,)*)>>())
        } else {
            insert_new(&head.seen, &head.next, invocation)
        };
        let evaluation = if let Some(scan) = &rule.scan {
            let binding = &scan.binding;
            let host = match scan.source {
                InputSource::Delta(i) => context.relations[i]
                    .delta_host
                    .as_ref()
                    .expect("checked delta host"),
                InputSource::Layout(i) => &context.layouts[i].host,
            };
            let consume = if materialize {
                quote!(::std::vec![#consume])
            } else {
                consume
            };
            quote!({let #binding=&#host;#consume})
        } else if materialize {
            quote!(::std::vec![#consume])
        } else {
            quote!({#consume})
        };
        if materialize {
            evaluations.push(quote!(let #candidates=#evaluation;));
            let insert = insert_new(&head.seen, &head.next, &quote!(#admitted));
            admissions.push(quote!(for #admitted in #candidates {#insert}));
        } else {
            evaluations.push(evaluation);
        }
    }
    (quote!(#(#evaluations)*), quote!(#(#admissions)*))
}
pub fn round_body(
    allocate: &[TokenStream],
    (evaluate, admit): (TokenStream, TokenStream),
    publish: &[TokenStream],
    timing: bool,
) -> TokenStream {
    if timing {
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
pub fn insert_layout(layout: &Layout, borrowed_columns: bool) -> TokenStream {
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
pub fn insert_new(seen: &Ident, next: &Ident, invocation: &TokenStream) -> TokenStream {
    quote!(for row in #invocation {
        if #seen.insert(::core::clone::Clone::clone(&row)) {
            #next.push(row);
        }
    })
}
