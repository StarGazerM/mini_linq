//! Reusable realization of checked storage, rounds, admission and outputs.
//! Alternative terminals supply phase evaluators and alternative publication
//! realizations. CQ joins remain inside the recorded queues.
//! Concrete provider types and initializers come from Storage; Round supplies
//! admission, ordered publication operations, phase/loop control, termination,
//! observer/key-set scope and output conversions. This terminal realizes those
//! checked records rather than selecting a second execution plan.
use cq_datalog_support::ensure;
pub use cq_lang_datalog_round as round;
use cq_syntax::rust;
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
pub use round::{
    Execution, Hole, HoleInterface, InputSource, KeyRepresentation, Layout, Program, RoundContext,
    RuleHole, Stratum,
};
use std::collections::BTreeSet;
use syn::Ident;
mod codegen;

pub fn splice(program: &Program) -> syn::Result<Vec<TokenStream>> {
    splice_with(program, |hole| {
        let mut pipeline = hole.pipeline.clone();
        if hole.interface == HoleInterface::Push {
            let terminal = pipeline.passes.last_mut().expect("checked terminal");
            let arguments = &terminal.arguments;
            let consumer = consumer_name();
            terminal.arguments = quote!(consumer { #consumer }; #arguments);
        }
        pipeline.start(hole.cq.to_token_stream())
    })
}

fn consumer_name() -> Ident {
    Ident::new("__cq_datalog_consumer", Span::mixed_site())
}

/// Formal rule P-DL-Push: Pass each answer directly to the caller's admission consumer.
///
/// Push expansions use the consumer bound here through their terminal's
/// `consumer { ... };` argument. Pull expansions are iterated normally.
/// Neither branch introduces a result buffer.
pub fn consume_hole(
    interface: HoleInterface,
    invocation: &TokenStream,
    consumer: TokenStream,
) -> TokenStream {
    let name = consumer_name();
    match interface {
        HoleInterface::Pull => quote!({
            let mut #name = #consumer;
            for row in #invocation { (#name)(row); }
        }),
        HoleInterface::Push => quote!({
            let #name = #consumer;
            #invocation;
        }),
    }
}

/// Collect directly into the requested provider, without an intermediate buffer.
/// Push providers implement `Default + Extend<Row>`; pull retains `FromIterator`.
pub fn collect_hole(
    interface: HoleInterface,
    invocation: &TokenStream,
    provider: TokenStream,
) -> TokenStream {
    match interface {
        HoleInterface::Pull => quote!((#invocation).collect::<#provider>()),
        HoleInterface::Push => {
            let rows = Ident::new("__cq_collected_rows", Span::mixed_site());
            let consume = consume_hole(
                interface,
                invocation,
                quote!(|row| {
                    #rows.extend(::core::iter::once(row));
                }),
            );
            quote!({
                let mut #rows: #provider = ::core::default::Default::default();
                #consume;
                #rows
            })
        }
    }
}
/// Splice one caller-owned expansion per checked hole, in recorded order.
/// Push callbacks must emit an expression consuming the terminal's
/// `consumer { ... };` argument, as the ordinary [`splice`] implementation does.
pub fn splice_with(
    program: &Program,
    expand: impl FnMut(&Hole) -> syn::Result<TokenStream>,
) -> syn::Result<Vec<TokenStream>> {
    round::contract::check(program)?;
    splice_admitted(program, expand)
}
fn splice_admitted(
    program: &Program,
    mut expand: impl FnMut(&Hole) -> syn::Result<TokenStream>,
) -> syn::Result<Vec<TokenStream>> {
    ensure(
        program.storage.policy.insertion == "all_columns",
        program,
        "CPU Datalog printer requires all_columns insertion",
    )?;
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
        Ok(evaluate(rules, context, holes, &program.holes, timed))
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
    render_admitted(program, &holes, evaluator, publication)
}
/// Realize already-spliced holes. The length and complete Round contract are checked.
pub fn render_with(
    program: &Program,
    holes: &[TokenStream],
    evaluator: impl FnMut(
        &[RuleHole],
        &RoundContext,
        &[TokenStream],
        &[Execution],
    ) -> syn::Result<(TokenStream, TokenStream)>,
) -> syn::Result<syn::Expr> {
    render_with_publication(program, holes, evaluator, publication)
}
/// Realize checked holes with alternative publication realizations.
pub fn render_with_publication(
    program: &Program,
    holes: &[TokenStream],
    evaluator: impl FnMut(
        &[RuleHole],
        &RoundContext,
        &[TokenStream],
        &[Execution],
    ) -> syn::Result<(TokenStream, TokenStream)>,
    publish: impl FnMut(&RoundContext, &Stratum) -> Vec<TokenStream>,
) -> syn::Result<syn::Expr> {
    round::contract::check(program)?;
    render_admitted(program, holes, evaluator, publish)
}
// Internal callers retain the Round admission performed before splicing.
fn render_admitted(
    program: &Program,
    holes: &[TokenStream],
    mut evaluator: impl FnMut(
        &[RuleHole],
        &RoundContext,
        &[TokenStream],
        &[Execution],
    ) -> syn::Result<(TokenStream, TokenStream)>,
    mut publish: impl FnMut(&RoundContext, &Stratum) -> Vec<TokenStream>,
) -> syn::Result<syn::Expr> {
    ensure(
        program.storage.policy.insertion == "all_columns",
        program,
        "CPU Datalog printer requires all_columns insertion",
    )?;
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
    for (stratum_index, stratum) in context.strata.iter().enumerate() {
        let keys = stratum.keys.iter().map(|key| {
            let name = &key.name;
            let invocation = &holes[key.hole];
            let provider = &key.provider;
            let collection = collect_hole(
                program.holes[key.hole].interface,
                invocation,
                quote!(#provider),
            );
            match key.representation {
                KeyRepresentation::Borrowed => quote!(let #name = &#collection;),
                KeyRepresentation::DeviceSetOwned => {
                    unreachable!("CPU printer rejects device policy")
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
                let provider = &relation.providers.next;
                let init = &relation.providers.next_init;
                quote!(let mut #next: #provider = #init;)
            })
            .collect();
        let publish = publish(context, stratum);
        let lengths = stratum.termination.iter().map(|&i| {
            let delta = context.relations[i]
                .delta_host
                .as_ref()
                .expect("checked IDB delta host");
            quote!(#delta.len())
        });
        let nonempty = quote!(0usize #(+ #lengths)*);
        let condition = match stratum.termination_test.to_string().as_str() {
            "sum_delta_lengths" => quote!(#nonempty != 0),
            "any_delta" => {
                let tests = stratum.termination.iter().map(|&index| {
                    let delta = context.relations[index]
                        .delta_host
                        .as_ref()
                        .expect("checked IDB delta host");
                    quote!(!#delta.is_empty())
                });
                quote!(false #(|| #tests)*)
            }
            _ => unreachable!("checked termination test"),
        };
        let observe = match stratum.observer.to_string().as_str() {
            "before_step" => context
                .observer
                .as_ref()
                .map(|_| quote!((#observer)(#round, #nonempty);)),
            _ => unreachable!("checked observer placement"),
        };
        let seed_round = round_body(
            &allocate,
            seeds,
            &publish,
            &stratum.phases,
            program.policy.timing == "phases",
        );
        let step_round = round_body(
            &allocate,
            steps,
            &publish,
            &stratum.phases,
            program.policy.timing == "phases",
        );
        let loop_body = match stratum.loop_form.to_string().as_str() {
            "while_nonempty" => quote!(
                while #condition {
                    #observe
                    #step_round
                    #round += 1;
                }
            ),
            "loop_until_empty" => quote!(
                loop {
                    if !(#condition) { break; }
                    #observe
                    #step_round
                    #round += 1;
                }
            ),
            _ => unreachable!("checked loop form"),
        };
        match stratum.key_scope.to_string().as_str() {
            "component" => {
                let body = quote!({
                    #(#keys)*
                    #seed_round
                    let mut #round = 0usize;
                    #loop_body
                });
                let mut mutable = BTreeSet::new();
                let mut locals = BTreeSet::new();
                for &index in &stratum.members {
                    let relation = &context.relations[index];
                    mutable.insert(relation.all.to_string());
                    mutable.insert(relation.seen.to_string());
                    mutable.extend(relation.delta_host.iter().map(ToString::to_string));
                    locals.insert(relation.next.to_string());
                }
                for transaction in &stratum.publication {
                    for operation in &transaction.operations {
                        let layouts = if operation.kind == "delta_rebuild" {
                            &context.delta_layouts
                        } else {
                            &context.layouts
                        };
                        mutable.extend(
                            operation
                                .layouts
                                .iter()
                                .map(|&i| layouts[i].host.to_string()),
                        );
                    }
                }
                strata.push(codegen::split(
                    body,
                    context,
                    program.holes.len() + stratum_index,
                    &mutable,
                    &locals,
                ));
            }
            _ => unreachable!("checked key-set scope"),
        }
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
        let providers = &relation.providers;
        if let Some(value) = &declaration.value {
            let expression = &value.expression;
            let canonical = &providers.canonical_name;
            let set = &providers.canonical_set;
            // Keep the original host alive, including temporary and grouped hosts.
            // Rows already supplies Copy tuples of borrowed columns: hash those
            // tuples once, without cloning values or adding another reference layer.
            bindings.push(quote!(
                let #all = &(#expression);
                let #all = &{
                    struct #canonical<R>(#set);
                    impl<R: ::core::marker::Copy> ::cq_storage::rows::Rows for &#canonical<R> {
                        type Row = R;
                        fn rows(self) -> impl ::core::iter::Iterator<Item = R> {
                            self.0.iter().copied()
                        }
                    }
                    #canonical(::cq_storage::rows::Rows::rows(#all).collect())
                };
            ));
        } else {
            let seen = &relation.seen;
            let all_provider = &providers.all;
            let all_init = &providers.all_init;
            let seen_provider = &providers.seen;
            bindings.push(quote!(
                let mut #all: #all_provider = #all_init;
                let mut #seen: #seen_provider = ::core::default::Default::default();
            ));
        }
        if let Some(delta) = delta {
            let provider = &providers.delta;
            let init = &providers.delta_init;
            bindings.push(quote!(let mut #delta: #provider = #init;));
        }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        let relation = &context.relations[layout.relation];
        let host = &layout.host;
        let provider = &layout.provider;
        bindings.push(quote!(
            let mut #host: #provider
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
    stratum
        .publication
        .iter()
        .map(|publication| {
            let relation = &context.relations[publication.relation];
            let delta = relation
                .delta_host
                .as_ref()
                .expect("checked IDB delta host");
            let operations = publication.operations.iter().map(|operation| {
                match operation.kind.to_string().as_str() {
                    "full_layouts" => full_insert(
                        operation
                            .layouts
                            .iter()
                            .map(|&index| &context.layouts[index]),
                        &relation.next,
                    ),
                    "all_extend" => all_extend(relation),
                    "delta_swap" => delta_swap(relation),
                    "delta_rebuild" => {
                        let rebuilds = operation
                            .layouts
                            .iter()
                            .map(|&index| delta_rebuild(&context.delta_layouts[index], delta));
                        quote!(#(#rebuilds)*)
                    }
                    _ => unreachable!("checked publication operation"),
                }
            });
            quote!(#(#operations)*)
        })
        .collect()
}
/// Insert admitted rows into the supplied full layouts in one shared row loop.
pub fn full_insert<'a>(layouts: impl IntoIterator<Item = &'a Layout>, next: &Ident) -> TokenStream {
    let updates = layouts
        .into_iter()
        .map(|layout| insert_layout(layout, false));
    quote!({ for row in &#next { #(#updates)* } })
}
/// Extend a relation's accumulated rows with its admitted rows.
pub fn all_extend(relation: &round::Relation) -> TokenStream {
    let all = &relation.all;
    let next = &relation.next;
    quote!(#all.extend(#next.iter().cloned());)
}
/// Publish admitted rows as the relation's next delta.
pub fn delta_swap(relation: &round::Relation) -> TokenStream {
    let delta = relation
        .delta_host
        .as_ref()
        .expect("checked IDB delta host");
    let next = &relation.next;
    quote!(#delta = #next;)
}
/// Replace a delta layout with its default host, then populate the new delta.
pub fn delta_rebuild(layout: &Layout, delta: &Ident) -> TokenStream {
    let host = &layout.host;
    let insert = insert_layout(layout, false);
    quote!({
        #host = ::core::default::Default::default();
        for row in &#delta { #insert }
    })
}
pub fn outputs(context: &RoundContext) -> TokenStream {
    let outputs: Vec<_> = context
        .outputs
        .iter()
        .map(|conversion| {
            let relation = &context.relations[conversion.relation];
            let output = &relation.all;
            match conversion.conversion.to_string().as_str() {
                "borrowed_cloned_into_iter" => {
                    let columns = (0..relation.declaration.arity()).map(|column| {
                        let column = syn::Index::from(column);
                        quote!(::core::clone::Clone::clone(row.#column))
                    });
                    quote!(::cq_storage::rows::Rows::rows(#output)
                .map(|row| (#(#columns,)*))
                .collect::<::std::vec::Vec<_>>()
                .into_iter())
                }
                "owned_into_iter" => quote!(#output.into_iter()),
                _ => unreachable!("checked output conversion"),
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
    specifications: &[Hole],
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
            collect_hole(
                specifications[rule.hole].interface,
                invocation,
                quote!(::std::vec::Vec<(#(#types,)*)>),
            )
        } else if specifications[rule.hole].interface == HoleInterface::Push {
            let seen = &head.seen;
            let next = &head.next;
            let insertion = &rule.insertion;
            consume_hole(
                HoleInterface::Push,
                invocation,
                quote!(|row: (#(#types,)*)| {
                    if #seen.#insertion(::core::clone::Clone::clone(&row)) {
                        #next.push(row);
                    }
                }),
            )
        } else {
            insert_new(rule, context, invocation)
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
        let evaluation = split_evaluation(rule, context, evaluation);
        if materialize {
            evaluations.push(quote!(let #candidates=#evaluation;));
            let insert = insert_new(rule, context, &quote!(#admitted));
            admissions.push(quote!(for #admitted in #candidates {#insert}));
        } else {
            evaluations.push(evaluation);
        }
    }
    (quote!(#(#evaluations)*), quote!(#(#admissions)*))
}

/// Give a hole's consumed evaluation its own codegen partition without moving
/// captured Rust expressions out of their lexical scope.
pub fn split_evaluation(rule: &RuleHole, context: &RoundContext, body: TokenStream) -> TokenStream {
    let head = &context.relations[rule.head];
    codegen::split(
        body,
        context,
        rule.hole,
        &[head.seen.to_string(), head.next.to_string()]
            .into_iter()
            .collect(),
        &BTreeSet::new(),
    )
}
pub fn round_body(
    allocate: &[TokenStream],
    (evaluate, admit): (TokenStream, TokenStream),
    publish: &[TokenStream],
    phases: &[Ident],
    timing: bool,
) -> TokenStream {
    let phase_start = Ident::new("__cq_phase_start", Span::mixed_site());
    let evaluation_time = Ident::new("__cq_evaluation_time", Span::mixed_site());
    let admission_time = Ident::new("__cq_admission_time", Span::mixed_site());
    let bodies = phases.iter().map(|phase| {
        let (body, elapsed) = match phase.to_string().as_str() {
            "evaluate" => (evaluate.clone(), quote!(let #evaluation_time = #phase_start.elapsed();)),
            "admit" => (admit.clone(), quote!(let #admission_time = #phase_start.elapsed();)),
            "publish" => (quote!(#(#publish)*), quote!(
                ::std::eprintln!("datalog-phase evaluation_ns={} admission_ns={} publication_ns={}",
                    #evaluation_time.as_nanos(), #admission_time.as_nanos(), #phase_start.elapsed().as_nanos());
            )),
            _ => unreachable!("checked round phase"),
        };
        if timing {
            quote!(let #phase_start = ::std::time::Instant::now(); #body #elapsed)
        } else {
            body
        }
    });
    quote!(#(#allocate)* #(#bodies)*)
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
pub fn insert_new(
    rule: &RuleHole,
    context: &RoundContext,
    invocation: &TokenStream,
) -> TokenStream {
    let head = &context.relations[rule.head];
    let seen = &head.seen;
    let next = &head.next;
    let insertion = &rule.insertion;
    quote!(for row in #invocation {
        if #seen.#insertion(::core::clone::Clone::clone(&row)) {
            #next.push(row);
        }
    })
}
