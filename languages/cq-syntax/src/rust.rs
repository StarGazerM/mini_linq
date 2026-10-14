//! Supplied lexical analysis for the Rust expressions embedded in CQ.
//! This does not resolve Rust names, expand macros, or decide evaluation order.

use std::collections::BTreeSet;

use syn::{
    Expr, Ident, Pat,
    visit::{self, Visit},
};

use crate::symbol_name;

/// Free value names visible in a parsed Rust expression.
///
/// A simple callee such as `accept(x)` contributes both `accept` and `x`.
/// Field names, method names, and qualified path components are not variables.
/// Rust still resolves names and distinguishes constants from pattern bindings.
///
/// If `opaque` is true, `names` is only a syntactic observation, not an exhaustive
/// free-variable set. Macros, local items, attributes, and verbatim AST nodes can
/// hide uses or introduce bindings. Use [`Self::dependencies`] for liveness.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FreeVariables {
    pub names: BTreeSet<String>,
    pub opaque: bool,
}

impl FreeVariables {
    /// Logical inputs that must be retained from the caller's environment.
    ///
    /// Opaque Rust retains the entire environment. An empty `names` set does
    /// not imply that an expression containing a macro has no dependencies.
    pub fn dependencies(&self, environment: &BTreeSet<String>) -> BTreeSet<String> {
        if self.opaque {
            environment.clone()
        } else {
            self.names.intersection(environment).cloned().collect()
        }
    }
}

/// Collect free names before intersecting them with any CQ environment.
///
/// ```
/// use cq_syntax::rust::free_variables;
/// let expr: syn::Expr = syn::parse_quote!({ let x = y; (|y| x + y)(z) });
/// let free = free_variables(&expr);
/// assert_eq!(free.names, ["y", "z"].map(String::from).into());
/// assert!(!free.opaque);
/// ```
pub fn free_variables(expr: &Expr) -> FreeVariables {
    let mut visitor = FreeVariableVisitor::default();
    visitor.visit_expr(expr);
    visitor.result
}

/// A bare logical-variable argument, as distinct from a Rust computation.
pub fn variable(expr: &Expr) -> Option<&Ident> {
    match expr {
        Expr::Path(path) if path.qself.is_none() => path.path.get_ident(),
        _ => None,
    }
}

/// Syntactic pattern bindings, in written order with raw names deduplicated.
/// Rust validates refutability and resolves ambiguous constant patterns later.
pub fn bindings(pattern: &Pat) -> Vec<Ident> {
    #[derive(Default)]
    struct Bindings(Vec<Ident>);
    impl<'ast> Visit<'ast> for Bindings {
        fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
            self.0.push(pattern.ident.clone());
            visit::visit_pat_ident(self, pattern);
        }
    }
    let mut visitor = Bindings::default();
    visitor.visit_pat(pattern);
    let mut seen = BTreeSet::new();
    visitor.0.retain(|id| seen.insert(symbol_name(id)));
    visitor.0
}

#[derive(Default)]
struct FreeVariableVisitor {
    result: FreeVariables,
    bound: BTreeSet<String>,
}

impl FreeVariableVisitor {
    fn bind(&mut self, pattern: &Pat) {
        self.visit_pat(pattern);
        self.bound.extend(bindings(pattern).iter().map(symbol_name));
    }

    fn scoped(&mut self, body: impl FnOnce(&mut Self)) {
        let outer = self.bound.clone();
        body(self);
        self.bound = outer;
    }

    fn condition(&mut self, expr: &Expr) {
        match expr {
            Expr::Let(expr) => {
                self.result.opaque |= !expr.attrs.is_empty();
                self.visit_expr(&expr.expr);
                self.bind(&expr.pat);
            }
            Expr::Binary(expr) if matches!(expr.op, syn::BinOp::And(_)) => {
                self.result.opaque |= !expr.attrs.is_empty();
                self.condition(&expr.left);
                self.condition(&expr.right);
            }
            _ => self.visit_expr(expr),
        }
    }
}

impl<'ast> Visit<'ast> for FreeVariableVisitor {
    fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
        if expr.qself.is_none()
            && expr.path.leading_colon.is_none()
            && expr.path.segments.len() == 1
        {
            let name = symbol_name(&expr.path.segments[0].ident);
            if !self.bound.contains(&name) {
                self.result.names.insert(name);
            }
        }
        // Const generic arguments may themselves contain value expressions.
        visit::visit_expr_path(self, expr);
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.scoped(|visitor| {
            for statement in &block.stmts {
                match statement {
                    syn::Stmt::Local(local) => {
                        visitor.result.opaque |= !local.attrs.is_empty();
                        if let Some(init) = &local.init {
                            visitor.visit_expr(&init.expr);
                            if let Some((_, diverge)) = &init.diverge {
                                visitor.visit_expr(diverge);
                            }
                        }
                        visitor.bind(&local.pat);
                    }
                    syn::Stmt::Item(_) => visitor.result.opaque = true,
                    _ => visitor.visit_stmt(statement),
                }
            }
        });
    }

    fn visit_expr_closure(&mut self, expr: &'ast syn::ExprClosure) {
        self.scoped(|visitor| {
            for pattern in &expr.inputs {
                visitor.bind(pattern);
            }
            visit::visit_expr_closure(visitor, expr);
        });
    }

    fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
        self.result.opaque |= !expr.attrs.is_empty();
        self.visit_expr(&expr.expr);
        self.scoped(|visitor| {
            visitor.bind(&expr.pat);
            visitor.visit_block(&expr.body);
        });
    }

    fn visit_expr_match(&mut self, expr: &'ast syn::ExprMatch) {
        self.result.opaque |= !expr.attrs.is_empty();
        self.visit_expr(&expr.expr);
        for arm in &expr.arms {
            self.scoped(|visitor| {
                visitor.result.opaque |= !arm.attrs.is_empty();
                visitor.bind(&arm.pat);
                if let Some((_, guard)) = &arm.guard {
                    visitor.condition(guard);
                }
                visitor.visit_expr(&arm.body);
            });
        }
    }

    fn visit_expr_if(&mut self, expr: &'ast syn::ExprIf) {
        self.result.opaque |= !expr.attrs.is_empty();
        self.scoped(|visitor| {
            visitor.condition(&expr.cond);
            visitor.visit_block(&expr.then_branch);
        });
        if let Some((_, otherwise)) = &expr.else_branch {
            self.visit_expr(otherwise);
        }
    }

    fn visit_expr_while(&mut self, expr: &'ast syn::ExprWhile) {
        self.result.opaque |= !expr.attrs.is_empty();
        self.scoped(|visitor| {
            visitor.condition(&expr.cond);
            visitor.visit_block(&expr.body);
        });
    }

    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {
        self.result.opaque = true;
    }

    fn visit_macro(&mut self, _: &'ast syn::Macro) {
        self.result.opaque = true;
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        self.result.opaque |= matches!(expr, Expr::Verbatim(_));
        visit::visit_expr(self, expr);
    }

    fn visit_pat(&mut self, pattern: &'ast Pat) {
        self.result.opaque |= matches!(pattern, Pat::Verbatim(_));
        visit::visit_pat(self, pattern);
    }

    fn visit_type(&mut self, ty: &'ast syn::Type) {
        self.result.opaque |= matches!(ty, syn::Type::Verbatim(_));
        visit::visit_type(self, ty);
    }
}

pub fn unparen(mut expr: &Expr) -> &Expr {
    while let Expr::Paren(p) = expr {
        expr = &p.expr;
    }
    expr
}

/// Visible logical uses for diagnostics; opaque Rust is resolved by rustc.
/// Liveness must use `free_variables(expr).dependencies(environment)` instead.
pub fn known_uses(expr: &Expr, logical_names: &BTreeSet<String>) -> BTreeSet<String> {
    let free = free_variables(expr);
    if free.opaque {
        BTreeSet::new()
    } else {
        free.names.intersection(logical_names).cloned().collect()
    }
}
