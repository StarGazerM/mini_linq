//! Rust syntax embedded in CQ. This module analyzes lexical uses, not Rust
//! types or effects. Opaque syntax conservatively keeps the whole environment.
use crate::symbol_name;
use proc_macro2::{Ident, TokenStream, TokenTree};
use std::collections::BTreeSet;
use syn::{
    Expr, Pat,
    parse::{ParseStream, Parser},
    visit::{self, Visit},
};

pub fn variable(expr: &Expr) -> Option<&Ident> {
    match expr {
        Expr::Path(path) if path.qself.is_none() => path.path.get_ident(),
        _ => None,
    }
}

pub fn unparen(mut expr: &Expr) -> &Expr {
    while let Expr::Paren(p) = expr {
        expr = &p.expr;
    }
    expr
}

/// Let Syn parse the Rust grammar, including closure and turbofish commas.
/// In the scratch stream only, shield the enclosing DSL's final period from
/// Rust's field-access parser. The original expression tokens keep their spans.
pub fn parse_expression(input: ParseStream<'_>) -> syn::Result<Expr> {
    let mut tokens: Vec<_> = input.fork().parse::<TokenStream>()?.into_iter().collect();
    let count = tokens.len();
    for i in 0..count {
        let end = i + 1 == count
            || matches!(tokens.get(i + 1), Some(TokenTree::Ident(id)) if id == "relational" || id == "indexes");
        if end && matches!(&tokens[i], TokenTree::Punct(p) if p.as_char() == '.') {
            let mut semi = proc_macro2::Punct::new(';', proc_macro2::Spacing::Alone);
            semi.set_span(tokens[i].span());
            tokens[i] = TokenTree::Punct(semi);
        }
    }
    let (expr, remaining) = (|stream: ParseStream<'_>| {
        let expr: Expr = stream.parse()?;
        let remaining: TokenStream = stream.parse()?;
        Ok((expr, remaining.into_iter().count()))
    })
    .parse2(tokens.into_iter().collect())?;
    for _ in 0..count - remaining {
        input.parse::<TokenTree>()?;
    }
    Ok(expr)
}

pub fn bindings(pattern: &Pat) -> Vec<Ident> {
    struct Bindings(Vec<Ident>);
    impl<'ast> Visit<'ast> for Bindings {
        fn visit_pat_ident(&mut self, p: &'ast syn::PatIdent) {
            self.0.push(p.ident.clone());
            visit::visit_pat_ident(self, p);
        }
    }
    let mut result = Bindings(Vec::new());
    result.visit_pat(pattern);
    let mut seen = BTreeSet::new();
    result.0.retain(|id| seen.insert(symbol_name(id)));
    result.0
}

#[derive(Default)]
struct Uses {
    free: BTreeSet<String>,
    bound: BTreeSet<String>,
    opaque: bool,
}
impl Uses {
    fn bind(&mut self, pat: &Pat) {
        self.bound.extend(bindings(pat).iter().map(symbol_name));
    }
    fn scoped(&mut self, f: impl FnOnce(&mut Self)) {
        let saved = self.bound.clone();
        f(self);
        self.bound = saved;
    }
    fn condition(&mut self, expr: &Expr) {
        match expr {
            Expr::Let(e) => {
                self.visit_expr(&e.expr);
                self.bind(&e.pat);
            }
            Expr::Binary(e) if matches!(e.op, syn::BinOp::And(_)) => {
                self.condition(&e.left);
                self.condition(&e.right);
            }
            _ => self.visit_expr(expr),
        }
    }
}
impl<'ast> Visit<'ast> for Uses {
    fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
        if p.qself.is_none() {
            if let Some(id) = p.path.get_ident() {
                let name = symbol_name(id);
                if !self.bound.contains(&name) {
                    self.free.insert(name);
                }
            }
        }
    }
    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.scoped(|s| {
            for stmt in &block.stmts {
                match stmt {
                    syn::Stmt::Local(local) => {
                        if let Some(init) = &local.init {
                            s.visit_expr(&init.expr);
                            if let Some((_, diverge)) = &init.diverge {
                                s.visit_expr(diverge);
                            }
                        }
                        s.bind(&local.pat);
                    }
                    syn::Stmt::Item(_) => {
                        s.opaque = true;
                    }
                    _ => s.visit_stmt(stmt),
                }
            }
        });
    }
    fn visit_expr_closure(&mut self, e: &'ast syn::ExprClosure) {
        self.scoped(|s| {
            for p in &e.inputs {
                s.bind(p);
            }
            s.visit_expr(&e.body);
        });
    }
    fn visit_expr_for_loop(&mut self, e: &'ast syn::ExprForLoop) {
        self.visit_expr(&e.expr);
        self.scoped(|s| {
            s.bind(&e.pat);
            s.visit_block(&e.body);
        });
    }
    fn visit_expr_match(&mut self, e: &'ast syn::ExprMatch) {
        self.visit_expr(&e.expr);
        for arm in &e.arms {
            self.scoped(|s| {
                s.bind(&arm.pat);
                if let Some((_, guard)) = &arm.guard {
                    s.visit_expr(guard);
                }
                s.visit_expr(&arm.body);
            });
        }
    }
    fn visit_expr_if(&mut self, e: &'ast syn::ExprIf) {
        self.scoped(|s| {
            s.condition(&e.cond);
            s.visit_block(&e.then_branch);
        });
        if let Some((_, other)) = &e.else_branch {
            self.visit_expr(other);
        }
    }
    fn visit_expr_while(&mut self, e: &'ast syn::ExprWhile) {
        self.scoped(|s| {
            s.condition(&e.cond);
            s.visit_block(&e.body);
        });
    }
    fn visit_macro(&mut self, _: &'ast syn::Macro) {
        self.opaque = true;
    }
    fn visit_expr(&mut self, expr: &'ast Expr) {
        if matches!(expr, Expr::Verbatim(_)) {
            self.opaque = true;
        }
        visit::visit_expr(self, expr);
    }
}

pub fn uses(expr: &Expr, environment: &BTreeSet<String>) -> BTreeSet<String> {
    let mut uses = Uses::default();
    uses.visit_expr(expr);
    if uses.opaque {
        environment.clone()
    } else {
        uses.free.intersection(environment).cloned().collect()
    }
}

/// Statically visible uses only, for scope diagnostics. Opaque macro syntax is
/// retained for Rust to resolve; it must still retain the full available
/// environment when computing liveness with `uses`.
pub fn known_uses(expr: &Expr, logical_names: &BTreeSet<String>) -> BTreeSet<String> {
    let mut uses = Uses::default();
    uses.visit_expr(expr);
    uses.free.intersection(logical_names).cloned().collect()
}

pub fn parse_binding_pattern(input: ParseStream<'_>) -> syn::Result<Pat> {
    let pat = Pat::parse_single(input)?;
    if input.peek(syn::Token![:]) {
        Ok(Pat::Type(syn::PatType {
            attrs: Vec::new(),
            pat: Box::new(pat),
            colon_token: input.parse()?,
            ty: input.parse()?,
        }))
    } else {
        Ok(pat)
    }
}

pub fn binding_types(pattern: &Pat) -> Vec<(Ident, syn::Type)> {
    let mut vars: Vec<_> = bindings(pattern)
        .into_iter()
        .map(|id| (id, syn::parse_quote!(_)))
        .collect();
    if let Pat::Type(p) = pattern {
        if vars.len() == 1 && matches!(*p.pat, Pat::Ident(_)) {
            vars[0].1 = (*p.ty).clone();
        }
    }
    vars
}

pub fn basic(source: &super::Module) -> bool {
    let bound: BTreeSet<_> = source
        .program
        .query
        .body
        .iter()
        .filter_map(super::BodyItem::atom)
        .flat_map(super::Atom::variables)
        .map(symbol_name)
        .collect();
    source.program.query.head.is_normalized()
        && source
            .program
            .query
            .head
            .variables()
            .all(|id| bound.contains(&symbol_name(id)))
        && source
            .program
            .query
            .body
            .iter()
            .all(|item| matches!(item, super::BodyItem::Positive { atom } if atom.is_normalized()))
}

pub fn reserved(source: &super::Module) -> BTreeSet<String> {
    use quote::ToTokens;
    fn visit(tokens: TokenStream, names: &mut BTreeSet<String>) {
        for token in tokens {
            match token {
                TokenTree::Ident(id) => {
                    names.insert(symbol_name(&id));
                }
                TokenTree::Group(g) => visit(g.stream(), names),
                _ => (),
            }
        }
    }
    let mut names = BTreeSet::new();
    visit(source.to_token_stream(), &mut names);
    names
}

pub fn fresh(names: &mut BTreeSet<String>, clause: usize, arg: usize) -> Ident {
    let mut name = format!("__cq_{clause}_{arg}");
    while !names.insert(name.clone()) {
        name.push('_');
    }
    quote::format_ident!("{name}")
}

pub fn comparison(expr: &Expr) -> Option<super::Predicate> {
    use quote::ToTokens;
    syn::parse2(unparen(expr).to_token_stream()).ok()
}

/// Syntactically known primitive type; aliases remain for Rust to resolve.
pub fn primitive_i32(ty: &syn::Type) -> bool {
    use quote::ToTokens;
    matches!(ty, syn::Type::Path(p) if p.qself.is_none() && matches!(p.path.to_token_stream().to_string().replace(' ', "").as_str(), "i32" | "::core::primitive::i32" | "core::primitive::i32" | "::std::primitive::i32" | "std::primitive::i32"))
}

/// Closed literal expressions have no logical inputs or user computation.
pub fn literal(expr: &Expr) -> bool {
    match unparen(expr) {
        Expr::Lit(_) => true,
        Expr::Unary(e) if matches!(e.op, syn::UnOp::Neg(_)) => {
            matches!(unparen(&e.expr), Expr::Lit(_))
        }
        _ => false,
    }
}

pub fn literal_binding(item: &super::BodyItem) -> bool {
    let super::BodyItem::Let(binding) = item else {
        return false;
    };
    let pat = match &binding.pattern {
        Pat::Type(p) => &*p.pat,
        p => p,
    };
    matches!(pat, Pat::Ident(p) if p.by_ref.is_none() && p.subpat.is_none())
        && literal(&binding.expression)
}

/// Sufficient purity premises for scheduling inside a Rust evaluation boundary.
/// Relational equality follows the ordinary Eq/Hash set contract.
pub fn movable(source: &super::Module, item: &super::BodyItem) -> bool {
    match item {
        super::BodyItem::Positive { atom } => atom
            .args
            .iter()
            .all(|arg| variable(arg).is_some() || matches!(arg, Expr::Infer(_)) || literal(arg)),
        super::BodyItem::Filter(filter) => total_comparison(source, &filter.condition),
        super::BodyItem::Let(_) => literal_binding(item),
    }
}

fn total_comparison(source: &super::Module, expression: &Expr) -> bool {
    let Some(predicate) = comparison(expression) else {
        return false;
    };
    let mut types = std::collections::BTreeMap::<String, bool>::new();
    for item in &source.program.query.body {
        match item {
            super::BodyItem::Positive { atom } => {
                let Some(declaration) = source
                    .program
                    .inputs
                    .iter()
                    .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
                else {
                    return false;
                };
                for (arg, column) in atom.args.iter().zip(&declaration.columns) {
                    if let Some(id) = variable(arg) {
                        *types.entry(symbol_name(id)).or_insert(true) &= primitive_i32(&column.ty);
                    }
                }
            }
            super::BodyItem::Let(binding) => {
                for (id, ty) in binding_types(&binding.pattern) {
                    types.insert(symbol_name(&id), primitive_i32(&ty));
                }
            }
            super::BodyItem::Filter(_) => (),
        }
    }
    predicate
        .variables()
        .iter()
        .all(|v| types.get(v) == Some(&true))
}
