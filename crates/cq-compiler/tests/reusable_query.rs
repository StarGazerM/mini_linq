//! The same indexed computation embedded in independent Rust hosts.
#[allow(dead_code)]
#[path = "support/compiled.rs"]
mod compiled;

use cq_ir::{index_requirements as ir, relational_plan as ra};
use quote::{ToTokens, quote};
use syn::visit_mut::VisitMut;

fn indexed() -> ir::Module {
    let source: ra::Module = syn::parse_quote! {
        pub struct NamedQuery;
        relation R(a:String,b:String); relation S(b:String,c:String);
        answer(a,c) :- R(a,b), S(b,c);
        relational {
            r = rename R {a->a,b->b};
            s = rename S {b->b,c->c};
            reduced = semijoin r with s;
            joined = natural_join reduced with s;
            projected = project joined keep {a,c};
            output projected as answer(a,c);
        }
    };
    cq_compiler::compile_result_indexes(&source).unwrap()
}

fn lower_access(expression: &mut syn::Expr) {
    struct Lower;
    impl VisitMut for Lower {
        fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
            if let syn::Expr::Macro(m) = expr {
                assert_eq!(m.mac.path.segments.last().unwrap().ident, "collect_access");
                let access = syn::parse2(m.mac.tokens.clone()).unwrap();
                let pipeline = cq_compiler::compile_pull(&access).unwrap();
                *expr = cq_compiler::compile_collect_pipeline(&pipeline).unwrap();
            } else {
                syn::visit_mut::visit_expr_mut(self, expr);
            }
        }
    }
    Lower.visit_expr_mut(expression);
}

#[test]
fn borrowed_repeated_calls_and_standalone_host_share_set_semantics() {
    let indexed = indexed();
    let standalone = compiled::compile_indexed(&indexed).unwrap();
    let access = syn::parse_quote!(::mini_linq::collect_access);
    let mut body = cq_compiler::compile_materialized_query(
        &indexed,
        &access,
        &[
            syn::parse_quote!(left.iter().cloned()),
            syn::parse_quote!(right.iter().cloned()),
        ],
    )
    .unwrap();
    let mut function = cq_compiler::compile_materialized_callable(&indexed, &access).unwrap();
    let text = body.to_token_stream().to_string();
    for observation in [
        "__stats",
        "__pairs",
        "__probes",
        "ExecutionStats",
        "Storage",
    ] {
        assert!(
            !text.contains(observation),
            "plain body contains {observation}"
        );
    }
    lower_access(&mut body);
    lower_access(&mut function);
    compiled::run_rust(quote! {
        #standalone
        fn query(left: &[(String,String)], right: &[(String,String)]) -> std::collections::HashSet<(String,String)> { #body }
        fn main() {
            let left = vec![("a".to_owned(),"x".to_owned()),("a".to_owned(),"x".to_owned()),("b".to_owned(),"y".to_owned())];
            let mut right = vec![("x".to_owned(),"c".to_owned()),("x".to_owned(),"c".to_owned())];
            for _ in 0..3 {
                let expected: std::collections::HashSet<_> = left.iter().flat_map(|(a,b)| right.iter().filter(move |(key,_)| b==key).map(move |(_,c)| (a.clone(),c.clone()))).collect();
                assert_eq!(query(&left,&right), expected);
                assert_eq!((#function)(left.iter().cloned(), right.iter().cloned()), expected);
                let (rows, stats) = NamedQuery::run_with_stats(left.iter().cloned(), right.iter().cloned());
                assert_eq!(rows.into_iter().collect::<std::collections::HashSet<_>>(), expected);
                assert_eq!(stats.semijoin_probes, 2);
                right.push(("y".to_owned(),"d".to_owned()));
            }
            assert!(query(&left,&[]).is_empty());
            assert!(query(&[],&right).is_empty());
            assert_eq!(left.len(),3);
        }
    });
}

#[test]
fn input_expressions_evaluate_once_in_declaration_order_before_local_bindings() {
    let indexed = indexed();
    let mut body = cq_compiler::compile_materialized_query(
        &indexed,
        &syn::parse_quote!(::mini_linq::collect_access),
        &[
            syn::parse_quote!({
                calls.push(1);
                input1.clone()
            }),
            syn::parse_quote!({
                calls.push(2);
                input0.clone()
            }),
        ],
    )
    .unwrap();
    lower_access(&mut body);
    compiled::run_rust(quote! {
        fn main() {
            let input1 = vec![("a".to_owned(), "key".to_owned())];
            let input0 = vec![("key".to_owned(), "z".to_owned())];
            let mut calls = Vec::new();
            let answer = #body;
            assert_eq!(calls, [1,2]);
            assert_eq!(answer, std::collections::HashSet::from([("a".to_owned(),"z".to_owned())]));
        }
    });
}

#[test]
fn plain_host_accepts_named_noncopy_rows_without_ordering() {
    let source: ra::Module = syn::parse_quote! {
        struct Unordered;
        relation R(key:Key); relation S(key:Key);
        answer(x) :- R(x), S(x);
        relational {
            r = rename R {key->x}; s = rename S {key->x};
            reduced = semijoin r with s;
            output reduced as answer(x);
        }
    };
    let indexed = cq_compiler::compile_result_indexes(&source).unwrap();
    let mut function = cq_compiler::compile_materialized_callable(
        &indexed,
        &syn::parse_quote!(::mini_linq::collect_access),
    )
    .unwrap();
    lower_access(&mut function);
    compiled::run_rust(quote! {
        #[derive(Clone, Debug, PartialEq, Eq, Hash)] struct Key(String);
        fn main() {
            let left = [ (Key("a".into()),), (Key("a".into()),), (Key("b".into()),) ];
            let right = [(Key("a".into()),)];
            let answer = (#function)(left.iter().cloned(),right.iter().cloned());
            assert_eq!(answer, std::collections::HashSet::from([(Key("a".into()),)]));
            assert_eq!(left.len(),3);
        }
    });
}

#[test]
fn embedding_checks_input_arity_and_index_coverage() {
    let mut source = indexed();
    let access = syn::parse_quote!(::mini_linq::collect_access);
    let error = cq_compiler::compile_materialized_query(&source, &access, &[]).unwrap_err();
    assert!(error.to_string().contains("expected 2 input expressions"));
    source.indexes.clear();
    assert!(
        cq_compiler::compile_materialized_query(
            &source,
            &access,
            &[syn::parse_quote!(left), syn::parse_quote!(right)]
        )
        .is_err()
    );
}

#[test]
fn empty_and_composite_semijoin_keys_preserve_exact_existence() {
    let empty: ra::Module = syn::parse_quote! {
        struct EmptyKey;
        relation R(a:i32); relation S(b:i32);
        answer(a) :- R(a), S(b);
        relational {
            r = rename R {a->a}; s = rename S {b->b};
            reduced = semijoin r with s;
            output reduced as answer(a);
        }
    };
    let composite: ra::Module = syn::parse_quote! {
        struct Composite;
        relation R(a:i32,b:i32,p:i32); relation S(a:i32,b:i32,q:i32);
        answer(p) :- R(a,b,p), S(a,b,q);
        relational {
            r = rename R {a->a,b->b,p->p}; s = rename S {a->a,b->b,q->q};
            reduced = semijoin r with s;
            projected = project reduced keep {p};
            output projected as answer(p);
        }
    };
    let access = syn::parse_quote!(::mini_linq::collect_access);
    let mut empty_function = cq_compiler::compile_materialized_callable(
        &cq_compiler::compile_result_indexes(&empty).unwrap(),
        &access,
    )
    .unwrap();
    let mut composite_function = cq_compiler::compile_materialized_callable(
        &cq_compiler::compile_result_indexes(&composite).unwrap(),
        &access,
    )
    .unwrap();
    lower_access(&mut empty_function);
    lower_access(&mut composite_function);
    compiled::run_rust(quote! {
        fn main() {
            assert!((#empty_function)([(1,), (2,)], []).is_empty());
            assert_eq!((#empty_function)([(1,), (1,), (2,)], [(9,), (8,)]), std::collections::HashSet::from([(1,), (2,)]));
            assert!((#composite_function)([(1,2,3)], [(1,9,0), (8,2,0)]).is_empty());
            assert_eq!((#composite_function)([(1,2,3), (1,2,3)], [(1,2,0), (1,2,1)]), std::collections::HashSet::from([(3,)]));
        }
    });
}
