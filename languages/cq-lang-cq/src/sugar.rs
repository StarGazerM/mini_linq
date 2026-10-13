//! Multiple rules elaborate hygienically to one top-level block. Every source
//! branch gets its own local names and a positional head projection; only the
//! fresh head columns are shared. Opaque Rust keeps its tokens inside a lexical
//! alias context rather than rewriting identifiers hidden inside macro syntax.

use crate::{Atom, Block, BodyItem, Branch, Program, Query, blocks, rust, symbol_name};
use proc_macro2::{Span, TokenTree};
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Expr, Ident,
    visit_mut::{self, VisitMut},
};

struct Fresh(BTreeSet<String>);
impl Fresh {
    fn of(tokens: proc_macro2::TokenStream) -> Self {
        fn visit(tokens: proc_macro2::TokenStream, names: &mut BTreeSet<String>) {
            for token in tokens {
                match token {
                    TokenTree::Ident(id) => {
                        names.insert(symbol_name(&id));
                    }
                    TokenTree::Group(group) => visit(group.stream(), names),
                    _ => (),
                }
            }
        }
        let mut names = BTreeSet::new();
        visit(tokens, &mut names);
        Self(names)
    }
    fn name(&mut self, role: &str) -> Ident {
        let mut name = format!("__cq_{role}");
        while !self.0.insert(name.clone()) {
            name.push('_');
        }
        format_ident!("{name}", span = Span::mixed_site())
    }
}

pub(super) fn combine(program: &mut Program, queries: Vec<Query>) -> syn::Result<()> {
    for query in &queries {
        if symbol_name(&query.head.relation) != symbol_name(&program.query.head.relation)
            || query.head.args.len() != program.query.head.args.len()
        {
            return Err(syn::Error::new_spanned(
                &query.head,
                "multiple rules must have the same head relation and arity",
            ));
        }
    }
    let mut fresh = Fresh::of(
        program
            .to_token_stream()
            .into_iter()
            .chain(queries.iter().flat_map(|query| query.to_token_stream()))
            .collect(),
    );
    let head_columns: Vec<_> = (0..program.query.head.args.len())
        .map(|i| fresh.name(&format!("answer_{i}")))
        .collect();
    // Variable-only heads with the same positional equality pattern are
    // relational renamings. Preserve that fact: no synthetic Rust lets.
    let shape = |query: &Query| -> Option<Vec<usize>> {
        let bound = blocks::bound(&query.body);
        let mut names = Vec::new();
        let mut shape = Vec::new();
        for expression in &query.head.args {
            let id = rust::variable(expression)?;
            let name = symbol_name(id);
            if !bound.contains_key(&name) {
                return None;
            }
            let position = names
                .iter()
                .position(|old| old == &name)
                .unwrap_or_else(|| {
                    let position = names.len();
                    names.push(name);
                    position
                });
            shape.push(position);
        }
        Some(shape)
    };
    if let Some(head_shape) = shape(&queries[0]).filter(|first| {
        queries
            .iter()
            .all(|query| shape(query).as_ref() == Some(first))
    }) {
        let mut branches = Vec::new();
        for (j, query) in queries.iter().enumerate() {
            let mut names: BTreeMap<_, _> = blocks::walk(&query.body)
                .into_iter()
                .flat_map(|item| match item {
                    BodyItem::Positive { atom } => atom.variables().cloned().collect(),
                    BodyItem::Let(binding) => rust::bindings(&binding.pattern),
                    _ => Vec::new(),
                })
                .map(|id| {
                    (
                        symbol_name(&id),
                        fresh.name(&format!("rule_{j}_{}", symbol_name(&id))),
                    )
                })
                .collect();
            for (i, expression) in query.head.args.iter().enumerate() {
                names.insert(
                    symbol_name(rust::variable(expression).expect("variable head")),
                    head_columns[head_shape[i]].clone(),
                );
            }
            let mut body = query.body.clone();
            rename_body(&mut body, &names, &BTreeMap::new());
            branches.push(Branch {
                brace: Default::default(),
                body,
            });
        }
        program.query.head.args = head_shape
            .iter()
            .map(|i| -> Expr {
                let id = &head_columns[*i];
                syn::parse_quote!(#id)
            })
            .collect();
        program.query.body = std::iter::once(BodyItem::Block(Block {
            kind: blocks::Kind::Union,
            branches,
        }))
        .collect();
        return Ok(());
    }
    let schema: BTreeMap<_, _> = program
        .inputs()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut branches = Vec::new();
    for (j, query) in queries.into_iter().enumerate() {
        let types = blocks::types(&schema, &query.body, &BTreeMap::new())?;
        let bound = blocks::bound(&query.body);
        let computed: BTreeSet<_> = blocks::walk(&query.body)
            .into_iter()
            .filter_map(|item| match item {
                BodyItem::Let(binding) => Some(rust::bindings(&binding.pattern)),
                _ => None,
            })
            .flatten()
            .map(|v| symbol_name(&v))
            .collect();
        let names: BTreeMap<_, _> = blocks::walk(&query.body)
            .into_iter()
            .flat_map(|item| match item {
                BodyItem::Positive { atom } => atom.variables().cloned().collect(),
                BodyItem::Let(binding) => rust::bindings(&binding.pattern),
                _ => Vec::new(),
            })
            .map(|id| {
                (
                    symbol_name(&id),
                    fresh.name(&format!("rule_{j}_{}", symbol_name(&id))),
                )
            })
            .collect();
        let mut body = query.body.clone();
        rename_body(&mut body, &names, &BTreeMap::new());
        for (i, expression) in query.head.args.iter().enumerate() {
            let variable =
                rust::variable(expression).filter(|id| bound.contains_key(&symbol_name(id)));
            let (expression, ty) = if let Some(variable) = variable {
                let name = symbol_name(variable);
                let id = &names[&name];
                let ty = &types[&name];
                if computed.contains(&name) {
                    (
                        syn::parse_quote!(::core::clone::Clone::clone(#id)),
                        ty.clone(),
                    )
                } else {
                    (syn::parse_quote!(#id), syn::parse_quote!(&#ty))
                }
            } else {
                let output = program.output().ok_or_else(|| {
                    syn::Error::new_spanned(
                        expression,
                        "declare result column types for a computed multi-rule head",
                    )
                })?;
                let mut expression = expression.clone();
                rename_expression(&mut expression, &names, &bound);
                (expression, output.columns[i].ty.clone())
            };
            let id = &head_columns[i];
            body.push(BodyItem::Let(Box::new(syn::parse2(
                quote!(let #id: #ty = #expression),
            )?)));
        }
        branches.push(Branch {
            brace: Default::default(),
            body,
        });
    }
    program.query.head = Atom {
        relation: program.query.head.relation.clone(),
        paren_token: program.query.head.paren_token,
        args: head_columns
            .iter()
            .map(|id| -> Expr { syn::parse_quote!(#id) })
            .collect(),
    };
    program.query.body = std::iter::once(BodyItem::Block(Block {
        kind: blocks::Kind::Union,
        branches,
    }))
    .collect();
    Ok(())
}

fn rename_body(
    body: &mut crate::CommaList<BodyItem>,
    names: &BTreeMap<String, Ident>,
    context: &BTreeMap<String, Ident>,
) {
    let mut bound = context.clone();
    for item in body {
        match item {
            BodyItem::Positive { atom } => {
                let mut local = bound.clone();
                local.extend(atom.variables().map(|id| (symbol_name(id), id.clone())));
                for argument in &mut atom.args {
                    if let Some(id) = rust::variable(argument) {
                        if let Some(new) = names.get(&symbol_name(id)) {
                            *argument = syn::parse_quote!(#new);
                        }
                    } else {
                        rename_expression(argument, names, &local);
                    }
                }
                bound = local;
            }
            BodyItem::Let(binding) => {
                rename_expression(&mut binding.expression, names, &bound);
                let bindings = rust::bindings(&binding.pattern);
                struct Pattern<'a>(&'a BTreeMap<String, Ident>);
                impl VisitMut for Pattern<'_> {
                    fn visit_pat_ident_mut(&mut self, pat: &mut syn::PatIdent) {
                        if let Some(new) = self.0.get(&symbol_name(&pat.ident)) {
                            pat.ident = new.clone();
                        }
                        visit_mut::visit_pat_ident_mut(self, pat);
                    }
                }
                Pattern(names).visit_pat_mut(&mut binding.pattern);
                bound.extend(bindings.iter().map(|id| (symbol_name(id), id.clone())));
            }
            BodyItem::Filter(filter) => rename_expression(&mut filter.condition, names, &bound),
            BodyItem::Constraint(crate::Constraint::Total { expression, bindings }) => {
                rename_expression(expression, names, &bound);
                let logical = bound.keys().map(|name| {
                    names.get(name).map(symbol_name).unwrap_or_else(|| name.clone())
                }).collect();
                let crate::Constraint::Total { bindings: renamed, .. } =
                    crate::Constraint::total((**expression).clone(), &logical) else { unreachable!() };
                *bindings = renamed;
            }
            BodyItem::Equality(equality) => {
                if let Some(new) = names.get(&symbol_name(&equality.left)) {
                    equality.left = new.clone();
                }
                if let crate::EqualityValue::Variable(id) = &mut equality.right {
                    if let Some(new) = names.get(&symbol_name(id)) {
                        *id = new.clone();
                    }
                }
            }
            BodyItem::Constraint(constraint) => constraint.map_terms(&mut |term| {
                if let cq_syntax::constraint::Term::Variable(id) = term {
                    if let Some(new) = names.get(&symbol_name(id)) {
                        *id = new.clone();
                    }
                }
            }),
            BodyItem::Block(block) => {
                let interface = block.analyze(&bound.keys().cloned().collect()).interface;
                for branch in &mut block.branches {
                    rename_body(&mut branch.body, names, &bound);
                }
                bound.extend(interface);
            }
            // An extension expands before the core contract. Preserve its
            // tokens and give it the same lexical input names in its own pass.
            _ => (),
        }
    }
}

fn rename_expression(
    expression: &mut Expr,
    names: &BTreeMap<String, Ident>,
    environment: &BTreeMap<String, Ident>,
) {
    if rust::free_variables(expression).opaque {
        let aliases = environment
            .iter()
            .filter_map(|(name, old)| names.get(name).map(|new| quote!(let #old = #new;)))
            .collect::<Vec<_>>();
        if !aliases.is_empty() {
            *expression = syn::parse_quote!({ #(#aliases)* #expression });
        }
        return;
    }
    Rename {
        names,
        bound: BTreeSet::new(),
    }
    .visit_expr_mut(expression);
}

struct Rename<'a> {
    names: &'a BTreeMap<String, Ident>,
    bound: BTreeSet<String>,
}
impl Rename<'_> {
    fn bind(&mut self, pattern: &syn::Pat) {
        self.bound
            .extend(rust::bindings(pattern).iter().map(symbol_name));
    }
    fn condition(&mut self, expression: &mut Expr) {
        match expression {
            Expr::Let(binding) => {
                self.visit_expr_mut(&mut binding.expr);
                self.bind(&binding.pat);
            }
            Expr::Binary(binary) if matches!(binary.op, syn::BinOp::And(_)) => {
                self.condition(&mut binary.left);
                self.condition(&mut binary.right);
            }
            _ => self.visit_expr_mut(expression),
        }
    }
}
impl VisitMut for Rename<'_> {
    fn visit_expr_path_mut(&mut self, expression: &mut syn::ExprPath) {
        if expression.qself.is_none()
            && expression.path.leading_colon.is_none()
            && expression.path.segments.len() == 1
        {
            let id = &mut expression.path.segments[0].ident;
            if !self.bound.contains(&symbol_name(id)) {
                if let Some(new) = self.names.get(&symbol_name(id)) {
                    *id = new.clone();
                }
            }
        }
        visit_mut::visit_expr_path_mut(self, expression);
    }
    fn visit_block_mut(&mut self, block: &mut syn::Block) {
        let outer = self.bound.clone();
        for statement in &mut block.stmts {
            if let syn::Stmt::Local(local) = statement {
                if let Some(init) = &mut local.init {
                    self.visit_expr_mut(&mut init.expr);
                    if let Some((_, otherwise)) = &mut init.diverge {
                        self.visit_expr_mut(otherwise);
                    }
                }
                self.bind(&local.pat);
            } else {
                self.visit_stmt_mut(statement);
            }
        }
        self.bound = outer;
    }
    fn visit_expr_closure_mut(&mut self, closure: &mut syn::ExprClosure) {
        let outer = self.bound.clone();
        for pattern in &closure.inputs {
            self.bind(pattern);
        }
        self.visit_expr_mut(&mut closure.body);
        self.bound = outer;
    }
    fn visit_expr_for_loop_mut(&mut self, expression: &mut syn::ExprForLoop) {
        self.visit_expr_mut(&mut expression.expr);
        let outer = self.bound.clone();
        self.bind(&expression.pat);
        self.visit_block_mut(&mut expression.body);
        self.bound = outer;
    }
    fn visit_expr_match_mut(&mut self, expression: &mut syn::ExprMatch) {
        self.visit_expr_mut(&mut expression.expr);
        for arm in &mut expression.arms {
            let outer = self.bound.clone();
            self.bind(&arm.pat);
            if let Some((_, guard)) = &mut arm.guard {
                self.condition(guard);
            }
            self.visit_expr_mut(&mut arm.body);
            self.bound = outer;
        }
    }
    fn visit_expr_if_mut(&mut self, expression: &mut syn::ExprIf) {
        let outer = self.bound.clone();
        self.condition(&mut expression.cond);
        self.visit_block_mut(&mut expression.then_branch);
        self.bound = outer;
        if let Some((_, otherwise)) = &mut expression.else_branch {
            self.visit_expr_mut(otherwise);
        }
    }
    fn visit_expr_while_mut(&mut self, expression: &mut syn::ExprWhile) {
        let outer = self.bound.clone();
        self.condition(&mut expression.cond);
        self.visit_block_mut(&mut expression.body);
        self.bound = outer;
    }
}
