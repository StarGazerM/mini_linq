//! VO → RA: each valuation step as one named relational-algebra shape.
//!
//! The valuation plan fixed the order and the granularity; this pass decides
//! nothing about either. It records one thing: the operator each step
//! becomes over the running prefix `P`, the relation of valuations bound so
//! far.
//!
//! - A clause's relation is `rename R { col -> var, … }` over the whole
//!   clause, emitted where the clause is first mentioned — by a reduction or
//!   a subatom — and reused after.
//! - A subatom `R@i(x, …)` is the clause's relation, restricted by
//!   `project … keep { x, … }` when it drops columns. It is the prefix when
//!   there is none, else `natural_join P with it`; a partial subatom that
//!   binds no new variable only checks the prefix: `semijoin P with rC` on
//!   the clause's whole relation, whose shared heading with the prefix is
//!   exactly the bound columns the subatom lists.
//! - A reduction restricts the clause's relation itself, before the prefix
//!   sees it: `reduce C with D(cols)` is `semijoin rC with rD` over the two
//!   clauses' current relations — the RA semijoin keeps the tuples of the
//!   left agreeing with some tuple of the right on their shared heading,
//!   exactly the columns the subatom lists, so nothing is projected — and
//!   `reduce C if (e)` is `select rC if (e)`. The result becomes the
//!   clause's relation for every later reduction and subatom of it.
//! - A guard is `select P if (e)`; a binding is `extend P with { v: T, … }
//!   value ({ let pat: T = e; (v, …) })`. Either before every atom seeds
//!   the prefix with `unit`.
//! - The plan ends with `project P keep { … }` retaining the head variables
//!   — the whole heading under a computed head — then `output … as head`.
//!
//! Results are `r0, r1, …` in emission order. [`check`] compares the RA
//! kernel with the source CQ kernel and checks declarations and output head.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ra as ra;
use cq_lang_vo as valuation;
use cq_lang_vo::syntax as cq;
use cq_lang_vo::syntax::symbol_name;
use quote::format_ident;
use syn::Ident;

/// Elaborate a valuation plan into named RA, then check the RA contract.
pub fn compile(source: &valuation::Program) -> syn::Result<ra::Program> {
    let analysis = valuation::contract::check(source)?;
    let mut state = State {
        inputs: &source.cq.inputs().cloned().collect::<Vec<_>>(),
        atoms: &analysis.atoms,
        definitions: Vec::new(),
        relations: BTreeMap::new(),
        prefix: None,
        heading: BTreeSet::new(),
    };
    for (step, introduces) in source.valuation.steps.iter().zip(&analysis.introduces) {
        match step {
            valuation::Step::Atom(subatom, _) => state.subatom(subatom, introduces.is_empty()),
            valuation::Step::Reduce(reduce, _) => state.reduce(reduce),
            valuation::Step::Guard(filter, _) => {
                let input = state.prefix_or_unit();
                state.prefix = Some(state.select(input, filter));
            }
            valuation::Step::Let(binding, _) => state.extend(binding),
        }
        state.heading.extend(introduces.iter().cloned());
    }
    let output = state.finish(&source.cq.query.head);
    let target = ra::Program {
        relational_token: Default::default(),
        relational_brace: Default::default(),
        plan: ra::Plan {
            declarations: source.cq.declarations.clone(),
            definitions: state.definitions,
            output,
        },
    };
    ra::contract::check(&target)?;
    Ok(target)
}

mod kernel;
pub use kernel::{erase, kernel};

/// The RA definitions compute the query scheduled by VO.
pub fn check(source: &valuation::Program, target: &ra::Program) -> syn::Result<()> {
    valuation::contract::check(source)?;
    ra::contract::check(target)?;
    if target.plan.declarations != source.cq.declarations
        || target.plan.output.head != source.cq.query.head
        || kernel(target, &source.cq) != valuation::cq::normal::Kernel::of(&source.cq)
    {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            "output does not compute the source query",
        ));
    }
    Ok(())
}

/// The definitions so far, each clause's current relation, the prefix
/// result and its heading.
struct State<'a> {
    inputs: &'a [cq::RelationDecl],
    atoms: &'a BTreeMap<usize, cq::Atom>,
    definitions: Vec<ra::Definition>,
    relations: BTreeMap<usize, Ident>,
    prefix: Option<Ident>,
    heading: BTreeSet<String>,
}

impl State<'_> {
    /// The clause's relation: its rename, emitted at first mention, as
    /// reduced so far.
    fn relation(&mut self, clause: usize) -> Ident {
        if let Some(result) = self.relations.get(&clause) {
            return result.clone();
        }
        let atom = &self.atoms[&clause];
        let declaration = self
            .inputs
            .iter()
            .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
            .expect("the CQ contract declares every body relation");
        let mapping = declaration
            .columns
            .iter()
            .zip(atom.variables())
            .map(|(column, variable)| ra::AttributeRename {
                source: column.name.clone(),
                minus_token: Default::default(),
                gt_token: Default::default(),
                target: variable.clone(),
            })
            .collect();
        let result = self.emit(ra::Operator::Rename(Box::new(ra::Rename {
            rename_token: Default::default(),
            relation: atom.relation.clone(),
            mapping_brace: Default::default(),
            mapping,
        })));
        self.relations.insert(clause, result.clone());
        result
    }

    fn subatom(&mut self, subatom: &valuation::Subatom, check: bool) {
        let clause = subatom.clause();
        let mut right = self.relation(clause);
        let partial = subatom.columns.len() != subatom.variables().count();
        // A check binds nothing: it lists every bound column of its clause,
        // which is exactly the heading it shares with the prefix, so it is a
        // semijoin with the clause's own relation. Reading the relation — not
        // a projection of it — lets every prefix check of one clause land on
        // one result, where one curried structure answers them all.
        if partial && check {
            if let Some(left) = self.prefix.take() {
                self.prefix = Some(self.semijoin(left, right));
                return;
            }
        }
        if partial {
            let variables: Vec<&Ident> = self.atoms[&clause].variables().collect();
            let attributes = subatom
                .variables()
                .map(|(position, _)| variables[position].clone())
                .collect();
            right = self.project(right, attributes);
        }
        self.prefix = Some(match self.prefix.take() {
            None => right,
            Some(left) => self.emit(ra::Operator::NaturalJoin(Box::new(ra::NaturalJoin {
                natural_join_token: Default::default(),
                left,
                with_token: Default::default(),
                right,
            }))),
        });
    }

    fn reduce(&mut self, reduce: &valuation::Reduce) {
        let clause = reduce.clause();
        let left = self.relation(clause);
        let result = match &reduce.reduction {
            valuation::Reduction::With(_, subatom) => {
                let right = self.relation(subatom.clause());
                self.semijoin(left, right)
            }
            valuation::Reduction::Guard(filter) => self.select(left, filter),
        };
        self.relations.insert(clause, result);
    }

    fn semijoin(&mut self, left: Ident, right: Ident) -> Ident {
        self.emit(ra::Operator::Semijoin(Box::new(ra::Semijoin {
            semijoin_token: Default::default(),
            left,
            with_token: Default::default(),
            right,
        })))
    }

    fn select(&mut self, input: Ident, filter: &cq::Filter) -> Ident {
        self.emit(ra::Operator::Select(Box::new(ra::Select {
            select_token: Default::default(),
            input,
            if_token: Default::default(),
            condition_paren: Default::default(),
            condition: filter.condition.clone(),
        })))
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
        let mut bindings = BTreeMap::new();
        for definition in &self.definitions {
            match &definition.operator {
                ra::Operator::Rename(r) => {
                    for m in &r.mapping {
                        bindings
                            .entry(symbol_name(&m.target))
                            .or_insert_with(|| m.target.clone());
                    }
                }
                ra::Operator::Extend(e) => {
                    for c in &e.columns {
                        bindings
                            .entry(symbol_name(&c.name))
                            .or_insert_with(|| c.name.clone());
                    }
                }
                _ => (),
            }
        }
        let attributes = retained.iter().map(|v| bindings[v].clone()).collect();
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

    fn project(&mut self, input: Ident, attributes: cq_lang_vo::syntax::CommaList<Ident>) -> Ident {
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
