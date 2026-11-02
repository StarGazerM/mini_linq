//! Code domains: A-Domain (translation) and WF-Domain (well-formedness).
//!
//! DESIGN contract7-code-domain
//! A declared column `col: T in (D)` stores codes of the authored domain `D`
//! (a query-closed host value implementing `cq_storage::CodeDomain`). The
//! domain fact, never the column type or an operand's Rust shape, decides that
//! a closed operand compared with the column is translated once on the host:
//! A-Domain records the translation as a closed capture
//! `let __cq_c_i: T = ::cq_storage::domain::f(&(D), &(t));`, and the comparison
//! then reads the capture like any closed operand. Later IRs carry captures
//! and declarations unchanged; printers decide nothing about domains.
//! DO NOT inspect column types, recognize literal kinds, or translate per row.

use super::{BodyItem, CommaList, Program, normal, rust};
use crate::symbol_name;
use cq_syntax::{
    CodeDomain, ColumnDecl, EqualityValue,
    capture::{Capture, Captures},
    constraint::{Comparison, Constraint, Term},
};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::Ident;

/// The authored representation of the columns a query variable occupies.
#[derive(Clone)]
struct Column {
    domain: Option<CodeDomain>,
    ty: syn::Type,
}

impl Column {
    fn of(declaration: &ColumnDecl) -> Self {
        Self {
            domain: declaration.domain.clone(),
            ty: declaration.ty.clone(),
        }
    }
    fn key(&self) -> Option<String> {
        self.domain.as_ref().map(CodeDomain::key)
    }
}

/// The translation function a comparison needs and the comparison it keeps:
/// `x <= v ⇔ code(x) < upper(v)` and `x > v ⇔ code(x) >= upper(v)`.
fn translation(comparison: Comparison) -> (Comparison, &'static str) {
    match comparison {
        Comparison::Eq(_) | Comparison::Ne(_) => (comparison, "exact"),
        Comparison::Lt(_) | Comparison::Ge(_) => (comparison, "lower"),
        Comparison::Le(_) => (Comparison::Lt(Default::default()), "upper"),
        Comparison::Gt(_) => (Comparison::Ge(Default::default()), "upper"),
    }
}

fn flip(comparison: Comparison) -> Comparison {
    match comparison {
        Comparison::Lt(_) => Comparison::Gt(Default::default()),
        Comparison::Le(_) => Comparison::Ge(Default::default()),
        Comparison::Gt(_) => Comparison::Lt(Default::default()),
        Comparison::Ge(_) => Comparison::Le(Default::default()),
        same => same,
    }
}

fn error(tokens: impl ToTokens, message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new_spanned(tokens, format!("WF-Domain: {message}"))
}

/// WF-Domain: every variable occupies columns of one recorded domain (or of
/// none): code equality and joins exist only within one domain.
fn variables(
    body: &CommaList<BodyItem>,
    declarations: &BTreeMap<String, &cq_syntax::RelationDecl>,
) -> syn::Result<BTreeMap<String, Column>> {
    let mut columns: BTreeMap<String, Column> = BTreeMap::new();
    for item in super::blocks::walk(body) {
        let BodyItem::Positive { atom } = item else {
            continue;
        };
        let declaration = declarations[&symbol_name(&atom.relation)];
        for (argument, column) in atom.args.iter().zip(&declaration.columns) {
            let Some(variable) = rust::variable(argument) else {
                continue;
            };
            let column = Column::of(column);
            match columns.get(&symbol_name(variable)) {
                Some(seen) if seen.key() != column.key() => {
                    return Err(error(
                        variable,
                        format!("`{variable}` occupies columns of different code domains"),
                    ));
                }
                Some(_) => {}
                None => {
                    columns.insert(symbol_name(variable), column);
                }
            }
        }
    }
    Ok(columns)
}

/// The domain function of a recorded A-Domain capture named by `term`.
fn domain_function(term: &Term, domain: &CodeDomain, captures: &Captures) -> Option<String> {
    let Term::Closed(syn::Expr::Path(path)) = term else {
        return None;
    };
    let name = path.path.get_ident()?;
    captures.items.iter().find_map(|capture| {
        let Capture::Let(binding) = capture else {
            return None;
        };
        let syn::Pat::Ident(pattern) = &binding.pattern else {
            return None;
        };
        if symbol_name(&pattern.ident) != symbol_name(name) {
            return None;
        }
        let syn::Expr::Call(call) = &binding.expression else {
            return None;
        };
        let syn::Expr::Path(callee) = &*call.func else {
            return None;
        };
        let segments: Vec<_> = callee
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        let [crate_name, module, function] = segments.as_slice() else {
            return None;
        };
        let syn::Expr::Reference(reference) = call.args.first()? else {
            return None;
        };
        let syn::Expr::Paren(argument) = &*reference.expr else {
            return None;
        };
        (callee.path.leading_colon.is_some()
            && crate_name == "cq_storage"
            && module == "domain"
            && call.args.len() == 2
            && argument.expr.to_token_stream().to_string() == domain.key())
        .then(|| function.clone())
    })
}

/// A-Domain: translate every closed operand compared with a code-domain column
/// once on the host. Already translated operands (A-Domain captures of the
/// same domain) are kept, so normalization stays idempotent.
pub(super) fn translate(
    body: &mut CommaList<BodyItem>,
    declarations: &BTreeMap<String, &cq_syntax::RelationDecl>,
    captures: &mut Captures,
    names: &mut BTreeSet<String>,
) -> syn::Result<()> {
    let logical = super::blocks::bound(body).into_keys().collect();
    let mut translation = Translation {
        variables: variables(body, declarations)?,
        declarations,
        logical,
        captures,
        names,
    };
    translation.body(body)
}

struct Translation<'a> {
    variables: BTreeMap<String, Column>,
    declarations: &'a BTreeMap<String, &'a cq_syntax::RelationDecl>,
    logical: BTreeSet<String>,
    captures: &'a mut Captures,
    names: &'a mut BTreeSet<String>,
}

impl Translation<'_> {
    /// Record `let k: T = ::cq_storage::domain::f(&(D), &(operand));`.
    fn code(&mut self, column: &Column, operand: &syn::Expr, function: &str) -> Term {
        let domain = &column
            .domain
            .as_ref()
            .expect("a code-domain column")
            .expression;
        let function = Ident::new(function, proc_macro2::Span::call_site());
        let name = normal::fresh_capture(self.names);
        let expression: syn::Expr =
            syn::parse_quote!(::cq_storage::domain::#function(&(#domain), &(#operand)));
        self.captures.items.push(normal::capture(normal::binding(
            &name, &column.ty, expression,
        )));
        Term::Closed(normal::path(&name))
    }

    fn operand(&mut self, column: &Column, term: Term, function: &str) -> Term {
        let domain = column.domain.as_ref().expect("a code-domain column");
        if domain_function(&term, domain, self.captures).is_some() {
            return term;
        }
        let expression = match term {
            Term::Literal(value) => *value,
            Term::Closed(value) => value,
            Term::Variable(name) => normal::path(&name),
        };
        self.code(column, &expression, function)
    }

    fn column(&self, term: &Term) -> Option<Column> {
        let Term::Variable(name) = term else {
            return None;
        };
        self.variables.get(&symbol_name(name)).cloned()
    }

    fn same_domain(&self, left: &Ident, right: &Ident) -> syn::Result<()> {
        let key = |name: &Ident| self.variables.get(&symbol_name(name)).and_then(Column::key);
        if key(left) != key(right) {
            return Err(error(
                left,
                format!("`{left}` and `{right}` are compared across code domains"),
            ));
        }
        Ok(())
    }

    fn constraint(&mut self, constraint: Constraint) -> syn::Result<Vec<Constraint>> {
        Ok(match constraint {
            Constraint::Compare {
                left,
                comparison,
                right,
            } => {
                let (variable, comparison, operand) = match (left, right) {
                    (Term::Variable(left), Term::Variable(right))
                        if self.logical.contains(&symbol_name(&left))
                            && self.logical.contains(&symbol_name(&right)) =>
                    {
                        self.same_domain(&left, &right)?;
                        return Ok(vec![Constraint::Compare {
                            left: Term::Variable(left),
                            comparison,
                            right: Term::Variable(right),
                        }]);
                    }
                    (Term::Variable(left), right) if self.logical.contains(&symbol_name(&left)) => {
                        (left, comparison, right)
                    }
                    (left, Term::Variable(right))
                        if self.logical.contains(&symbol_name(&right)) =>
                    {
                        (right, flip(comparison), left)
                    }
                    (left, right) => {
                        return Ok(vec![Constraint::Compare {
                            left,
                            comparison,
                            right,
                        }]);
                    }
                };
                let Some(column) = self
                    .column(&Term::Variable(variable.clone()))
                    .filter(|c| c.domain.is_some())
                else {
                    return Ok(vec![Constraint::Compare {
                        left: Term::Variable(variable),
                        comparison,
                        right: operand,
                    }]);
                };
                let (comparison, function) = translation(comparison);
                vec![Constraint::Compare {
                    left: Term::Variable(variable),
                    comparison,
                    right: self.operand(&column, operand, function),
                }]
            }
            Constraint::Between { term, low, high } => {
                let Some(column) = self.column(&term).filter(|c| c.domain.is_some()) else {
                    return Ok(vec![Constraint::Between { term, low, high }]);
                };
                vec![
                    Constraint::Compare {
                        left: term.clone(),
                        comparison: Comparison::Ge(Default::default()),
                        right: self.operand(&column, low, "lower"),
                    },
                    Constraint::Compare {
                        left: term,
                        comparison: Comparison::Lt(Default::default()),
                        right: self.operand(&column, high, "upper"),
                    },
                ]
            }
            Constraint::In { term, literals } => {
                let Some(column) = self.column(&term).filter(|c| c.domain.is_some()) else {
                    return Ok(vec![Constraint::In { term, literals }]);
                };
                let mut branches: Vec<Vec<Constraint>> = literals
                    .into_iter()
                    .map(|literal| {
                        vec![Constraint::Compare {
                            left: term.clone(),
                            comparison: Comparison::Eq(Default::default()),
                            right: self.code(&column, &literal, "exact"),
                        }]
                    })
                    .collect();
                if branches.len() == 1 {
                    branches.remove(0)
                } else {
                    vec![Constraint::Or(branches)]
                }
            }
            Constraint::Or(branches) => vec![Constraint::Or(
                branches
                    .into_iter()
                    .map(|branch| {
                        let mut conjunction = Vec::new();
                        for constraint in branch {
                            conjunction.extend(self.constraint(constraint)?);
                        }
                        Ok(conjunction)
                    })
                    .collect::<syn::Result<_>>()?,
            )],
        })
    }

    fn body(&mut self, body: &mut CommaList<BodyItem>) -> syn::Result<()> {
        let mut translated = CommaList::new();
        for (clause, item) in std::mem::take(body).into_iter().enumerate() {
            match item {
                BodyItem::Block(mut block) => {
                    for branch in &mut block.branches {
                        self.body(&mut branch.body)?;
                    }
                    translated.push(BodyItem::Block(block));
                }
                BodyItem::Positive { mut atom } => {
                    let declaration = self.declarations[&symbol_name(&atom.relation)];
                    let mut after = Vec::new();
                    for (position, (argument, column)) in
                        atom.args.iter_mut().zip(&declaration.columns).enumerate()
                    {
                        let column = Column::of(column);
                        if column.domain.is_none()
                            || rust::variable(argument).is_some()
                            || matches!(argument, syn::Expr::Infer(_))
                        {
                            continue;
                        }
                        // A-Closed form: a fresh column variable restricted by
                        // the translated closed operand.
                        let key = normal::fresh(self.names, clause, position);
                        let right = self.code(&column, argument, "exact");
                        after.push(BodyItem::Constraint(Constraint::Compare {
                            left: Term::Variable(key.clone()),
                            comparison: Comparison::Eq(Default::default()),
                            right,
                        }));
                        self.variables.insert(symbol_name(&key), column);
                        self.logical.insert(symbol_name(&key));
                        *argument = normal::path(&key);
                    }
                    translated.push(BodyItem::Positive { atom });
                    translated.extend(after);
                }
                BodyItem::Negation(mut negation) => {
                    let declaration = self.declarations[&symbol_name(&negation.atom.relation)];
                    for (argument, column) in
                        negation.atom.args.iter_mut().zip(&declaration.columns)
                    {
                        let column = Column::of(column);
                        if matches!(argument, syn::Expr::Infer(_)) {
                            continue;
                        }
                        if let Some(variable) = rust::variable(argument) {
                            if self.logical.contains(&symbol_name(variable)) {
                                let seen = self
                                    .variables
                                    .get(&symbol_name(variable))
                                    .and_then(Column::key);
                                if seen != column.key() {
                                    return Err(error(
                                        &*argument,
                                        format!(
                                            "`{variable}` occupies columns of different code domains"
                                        ),
                                    ));
                                }
                                continue;
                            }
                        }
                        let Some(domain) = &column.domain else {
                            continue;
                        };
                        let term = Term::Closed(argument.clone());
                        if domain_function(&term, domain, self.captures).is_some() {
                            continue;
                        }
                        let Term::Closed(code) = self.code(&column, argument, "exact") else {
                            unreachable!("codes are closed terms")
                        };
                        *argument = code;
                    }
                    translated.push(BodyItem::Negation(negation));
                }
                BodyItem::Constraint(constraint) => {
                    let mut conjunction = self.constraint(constraint)?;
                    translated.push(BodyItem::Constraint(if conjunction.len() == 1 {
                        conjunction.remove(0)
                    } else {
                        Constraint::Or(vec![conjunction])
                    }));
                }
                BodyItem::Equality(equality) => match &equality.right {
                    EqualityValue::Variable(right) => {
                        self.same_domain(&equality.left, right)?;
                        translated.push(BodyItem::Equality(equality));
                    }
                    EqualityValue::Literal(value) => {
                        match self
                            .column(&Term::Variable(equality.left.clone()))
                            .filter(|c| c.domain.is_some())
                        {
                            Some(column) => {
                                let right = self.code(&column, value, "exact");
                                translated.push(BodyItem::Constraint(Constraint::Compare {
                                    left: Term::Variable(equality.left.clone()),
                                    comparison: Comparison::Eq(Default::default()),
                                    right,
                                }));
                            }
                            None => translated.push(BodyItem::Equality(equality)),
                        }
                    }
                },
                item => translated.push(item),
            }
        }
        *body = translated;
        Ok(())
    }
}

/// Formal rule WF-Domain: variables occupy columns of one code domain; every
/// operand compared with a code-domain column is an A-Domain capture of that
/// domain (`exact` for equality, `lower`/`upper` for `<`/`>=`, no `<=`/`>`,
/// ranges or literal membership), and a declared output domain is its head
/// variable's domain.
pub(super) fn check(module: &Program) -> syn::Result<()> {
    let declarations: BTreeMap<_, _> = module
        .declarations
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let body = &module.query.body;
    let columns = variables(body, &declarations)?;
    let logical: BTreeSet<String> = super::blocks::bound(body).into_keys().collect();
    let captures = &module.query.captures;
    let key = |name: &Ident| columns.get(&symbol_name(name)).and_then(Column::key);
    let domain = |name: &Ident| {
        columns
            .get(&symbol_name(name))
            .and_then(|c| c.domain.clone())
    };
    let operand = |variable: &Ident, comparison: Comparison, term: &Term| -> syn::Result<()> {
        if let Term::Variable(other) = term {
            if logical.contains(&symbol_name(other)) {
                return if key(variable) == key(other) {
                    Ok(())
                } else {
                    Err(error(
                        variable,
                        format!("`{variable}` and `{other}` are compared across code domains"),
                    ))
                };
            }
        }
        let Some(domain) = domain(variable) else {
            return Ok(());
        };
        let function = domain_function(term, &domain, captures);
        let allowed: &[&str] = match comparison {
            Comparison::Eq(_) | Comparison::Ne(_) => &["exact"],
            Comparison::Lt(_) | Comparison::Ge(_) => &["lower", "upper"],
            Comparison::Le(_) | Comparison::Gt(_) => &[],
        };
        if function.is_some_and(|f| allowed.contains(&f.as_str())) {
            Ok(())
        } else {
            Err(error(
                variable,
                format!(
                    "the operand compared with code-domain variable `{variable}` is not its A-Domain translation"
                ),
            ))
        }
    };
    fn constraints(
        constraint: &Constraint,
        domain: &dyn Fn(&Ident) -> Option<CodeDomain>,
        operand: &dyn Fn(&Ident, Comparison, &Term) -> syn::Result<()>,
    ) -> syn::Result<()> {
        match constraint {
            Constraint::Compare {
                left: Term::Variable(variable),
                comparison,
                right,
            } => operand(variable, *comparison, right),
            Constraint::Compare {
                left,
                comparison,
                right: Term::Variable(variable),
            } => operand(variable, flip(*comparison), left),
            Constraint::Compare { .. } => Ok(()),
            Constraint::Between {
                term: Term::Variable(variable),
                ..
            }
            | Constraint::In {
                term: Term::Variable(variable),
                ..
            } if domain(variable).is_some() => Err(error(
                variable,
                "a code-domain range or literal membership is translated into comparisons",
            )),
            Constraint::Between { .. } | Constraint::In { .. } => Ok(()),
            Constraint::Or(branches) => branches
                .iter()
                .flatten()
                .try_for_each(|constraint| constraints(constraint, domain, operand)),
        }
    }
    for item in super::blocks::walk(body) {
        match item {
            BodyItem::Positive { atom } => {
                let declaration = declarations[&symbol_name(&atom.relation)];
                for (argument, column) in atom.args.iter().zip(&declaration.columns) {
                    if column.domain.is_some() && rust::variable(argument).is_none() {
                        return Err(error(
                            argument,
                            format!(
                                "the closed operand at code-domain column `{}` is not translated",
                                column.name
                            ),
                        ));
                    }
                }
            }
            BodyItem::Negation(negation) => {
                let declaration = declarations[&symbol_name(&negation.atom.relation)];
                for (argument, column) in negation.atom.args.iter().zip(&declaration.columns) {
                    if matches!(argument, syn::Expr::Infer(_)) {
                        continue;
                    }
                    if let Some(variable) =
                        rust::variable(argument).filter(|v| logical.contains(&symbol_name(v)))
                    {
                        if key(variable) != column.domain.as_ref().map(CodeDomain::key) {
                            return Err(error(
                                argument,
                                format!("`{variable}` occupies columns of different code domains"),
                            ));
                        }
                        continue;
                    }
                    if let Some(domain) = &column.domain {
                        if domain_function(&Term::Closed(argument.clone()), domain, captures)
                            .as_deref()
                            != Some("exact")
                        {
                            return Err(error(
                                argument,
                                format!(
                                    "the closed operand at code-domain column `{}` is not translated",
                                    column.name
                                ),
                            ));
                        }
                    }
                }
            }
            BodyItem::Constraint(constraint) => constraints(constraint, &domain, &operand)?,
            BodyItem::Equality(equality) => match &equality.right {
                EqualityValue::Variable(right) => {
                    if key(&equality.left) != key(right) {
                        return Err(error(
                            &equality.left,
                            format!(
                                "`{}` and `{right}` are compared across code domains",
                                equality.left
                            ),
                        ));
                    }
                }
                EqualityValue::Literal(_) if domain(&equality.left).is_some() => {
                    return Err(error(
                        &equality.left,
                        "a literal equality on a code-domain variable is translated",
                    ));
                }
                EqualityValue::Literal(_) => {}
            },
            _ => {}
        }
    }
    if let Some(output) = module.output() {
        for (argument, column) in module.query.head.args.iter().zip(&output.columns) {
            let (Some(declared), Some(variable)) = (&column.domain, rust::variable(argument))
            else {
                continue;
            };
            if key(variable).is_some_and(|key| key != declared.key()) {
                return Err(error(
                    argument,
                    format!(
                        "output column `{}` declares a different code domain",
                        column.name
                    ),
                ));
            }
        }
    }
    Ok(())
}
