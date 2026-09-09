//! Administrative scheduling for embedded Rust. Opaque computations retain
//! source order and the complete prefix heading until output projection.
use super::{JoinPlanReport, JoinPolicy, PlanningOptions, Rule, error};
use cq_ir::{cq, relational_plan as ra, symbol_name};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

use cq::rust::{fresh, primitive_i32, reserved};

/// A sufficient condition for the existing total-comparison graph rules.
/// Rust aliases and arbitrary operators stay in the general expression route.
pub(crate) fn required(source: &cq::Module) -> bool {
    if !source.program.query.head.is_normalized() {
        return true;
    }
    let mut types = BTreeMap::new();
    for item in &source.program.query.body {
        match item {
            cq::BodyItem::Positive { atom } if atom.is_normalized() => {
                let Some(decl) = source
                    .program
                    .inputs
                    .iter()
                    .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
                else {
                    return true;
                };
                for (var, col) in atom.variables().zip(&decl.columns) {
                    types.insert(symbol_name(var), &col.ty);
                }
            }
            cq::BodyItem::Filter(_) => (),
            _ => return true,
        }
    }
    if source
        .program
        .query
        .head
        .variables()
        .any(|id| !types.contains_key(&symbol_name(id)))
    {
        return true;
    }
    source.program.query.body.iter().any(|item| match item {
        cq::BodyItem::Filter(f) => cq::rust::comparison(&f.condition).is_none_or(|p| {
            p.variables()
                .iter()
                .any(|v| !types.get(v).is_some_and(|ty| primitive_i32(ty)))
        }),
        _ => false,
    })
}

pub(crate) fn rule(clause: usize, item: &cq::BodyItem) -> Rule {
    match item {
        cq::BodyItem::Positive { .. } => Rule::Match { clause },
        cq::BodyItem::Filter(_) => Rule::Guard { clause },
        cq::BodyItem::Let(_) => Rule::Bind { clause },
    }
}

struct State {
    definitions: Vec<ra::Definition>,
    current: Option<Ident>,
    bound: BTreeSet<String>,
    names: BTreeSet<String>,
    constants: BTreeMap<(usize, usize), Ident>,
}
impl State {
    fn emit(&mut self, operator: ra::Operator) -> Ident {
        let id = format_ident!("r{}", self.definitions.len());
        self.definitions.push(ra::Definition {
            result: id.clone(),
            eq_token: Default::default(),
            operator,
            semi_token: Default::default(),
        });
        id
    }
    fn input(&mut self) -> Ident {
        if let Some(id) = &self.current {
            return id.clone();
        }
        let id = self.emit(syn::parse_quote!(unit));
        self.current = Some(id.clone());
        id
    }
    fn extend(&mut self, columns: TokenStream, value: TokenStream) {
        let input = self.input();
        let id = self.emit(syn::parse_quote!(extend #input with { #columns } value (#value)));
        self.current = Some(id);
    }
    fn select(&mut self, expression: TokenStream) {
        let input = self.input();
        let id = self.emit(syn::parse_quote!(select #input if (#expression)));
        self.current = Some(id);
    }
    fn binding(&mut self, binding: &cq::Let) {
        let vars = cq::rust::bindings(&binding.pattern);
        let typed = cq::rust::binding_types(&binding.pattern);
        let columns = typed.iter().map(|(v, ty)| quote!(#v: #ty));
        let pat = &binding.pattern;
        let expr = &binding.expression;
        self.extend(
            quote!(#(#columns),*),
            quote!({ let #pat = #expr; (#(#vars,)*) }),
        );
        self.bound.extend(vars.iter().map(symbol_name));
    }
    fn atom(&mut self, atom: &cq::Atom, decl: &cq::RelationDecl, clause: usize) {
        let before = self.bound.clone();
        let mut local = BTreeSet::new();
        let mut mapping = Vec::new();
        let mut conditions = Vec::new();
        for (position, (arg, column)) in atom.args.iter().zip(&decl.columns).enumerate() {
            let target = if let Some(id) = self.constants.get(&(clause, position)) {
                id.clone()
            } else if let Some(id) = cq::rust::variable(arg) {
                let name = symbol_name(id);
                if !local.insert(name.clone()) {
                    let temp = fresh(&mut self.names, clause, position);
                    conditions.push(quote!(#temp == #id));
                    temp
                } else {
                    self.bound.insert(name);
                    id.clone()
                }
            } else {
                let temp = fresh(&mut self.names, clause, position);
                if matches!(arg, syn::Expr::Infer(_)) {
                    // A distinct anonymous attribute preserves the source tuple
                    // until all observable prefix computations have executed.
                } else if !cq::rust::uses(arg, &local.difference(&before).cloned().collect())
                    .is_empty()
                {
                    conditions.push(quote!(#temp.eq(&(#arg))));
                } else {
                    let ty = &column.ty;
                    self.extend(quote!(#temp: #ty), quote!((#arg,)));
                }
                temp
            };
            self.bound.insert(symbol_name(&target));
            let source = &column.name;
            mapping.push(quote!(#source -> #target));
        }
        let relation = &atom.relation;
        let right = self.emit(syn::parse_quote!(rename #relation { #(#mapping),* }));
        self.current = Some(if let Some(left) = self.current.clone() {
            self.emit(syn::parse_quote!(natural_join #left with #right))
        } else {
            right
        });
        for condition in conditions {
            self.select(condition);
        }
    }
}

/// Literal arguments normalize to the same typed-let operation as source lets.
/// Their closed, total values can be bound before the first input scan.
fn constants(source: &cq::Module, state: &mut State) -> Vec<Rule> {
    let mut rules = Vec::new();
    for (clause, item) in source.program.query.body.iter().enumerate() {
        if let cq::BodyItem::Positive { atom } = item {
            let declaration = source
                .program
                .inputs
                .iter()
                .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
                .unwrap();
            for (argument, (expr, column)) in atom.args.iter().zip(&declaration.columns).enumerate()
            {
                if cq::rust::literal(expr) {
                    let id = fresh(&mut state.names, clause, argument);
                    let ty = &column.ty;
                    state.binding(&syn::parse_quote!(let #id: #ty = #expr));
                    state.constants.insert((clause, argument), id);
                    rules.push(Rule::Constant { clause, argument });
                }
            }
        }
    }
    rules
}

fn schedule(source: &cq::Module, state: &State, policy: JoinPolicy) -> Vec<usize> {
    let body = &source.program.query.body;
    let mut remaining: Vec<_> = (0..body.len()).collect();
    let mut bound = state.bound.clone();
    let mut order = Vec::new();
    while !remaining.is_empty() {
        let block = remaining
            .iter()
            .take_while(|&&i| cq::rust::movable(source, &body[i]))
            .count();
        let position = if policy == JoinPolicy::Written || block == 0 {
            0
        } else {
            // Ready total guards precede another join. All other Rust clauses
            // delimit the block and retain their source evaluation environment.
            let ready = remaining[..block].iter().position(|&i| match &body[i] {
                cq::BodyItem::Filter(f) => cq::rust::comparison(&f.condition)
                    .unwrap()
                    .variables()
                    .is_subset(&bound),
                cq::BodyItem::Let(_) => cq::rust::literal_binding(&body[i]),
                _ => false,
            });
            ready.unwrap_or_else(|| {
                remaining[..block]
                    .iter()
                    .enumerate()
                    .filter_map(|(pos, &i)| {
                        let cq::BodyItem::Positive { atom } = &body[i] else {
                            return None;
                        };
                        let heading: BTreeSet<_> = atom
                            .variables()
                            .map(symbol_name)
                            .chain(
                                state
                                    .constants
                                    .iter()
                                    .filter(|((clause, _), _)| *clause == i)
                                    .map(|(_, id)| symbol_name(id)),
                            )
                            .collect();
                        let shared = heading.intersection(&bound).count();
                        let width = if matches!(policy, JoinPolicy::GreedyWidth | JoinPolicy::Auto)
                        {
                            // Anonymous and repeated argument slots are real
                            // columns too. Previously bound hidden columns add
                            // the same constant to every candidate's width.
                            bound.len() + atom.args.len() - shared
                        } else {
                            0
                        };
                        Some(((shared == 0, width, i), pos))
                    })
                    .min_by_key(|(rank, _)| *rank)
                    .expect("a locally valid guard has a preceding producer")
                    .1
            })
        };
        let clause = remaining.remove(position);
        match &body[clause] {
            cq::BodyItem::Positive { atom } => bound.extend(atom.variables().map(symbol_name)),
            cq::BodyItem::Let(binding) => {
                bound.extend(cq::rust::bindings(&binding.pattern).iter().map(symbol_name))
            }
            cq::BodyItem::Filter(_) => (),
        }
        order.push(clause);
    }
    order
}

pub(crate) fn plan(
    source: &cq::Module,
    options: PlanningOptions,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    emit(source, options, None)
}

pub(crate) fn replay(source: &cq::Module, report: &JoinPlanReport) -> syn::Result<ra::Module> {
    if report.join_tree.is_some() || !report.analysis.is_empty() {
        return Err(error(
            "expression scheduling does not claim a join-tree certificate",
        ));
    }
    let (target, actual) = emit(source, report.options, Some(&report.derivation))?;
    if actual.derivation != report.derivation {
        return Err(error("incomplete expression derivation"));
    }
    Ok(target)
}

fn emit(
    source: &cq::Module,
    options: PlanningOptions,
    supplied: Option<&[Rule]>,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    let mut state = State {
        definitions: Vec::new(),
        current: None,
        bound: BTreeSet::new(),
        names: reserved(source),
        constants: BTreeMap::new(),
    };
    let mut derivation = constants(source, &mut state);
    let order = if let Some(rules) = supplied {
        if !rules.starts_with(&derivation) || rules.last() != Some(&Rule::Finish) {
            return Err(error("constant normalization and Finish must be preserved"));
        }
        rules[derivation.len()..rules.len() - 1]
            .iter()
            .map(|step| match step {
                Rule::Match { clause } | Rule::Guard { clause } | Rule::Bind { clause }
                    if source
                        .program
                        .query
                        .body
                        .get(*clause)
                        .is_some_and(|item| *step == rule(*clause, item)) =>
                {
                    Ok(*clause)
                }
                _ => Err(error("invalid expression clause rule")),
            })
            .collect::<syn::Result<Vec<_>>>()?
    } else {
        schedule(source, &state, options.strategy)
    };
    crate::pass_contracts::ordered::check_order(source, &order)?;
    for clause in order {
        let item = &source.program.query.body[clause];
        match item {
            cq::BodyItem::Positive { atom } => {
                let decl = source
                    .program
                    .inputs
                    .iter()
                    .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
                    .ok_or_else(|| error("undeclared atom"))?;
                state.atom(atom, decl, clause);
            }
            cq::BodyItem::Filter(filter) => {
                let expr = &filter.condition;
                state.select(quote!(#expr));
            }
            cq::BodyItem::Let(binding) => state.binding(binding),
        }
        derivation.push(rule(clause, item));
    }
    let input = state.input();
    let head = &source.program.query.head;
    let needed: BTreeSet<_> = if head.variables().count() < head.args.len() {
        state.bound.clone()
    } else {
        head.args
            .iter()
            .flat_map(|e| cq::rust::uses(e, &state.bound))
            .collect()
    };
    let attributes: Vec<_> = needed.iter().map(|v| format_ident!("r#{v}")).collect();
    let output = state.emit(syn::parse_quote!(project #input keep { #(#attributes),* }));
    let program = &source.program;
    let definitions = &state.definitions;
    let target: ra::Module =
        syn::parse2(quote!(#program relational { #(#definitions)* output #output as #head; }))?;
    let peak_arity = ra::check_in(&program.inputs, &target.plan)?
        .values()
        .map(|r| r.heading.len())
        .max()
        .unwrap_or(0);
    if options.max_arity.is_some_and(|limit| peak_arity > limit) {
        return Err(error("Rust clause schedule exceeds max_arity"));
    }
    derivation.push(Rule::Finish);
    Ok((
        target,
        JoinPlanReport {
            options,
            analysis: Vec::new(),
            join_tree: None,
            derivation,
            peak_arity,
        },
    ))
}
