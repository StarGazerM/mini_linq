use super::*;
use syn::visit_mut::{self, VisitMut};

fn fixture(lazy: bool) -> (ir::Module, syn::File) {
    let cq: cq::Module = syn::parse_quote! {
        struct P;
        relation R(a:i32,b:i32,c:i32);
        relation S(a:i32,b:i32,c:i32);
        answer(x,y,z,w) :- R(x,y,z), S(y,x,w), let () = ();
    };
    let logical = crate::compile_cq(&cq).unwrap();
    let indexed = crate::compile_result_indexes(&logical).unwrap();
    let file = if lazy {
        crate::compile_index_requirements(&indexed)
    } else {
        crate::compile_materialized(&indexed)
    }
    .unwrap();
    (indexed, file)
}

fn accepts(source: &ir::Module, file: &syn::File, is_lazy: bool) -> bool {
    if is_lazy {
        lazy(source, file).is_ok()
    } else {
        materialized_with(
            source,
            file,
            &syn::parse_quote!(::mini_linq::collect_access),
        )
        .is_ok()
    }
}

fn build(file: &mut syn::File) -> &mut syn::Block {
    file.items
        .iter_mut()
        .filter_map(|item| match item {
            syn::Item::Impl(item) => Some(item),
            _ => None,
        })
        .flat_map(|item| item.items.iter_mut())
        .find_map(|item| match item {
            syn::ImplItem::Fn(method) if method.sig.ident == "build" => Some(&mut method.block),
            _ => None,
        })
        .unwrap()
}

#[test]
fn configured_access_path_does_not_weaken_storage_or_clause_checks() {
    let (source, _) = fixture(false);
    let path = syn::parse_quote!(custom::access);
    let mut file = crate::compile_materialized_with(&source, &path).unwrap();
    assert!(materialized_with(&source, &file, &path).is_ok());
    assert!(
        materialized_with(
            &source,
            &file,
            &syn::parse_quote!(::mini_linq::collect_access)
        )
        .is_err()
    );
    build(&mut file)
        .stmts
        .insert(3, syn::parse_quote!(input0.clear();));
    assert!(materialized_with(&source, &file, &path).is_err());
}

#[test]
fn index_writes_and_build_order_are_part_of_storage_correspondence() {
    for lazy in [false, true] {
        let (source, original) = fixture(lazy);
        let mut cleared = original.clone();
        let statements = &mut build(&mut cleared).stmts;
        let position = statements
            .iter()
            .position(|s| matches!(s, Stmt::Expr(Expr::ForLoop(_), _)))
            .unwrap();
        statements.insert(position + 1, syn::parse_quote!(index0.clear();));
        assert!(
            !accepts(&source, &cleared, lazy),
            "extra index write accepted"
        );

        // In the lazy route the build loop is already immediately before
        // handing storage to query(), so moving it there would be a no-op.
        if !lazy {
            let mut late = original.clone();
            let statements = &mut build(&mut late).stmts;
            let position = statements
                .iter()
                .position(|s| matches!(s, Stmt::Expr(Expr::ForLoop(_), _)))
                .unwrap();
            let loop_ = statements.remove(position);
            statements.insert(statements.len() - 1, loop_);
            assert_ne!(late, original);
            assert!(
                !accepts(&source, &late, lazy),
                "index population after its consumer accepted"
            );
        }
    }
}

struct ChangeColumn {
    method: &'static str,
    changed: bool,
}

impl VisitMut for ChangeColumn {
    fn visit_expr_method_call_mut(&mut self, call: &mut syn::ExprMethodCall) {
        if call.method == self.method && !self.changed {
            struct Field(bool);
            impl VisitMut for Field {
                fn visit_expr_field_mut(&mut self, field: &mut syn::ExprField) {
                    if !self.0 {
                        let syn::Member::Unnamed(index) = &mut field.member else {
                            return;
                        };
                        index.index = (index.index + 1) % 3;
                        self.0 = true;
                    }
                }
            }
            let mut field = Field(false);
            for argument in &mut call.args {
                field.visit_expr_mut(argument);
            }
            self.changed = field.0;
        }
        visit_mut::visit_expr_method_call_mut(self, call);
    }
}

#[test]
fn composite_key_and_payload_columns_are_verified_independently() {
    for lazy in [false, true] {
        let (source, original) = fixture(lazy);
        for method in ["entry", "push"] {
            let mut target = original.clone();
            let mut mutation = ChangeColumn {
                method,
                changed: false,
            };
            mutation.visit_file_mut(&mut target);
            assert!(mutation.changed);
            assert!(
                !accepts(&source, &target, lazy),
                "wrong {method} field accepted"
            );
        }
    }
}

#[test]
fn deleting_a_zero_binding_computation_is_detected_even_when_access_is_well_formed() {
    struct RemoveLet(bool);
    impl VisitMut for RemoveLet {
        fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
            if self.0 {
                return;
            }
            let Some(mut plan) = pull_macro(mac)
                .or_else(|| access_macro(mac, &syn::parse_quote!(::mini_linq::collect_access)))
            else {
                return;
            };
            if !plan
                .body
                .iter()
                .any(|c| matches!(c.item, cq::BodyItem::Let(_)))
            {
                return;
            }
            let mut removed = false;
            plan.body = plan
                .body
                .into_iter()
                .filter(|c| {
                    if !removed && matches!(c.item, cq::BodyItem::Let(_)) {
                        removed = true;
                        false
                    } else {
                        true
                    }
                })
                .collect();
            ac::contract::check(&plan).unwrap();
            mac.tokens = quote!(#plan);
            self.0 = true;
        }
    }
    for lazy in [false, true] {
        let (source, mut target) = fixture(lazy);
        let mut mutation = RemoveLet(false);
        mutation.visit_file_mut(&mut target);
        assert!(mutation.0);
        assert!(!accepts(&source, &target, lazy));
    }
}

#[test]
fn wrong_operand_alias_and_storage_handoff_are_rejected() {
    struct WrongOperand(bool);
    impl VisitMut for WrongOperand {
        fn visit_local_mut(&mut self, local: &mut syn::Local) {
            if !self.0 && local_name(local).is_some_and(|n| n == "__left") {
                *local.init.as_mut().unwrap().expr = syn::parse_quote!(&input1);
                self.0 = true;
            }
            visit_mut::visit_local_mut(self, local);
        }
    }
    let (source, mut target) = fixture(false);
    let mut mutation = WrongOperand(false);
    mutation.visit_file_mut(&mut target);
    assert!(mutation.0);
    assert!(!accepts(&source, &target, false));

    let (source, mut target) = fixture(true);
    let Stmt::Expr(Expr::Struct(handoff), None) = build(&mut target).stmts.last_mut().unwrap()
    else {
        panic!("storage handoff");
    };
    let a = handoff.fields[0].expr.clone();
    handoff.fields[0].expr = handoff.fields[1].expr.clone();
    handoff.fields[1].expr = a;
    assert!(!accepts(&source, &target, true));
}

#[test]
fn materialized_answer_uses_the_selected_result_and_head_layout() {
    let (source, mut target) = fixture(false);
    let answer = build(&mut target)
        .stmts
        .iter_mut()
        .find_map(|statement| match statement {
            Stmt::Local(local) if local_name(local).is_some_and(|n| n == "answer") => Some(local),
            _ => None,
        })
        .unwrap();
    let mut mutation = ChangeColumn {
        method: "map",
        changed: false,
    };
    mutation.visit_expr_mut(&mut answer.init.as_mut().unwrap().expr);
    assert!(mutation.changed);
    assert!(!accepts(&source, &target, false));
}

#[test]
fn base_and_result_indexes_with_the_same_name_have_separate_representations() {
    let source: cq::Module = syn::parse_quote! {
        struct P;
        relation R(a:i32,b:i32);
        relation S(a:i32,b:i32);
        answer(x,y,z) :- S(x,y), R(y,z), if x>0;
    };
    let mut logical = crate::compile_cq(&source).unwrap();
    let renamed = logical
        .plan
        .definitions
        .iter_mut()
        .find(|d| matches!(&d.operator, ra::Operator::Rename(r) if r.relation == "R"))
        .unwrap();
    let old_name = renamed.result.clone();
    renamed.result = format_ident!("R");
    for d in &mut logical.plan.definitions {
        if let ra::Operator::NaturalJoin(j) = &mut d.operator {
            if j.right == old_name {
                j.right = format_ident!("R");
            }
        }
    }
    let indexed: ir::Module = syn::parse_quote! {
        #logical indexes { @R[0]; R[0]; }
    };
    ir::contract::check(&indexed).unwrap();
    let target = crate::compile_materialized(&indexed).unwrap();
    materialized_with(
        &indexed,
        &target,
        &syn::parse_quote!(::mini_linq::collect_access),
    )
    .unwrap();
}

#[test]
fn scaffold_types_cannot_redirect_storage_method_dispatch() {
    struct TypedAlias(bool);
    impl VisitMut for TypedAlias {
        fn visit_local_mut(&mut self, local: &mut syn::Local) {
            if !self.0 && local_name(local).is_some_and(|n| n == "__left") {
                let typed: Stmt = syn::parse_quote!(let __left: &dyn EmptyRows = &input0;);
                let Stmt::Local(typed) = typed else {
                    unreachable!()
                };
                local.pat = typed.pat;
                self.0 = true;
            }
            visit_mut::visit_local_mut(self, local);
        }
    }
    let (source, mut target) = fixture(false);
    let mut mutation = TypedAlias(false);
    mutation.visit_file_mut(&mut target);
    assert!(mutation.0);
    assert!(!accepts(&source, &target, false));

    let (source, mut target) = fixture(true);
    let index = build(&mut target)
        .stmts
        .iter_mut()
        .find_map(|s| match s {
            Stmt::Local(local) if local_name(local).is_some_and(|n| n == "index0") => Some(local),
            _ => None,
        })
        .unwrap();
    let syn::Pat::Type(pattern) = &mut index.pat else {
        panic!("typed lazy index");
    };
    let syn::Type::Path(path) = pattern.ty.as_mut() else {
        panic!("map type");
    };
    let syn::PathArguments::AngleBracketed(arguments) =
        &mut path.path.segments.last_mut().unwrap().arguments
    else {
        panic!("map arguments");
    };
    arguments.args[1] = syn::parse_quote!(CustomBucket);
    assert!(!accepts(&source, &target, true));
}
