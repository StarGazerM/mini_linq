//! Checked Datalog scheduling with resident device publication.
use cq_datalog_fork::{Fork, RoundContext};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{ToTokens, quote};

#[proc_macro]
pub fn datalog(input: TokenStream) -> TokenStream {
    expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(input: Tokens) -> syn::Result<Tokens> {
    let mut source: cq_lang_datalog::Program = syn::parse2(input)?;
    cq_lang_datalog::contract::check(&source)?;
    if source.observer.is_some() {
        return Err(syn::Error::new_spanned(
            &source,
            "CQ_GPU_DATALOG_OBSERVER: device Datalog does not execute Rust observers",
        ));
    }
    if source.strata.is_none() {
        source = cq_datalog_schedule::stratify::compile(&source)?;
    }
    if source.variants.is_none() {
        source = cq_datalog_schedule::seminaive::compile_with_representation(
            &source,
            cq_datalog_schedule::seminaive::Representation::DeviceSet,
        )?;
    }
    if source
        .insertion
        .as_ref()
        .is_none_or(|p| p != "device_set_all_columns")
    {
        return Err(syn::Error::new_spanned(
            &source,
            "CQ_GPU_DATALOG_POLICY: requires device_set_all_columns/device_set_owned; CPU HashSet schedules cannot be reinterpreted",
        ));
    }
    let fork = cq_datalog_fork::compile(&source)?;
    let literals = string_literals(source.rules.iter().map(ToTokens::to_token_stream).collect());
    render(fork, literals)
}

fn snapshots(context: &RoundContext, queries: Tokens) -> Tokens {
    let relations = context.relations.iter().map(|relation| {
        let state = &relation.seen;
        let all = &relation.all;
        let delta = relation
            .delta_host
            .as_ref()
            .map(|delta| quote!(let #delta = #state.delta();));
        quote!(let #all = #state.full(); #delta)
    });
    let layouts = context.layouts.iter().map(|layout| (layout, false))
        .chain(context.delta_layouts.iter().map(|layout| (layout, true)))
        .filter(|(layout, _)| mentions(queries.clone(), &layout.name))
        .map(|(layout, delta)| {
            let relation = &context.relations[layout.relation];
            let input = if delta { relation.delta_host.as_ref().unwrap() } else { &relation.all };
            let host = &layout.host;
            let columns = &layout.columns;
            let types = columns.iter().map(|&i| &relation.declaration.columns[i].ty);
            quote! {
                let #host: ::cq_gpu_runtime::DeviceColumns<'_, (#(#types,)*), ::cq_gpu_runtime::Plain> =
                    ::cq_gpu_runtime::with_runtime_for(Some(#input.pool().clone()), |runtime| {
                        let mut execution = runtime.execution();
                        let input = execution.bind_columns(#input)?;
                        execution.into_columns(input, &[#(#columns),*])
                    }).expect("GPU Datalog layout projection failed");
            }
        });
    quote!(#(#relations)* #(#layouts)*)
}

fn render(fork: Fork, literals: Vec<syn::LitStr>) -> syn::Result<Tokens> {
    let context = &fork.context;
    let anchor = context
        .relations
        .iter()
        .find(|r| r.declaration.value.is_some())
        .ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "CQ_GPU_DATALOG_CONTEXT: at least one resident EDB is required (it may be empty)",
            )
        })?;
    let anchor_state = &anchor.seen;
    let anchor_host = &anchor.all;
    // Reuse source-fresh context names; observers are rejected by this bridge.
    let dictionary = &context.round_name;
    let pool = &context.observer_name;
    let edbs: Vec<_> = context
        .relations
        .iter()
        .filter(|relation| relation.declaration.value.is_some())
        .collect();
    let hosts = edbs.iter().map(|relation| {
        let host = &relation.all;
        let expression = &relation.declaration.value.as_ref().unwrap().expression;
        let types = relation.declaration.columns.iter().map(|column| &column.ty);
        quote! {
            let #host: &::cq_gpu_runtime::DeviceColumns<'_, (#(#types,)*), _> = &(#expression);
        }
    });
    let merge = edbs.iter().map(|relation| {
        let host = &relation.all;
        quote! {
            assert!(::std::sync::Arc::ptr_eq(#host.pool(), &#pool),
                "CQ_GPU_CONTEXT: Datalog EDBs must share one pool, stream and context");
            #dictionary.merge(#host.dictionary())
                .expect("CQ_GPU_DICTIONARY_DOMAIN: Datalog EDB dictionaries must have compatible prefixes");
        }
    });
    let dictionary_setup = quote! {
        #(#hosts)*
        let #pool = #anchor_host.pool().clone();
        let mut #dictionary = ::cq_gpu_runtime::Dictionary::default();
        #(#merge)*
        #(#dictionary.intern(#literals).expect("GPU Datalog string literal interning failed");)*
        let #dictionary = ::std::sync::Arc::new(#dictionary);
    };
    let mut bindings = Vec::new();
    for relation in context
        .relations
        .iter()
        .filter(|r| r.declaration.value.is_some())
    {
        let state = &relation.seen;
        let host = &relation.all;
        let types = relation.declaration.columns.iter().map(|c| &c.ty);
        bindings.push(quote! {
            let mut #state: ::cq_gpu_runtime::datalog::Relation<'_, (#(#types,)*)> =
                ::cq_gpu_runtime::datalog::Relation::from_columns(
                    &#host.with_dictionary(#dictionary.clone())
                        .expect("GPU Datalog common dictionary binding failed"))
                    .expect("GPU Datalog requires resident DeviceColumns EDBs with compatible context and dictionary");
        });
    }
    for relation in context
        .relations
        .iter()
        .filter(|r| r.declaration.value.is_none())
    {
        let state = &relation.seen;
        let types = relation.declaration.columns.iter().map(|c| &c.ty);
        let width = relation.declaration.columns.len();
        bindings.push(quote! {
            let mut #state: ::cq_gpu_runtime::datalog::Relation<'_, (#(#types,)*)> =
                ::cq_gpu_runtime::datalog::Relation::empty(
                    #pool.clone(), #dictionary.clone(), #width)
                    .expect("GPU Datalog empty relation failed");
        });
    }
    let holes: Vec<_> = fork
        .holes
        .iter()
        .map(|hole| {
            let hole = hole.to_token_stream();
            let keys: Vec<_> = context
                .strata
                .iter()
                .flat_map(|s| &s.keys)
                .filter(|key| mentions(hole.clone(), &key.name))
                .map(|key| {
                    let name = &key.name;
                    let types = key.columns.iter().map(|&column| {
                        &context.relations[key.relation].declaration.columns[column].ty
                    });
                    quote!(#name:(#(#types,)*))
                })
                .collect();
            let print = if keys.is_empty() {
                quote!(::cq_gpu::print(completion(round), datalog_dictionary(#dictionary), datalog_pool(#pool)))
            } else {
                quote!(::cq_gpu::print(completion(round), datalog_dictionary(#dictionary), datalog_pool(#pool), datalog_keys(#(#keys),*)))
            };
            quote! {
                ::mini_linq::compile! {
                    pipeline {
                        physical = [::cq_rewrites::build_host_reads];
                        structures = [::cq_rewrites::sorted_runs, ::cq_rewrites::hashed_runs, ::cq_gpu::prefix_runs,
                            ::cq_rewrites::retain_lookups,
                            ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::Plain, ::cq_gpu_runtime::Compacting>)];
                        print = #print;
                    }
                    #hole
                }
            }
        })
        .collect();
    let key_snapshots = snapshots(context, Tokens::new());
    let strata = context.strata.iter().enumerate().map(|(scc, stratum)| {
        let scc_label = syn::LitCStr::new(
            &std::ffi::CString::new(format!("scc/{scc}")).unwrap(),
            proc_macro2::Span::call_site(),
        );
        let keys = stratum.keys.iter().map(|key| {
            let name = &key.name;
            let hole = &holes[key.hole];
            let label = syn::LitCStr::new(
                &std::ffi::CString::new(format!("hole/key/{name}")).unwrap(),
                proc_macro2::Span::call_site(),
            );
            quote!(let #name = {
                let _cq_hole = ::cq_gpu_runtime::nvtx::range(#label);
                #hole
            };)
        });
        let phase = |rules: &[cq_datalog_fork::RuleHole], phase: &str| {
            let round_label = syn::LitCStr::new(
                &std::ffi::CString::new(format!("round/{phase}")).unwrap(),
                proc_macro2::Span::call_site(),
            );
            let queries: Tokens = rules.iter()
                .map(|rule| fork.holes[rule.hole].query.to_token_stream()).collect();
            let snapshots = snapshots(context, queries);
            let candidates = stratum.members.iter().map(|&index| {
                let relation = &context.relations[index];
                let next = &relation.next;
                let types = relation.declaration.columns.iter().map(|c| &c.ty);
                quote!(let mut #next: ::std::vec::Vec<::cq_gpu_runtime::DeviceColumns<'_, (#(#types,)*), ::cq_gpu_runtime::Plain>> = ::std::vec::Vec::new();)
            });
            let calls = rules.iter().map(|rule| {
                let next = &context.relations[rule.head].next;
                let hole = &holes[rule.hole];
                let name = &context.relations[rule.head].declaration.name;
                let label = syn::LitCStr::new(
                    &std::ffi::CString::new(format!("hole/{name}/variant-{}", rule.variant)).unwrap(),
                    proc_macro2::Span::call_site(),
                );
                quote!({
                    let _cq_hole = ::cq_gpu_runtime::nvtx::range(#label);
                    #next.push(#hole);
                })
            });
            let prepare = stratum.members.iter().map(|&index| {
                let relation = &context.relations[index];
                let state = &relation.seen;
                let next = &relation.next;
                quote!(let #next = #state.prepare(&#next, &mut __cq_status).expect("GPU Datalog admission failed");)
            });
            let finish = stratum.members.iter().map(|&index| {
                let next = &context.relations[index].next;
                quote!(let #next = #next.finish(&__cq_checked).expect("GPU Datalog full snapshot failed");)
            });
            let publish = stratum.members.iter().map(|&index| {
                let relation = &context.relations[index];
                let state = &relation.seen;
                let next = &relation.next;
                quote!(#state = #next;)
            });
            quote! {{
                let _cq_round = ::cq_gpu_runtime::nvtx::range(#round_label);
                let mut __cq_status = ::cq_gpu_runtime::datalog::RoundStatus::new(#anchor_state.full().pool().clone());
                #(#candidates)*
                { #snapshots #(#calls)* }
                #(#prepare)*
                let __cq_checked = __cq_status.check().expect("GPU Datalog convergence failed");
                #(#finish)*
                #(#publish)*
                __cq_checked.any()
            }}
        };
        let seed = phase(&stratum.seeds, "seed");
        let step = phase(&stratum.steps, "step");
        quote! {
            {
                let _cq_scc = ::cq_gpu_runtime::nvtx::range(#scc_label);
                // Lower-stratum snapshots and projected negative keys stay frozen.
                #key_snapshots
                #(#keys)*
                let mut __cq_continue = #seed;
                while __cq_continue {
                    __cq_continue = #step;
                }
            }
        }
    });
    let outputs: Vec<_> = context
        .outputs
        .iter()
        .map(|&index| {
            let state = &context.relations[index].seen;
            quote!(#state.full().clone())
        })
        .collect();
    let result = if outputs.len() == 1 {
        outputs[0].clone()
    } else {
        quote!((#(#outputs,)*))
    };
    Ok(quote!({
        let _cq_program = ::cq_gpu_runtime::nvtx::range(c"program");
        #dictionary_setup #(#bindings)*
        let _cq_fixed_point = ::cq_gpu_runtime::nvtx::range(c"fixed-point");
        #(#strata)*
        ::cq_gpu_runtime::with_runtime_for(Some(#pool.clone()), |runtime| {
            runtime.execution().finish()
        }).expect("GPU Datalog completion failed");
        #result
    }))
}

fn mentions(tokens: Tokens, name: &syn::Ident) -> bool {
    tokens.into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(id) => id == *name,
        proc_macro2::TokenTree::Group(group) => mentions(group.stream(), name),
        _ => false,
    })
}

/// Intern only authored rule string literals, in stable source order. These
/// include heads, positive selections, guards, and negative constant filters.
fn string_literals(tokens: Tokens) -> Vec<syn::LitStr> {
    fn collect(
        tokens: Tokens,
        seen: &mut std::collections::BTreeSet<String>,
        literals: &mut Vec<syn::LitStr>,
    ) {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Group(group) => collect(group.stream(), seen, literals),
                proc_macro2::TokenTree::Literal(literal) => {
                    if let Ok(literal) = syn::parse2::<syn::LitStr>(literal.into_token_stream()) {
                        if seen.insert(literal.value()) {
                            literals.push(literal);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut literals = Vec::new();
    collect(
        tokens,
        &mut std::collections::BTreeSet::new(),
        &mut literals,
    );
    literals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_domain_uses_stable_authored_literal_order() {
        let literals = string_literals(quote! {
            T(*x, "a") :- E(x), !M(x, r#"b"#), #[total] if *x == "a";
            U(*x, "c") :- E(x);
        });
        assert_eq!(
            literals.iter().map(syn::LitStr::value).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        let emitted = expand(quote! {
            struct Literals;
            relation E(x:i64) = (input);
            relation T(x:i64,tag:&'static str);
            output T;
            T(*x,"a") :- E(x);
            T(*x,"b") :- E(x);
        })
        .unwrap()
        .to_string();
        assert_eq!(emitted.matches("datalog_dictionary").count(), 2);
        assert_eq!(emitted.matches("datalog_pool").count(), 2);
        assert!(
            emitted.find(". intern").unwrap() < emitted.find("Relation :: from_columns").unwrap()
        );
    }

    #[test]
    fn every_hole_uses_gpu_printer_and_resident_publication() {
        let emitted = expand(quote! {
            struct Reachability;
            relation Edge(x:i64,y:i64) = (edges);
            relation Reach(x:i64,y:i64);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Edge(y,z);
        })
        .unwrap()
        .to_string();
        assert_eq!(emitted.matches("cq_gpu :: print").count(), 2);
        assert!(emitted.contains("prepare"));
        assert_eq!(emitted.matches("completion (round)").count(), 2);
        assert_eq!(emitted.matches("cq_rewrites :: hashed_runs").count(), 2);
        assert_eq!(emitted.matches("cq_gpu_runtime :: Compacting").count(), 2);
        assert!(emitted.find(". check").unwrap() < emitted.find(". finish").unwrap());
        assert!(emitted.contains("RoundStatus"));
        assert!(!emitted.contains("to_host"));
        assert!(!emitted.contains("HashSet"));
    }

    #[test]
    fn single_output_preserves_direct_result_shape() {
        let emitted = expand(quote! {
            struct Edges;
            relation E(x:i64) = (input);
            output E;
        })
        .unwrap();
        let block: syn::ExprBlock = syn::parse2(emitted).unwrap();
        assert!(matches!(
            block.block.stmts.last(),
            Some(syn::Stmt::Expr(syn::Expr::MethodCall(_), None))
        ));
    }

    #[test]
    fn cpu_schedule_is_not_reinterpreted() {
        let source: cq_lang_datalog::Program = syn::parse_quote! {
            struct Copy;
            relation E(x:i64) = (input);
            relation R(x:i64);
            output R;
            R(*x) :- E(x);
        };
        let source = cq_datalog_schedule::stratify::compile(&source).unwrap();
        let source = cq_datalog_schedule::seminaive::compile(&source).unwrap();
        let error = expand(source.to_token_stream()).unwrap_err();
        assert!(error.to_string().contains("CQ_GPU_DATALOG_POLICY"));
    }
}
