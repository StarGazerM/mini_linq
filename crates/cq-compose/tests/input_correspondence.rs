use cq_ir::{cq, index_requirements as ir, rust_access_plan as ac};
use quote::quote;
use syn::visit_mut::{self, VisitMut};
use syn::{Expr, Stmt};

fn fixture() -> ir::Module {
    let source: cq::Module = syn::parse_quote! {
        struct BasicContract;
        relation R(a:i32,b:i32);
        relation S(a:i32,b:i32);
        relation T(a:i32,b:i32);
        relation Unused(a:i32,b:i32);
        answer(x,y,z) :- R(x,y), S(y,z), T(z,x);
    };
    let ra = cq_compose::compile_cq(&source).unwrap();
    let mut indexed = cq_index::input::compile(&ra).unwrap();
    for extra in [
        syn::parse_quote!(R[0]),
        syn::parse_quote!(R[1]),
        syn::parse_quote!(S[1]),
    ] {
        indexed.indexes.push(extra);
    }
    indexed
}

fn replace_expression(target: &mut syn::File, original: Expr, replacement: Expr) {
    struct Replace {
        original: Expr,
        replacement: Expr,
        count: usize,
    }
    impl VisitMut for Replace {
        fn visit_expr_mut(&mut self, expression: &mut Expr) {
            if *expression == self.original {
                *expression = self.replacement.clone();
                self.count += 1;
            } else {
                visit_mut::visit_expr_mut(self, expression);
            }
        }
    }
    let mut visitor = Replace {
        original,
        replacement,
        count: 0,
    };
    visitor.visit_file_mut(target);
    assert!(
        visitor.count > 0,
        "mutation did not reach a generated expression"
    );
}

fn mutate_access(target: &mut syn::File, change: impl FnOnce(&mut ac::Plan)) {
    for item in &mut target.items {
        let syn::Item::Impl(item) = item else {
            continue;
        };
        for method in &mut item.items {
            let syn::ImplItem::Fn(method) = method else {
                continue;
            };
            if method.sig.ident != "query" {
                continue;
            }
            let mac = match method.block.stmts.first_mut().unwrap() {
                Stmt::Expr(Expr::Macro(mac), None) => &mut mac.mac,
                Stmt::Macro(mac) => &mut mac.mac,
                _ => panic!("query lost its Access region"),
            };
            let mut plan = access_macro(mac, &syn::parse_quote!(::mini_linq::pull)).unwrap();
            change(&mut plan);
            mac.tokens = quote!(#plan);
            return;
        }
    }
    panic!("missing query method");
}

#[test]
fn base_storage_contract_checks_keys_payloads_and_ownership_schedule() {
    let source = fixture();
    let target =
        cq_physical::input::compile_input_indexed(&source, &syn::parse_quote!(::mini_linq::pull))
            .unwrap();
    cq_physical::input::check_input_indexed(
        &source,
        &target,
        &syn::parse_quote!(::mini_linq::pull),
    )
    .unwrap();
    for (original, replacement) in [
        (
            syn::parse_quote!(input1.into_iter().collect()),
            syn::parse_quote!(input0.into_iter().collect()),
        ),
        (
            syn::parse_quote!(::core::clone::Clone::clone(index0_column0)),
            syn::parse_quote!(::core::clone::Clone::clone(index0_column1)),
        ),
        (
            syn::parse_quote!(::core::clone::Clone::clone(index0_column1)),
            syn::parse_quote!(::core::clone::Clone::clone(index0_column0)),
        ),
        (
            syn::parse_quote!(relation1.iter()),
            syn::parse_quote!(relation1.into_iter()),
        ),
        (
            syn::parse_quote!(relation1.into_iter()),
            syn::parse_quote!(relation1.iter()),
        ),
        (
            syn::parse_quote!(BasicContractStorage {
                rows0,
                index0,
                index1,
                index2,
                index3,
                index4
            }),
            syn::parse_quote!(BasicContractStorage {
                rows0,
                index0: index4,
                index1,
                index2,
                index3,
                index4
            }),
        ),
    ] {
        let mut invalid = target.clone();
        replace_expression(&mut invalid, original, replacement);
        assert!(
            cq_physical::input::check_input_indexed(
                &source,
                &invalid,
                &syn::parse_quote!(::mini_linq::pull)
            )
            .is_err()
        );
    }
    let mut full = source;
    full.indexes.push(syn::parse_quote!(R[0,1]));
    let target =
        cq_physical::input::compile_input_indexed(&full, &syn::parse_quote!(::mini_linq::pull))
            .unwrap();
    cq_physical::input::check_input_indexed(&full, &target, &syn::parse_quote!(::mini_linq::pull))
        .unwrap();
}

#[test]
fn base_access_contract_rejects_wrong_stores_keys_and_missing_occurrences() {
    let source = fixture();
    let target =
        cq_physical::input::compile_input_indexed(&source, &syn::parse_quote!(::mini_linq::pull))
            .unwrap();
    for mutation in 0..4 {
        let mut invalid = target.clone();
        mutate_access(&mut invalid, |plan| match mutation {
            0 => {
                let ac::RustAccess::For { source, .. } = &mut plan.body[0].access else {
                    unreachable!()
                };
                *source = syn::parse_quote!(self.index1.iter());
            }
            1 => {
                let ac::RustAccess::For { source, .. } = &mut plan.body[1].access else {
                    unreachable!()
                };
                *source = syn::parse_quote!(
                    self.index0
                        .get(&(::core::clone::Clone::clone(x),))
                        .into_iter()
                        .flatten()
                );
            }
            2 => {
                plan.body = plan
                    .body
                    .iter()
                    .take(plan.body.len() - 1)
                    .cloned()
                    .collect();
            }
            _ => {
                plan.head.output = syn::parse_quote!((
                    ::core::clone::Clone::clone(x),
                    ::core::clone::Clone::clone(x),
                    ::core::clone::Clone::clone(z),
                ));
            }
        });
        ac::contract::check(&pull_from_file(&invalid)).unwrap();
        assert!(
            cq_physical::input::check_input_indexed(
                &source,
                &invalid,
                &syn::parse_quote!(::mini_linq::pull)
            )
            .is_err()
        );
    }
}

fn pull_from_file(target: &syn::File) -> ac::Plan {
    let query = method(target, &syn::parse_quote!(BasicContractStorage), "query").unwrap();
    match query.block.stmts.first().unwrap() {
        Stmt::Expr(expression, None) => {
            access_expression(expression, &syn::parse_quote!(::mini_linq::pull)).unwrap()
        }
        Stmt::Macro(mac) => access_macro(&mac.mac, &syn::parse_quote!(::mini_linq::pull)).unwrap(),
        _ => panic!("missing Access region"),
    }
}

fn access_macro(mac: &syn::Macro, path: &syn::Path) -> Option<ac::Plan> {
    if mac.path != *path {
        return None;
    }
    syn::parse2(mac.tokens.clone()).ok()
}
fn access_expression(expr: &Expr, path: &syn::Path) -> Option<ac::Plan> {
    let Expr::Macro(mac) = expr else {
        return None;
    };
    access_macro(&mac.mac, path)
}
fn method<'a>(file: &'a syn::File, owner: &syn::Ident, name: &str) -> Option<&'a syn::ImplItemFn> {
    file.items.iter().find_map(|item| {
        let syn::Item::Impl(item) = item else {
            return None;
        };
        if item.trait_.is_some() || *item.self_ty != syn::parse_quote!(#owner) {
            return None;
        }
        item.items.iter().find_map(|item| match item {
            syn::ImplItem::Fn(method) if method.sig.ident == name => Some(method),
            _ => None,
        })
    })
}
