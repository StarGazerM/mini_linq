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
