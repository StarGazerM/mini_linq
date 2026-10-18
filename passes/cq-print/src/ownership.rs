//! Move shared dispatch state into each closure that needs it. Nested closures
//! get their own handles, so repeated probes never move out of an outer closure.
use super::*;
use syn::visit::Visit;
use syn::visit_mut::VisitMut;

pub fn own_captures(expression: &mut Expr, owned: &[Ident]) {
    struct Captures<'a> {
        owned: &'a [Ident],
        used: Vec<Ident>,
    }
    impl<'ast> Visit<'ast> for Captures<'_> {
        fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
            if let Some(ident) = path.path.get_ident() {
                if self.owned.contains(ident) && !self.used.contains(ident) {
                    self.used.push(ident.clone());
                }
            }
        }
    }
    struct Owner<'a>(&'a [Ident]);
    impl VisitMut for Owner<'_> {
        fn visit_expr_mut(&mut self, expression: &mut Expr) {
            syn::visit_mut::visit_expr_mut(self, expression);
            if let Expr::Closure(closure) = expression {
                if closure.capture.is_none() {
                    return;
                }
                let mut captures = Captures {
                    owned: self.0,
                    used: Vec::new(),
                };
                captures.visit_expr(&closure.body);
                if !captures.used.is_empty() {
                    let used = captures.used;
                    *expression = syn::parse_quote!({
                        #(let #used = ::core::clone::Clone::clone(&#used);)*
                        #closure
                    });
                }
            }
        }
    }
    Owner(owned).visit_expr_mut(expression);
}

/// Borrow visible generator captures before fused `move` closures are formed.
/// Rewriting only free expression paths preserves Rust value types and local
/// shadowing; generated iterator items themselves are never cloned here.
pub(super) fn generator_captures(
    module: &structures::Program,
    analysis: &structures::Analysis,
    statements: &mut Vec<Stmt>,
    used: &mut std::collections::BTreeSet<String>,
) -> BTreeMap<String, Expr> {
    use structures::syntax::rust;
    use syn::visit_mut::{self, VisitMut};

    struct Borrow<'a> {
        free: std::collections::BTreeSet<String>,
        statements: &'a mut Vec<Stmt>,
        used: &'a mut std::collections::BTreeSet<String>,
    }
    impl Borrow<'_> {
        fn bind(&mut self, pattern: &syn::Pat) {
            for name in rust::bindings(pattern) {
                self.free.remove(&symbol_name(&name));
            }
        }
        fn scoped(&mut self, visit: impl FnOnce(&mut Self)) {
            let free = self.free.clone();
            visit(self);
            self.free = free;
        }
        fn condition(&mut self, expression: &mut Expr) {
            match expression {
                Expr::Let(value) => {
                    self.visit_expr_mut(&mut value.expr);
                    self.bind(&value.pat);
                }
                Expr::Binary(value) if matches!(value.op, syn::BinOp::And(_)) => {
                    self.condition(&mut value.left);
                    self.condition(&mut value.right);
                }
                _ => self.visit_expr_mut(expression),
            }
        }
    }
    impl VisitMut for Borrow<'_> {
        fn visit_expr_mut(&mut self, expression: &mut Expr) {
            if let Expr::Path(path) = expression {
                if path.qself.is_none() {
                    if let Some(name) = path.path.get_ident() {
                        let key = symbol_name(name);
                        if self.free.contains(&key) {
                            // Keep generic function/constructor occurrences
                            // independently inferred rather than sharing one
                            // monomorphic alias for their spelling.
                            let mut n = self.used.len();
                            while self.used.contains(&format!("__cq_generator_capture_{n}")) {
                                n += 1;
                            }
                            let alias = format_ident!(
                                "__cq_generator_capture_{n}",
                                span = proc_macro2::Span::mixed_site()
                            );
                            self.used.insert(alias.to_string());
                            self.statements
                                .push(syn::parse_quote!(let #alias = &#name;));
                            let attributes = &path.attrs;
                            *expression = syn::parse_quote!(#(#attributes)* (*#alias));
                            return;
                        }
                    }
                }
            }
            visit_mut::visit_expr_mut(self, expression);
        }
        fn visit_block_mut(&mut self, block: &mut syn::Block) {
            self.scoped(|visitor| {
                for statement in &mut block.stmts {
                    match statement {
                        Stmt::Local(local) => {
                            if let Some(init) = &mut local.init {
                                visitor.visit_expr_mut(&mut init.expr);
                                if let Some((_, diverge)) = &mut init.diverge {
                                    visitor.visit_expr_mut(diverge);
                                }
                            }
                            visitor.bind(&local.pat);
                        }
                        Stmt::Item(_) => {}
                        _ => visitor.visit_stmt_mut(statement),
                    }
                }
            });
        }
        fn visit_expr_closure_mut(&mut self, expression: &mut syn::ExprClosure) {
            self.scoped(|visitor| {
                for input in &expression.inputs {
                    visitor.bind(input);
                }
                visitor.visit_expr_mut(&mut expression.body);
            });
        }
        fn visit_expr_for_loop_mut(&mut self, expression: &mut syn::ExprForLoop) {
            self.visit_expr_mut(&mut expression.expr);
            self.scoped(|visitor| {
                visitor.bind(&expression.pat);
                visitor.visit_block_mut(&mut expression.body);
            });
        }
        fn visit_expr_match_mut(&mut self, expression: &mut syn::ExprMatch) {
            self.visit_expr_mut(&mut expression.expr);
            for arm in &mut expression.arms {
                self.scoped(|visitor| {
                    visitor.bind(&arm.pat);
                    if let Some((_, guard)) = &mut arm.guard {
                        visitor.condition(guard);
                    }
                    visitor.visit_expr_mut(&mut arm.body);
                });
            }
        }
        fn visit_expr_if_mut(&mut self, expression: &mut syn::ExprIf) {
            self.scoped(|visitor| {
                visitor.condition(&mut expression.cond);
                visitor.visit_block_mut(&mut expression.then_branch);
            });
            if let Some((_, otherwise)) = &mut expression.else_branch {
                self.visit_expr_mut(otherwise);
            }
        }
        fn visit_expr_while_mut(&mut self, expression: &mut syn::ExprWhile) {
            self.scoped(|visitor| {
                visitor.condition(&mut expression.cond);
                visitor.visit_block_mut(&mut expression.body);
            });
        }
    }

    let mut expressions = BTreeMap::new();
    for definition in &module.physical.program.definitions {
        let Operator::Bind(bind) = &definition.operator else {
            continue;
        };
        if bind.for_token.is_none() {
            continue;
        }
        let environment = bind
            .environment
            .as_ref()
            .map(|environment| environment.variables.iter().map(symbol_name).collect())
            .unwrap_or_else(|| {
                analysis.layouts[&symbol_name(&bind.input)]
                    .iter()
                    .cloned()
                    .collect()
            });
        let free = rust::free_variables(&bind.expression)
            .names
            .difference(&environment)
            .cloned()
            .collect();
        let mut expression = bind.expression.clone();
        Borrow {
            free,
            statements,
            used,
        }
        .visit_expr_mut(&mut expression);
        expressions.insert(symbol_name(&definition.name), expression);
    }
    expressions
}
