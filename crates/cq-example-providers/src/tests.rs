//! Exercise the real callbacks across the provider/composition boundary.

use cq_ir::{
    physical,
    provider_protocol::{self as protocol, exact},
    rust_access_plan,
};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

use crate::{rules, storage, syntax::AccessState};

fn plan() -> physical::Module {
    let plan: physical::Module = syn::parse_quote! {
        struct Joined;
        relation L(a:i32);
        relation R(a:i32,b:i32);
        answer(a,b) :- L(a), R(a,b);
        relational {
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            j = natural_join l with r;
            output j as answer(a,b);
        }
        indexes {i = @r[0];}
        uses {j.right = i;}
        physical {
            relations {
                L = ext::slice; R = ext::map;
                l = ext::slice; r = ext::map; j = ext::fields;
            }
            indexes {i = ext::map;}
            lowerings {l = ext::rename; r = ext::rename; j = ext::join;}
        }
    };
    physical::contract::check_structure(&plan).unwrap();
    plan
}

fn invocation(tokens: TokenStream) -> syn::ExprMacro {
    syn::parse2(tokens).unwrap()
}

#[test]
fn providers_describe_items_and_join_rule_composes_dependent_bindings() {
    let plan = plan();
    let start = quote! {
        reply_to (driver::resume) { opaque_driver_state }
        plan {#plan}
        environment {
            stored l at (left) operations ext::slice
            stored r at (right) operations ext::map
            indexed i at (index) operations ext::map
        }
        request {lower j}
    };

    let rows_request = invocation(rules::indexed_join(start).unwrap());
    assert_eq!(rows_request.mac.path, syn::parse_quote!(ext::slice));
    let rows_callback = invocation(storage::slice(rows_request.mac.tokens).unwrap());
    let rows: exact::IteratorReply = syn::parse2(rows_callback.mac.tokens.clone()).unwrap();
    assert_eq!(rows.source, syn::parse_quote!((left).iter()));
    let pattern = &rows.pattern;
    assert_eq!(quote!(#pattern).to_string(), "a");
    // The provider reply has no query clause. The selected rule supplies the
    // relation annotation and places the binding before the dependent probe.
    assert!(syn::parse2::<exact::WitnessReply>(rows_callback.mac.tokens.clone()).is_err());

    let matches_request = invocation(rules::indexed_join_rows(rows_callback.mac.tokens).unwrap());
    let request: protocol::Request<exact::Task> =
        syn::parse2(matches_request.mac.tokens.clone()).unwrap();
    let state: AccessState = syn::parse2(request.state).unwrap();
    assert_eq!(state.clauses.len(), 1);
    assert_eq!(
        state.clauses[0],
        syn::parse_quote! {
            l(a) => bind a <- ((left).iter())
        }
    );
    assert!(matches!(request.request, exact::Task::Matches { .. }));

    let matches_callback = invocation(storage::hash(matches_request.mac.tokens).unwrap());
    let matched: exact::IteratorReply = syn::parse2(matches_callback.mac.tokens.clone()).unwrap();
    assert_eq!(
        matched.source,
        syn::parse_quote!((index).get(a).into_iter().flatten())
    );
    let pattern = &matched.pattern;
    assert_eq!(quote!(#pattern).to_string(), "b");
    assert_eq!(matched.witness.fields[0].value, syn::parse_quote!(a));
    assert_eq!(matched.witness.fields[1].value, syn::parse_quote!(b));

    let output_request =
        invocation(rules::indexed_join_matches(matches_callback.mac.tokens).unwrap());
    let request: protocol::Request<exact::Task> =
        syn::parse2(output_request.mac.tokens.clone()).unwrap();
    let state: AccessState = syn::parse2(request.state).unwrap();
    assert_eq!(state.clauses.len(), 2);
    assert_eq!(
        state.clauses[1],
        syn::parse_quote! {
            r(a,b) => bind b <- ((index).get(a).into_iter().flatten())
        }
    );

    let output_callback =
        invocation(crate::exact::borrowed_fields(output_request.mac.tokens).unwrap());
    let output: exact::WitnessReply = syn::parse2(output_callback.mac.tokens.clone()).unwrap();
    assert_eq!(output.witness.row, syn::parse_quote!((a, b,)));
    assert!(syn::parse2::<exact::IteratorReply>(output_callback.mac.tokens.clone()).is_err());

    let finished = invocation(rules::indexed_join_output(output_callback.mac.tokens).unwrap());
    let reply: protocol::Reply = syn::parse2(finished.mac.tokens).unwrap();
    assert!(
        reply.emitted.is_empty(),
        "dependent source must not be hoisted"
    );
    let protocol::Binding::Region { access, .. } = reply.result else {
        panic!("join must return an Access region");
    };
    let expected: rust_access_plan::Plan = syn::parse_quote! {
        j(a,b) => ((a,b,)) :-
            l(a) => bind a <- ((left).iter()),
            r(a,b) => bind b <- ((index).get(a).into_iter().flatten());
    };
    assert_eq!(access, expected);
    rust_access_plan::contract::check(&access).unwrap();
}

#[test]
fn map_rows_reply_explains_items_without_owning_relation_composition() {
    let plan = plan();
    let callback = invocation(
        storage::hash(quote! {
            reply_to (caller::rows) { caller_state }
            plan {#plan}
            environment {stored r at (host_map) operations ext::map}
            request {rows r}
        })
        .unwrap(),
    );
    let reply: exact::IteratorReply = syn::parse2(callback.mac.tokens).unwrap();
    assert_eq!(
        reply.source,
        syn::parse_quote! {
            (host_map).iter().flat_map(|(a, __cq_values)|
                __cq_values.iter().map(move |b| (a,b,)))
        }
    );
    assert_eq!(reply.pattern.to_token_stream().to_string(), "(a , b ,)");
    assert_eq!(reply.witness.row, syn::parse_quote!((a, b,)));
    assert_eq!(reply.witness.fields.len(), 2);
}

#[test]
fn obsolete_access_fragment_replies_are_rejected_by_rule_callbacks() {
    let rows = quote! {
        resume {}
        access {l(a) => bind a <- (input.iter())}
        witness {row(a) fields {0 = (a);}}
    };
    assert!(rules::indexed_join_rows(rows.clone()).is_err());
    assert!(crate::project::rows(rows).is_err());

    let output = quote! {
        resume {}
        access {}
        witness {row((a,b)) fields {0 = (a); 1 = (b);}}
    };
    assert!(rules::indexed_join_output(output.clone()).is_err());
    assert!(crate::project::output(output).is_err());
}
