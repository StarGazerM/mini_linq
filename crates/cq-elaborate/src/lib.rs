//! VO → RA: each valuation step as one named relational-algebra shape.
//!
//! The valuation plan fixed the order and the granularity; this pass decides
//! nothing about either. It records one thing: the operator each step
//! becomes over the running prefix `P`, the relation of valuations bound so
//! far.
//!
//! - A subatom `R@i(x, …)` is `rename R { col -> var, … }` over the whole
//!   clause, restricted by `project … keep { x, … }` when it drops columns.
//!   It is the prefix when there is none, else `natural_join P with it`;
//!   a partial subatom that binds no new variable only checks the prefix,
//!   `semijoin P with it`.
//! - A guard is `select P if (e)`; a binding is `extend P with { v: T, … }
//!   value ({ let pat: T = e; (v, …) })`. Either before every atom seeds
//!   the prefix with `unit`.
//! - The plan ends with `project P keep { … }` retaining the head variables
//!   — the whole heading under a computed head — then `output … as head`.
//!
//! Results are `r0, r1, …` in emission order. A relation-at-a-time plan in
//! written order elaborates to exactly the RA the clause planner emitted, so
//! later passes see nothing new. A plan's erasure is its query:
//! [`check`] compares [`ra::kernel`] with [`valuation::kernel`].

use std::collections::BTreeSet;

use cq_ir::{cq, relational_plan as ra, symbol_name, valuation};
use quote::format_ident;
use syn::Ident;

/// Elaborate a valuation plan into named RA, then check the RA contract.
pub fn compile(source: &valuation::Module) -> syn::Result<ra::Module> {
    let analysis = valuation::contract::check(source)?;
    let mut state = State {
        definitions: Vec::new(),
        prefix: None,
        heading: BTreeSet::new(),
    };
    for (step, introduces) in source.steps.iter().zip(&analysis.introduces) {
        match step {
            valuation::Step::Atom(subatom, _) => {
                let atom = &analysis.atoms[&subatom.clause()];
                let declaration = source
                    .program
                    .inputs()
                    .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
                    .expect("the CQ contract declares every body relation");
                state.subatom(subatom, atom, declaration, introduces.is_empty());
            }
            valuation::Step::Guard(filter, _) => state.select(filter),
            valuation::Step::Let(binding, _) => state.extend(binding),
        }
        state.heading.extend(introduces.iter().cloned());
    }
    let output = state.finish(&source.program.query.head);
    let target = ra::Module {
        program: source.program.clone(),
        relational_token: Default::default(),
        relational_brace: Default::default(),
        plan: ra::Plan {
            definitions: state.definitions,
            output,
        },
    };
    ra::contract::check(&target)?;
    Ok(target)
}

/// VO → RA correspondence: the program preserved, the RA contract holds,
/// and the plan computes the query the valuation schedules.
pub fn check(source: &valuation::Module, target: &ra::Module) -> bool {
    target.program == source.program
        && ra::contract::well_formed(target)
        && ra::kernel(target) == valuation::kernel(source)
}

/// The definitions so far, the prefix result and its heading.
struct State {
    definitions: Vec<ra::Definition>,
    prefix: Option<Ident>,
    heading: BTreeSet<String>,
}

impl State {
    fn subatom(
        &mut self,
        subatom: &valuation::Subatom,
        atom: &cq::Atom,
        declaration: &cq::RelationDecl,
        check: bool,
    ) {
        let variables: Vec<&Ident> = atom.variables().collect();
        let mapping = declaration
            .columns
            .iter()
            .zip(&variables)
            .map(|(column, variable)| ra::AttributeRename {
                source: column.name.clone(),
                minus_token: Default::default(),
                gt_token: Default::default(),
                target: (*variable).clone(),
            })
            .collect();
        let mut right = self.emit(ra::Operator::Rename(Box::new(ra::Rename {
            rename_token: Default::default(),
            relation: atom.relation.clone(),
            mapping_brace: Default::default(),
            mapping,
        })));
        let partial = subatom.columns.len() != subatom.variables().count();
        if partial {
            let attributes = subatom
                .variables()
                .map(|(position, _)| variables[position].clone())
                .collect();
            right = self.project(right, attributes);
        }
        self.prefix = Some(match self.prefix.take() {
            None => right,
            Some(left) if partial && check => {
                self.emit(ra::Operator::Semijoin(Box::new(ra::Semijoin {
                    semijoin_token: Default::default(),
                    left,
                    with_token: Default::default(),
                    right,
                })))
            }
            Some(left) => self.emit(ra::Operator::NaturalJoin(Box::new(ra::NaturalJoin {
                natural_join_token: Default::default(),
                left,
                with_token: Default::default(),
                right,
            }))),
        });
    }

    fn select(&mut self, filter: &cq::Filter) {
        let input = self.prefix_or_unit();
        self.prefix = Some(self.emit(ra::Operator::Select(Box::new(ra::Select {
            select_token: Default::default(),
            input,
            if_token: Default::default(),
            condition_paren: Default::default(),
            condition: filter.condition.clone(),
        }))));
    }

    /// `extend P with { v: T, … } value ({ let pat: T = e; (v, …) })`: the
    /// binding evaluated per prefix tuple, its bound variables the columns.
    fn extend(&mut self, binding: &cq::Let) {
        let variables = cq::rust::bindings(&binding.pattern);
        let columns = binding
            .binding_types()
            .into_iter()
            .map(|(name, ty)| cq::ColumnDecl {
                name,
                colon_token: Default::default(),
                ty,
            })
            .collect();
        let pat = match &binding.annotation {
            Some(annotation) => syn::Pat::Type(syn::PatType {
                attrs: Vec::new(),
                pat: Box::new(binding.pattern.clone()),
                colon_token: annotation.colon_token,
                ty: Box::new(annotation.ty.clone()),
            }),
            None => binding.pattern.clone(),
        };
        let mut stmts = vec![syn::Stmt::Local(syn::Local {
            attrs: Vec::new(),
            let_token: binding.let_token,
            pat,
            init: Some(syn::LocalInit {
                eq_token: binding.eq_token,
                expr: Box::new(binding.expression.clone()),
                diverge: None,
            }),
            semi_token: Default::default(),
        })];
        if !variables.is_empty() {
            let mut elems = syn::punctuated::Punctuated::new();
            for variable in &variables {
                elems.push(syn::Expr::Path(syn::ExprPath {
                    attrs: Vec::new(),
                    qself: None,
                    path: variable.clone().into(),
                }));
                elems.push_punct(Default::default());
            }
            stmts.push(syn::Stmt::Expr(
                syn::Expr::Tuple(syn::ExprTuple {
                    attrs: Vec::new(),
                    paren_token: Default::default(),
                    elems,
                }),
                None,
            ));
        }
        let value = syn::Expr::Block(syn::ExprBlock {
            attrs: Vec::new(),
            label: None,
            block: syn::Block {
                brace_token: Default::default(),
                stmts,
            },
        });
        let input = self.prefix_or_unit();
        self.prefix = Some(self.emit(ra::Operator::Extend(Box::new(ra::Extend {
            extend_token: Default::default(),
            input,
            with_token: Default::default(),
            columns_brace: Default::default(),
            columns,
            value_token: Default::default(),
            value_paren: Default::default(),
            value,
        }))));
    }

    /// Retain the head's variables — every column under a computed head, so
    /// embedded Rust observes the whole prefix — and name the output.
    fn finish(&mut self, head: &cq::Atom) -> ra::Output {
        let retained: BTreeSet<String> = if head.variables().count() < head.args.len() {
            self.heading.clone()
        } else {
            head.variables()
                .map(symbol_name)
                .filter(|v| self.heading.contains(v))
                .collect()
        };
        let attributes = retained.iter().map(|v| format_ident!("r#{v}")).collect();
        let input = self.prefix_or_unit();
        let result = self.project(input, attributes);
        ra::Output {
            output_token: Default::default(),
            input: result,
            as_token: Default::default(),
            head: head.clone(),
            semi_token: Default::default(),
        }
    }

    fn project(&mut self, input: Ident, attributes: cq_ir::CommaList<Ident>) -> Ident {
        self.emit(ra::Operator::Project(Box::new(ra::Project {
            project_token: Default::default(),
            input,
            keep_token: Default::default(),
            attributes_brace: Default::default(),
            attributes,
        })))
    }

    fn prefix_or_unit(&mut self) -> Ident {
        if let Some(prefix) = &self.prefix {
            return prefix.clone();
        }
        let unit = self.emit(ra::Operator::Unit(ra::Unit {
            unit_token: Default::default(),
        }));
        self.prefix = Some(unit.clone());
        unit
    }

    fn emit(&mut self, operator: ra::Operator) -> Ident {
        let result = format_ident!("r{}", self.definitions.len());
        self.definitions.push(ra::Definition {
            result: result.clone(),
            eq_token: Default::default(),
            operator,
            semi_token: Default::default(),
        });
        result
    }
}
