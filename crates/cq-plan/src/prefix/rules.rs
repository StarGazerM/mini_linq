//! C-CONSTANT / C-MATCH / C-GUARD / C-BIND / C-FINISH over a prefix relation.
//! Each application checks its premises before changing the state. Argument
//! judgments A-* compose C-MATCH. See doc/RUST-EXPRESSIONS.md for the equations.
use super::{Heading, JoinPlanReport, PlanningOptions, Rule, error, fresh, reserved};
use crate::dependencies::{ClauseDependencies, ClauseFacts};
use cq_ir::{cq, relational_plan as ra, symbol_name};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

/// (D, P, B, U, K, O): definitions, prefix, heading, pending clauses,
/// normalized literal keys, and optional output. Witness columns stay in B
/// until C-FINISH so embedded Rust observes the complete prefix relation.
pub(super) struct ClauseState<'a> {
    source: &'a cq::Module,
    dependencies: &'a ClauseDependencies,
    definitions: Vec<ra::Definition>,
    prefix: Option<Ident>,
    heading: Heading,
    pending: BTreeSet<usize>,
    literal_keys: BTreeMap<(usize, usize), Ident>,
    output: Option<ra::Output>,
    names: BTreeSet<String>,
}

impl<'a> ClauseState<'a> {
    fn new(source: &'a cq::Module, dependencies: &'a ClauseDependencies) -> Self {
        Self {
            source,
            dependencies,
            definitions: Vec::new(),
            prefix: None,
            heading: Heading::new(),
            pending: (0..dependencies.clauses.len()).collect(),
            literal_keys: BTreeMap::new(),
            output: None,
            names: reserved(source),
        }
    }

    fn apply(&mut self, rule: &Rule) -> syn::Result<()> {
        if self.output.is_some() {
            return Err(error("no rule may follow Finish"));
        }
        match *rule {
            Rule::Constant { clause, argument } => self.normalize_constant(clause, argument),
            Rule::Match { clause } | Rule::Guard { clause } | Rule::Bind { clause } => {
                self.require_normalized()?;
                self.require_ready(clause)?;
                match (rule, &self.source.program.query.body[clause]) {
                    (Rule::Match { .. }, cq::BodyItem::Positive { atom }) => {
                        self.match_clause(clause, atom)?;
                    }
                    (Rule::Guard { .. }, cq::BodyItem::Filter(filter)) => {
                        self.guard_clause(clause, filter);
                    }
                    (Rule::Bind { .. }, cq::BodyItem::Let(binding)) => {
                        self.bind_clause(clause, binding);
                    }
                    _ => return Err(error("rule does not match the source clause")),
                }
                Ok(())
            }
            Rule::Finish => self.finish(),
            _ => Err(error("graph rule cannot apply to a clause state")),
        }
    }

    /// C-READY: one pending clause, all inputs present, no crossed barrier.
    /// The same premise determines legal candidates and checks supplied rules.
    fn require_ready(&self, clause: usize) -> syn::Result<()> {
        let facts = self
            .dependencies
            .clauses
            .get(clause)
            .ok_or_else(|| error("invalid expression clause index"))?;
        if !self.pending.contains(&clause) {
            return Err(error("clause must be pending"));
        }
        if !facts.required.is_subset(&self.heading) {
            return Err(error("schedule uses a logical input before its producer"));
        }
        if matches!(self.source.program.query.body[clause], cq::BodyItem::Let(_))
            && !facts.provided.is_disjoint(&self.heading)
        {
            return Err(error("Bind requires fresh logical names"));
        }
        if self
            .pending
            .range(..clause)
            .any(|&earlier| !facts.reorderable || !self.dependencies.clauses[earlier].reorderable)
        {
            return Err(error("schedule crossed a Rust evaluation boundary"));
        }
        Ok(())
    }

    fn require_normalized(&self) -> syn::Result<()> {
        if self.dependencies.normalization_rules().count() != self.literal_keys.len() {
            return Err(error(
                "normalize every literal argument before clause execution",
            ));
        }
        Ok(())
    }

    pub(super) fn ready_clauses(&self) -> impl Iterator<Item = (usize, &ClauseFacts)> {
        let clauses_enabled = self.output.is_none() && self.require_normalized().is_ok();
        self.pending
            .iter()
            .copied()
            .filter(move |&clause| clauses_enabled && self.require_ready(clause).is_ok())
            .map(|clause| (clause, &self.dependencies.clauses[clause]))
    }

    pub(super) fn heading(&self) -> &Heading {
        &self.heading
    }

    /// C-CONSTANT: take the next literal in source/argument order and use the
    /// very same typed binding operation as C-BIND; record its fresh join key.
    fn normalize_constant(&mut self, clause: usize, argument: usize) -> syn::Result<()> {
        let expected = self
            .dependencies
            .normalization_rules()
            .nth(self.literal_keys.len());
        if expected != Some(Rule::Constant { clause, argument })
            || self.pending.len() != self.dependencies.clauses.len()
        {
            return Err(error(
                "constant normalization must be complete, unique, and first",
            ));
        }
        let atom = self.source.program.query.body[clause].positive_atom()?;
        let declaration = self.declaration(atom)?;
        let expression = &atom.args[argument];
        let ty = &declaration.columns[argument].ty;
        let key = fresh(&mut self.names, clause, argument);
        self.extend_binding(&syn::parse_quote!(let #key: #ty = #expression));
        self.literal_keys.insert((clause, argument), key);
        Ok(())
    }

    /// C-GUARD: P := select P if e; consume the ready guard exactly once.
    fn guard_clause(&mut self, clause: usize, filter: &cq::Filter) {
        let expression = &filter.condition;
        self.select_prefix(quote!(#expression));
        self.pending.remove(&clause);
    }

    /// C-BIND: P := extend P with the fresh pattern bindings evaluated per row.
    fn bind_clause(&mut self, clause: usize, binding: &cq::Let) {
        self.extend_binding(binding);
        self.pending.remove(&clause);
    }

    /// C-MATCH: A-* prepares each positional attribute, then Rename, Join,
    /// and the accumulated same-occurrence equalities consume this atom.
    fn match_clause(&mut self, clause: usize, atom: &cq::Atom) -> syn::Result<()> {
        let declaration = self.declaration(atom)?;
        let mut arguments = AtomBindings {
            prefix_heading: self.heading.clone(),
            local_variables: Heading::new(),
            equalities: Vec::new(),
        };
        let mut mapping = Vec::new();
        for (position, (argument, column)) in atom.args.iter().zip(&declaration.columns).enumerate()
        {
            let target = arguments.match_argument(self, clause, position, argument, &column.ty);
            self.heading.insert(symbol_name(&target));
            let source = &column.name;
            mapping.push(quote!(#source -> #target));
        }
        let relation = &atom.relation;
        let right = self.emit(syn::parse_quote!(rename #relation { #(#mapping),* }));
        self.prefix = Some(if let Some(left) = self.prefix.clone() {
            self.emit(syn::parse_quote!(natural_join #left with #right))
        } else {
            right
        });
        for equality in arguments.equalities {
            self.select_prefix(equality);
        }
        self.pending.remove(&clause);
        Ok(())
    }

    /// C-FINISH: require complete clause coverage, retain head dependencies
    /// (all witnesses for a computed head), then establish the unique output.
    fn finish(&mut self) -> syn::Result<()> {
        self.require_normalized()?;
        if !self.pending.is_empty() {
            return Err(error("Finish requires every clause exactly once"));
        }
        let head = &self.source.program.query.head;
        let retained = if head.variables().count() < head.args.len() {
            self.heading.clone()
        } else {
            head.args
                .iter()
                .flat_map(|e| cq::rust::free_variables(e).dependencies(&self.heading))
                .collect()
        };
        let attributes: Vec<_> = retained.iter().map(|v| format_ident!("r#{v}")).collect();
        let input = self.prefix_or_unit();
        let result = self.emit(syn::parse_quote!(project #input keep { #(#attributes),* }));
        self.prefix = Some(result.clone());
        self.heading = retained;
        self.output = Some(syn::parse_quote!(output #result as #head;));
        Ok(())
    }

    fn declaration(&self, atom: &cq::Atom) -> syn::Result<&'a cq::RelationDecl> {
        self.source
            .program
            .inputs()
            .find(|d| symbol_name(&d.name) == symbol_name(&atom.relation))
            .ok_or_else(|| error("undeclared atom"))
    }

    // RA constructors used by the rules above. These do not choose a schedule.
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

    fn prefix_or_unit(&mut self) -> Ident {
        if let Some(result) = &self.prefix {
            return result.clone();
        }
        let result = self.emit(syn::parse_quote!(unit));
        self.prefix = Some(result.clone());
        result
    }

    fn extend_prefix(&mut self, columns: TokenStream, value: TokenStream) {
        let input = self.prefix_or_unit();
        self.prefix =
            Some(self.emit(syn::parse_quote!(extend #input with { #columns } value (#value))));
    }

    fn select_prefix(&mut self, expression: TokenStream) {
        let input = self.prefix_or_unit();
        self.prefix = Some(self.emit(syn::parse_quote!(select #input if (#expression))));
    }

    fn extend_binding(&mut self, binding: &cq::Let) {
        let variables = cq::rust::bindings(&binding.pattern);
        let typed = binding.binding_types();
        let columns = typed.iter().map(|(v, ty)| quote!(#v: #ty));
        let pattern = &binding.pattern;
        let annotation = &binding.annotation;
        let expression = &binding.expression;
        let result = (!variables.is_empty()).then(|| quote!((#(#variables,)*)));
        self.extend_prefix(
            quote!(#(#columns),*),
            quote!({ let #pattern #annotation = #expression; #result }),
        );
        self.heading.extend(variables.iter().map(symbol_name));
    }

    fn into_module(self) -> syn::Result<ra::Module> {
        let output = self
            .output
            .ok_or_else(|| error("clause derivation requires Finish"))?;
        Ok(ra::Module {
            program: self.source.program.clone(),
            relational_token: Default::default(),
            relational_brace: Default::default(),
            plan: ra::Plan {
                definitions: self.definitions,
                output,
            },
        })
    }
}

/// A-* judgment context for one atom: the original prefix B, earlier logical
/// arguments L, and equalities to apply after the atom's positional rename/join.
struct AtomBindings {
    prefix_heading: Heading,
    local_variables: Heading,
    equalities: Vec<TokenStream>,
}

impl AtomBindings {
    fn match_argument(
        &mut self,
        state: &mut ClauseState<'_>,
        clause: usize,
        position: usize,
        argument: &syn::Expr,
        ty: &syn::Type,
    ) -> Ident {
        // A-CONSTANT: the C-CONSTANT binding is the column's natural-join key.
        if let Some(key) = state.literal_keys.get(&(clause, position)) {
            return key.clone();
        }
        if let Some(variable) = cq::rust::variable(argument) {
            if self.local_variables.insert(symbol_name(variable)) {
                // A-VARIABLE: retain its name, also joining any prefix binding.
                return variable.clone();
            }
            // A-REPEAT: positional columns remain distinct until equality.
            let fresh = fresh(&mut state.names, clause, position);
            self.equalities.push(quote!(#fresh == #variable));
            return fresh;
        }
        let fresh = fresh(&mut state.names, clause, position);
        if matches!(argument, syn::Expr::Infer(_)) {
            // A-WILDCARD: retain a distinct witness column without an equality.
            return fresh;
        }
        let introduced_here = self
            .local_variables
            .difference(&self.prefix_heading)
            .cloned()
            .collect();
        if cq::rust::free_variables(argument)
            .dependencies(&introduced_here)
            .is_empty()
        {
            // A-PREFIX: evaluate once per prefix tuple, before matching this atom.
            state.extend_prefix(quote!(#fresh: #ty), quote!((#argument,)));
        } else {
            // A-LOCAL: earlier columns of this atom are required by the expression.
            self.equalities.push(quote!(#fresh.eq(&(#argument))));
        }
        fresh
    }
}

pub(in crate::prefix) fn plan(
    source: &cq::Module,
    dependencies: &ClauseDependencies,
    options: PlanningOptions,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    let mut state = ClauseState::new(source, dependencies);
    let mut derivation = Vec::new();
    for rule in dependencies.normalization_rules() {
        state.apply(&rule)?;
        derivation.push(rule);
    }
    while !state.pending.is_empty() {
        let clause = super::strategy::choose_clause(&state, options.strategy)?;
        let rule = match &source.program.query.body[clause] {
            cq::BodyItem::Positive { .. } => Rule::Match { clause },
            cq::BodyItem::Filter(_) => Rule::Guard { clause },
            cq::BodyItem::Let(_) => Rule::Bind { clause },
            cq::BodyItem::Negation(_) | cq::BodyItem::Aggregate(_) => {
                return Err(error("clause has no implemented administrative rule"));
            }
        };
        state.apply(&rule)?;
        derivation.push(rule);
    }
    state.apply(&Rule::Finish)?;
    derivation.push(Rule::Finish);
    let target = state.into_module()?;
    let peak_arity = check_arity(&target, options)?;
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

pub(in crate::prefix) fn replay(
    source: &cq::Module,
    dependencies: &ClauseDependencies,
    report: &JoinPlanReport,
) -> syn::Result<ra::Module> {
    if report.join_tree.is_some() || !report.analysis.is_empty() {
        return Err(error("clause rules do not claim a join-tree certificate"));
    }
    let mut state = ClauseState::new(source, dependencies);
    for rule in &report.derivation {
        state.apply(rule)?;
    }
    let target = state.into_module()?;
    check_arity(&target, report.options)?;
    Ok(target)
}

fn check_arity(target: &ra::Module, options: PlanningOptions) -> syn::Result<usize> {
    let peak = ra::check_in(target.program.inputs(), &target.plan)?
        .values()
        .map(|r| r.heading.len())
        .max()
        .unwrap_or(0);
    if options.max_arity.is_some_and(|limit| peak > limit) {
        return Err(error("Rust clause schedule exceeds max_arity"));
    }
    Ok(peak)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reject_unchanged(state: &mut ClauseState<'_>, rule: Rule) {
        let snapshot = |state: &ClauseState<'_>| {
            (
                state.definitions.clone(),
                state.prefix.clone(),
                state.heading.clone(),
                state.pending.clone(),
                state.literal_keys.clone(),
                state.output.clone(),
                state.names.clone(),
            )
        };
        let before = snapshot(state);
        assert!(state.apply(&rule).is_err(), "{rule:?}");
        assert_eq!(
            snapshot(state),
            before,
            "rejected {rule:?} changed the state"
        );
    }

    #[test]
    fn clause_transitions_check_premises_before_any_state_change() {
        let source: cq::Module = syn::parse_quote! {
            struct P; relation R(a:i32,b:i32); relation S(c:i32);
            answer(x) :- R(7,x), let y:i32 = *x + 1, if y > 0, S(z);
        };
        cq::contract::check(&source).unwrap();
        let dependencies = ClauseDependencies::analyze(&source).unwrap();
        let mut state = ClauseState::new(&source, &dependencies);
        reject_unchanged(&mut state, Rule::Finish);
        reject_unchanged(&mut state, Rule::Match { clause: 0 });
        reject_unchanged(
            &mut state,
            Rule::Constant {
                clause: 0,
                argument: 1,
            },
        );
        state
            .apply(&Rule::Constant {
                clause: 0,
                argument: 0,
            })
            .unwrap();
        reject_unchanged(
            &mut state,
            Rule::Constant {
                clause: 0,
                argument: 0,
            },
        );
        reject_unchanged(&mut state, Rule::Guard { clause: 0 });
        reject_unchanged(&mut state, Rule::Match { clause: 99 });
        reject_unchanged(&mut state, Rule::Bind { clause: 1 });
        reject_unchanged(&mut state, Rule::Match { clause: 3 });
        assert_eq!(
            state.ready_clauses().map(|(i, _)| i).collect::<Vec<_>>(),
            [0]
        );
        state.apply(&Rule::Match { clause: 0 }).unwrap();
        reject_unchanged(&mut state, Rule::Match { clause: 0 });
        reject_unchanged(&mut state, Rule::Finish);
        state.apply(&Rule::Bind { clause: 1 }).unwrap();
        assert_eq!(
            state.ready_clauses().map(|(i, _)| i).collect::<Vec<_>>(),
            [2, 3]
        );
        // Both remaining clauses are legal: replay need not obey policy preference.
        state.apply(&Rule::Match { clause: 3 }).unwrap();
        state.apply(&Rule::Guard { clause: 2 }).unwrap();
        state.apply(&Rule::Finish).unwrap();
        reject_unchanged(&mut state, Rule::Finish);
        reject_unchanged(&mut state, Rule::Match { clause: 0 });
        let target = state.into_module().unwrap();
        crate::prefix::check::clauses::check_scheduled(&source, &target, &[0, 1, 3, 2]).unwrap();
    }
}
