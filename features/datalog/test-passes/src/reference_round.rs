//! Independent set realization of recorded Round bindings and CQ holes.
//!
//! Layouts are flat sets in their recorded column order, not production maps.
//! Provider representation, pull/push plumbing, timing, and observers are not
//! denotational operations. We retain the recorded scans, CQ Rust expressions,
//! seed/step schedules, admission, publication transactions, and termination.
use cq_datalog_support::Fresh;
use cq_lang_datalog_round::{self as round, InputSource, Layout, Program, RuleHole, Stratum};
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;

pub fn emit(program: &Program) -> syn::Result<TokenStream> {
    round::contract::check(program)?;
    let context = &program.context;
    let mut fresh = Fresh::new(program);
    let mut bindings = Vec::new();
    for relation in &context.relations {
        let all = &relation.all;
        let types = relation.declaration.columns.iter().map(|c| &c.ty);
        let tuple = quote!((#(#types,)*));
        if let Some(value) = &relation.declaration.value {
            let expression = &value.expression;
            let row = fresh.id("round_input");
            bindings.push(quote!(
                let #all = ::std::iter::IntoIterator::into_iter(#expression)
                    .map(|#row| ::std::borrow::Borrow::<#tuple>::borrow(&#row).clone())
                    .collect::<::std::collections::HashSet<#tuple>>();
            ));
        } else {
            let seen = &relation.seen;
            bindings.push(quote!(
                let mut #all = ::std::collections::HashSet::<#tuple>::new();
                let mut #seen = ::std::collections::HashSet::<#tuple>::new();
            ));
        }
        if let Some(delta) = &relation.delta_host {
            bindings.push(quote!(let mut #delta = ::std::collections::HashSet::<#tuple>::new();));
        }
    }
    for layout in context.layouts.iter().chain(&context.delta_layouts) {
        let relation = &context.relations[layout.relation];
        let host = &layout.host;
        let types = layout
            .columns
            .iter()
            .map(|&i| &relation.declaration.columns[i].ty);
        bindings.push(quote!(let mut #host = ::std::collections::HashSet::<(#(#types,)*)>::new();));
        if relation.declaration.value.is_some() {
            bindings.push(extend_layout(layout, &relation.all, &mut fresh));
        }
    }
    let mut components = Vec::new();
    for stratum in &context.strata {
        let mut keys = Vec::new();
        for key in &stratum.keys {
            let name = &key.name;
            let evaluation = hole(program, key.hole, &mut fresh)?;
            keys.push(quote!(let #name = &#evaluation;));
        }
        let seeds = schedule(program, stratum, &stratum.seeds, &mut fresh)?;
        let steps = schedule(program, stratum, &stratum.steps, &mut fresh)?;
        let deltas = stratum
            .termination
            .iter()
            .map(|&index| {
                context.relations[index]
                    .delta_host
                    .as_ref()
                    .expect("checked delta binding")
            })
            .collect::<Vec<_>>();
        let termination = match stratum.termination_test.to_string().as_str() {
            "sum_delta_lengths" => quote!((0usize #(+ #deltas.len())*) != 0),
            "any_delta" => quote!(false #(|| !#deltas.is_empty())*),
            _ => unreachable!("checked termination test"),
        };
        // The two admitted loop spellings have identical seed/step control flow.
        // Component scope keeps borrowed key rows alive without cloning keys.
        let seed = fresh.id("round_seed");
        components.push(quote!({
            #(#keys)*
            ::cq_datalog_testkit::eval_round(|#seed| {
                if #seed { #seeds } else { #steps }
                #termination
            });
        }));
    }
    let outputs = context
        .outputs
        .iter()
        .map(|output| {
            let all = &context.relations[output.relation].all;
            match output.conversion.to_string().as_str() {
                // The reference owns its canonical EDB copy already. Both recorded
                // conversions therefore return that owned set without another copy.
                "borrowed_cloned_into_iter" | "owned_into_iter" => quote!(#all),
                _ => unreachable!("checked output conversion"),
            }
        })
        .collect::<Vec<_>>();
    let result = if outputs.len() == 1 {
        outputs[0].clone()
    } else {
        quote!((#(#outputs,)*))
    };
    Ok(quote!({ #(#bindings)* #(#components)* #result }))
}

/// Read the CQ declarations' actual host expressions, not Source rules or the
/// Incremental variant behind this hole. In particular, scan and execution
/// bindings remain visible in the generated native query.
fn hole(program: &Program, index: usize, fresh: &mut Fresh) -> syn::Result<TokenStream> {
    let cq = &program.holes[index].cq;
    let inputs = cq.inputs().collect::<Vec<_>>();
    let positions: BTreeMap<_, _> = inputs
        .iter()
        .enumerate()
        .map(|(i, declaration)| (symbol_name(&declaration.name), i))
        .collect();
    let values = inputs.iter().map(|declaration| {
        let expression = &declaration
            .value
            .as_ref()
            .expect("checked hole input")
            .expression;
        quote!(&(#expression))
    });
    let output = cq.output().expect("checked hole output");
    let key_hole = program
        .context
        .strata
        .iter()
        .any(|stratum| stratum.keys.iter().any(|key| key.hole == index));
    let borrowed = output
        .columns
        .iter()
        .map(|column| key_hole && matches!(&column.ty, syn::Type::Reference(_)))
        .collect::<Vec<_>>();
    // CPU key rows store column references, while their CQ input schemas describe
    // the referents. Keep the reference evaluator's comparisons at logical depth.
    let borrowed_negative_inputs = program
        .context
        .strata
        .iter()
        .flat_map(|stratum| &stratum.keys)
        .filter(|key| {
            key.representation == round::KeyRepresentation::Borrowed && !key.columns.is_empty()
        })
        .map(|key| symbol_name(&key.name))
        .collect::<Vec<_>>();
    let types = output.columns.iter().map(|column| &column.ty);
    let state = fresh.id("round_hole_inputs");
    let candidates = fresh.id("round_hole_rows");
    let evaluation = crate::reference::emit_query(
        &cq.query,
        &state,
        &positions,
        &candidates,
        &borrowed,
        &borrowed_negative_inputs,
        fresh,
    )?;
    Ok(quote!({
        let #state = (#(#values,)*);
        let mut #candidates = ::std::collections::HashSet::<(#(#types,)*)>::new();
        #evaluation
        #candidates
    }))
}

fn schedule(
    program: &Program,
    stratum: &Stratum,
    rules: &[RuleHole],
    fresh: &mut Fresh,
) -> syn::Result<TokenStream> {
    let context = &program.context;
    let allocate = stratum
        .members
        .iter()
        .map(|&index| {
            let relation = &context.relations[index];
            let next = &relation.next;
            let types = relation.declaration.columns.iter().map(|column| &column.ty);
            quote!(let mut #next = ::std::collections::HashSet::<(#(#types,)*)>::new();)
        })
        .collect::<Vec<_>>();
    let mut evaluate = Vec::new();
    let mut admit = Vec::new();
    for rule in rules {
        let rows = fresh.id("round_candidates");
        let query = hole(program, rule.hole, fresh)?;
        let mut scan_bindings = Vec::new();
        if let Some(scan) = &rule.scan {
            let host = match scan.source {
                InputSource::Delta(index) => context.relations[index]
                    .delta_host
                    .as_ref()
                    .expect("checked scan delta"),
                InputSource::Layout(index) => &context.layouts[index].host,
            };
            let binding = &scan.binding;
            scan_bindings.push(quote!(let #binding = &#host;));
            // Certified execution may rename this one immutable whole-input
            // scan. Sequential reference evaluation binds the exact same set.
            for execution in program
                .execution
                .iter()
                .filter(|entry| entry.hole == rule.hole)
            {
                let binding = &execution.binding;
                scan_bindings.push(quote!(let #binding = &#host;));
            }
        }
        evaluate.push(quote!(let #rows = { #(#scan_bindings)* #query };));
        let relation = &context.relations[rule.head];
        let seen = &relation.seen;
        let next = &relation.next;
        let insertion = &rule.insertion;
        let row = fresh.id("round_candidate");
        admit.push(quote!(for #row in #rows {
            if #seen.#insertion(#row.clone()) { #next.insert(#row); }
        }));
    }
    let mut publish = Vec::new();
    for transaction in &stratum.publication {
        let relation = &context.relations[transaction.relation];
        let all = &relation.all;
        let next = &relation.next;
        let delta = relation
            .delta_host
            .as_ref()
            .expect("checked publication delta");
        for operation in &transaction.operations {
            match operation.kind.to_string().as_str() {
                "full_layouts" => {
                    for &index in &operation.layouts {
                        publish.push(extend_layout(&context.layouts[index], next, fresh));
                    }
                }
                "all_extend" => publish.push(quote!(#all.extend(#next.iter().cloned());)),
                "delta_swap" => publish.push(quote!(#delta = #next;)),
                "delta_rebuild" => {
                    for &index in &operation.layouts {
                        let layout = &context.delta_layouts[index];
                        let host = &layout.host;
                        let extend = extend_layout(layout, delta, fresh);
                        publish.push(quote!(#host.clear(); #extend));
                    }
                }
                _ => unreachable!("checked publication operation"),
            }
        }
    }
    let phases = stratum
        .phases
        .iter()
        .map(|phase| match phase.to_string().as_str() {
            "evaluate" => quote!(#(#evaluate)*),
            "admit" => quote!(#(#admit)*),
            "publish" => quote!(#(#publish)*),
            _ => unreachable!("checked round phase"),
        });
    Ok(quote!(#(#allocate)* #(#phases)*))
}

fn extend_layout(layout: &Layout, input: &syn::Ident, fresh: &mut Fresh) -> TokenStream {
    let host = &layout.host;
    let row = fresh.id("round_layout_row");
    let columns = layout.columns.iter().map(|&index| {
        let index = syn::Index::from(index);
        quote!(#row.#index.clone())
    });
    quote!(#host.extend(#input.iter().map(|#row| (#(#columns,)*)));)
}
