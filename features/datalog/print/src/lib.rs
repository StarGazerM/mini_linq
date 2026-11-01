//! Reusable realization of checked storage, rounds, admission and outputs.
//! Alternative terminals supply phase evaluators and alternative publication
//! realizations. CQ joins remain inside the recorded queues.
//! Concrete provider types and initializers come from Storage; Round supplies
//! admission, ordered publication operations, phase/loop control, termination,
//! observer/key-set scope and output conversions. This terminal realizes those
//! checked records rather than selecting a second execution plan.
//! Evaluators receive each phase's holes in flattened schedule-wave order.
//! Sequential admission remains inline (or in the timed admission phase);
//! publication still occurs only after the complete phase has evaluated.
//!
//! A hole's CQ input is fact-free, so its first invocation has the empty context.
//! Round records discharged facts in the checked lower-owner argument packet;
//! the core protocol introduces them there and emits their exact `context`
//! envelope on every subsequent factful continuation, including key holes.
use cq_datalog_support::ensure;
pub use cq_lang_datalog_round as round;
use cq_syntax::rust;
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
pub use round::{
    Execution, Hole, HoleInterface, InputSource, Layout, Program, RoundContext, RuleHole, Stratum,
};
use std::collections::BTreeSet;
use syn::Ident;
mod codegen;

/// Host realization supports iterator and consumer holes, not device batches.
pub fn check_host_interfaces(program: &Program) -> syn::Result<()> {
    for hole in &program.holes {
        ensure(
            hole.interface != HoleInterface::Batch,
            hole,
            "host Datalog printers do not support batch holes; use a batch-capable printer",
        )?;
    }
    // DL-WaveLanes realize device streams; host printers have their own
    // wave concurrency and must not silently drop a recorded lane split.
    ensure(
        program.schedule.strata.iter().all(|stratum| stratum.lanes.is_empty()),
        &program.schedule,
        "host Datalog printers do not realize wave lanes; use a device printer",
    )?;
    Ok(())
}

pub fn splice(program: &Program) -> syn::Result<Vec<TokenStream>> {
    check_host_interfaces(program)?;
    splice_with(program, |hole| {
        let mut pipeline = hole.pipeline.clone();
        if hole.interface == HoleInterface::Push {
            let terminal = pipeline.passes.last_mut().expect("checked terminal");
            let arguments = &terminal.arguments;
            let consumer = consumer_name();
            terminal.arguments = quote!(consumer { #consumer }; #arguments);
        }
        // Do not attach the hole's eventual facts to its fact-free CQ source.
        // The recorded lower packet introduces them at the authorized boundary.
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
        HoleInterface::Batch => quote!(compile_error!(
            "host Datalog printers do not support batch holes"
        )),
    }
}

/// Collect directly into the requested provider, without an intermediate buffer.
/// Push providers implement `Default + Extend<Row>`; pull retains `FromIterator`.
/// `owned` converts each answer before collection (see [`owned_answer`]).
pub fn collect_hole(
    interface: HoleInterface,
    invocation: &TokenStream,
    provider: TokenStream,
    owned: Option<TokenStream>,
) -> TokenStream {
    match interface {
        HoleInterface::Pull => match owned {
            Some(owned) => quote!(
                ::core::iter::Iterator::map(#invocation, #owned).collect::<#provider>()
            ),
            None => quote!((#invocation).collect::<#provider>()),
        },
        HoleInterface::Push => {
            let rows = Ident::new("__cq_collected_rows", Span::mixed_site());
            let row = owned.map_or_else(|| quote!(row), |owned| quote!((#owned)(row)));
            let consume = consume_hole(
                interface,
                invocation,
                quote!(|row| {
                    #rows.extend(::core::iter::once(#row));
                }),
            );
            quote!({
                let mut #rows: #provider = ::core::default::Default::default();
                #consume;
                #rows
            })
        }
        HoleInterface::Batch => quote!(compile_error!(
            "host Datalog printers do not support batch holes"
        )),
    }
}

/// The answer row type of a hole: `&T` for borrowed columns, `T` for owned.
pub fn answer_types(
    declaration: &cq_syntax::RelationDecl,
    answer: &[round::AnswerColumn],
) -> Vec<TokenStream> {
    declaration
        .columns
        .iter()
        .zip(answer)
        .map(|(column, ownership)| {
            let ty = &column.ty;
            match ownership {
                round::AnswerColumn::Borrowed => quote!(&#ty),
                round::AnswerColumn::Owned => quote!(#ty),
            }
        })
        .collect()
}

/// Formal rule DL-AnswerOwnership: a closure from an answer row to its owned
/// relation row. Borrowed columns are cloned and owned columns move; `None`
/// when every column is already owned, so the answer itself is the row.
pub fn owned_answer(
    declaration: &cq_syntax::RelationDecl,
    answer: &[round::AnswerColumn],
) -> Option<TokenStream> {
    answer.contains(&round::AnswerColumn::Borrowed).then(|| {
        let types = answer_types(declaration, answer);
        let columns = answer.iter().enumerate().map(|(index, column)| {
            let index = syn::Index::from(index);
            match column {
                round::AnswerColumn::Borrowed => quote!(::core::clone::Clone::clone(row.#index)),
                round::AnswerColumn::Owned => quote!(row.#index),
            }
        });
        quote!((|row: (#(#types,)*)| (#(#columns,)*)))
    })
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
    render_admitted(program, &holes, evaluator, publication, |_, _, body| {
        Ok(body)
    })
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
/// The evaluator's rules follow the recorded waves, preserving each wave's
/// hole order. Context and splice indices retain their original identities.
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
    render_with_step_loop(program, holes, evaluator, publish, |_, _, body| Ok(body))
}
/// Replace only the step loop, retaining synchronous seed publication and outputs.
pub fn render_with_step_loop(
    program: &Program,
    holes: &[TokenStream],
    evaluator: impl FnMut(
        &[RuleHole],
        &RoundContext,
        &[TokenStream],
        &[Execution],
    ) -> syn::Result<(TokenStream, TokenStream)>,
    publish: impl FnMut(&RoundContext, &Stratum) -> Vec<TokenStream>,
    step_loop: impl FnMut(usize, &Stratum, TokenStream) -> syn::Result<TokenStream>,
) -> syn::Result<syn::Expr> {
    round::contract::check(program)?;
    render_admitted(program, holes, evaluator, publish, step_loop)
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
    mut step_loop: impl FnMut(usize, &Stratum, TokenStream) -> syn::Result<TokenStream>,
) -> syn::Result<syn::Expr> {
    check_host_interfaces(program)?;
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
                None,
            );
            quote!(let #name = &#collection;)
        });
        let scheduled_rules = |phase: &str, rules: &[RuleHole]| {
            program.schedule.strata[stratum_index]
                .waves
                .iter()
                .filter(|wave| wave.phase == phase)
                .flat_map(|wave| wave.holes.iter().map(|&index| rules[index].clone()))
                .collect::<Vec<_>>()
        };
        let seeds = evaluator(
            &scheduled_rules(round::schedule::SEED, &stratum.seeds),
            context,
            holes,
            &program.execution,
        )?;
        let steps = evaluator(
            &scheduled_rules(round::schedule::STEP, &stratum.steps),
            context,
            holes,
            &program.execution,
        )?;
        let allocate: Vec<_> = stratum
            .members
            .iter()
            .map(|&i| {
                let relation = &context.relations[i];
                let next = &relation.next;
                let provider = &relation.providers.next;
                let init = &relation.providers.next_init;
                let next_canonical = &relation.next_canonical;
                let canonical = &context.layouts[relation.providers.canonical].provider;
                quote!(
                    let mut #next: #provider = #init;
                    // Sorted-run admission builds NEW as a sorted vector instead.
                    #[allow(unused_mut, unused_variables)]
                    let mut #next_canonical: #canonical = ::core::default::Default::default();
                )
            })
            .collect();
        let fresh = {
            let lengths = stratum
                .members
                .iter()
                .map(|&i| &context.relations[i].next)
                .map(|next| quote!(#next.len()));
            quote!(0usize #(+ #lengths)*)
        };
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
            &fresh,
        );
        let step_round = round_body(
            &allocate,
            steps,
            &publish,
            &stratum.phases,
            program.policy.timing == "phases",
            &fresh,
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
        let loop_body = step_loop(stratum_index, stratum, loop_body)?;
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
                    mutable.extend(relation.delta_host.iter().map(ToString::to_string));
                    locals.insert(relation.next.to_string());
                    locals.insert(relation.next_canonical.to_string());
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
        if let Some(delta) = &relation.delta_host {
            let provider = &relation.providers.delta;
            let init = &relation.providers.delta_init;
            bindings.push(quote!(let mut #delta: #provider = #init;));
        }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        let host = &layout.host;
        let provider = &layout.provider;
        bindings.push(quote!(
            let mut #host: #provider
                = ::core::default::Default::default();
        ));
    }
    // EDB loading is one admission: each input row new to the canonical layout
    // extends every full layout. Delta layouts name IDBs, which start empty.
    for (index, relation) in context.relations.iter().enumerate() {
        let Some(value) = &relation.declaration.value else {
            continue;
        };
        let expression = &value.expression;
        let canonical = &context.layouts[relation.providers.canonical];
        let inserts = context
            .layouts
            .iter()
            .enumerate()
            .filter(|&(position, layout)| {
                layout.relation == index && position != relation.providers.canonical
            })
            .map(|(_, layout)| insert_layout(layout, true));
        // `Rows::rows` yields every input column borrowed.
        let borrowed = vec![round::AnswerColumn::Borrowed; relation.declaration.arity()];
        let admit = admit_canonical(
            canonical,
            None,
            &canonical.host,
            &borrowed,
            quote!(#(#inserts)*),
        );
        let input = Ident::new("__cq_input", Span::mixed_site());
        // Keep the original host alive, including temporary and grouped hosts.
        bindings.push(quote!({
            let #input = &(#expression);
            for row in ::cq_storage::rows::Rows::rows(#input) { #admit }
        }));
    }
    bindings
}
/// Formal rule DL-InsertNew: a row is new iff its all-column key is absent from
/// the committed canonical layout and first inserted into `target`, the round's
/// NEW version of that layout (or the canonical layout itself while loading EDBs).
/// `then` runs exactly once per new row; `row` is the candidate in scope, with
/// relation-order columns represented as `columns` records (DL-AnswerOwnership):
/// the owned key clones each column from its representation.
pub fn admit_canonical(
    canonical: &Layout,
    committed: Option<&Ident>,
    target: &Ident,
    columns: &[round::AnswerColumn],
    then: TokenStream,
) -> TokenStream {
    let key = Ident::new("__cq_key", Span::mixed_site());
    let identity = !columns.contains(&round::AnswerColumn::Borrowed)
        && canonical
            .columns
            .iter()
            .copied()
            .eq(0..canonical.columns.len());
    let (bind, probe, owned) = if identity {
        (
            TokenStream::new(),
            quote!(&row),
            quote!(::core::clone::Clone::clone(&row)),
        )
    } else {
        let columns = layout_columns(canonical, columns);
        (
            quote!(let #key = (#(#columns,)*);),
            quote!(&#key),
            quote!(#key),
        )
    };
    match canonical.protocol {
        round::storage::LayoutProtocol::Bulk => {
            let committed = committed.map(|committed| {
                quote!(!::cq_storage::ExactMembership::contains(&#committed, #probe) &&)
            });
            quote!({
                #bind
                if #committed !::cq_storage::ExactMembership::contains(&#target, #probe) {
                    #target.push(#owned, ());
                    #then
                }
            })
        }
        // The set's own insertion reports whether the key was absent.
        round::storage::LayoutProtocol::Set => {
            let committed = committed.map(|committed| quote!(!#committed.contains(#probe) &&));
            quote!({
                #bind
                if #committed #target.insert(#owned) {
                    #then
                }
            })
        }
        round::storage::LayoutProtocol::GroupedMap => {
            let bucket = Ident::new("__cq_bucket", Span::mixed_site());
            let insert = quote!({
                let #bucket = #target.entry(#owned).or_default();
                if #bucket.is_empty() {
                    #bucket.push(());
                    #then
                }
            });
            match committed {
                Some(committed) => quote!({ #bind if !#committed.contains_key(#probe) #insert }),
                None => quote!({ #bind #insert }),
            }
        }
    }
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
/// Insert admitted rows with one shared loop of map/set insertions and one bulk
/// call per bulk layout.
pub fn full_insert<'a>(layouts: impl IntoIterator<Item = &'a Layout>, next: &Ident) -> TokenStream {
    let mut updates = Vec::new();
    let mut bulk = Vec::new();
    for layout in layouts {
        if layout.protocol == round::storage::LayoutProtocol::Bulk {
            bulk.push(extend_layout(layout, next));
        } else {
            updates.push(insert_layout(layout, false));
        }
    }
    let rows = if updates.is_empty() && !bulk.is_empty() {
        TokenStream::new()
    } else {
        quote!(for row in &#next { #(#updates)* })
    };
    quote!({ #rows #(#bulk)* })
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
/// Replace a delta's rows, retaining bulk layout host capacity across rounds.
pub fn delta_rebuild(layout: &Layout, delta: &Ident) -> TokenStream {
    let host = &layout.host;
    if layout.protocol == round::storage::LayoutProtocol::Bulk {
        let extend = extend_layout(layout, delta);
        return quote!({
            #host.clear();
            #extend
        });
    }
    let insert = insert_layout(layout, false);
    quote!({
        #host = ::core::default::Default::default();
        for row in &#delta { #insert }
    })
}
/// Return each output relation's rows from its canonical layout in declaration order.
pub fn outputs(context: &RoundContext) -> TokenStream {
    let outputs: Vec<_> = context
        .outputs
        .iter()
        .map(|conversion| {
            let relation = &context.relations[conversion.relation];
            let canonical = &context.layouts[relation.providers.canonical];
            let output = &canonical.host;
            let position = |column: usize| {
                syn::Index::from(
                    canonical
                        .columns
                        .iter()
                        .position(|&c| c == column)
                        .expect("checked complete canonical permutation"),
                )
            };
            match conversion.conversion.to_string().as_str() {
                "borrowed_cloned_into_iter" => {
                    let columns = (0..relation.declaration.arity()).map(|column| {
                        let column = position(column);
                        quote!(::core::clone::Clone::clone(row.#column))
                    });
                    quote!(::cq_storage::rows::Rows::rows(&#output)
                .map(|row| (#(#columns,)*))
                .collect::<::std::vec::Vec<_>>()
                .into_iter())
                }
                "owned_into_iter" => {
                    let columns = (0..relation.declaration.arity()).map(|column| {
                        let column = position(column);
                        quote!(key.#column)
                    });
                    // A set yields its keys; a grouped map yields `(key, bucket)`.
                    let pattern = if canonical.protocol == round::storage::LayoutProtocol::Set {
                        quote!(key)
                    } else {
                        quote!((key, _))
                    };
                    quote!(#output
                .into_iter()
                .map(|#pattern| (#(#columns,)*))
                .collect::<::std::vec::Vec<_>>()
                .into_iter())
                }
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
                owned_answer(&head.declaration, &specifications[rule.hole].answer),
            )
        } else if specifications[rule.hole].interface == HoleInterface::Push {
            let admit = admit_row(head, context, &specifications[rule.hole].answer, rule.known);
            let types = answer_types(&head.declaration, &specifications[rule.hole].answer);
            consume_hole(
                HoleInterface::Push,
                invocation,
                quote!(|row: (#(#types,)*)| #admit),
            )
        } else {
            insert_new(rule, context, invocation, &specifications[rule.hole].answer)
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
            // Materialized candidates are owned rows.
            let owned = vec![round::AnswerColumn::Owned; head.declaration.arity()];
            let insert = insert_new(rule, context, &quote!(#admitted), &owned);
            let count = candidate_count();
            admissions.push(quote!(
                #count.fetch_add(
                    #candidates.iter().map(::std::vec::Vec::len).sum::<usize>(),
                    ::core::sync::atomic::Ordering::Relaxed,
                );
                for #admitted in #candidates {#insert}
            ));
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
        &[head.next_canonical.to_string(), head.next.to_string()]
            .into_iter()
            .collect(),
        &BTreeSet::new(),
    )
}
/// Candidate rows reaching admission in one timed round, before deduplication.
/// Atomic so parallel admission tasks may add their counts.
pub fn candidate_count() -> Ident {
    Ident::new("__cq_candidate_count", Span::mixed_site())
}
/// `fresh` sums the members' admitted rows after admission, before publication.
pub fn round_body(
    allocate: &[TokenStream],
    (evaluate, admit): (TokenStream, TokenStream),
    publish: &[TokenStream],
    phases: &[Ident],
    timing: bool,
    fresh: &TokenStream,
) -> TokenStream {
    let phase_start = Ident::new("__cq_phase_start", Span::mixed_site());
    let evaluation_time = Ident::new("__cq_evaluation_time", Span::mixed_site());
    let admission_time = Ident::new("__cq_admission_time", Span::mixed_site());
    let fresh_count = Ident::new("__cq_fresh_count", Span::mixed_site());
    let candidates = candidate_count();
    let bodies = phases.iter().map(|phase| {
        let (body, elapsed) = match phase.to_string().as_str() {
            "evaluate" => (evaluate.clone(), quote!(let #evaluation_time = #phase_start.elapsed();)),
            "admit" => (admit.clone(), quote!(
                let #admission_time = #phase_start.elapsed();
                let #fresh_count: usize = #fresh;
            )),
            "publish" => (quote!(#(#publish)*), quote!(
                ::std::eprintln!("datalog-phase evaluation_ns={} admission_ns={} publication_ns={} candidates={} fresh={}",
                    #evaluation_time.as_nanos(), #admission_time.as_nanos(), #phase_start.elapsed().as_nanos(),
                    #candidates.load(::core::sync::atomic::Ordering::Relaxed), #fresh_count);
            )),
            _ => unreachable!("checked round phase"),
        };
        if timing {
            quote!(let #phase_start = ::std::time::Instant::now(); #body #elapsed)
        } else {
            body
        }
    });
    let count =
        timing.then(|| quote!(let #candidates = ::core::sync::atomic::AtomicUsize::new(0);));
    quote!(#(#allocate)* #count #(#bodies)*)
}

/// Owned layout-order key columns of `row`; `columns` gives each relation
/// column's representation in `row`.
fn layout_columns(layout: &Layout, columns: &[round::AnswerColumn]) -> Vec<TokenStream> {
    layout
        .columns
        .iter()
        .map(|&i| {
            let index = syn::Index::from(i);
            match columns[i] {
                round::AnswerColumn::Borrowed => quote!(::core::clone::Clone::clone(row.#index)),
                round::AnswerColumn::Owned => quote!(::core::clone::Clone::clone(&row.#index)),
            }
        })
        .collect()
}

/// One representation for every relation column a layout reads.
fn uniform(layout: &Layout, column: round::AnswerColumn) -> Vec<round::AnswerColumn> {
    vec![column; layout.columns.iter().max().map_or(0, |&last| last + 1)]
}

fn extend_layout(layout: &Layout, rows: &Ident) -> TokenStream {
    let host = &layout.host;
    let columns = layout_columns(layout, &uniform(layout, round::AnswerColumn::Owned));
    let (keys, values) = columns.split_at(layout.key);
    quote!(#host.extend_from(&#rows, |row| (#(#keys,)*), |row| (#(#values,)*));)
}

// Every full layout is immutable while the round's holes run. Only admitted
// rows reach publication, so no repeated whole-IDB scans or rebuilds occur.
// EDB bindings use maps and sets; sharded and sorted-run IDBs use bulk publication above.
pub fn insert_layout(layout: &Layout, borrowed_columns: bool) -> TokenStream {
    let host = &layout.host;
    let column = if borrowed_columns {
        round::AnswerColumn::Borrowed
    } else {
        round::AnswerColumn::Owned
    };
    let columns = layout_columns(layout, &uniform(layout, column));
    let (keys, values) = columns.split_at(layout.key);
    match layout.protocol {
        round::storage::LayoutProtocol::Bulk => {
            quote!(#host.push((#(#keys,)*), (#(#values,)*));)
        }
        round::storage::LayoutProtocol::Set => quote!(#host.insert((#(#keys,)*));),
        round::storage::LayoutProtocol::GroupedMap => {
            quote!(#host.entry((#(#keys,)*)).or_default().push((#(#values,)*));)
        }
    }
}

/// Admit every candidate row of one hole into its head relation; `answer`
/// is the hole's recorded answer ownership (DL-AnswerOwnership).
pub fn insert_new(
    rule: &RuleHole,
    context: &RoundContext,
    invocation: &TokenStream,
    answer: &[round::AnswerColumn],
) -> TokenStream {
    let admit = admit_row(&context.relations[rule.head], context, answer, rule.known);
    quote!(for row in #invocation #admit)
}
/// Admit `row` into the round's NEW version of the canonical layout, pushing
/// each new row once to `next`. An `admit` hole is also anti-joined with the
/// committed canonical layout here; an `in_plan` hole already discharged that
/// read in its plan (DL-AdmitOnce), so only the round-local δ remains.
/// Borrowed answer columns form the owned NEW row only for admitted candidates.
pub fn admit_row(
    relation: &round::Relation,
    context: &RoundContext,
    answer: &[round::AnswerColumn],
    known: round::Admission,
) -> TokenStream {
    let canonical = &context.layouts[relation.providers.canonical];
    let next = &relation.next;
    let row = owned_answer(&relation.declaration, answer)
        .map_or_else(|| quote!(row), |owned| quote!((#owned)(row)));
    admit_canonical(
        canonical,
        (known == round::Admission::Admit).then_some(&canonical.host),
        &relation.next_canonical,
        answer,
        quote!(#next.push(#row);),
    )
}
