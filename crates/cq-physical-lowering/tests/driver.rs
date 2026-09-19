use cq_ir::{
    physical,
    provider_protocol::{self as protocol, Binding, Reply, Request, State, Task},
};
use cq_physical_lowering::{begin, resume};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn plan() -> physical::Module {
    syn::parse_quote! {
        struct Example;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        answer(a, b) :- L(a), R(a, b);
        relational {
            l = rename L { a -> a };
            r = rename R { a -> a, b -> b };
            joined = natural_join l with r;
            output joined as answer(a, b);
        }
        indexes { idx = @r[0]; }
        uses { joined.right = idx; }
        physical {
            relations {
                L = arbitrary::one;
                R = arbitrary::two;
                l = other::alpha;
                r = other::beta;
                joined = result::representation;
            }
            indexes { idx = indexes::chosen; }
            lowerings {
                l = algorithms::rename_a;
                r = algorithms::rename_b;
                joined = algorithms::join;
            }
        }
    }
}

fn request(tokens: TokenStream) -> (syn::Path, Request) {
    let expr: syn::ExprMacro = syn::parse2(tokens).unwrap();
    (expr.mac.path, syn::parse2(expr.mac.tokens).unwrap())
}

fn first() -> Request {
    request(
        begin(
            plan(),
            syn::parse_quote!(driver::resume),
            syn::parse_quote!(consumer::access),
        )
        .unwrap(),
    )
    .1
}

fn reply(request: &Request, result: TokenStream) -> Reply {
    let state = &request.state;
    syn::parse2(quote! {
        resume { #state } emitted {} result { #result }
    })
    .unwrap()
}

fn stored(name: &syn::Ident) -> TokenStream {
    quote! { stored #name at (#name) operations any::operations }
}

fn finish_reply(request: &Request) -> Reply {
    reply(
        request,
        quote! {
            region joined {
                joined(a, b) => ((a, b)) :-
                    L(a) => bind a <- (L.iter()),
                    R(a, b) => bind b <- (idx.get(a).into_iter().flatten());
            }
        },
    )
}

fn reach_join() -> Request {
    let mut request = first();
    for _ in 0..5 {
        let name = request.request.name();
        let result = match request.request {
            Task::Index { .. } => quote! { indexed #name at (#name) operations selected::lookup },
            _ => stored(name),
        };
        request = self::request(resume(reply(&request, result)).unwrap()).1;
    }
    request
}

#[test]
fn explicit_paths_schedule_inputs_definitions_and_indexes_without_reselection() {
    let source = plan();
    let mut tokens = begin(
        source.clone(),
        syn::parse_quote!(driver::resume),
        syn::parse_quote!(consumer::access),
    )
    .unwrap();
    let expected = [
        ("arbitrary :: one", "input L from (L)"),
        ("arbitrary :: two", "input R from (R)"),
        ("algorithms :: rename_a", "lower l"),
        ("algorithms :: rename_b", "lower r"),
        ("indexes :: chosen", "index idx"),
        ("algorithms :: join", "lower joined"),
    ];
    for (ordinal, (path, task)) in expected.iter().enumerate() {
        let (actual_path, request) = request(tokens);
        assert_eq!(actual_path.to_token_stream().to_string(), *path);
        assert_eq!(request.request.to_token_stream().to_string(), *task);
        assert_eq!(request.plan, source);
        assert_eq!(request.environment.bindings.len(), ordinal);
        let name = request.request.name();
        let mut response = if ordinal == 5 {
            finish_reply(&request)
        } else {
            let result = if matches!(request.request, Task::Index { .. }) {
                quote! { indexed #name at (#name) operations opaque::next }
            } else {
                stored(name)
            };
            reply(&request, result)
        };
        let marker = quote::format_ident!("marker_{ordinal}");
        response.emitted.push(syn::parse_quote!(let #marker = ();));
        tokens = resume(response).unwrap();
    }
    let block: syn::Block = syn::parse2(tokens.clone()).unwrap();
    assert_eq!(block.stmts.len(), 7);
    let text = tokens.to_string();
    assert!(text.contains("consumer :: access !"));
    assert!(text.contains("answer (a , b)"));
    for ordinal in 0..6 {
        assert!(
            block.stmts[ordinal]
                .to_token_stream()
                .to_string()
                .contains(&format!("marker_{ordinal}"))
        );
    }
}

#[test]
fn wrong_identity_or_kind_rejects_instead_of_advancing() {
    let request = first();
    let wrong = reply(&request, quote!(stored R at (R) operations any::ops));
    assert!(
        resume(wrong)
            .unwrap_err()
            .to_string()
            .contains("requested identity `L`")
    );
    let wrong = reply(&request, quote!(indexed L at (L) operations any::ops));
    assert!(
        resume(wrong)
            .unwrap_err()
            .to_string()
            .contains("reply kind")
    );
}

#[test]
fn repeated_reply_in_serialized_environment_rejects() {
    let request = first();
    let mut state: State = syn::parse2(request.state.clone()).unwrap();
    state
        .environment
        .bindings
        .push(syn::parse_quote!(stored L at (L) operations any::ops));
    state
        .environment
        .bindings
        .push(syn::parse_quote!(stored L at (L) operations any::ops));
    let response: Reply = syn::parse_quote! {
        resume { #state } emitted {} result { stored R at (R) operations any::ops }
    };
    assert!(
        resume(response)
            .unwrap_err()
            .to_string()
            .contains("requested identity `R`")
    );
}

#[test]
fn pending_task_must_be_next_in_dependency_order() {
    let request = first();
    let mut state: State = syn::parse2(request.state.clone()).unwrap();
    state.pending = syn::parse_quote!(lower joined);
    let response: Reply = syn::parse_quote! {
        resume { #state } emitted {} result { stored joined at (fake) operations any::ops }
    };
    assert!(
        resume(response)
            .unwrap_err()
            .to_string()
            .contains("next physical request")
    );
}

#[test]
fn region_cannot_be_implicitly_indexed() {
    let mut request = first();
    for _ in 0..3 {
        let result = stored(request.request.name());
        request = self::request(resume(reply(&request, result)).unwrap()).1;
    }
    assert_eq!(request.request.name(), "r");
    let response = reply(
        &request,
        quote! {
            region r { r(a, b) => ((a, b)) :- R(a, b) => bind (a,b) <- (R.iter()); }
        },
    );
    assert!(
        resume(response)
            .unwrap_err()
            .to_string()
            .contains("cannot build an index from an Access region")
    );
}

#[test]
fn invalid_region_heading_and_invalid_access_binding_reject() {
    let request = reach_join();
    let mut response = finish_reply(&request);
    let Binding::Region { access, .. } = &mut response.result else {
        unreachable!()
    };
    access.head.atom = syn::parse_quote!(joined(b, a));
    assert!(
        resume(response)
            .unwrap_err()
            .to_string()
            .contains("logical column order")
    );
    let mut response = finish_reply(&request);
    let Binding::Region { access, .. } = &mut response.result else {
        unreachable!()
    };
    access.head.atom = syn::parse_quote!(joined(a, missing));
    assert!(
        resume(response)
            .unwrap_err()
            .to_string()
            .contains("not bound")
    );
}

#[test]
fn stored_final_is_not_implicitly_scanned() {
    let request = reach_join();
    let response = reply(&request, stored(request.request.name()));
    assert!(
        resume(response)
            .unwrap_err()
            .to_string()
            .contains("final lowering must return an Access region")
    );
}

#[test]
fn provider_tail_value_is_not_silently_changed_into_a_statement() {
    let request = first();
    let mut response = reply(&request, stored(request.request.name()));
    response
        .emitted
        .push(syn::Stmt::Expr(syn::parse_quote!(build()), None));
    assert!(
        resume(response)
            .unwrap_err()
            .to_string()
            .contains("trailing value expression")
    );
}

#[test]
fn semicolonless_control_statements_preserve_their_order_and_tokens() {
    let request = first();
    let state = &request.state;
    let response: Reply = syn::parse_quote! {
        resume { #state }
        emitted {
            if enabled { observe(); }
            for item in items { consume(item); }
            let completed = true;
            if completed { finish(); }
        }
        result { stored L at (L) operations any::operations }
    };
    let expected = response.emitted.to_vec();
    let next = self::request(resume(response).unwrap()).1;
    let state: State = syn::parse2(next.state).unwrap();
    assert_eq!(state.emitted, expected);
    assert!(matches!(
        state.emitted[0],
        syn::Stmt::Expr(syn::Expr::If(_), None)
    ));
    assert!(matches!(
        state.emitted[1],
        syn::Stmt::Expr(syn::Expr::ForLoop(_), None)
    ));
    assert!(matches!(
        state.emitted[3],
        syn::Stmt::Expr(syn::Expr::If(_), None)
    ));
}

#[test]
fn family_messages_roundtrip_without_driver_operation_cases() {
    let first = first();
    let state = first.state;
    let plan = first.plan;
    let request: protocol::Request<protocol::exact::Task> = syn::parse_quote! {
        reply_to (foreign::resume) { #state }
        plan { #plan } environment {}
        request { output joined fields { 0 = (a); 1 = (b); } whole { l = (a); } }
    };
    let parsed: protocol::Request<protocol::exact::Task> =
        syn::parse2(request.to_token_stream()).unwrap();
    assert!(matches!(
        parsed.request,
        protocol::exact::Task::Output { .. }
    ));
    let fragment: protocol::exact::IteratorReply = syn::parse_quote! {
        resume { opaque provider state }
        source (index.get(a).into_iter().flatten())
        item (b)
        witness { row ((a,b)) fields { 0 = (a); 1 = (b); } }
    };
    let parsed: protocol::exact::IteratorReply = syn::parse2(fragment.to_token_stream()).unwrap();
    assert_eq!(parsed.pattern.to_token_stream().to_string(), "b");
    assert_eq!(parsed.witness.fields.len(), 2);
    let output: protocol::exact::WitnessReply = syn::parse_quote! {
        resume { opaque provider state }
        witness { row ((a,b)) fields { 0 = (a); 1 = (b); } }
    };
    let parsed: protocol::exact::WitnessReply = syn::parse2(output.to_token_stream()).unwrap();
    assert_eq!(parsed.witness.fields.len(), 2);
}

#[test]
fn providers_cannot_return_the_removed_access_clause_reply() {
    let obsolete = quote! {
        resume { opaque provider state }
        access { R(a,b) => bind b <- (index.get(a).into_iter().flatten()) }
        witness { row ((a,b)) fields { 0 = (a); 1 = (b); } }
    };
    assert!(syn::parse2::<protocol::exact::IteratorReply>(obsolete.clone()).is_err());
    assert!(syn::parse2::<protocol::exact::WitnessReply>(obsolete).is_err());
}

#[test]
fn host_names_and_index_names_have_distinct_protocol_namespaces() {
    for host in ["idx", "__cq_store_L", "r#type"] {
        let host: syn::Ident = syn::parse_str(host).unwrap();
        let source: physical::Module = syn::parse_quote! {
            struct Names;
            relation #host(a: i32);
            relation R(a: i32, b: i32);
            answer(a,b) :- #host(a), R(a,b);
            relational {
                l = rename #host {a -> a};
                r = rename R {a -> a, b -> b};
                joined = natural_join l with r;
                output joined as answer(a,b);
            }
            indexes {idx = @r[0];}
            uses {joined.right = idx;}
            physical {
                relations {
                    #host = external::host;
                    R = external::host;
                    l = external::host;
                    r = external::host;
                    joined = external::result;
                }
                indexes {idx = external::index;}
                lowerings {l = external::rename; r = external::rename; joined = external::join;}
            }
        };
        let mut current = request(
            begin(
                source,
                syn::parse_quote!(driver::resume),
                syn::parse_quote!(consumer::access),
            )
            .unwrap(),
        )
        .1;
        let Task::Input { source, .. } = &current.request else {
            panic!("expected input")
        };
        assert_eq!(source.to_token_stream().to_string(), host.to_string());
        for _ in 0..5 {
            let name = current.request.name();
            let result = if matches!(current.request, Task::Index { .. }) {
                // Physical Rust values need not use an index's logical name.
                quote!(indexed #name at (provider_chosen_index) operations external::index)
            } else {
                stored(name)
            };
            current = request(resume(reply(&current, result)).unwrap()).1;
        }
        assert!(matches!(
            current.environment.relation(&host),
            Some(Binding::Stored { .. })
        ));
        assert!(matches!(
            current.environment.index(&syn::parse_quote!(idx)),
            Some(Binding::Indexed { .. })
        ));
        resume(finish_reply(&current)).unwrap();
    }
}
