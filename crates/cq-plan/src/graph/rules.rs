//! P-* assembly and Y-* semijoin transitions over an occurrence frontier.
//! Rule premises and state updates follow doc/formal/planning.tex.
//! The independent prefix module owns C-* clause transitions.
use super::{Heading, JoinTree, Rule, error};
use cq_ir::{cq, cq::Predicate, relational_plan as ra, symbol_name};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct FrontierEntry {
    pub coverage: BTreeSet<usize>,
    pub heading: Heading,
    pub result: syn::Ident,
}

struct AssemblyState {
    frontier: BTreeMap<usize, FrontierEntry>,
    pending: BTreeSet<usize>,
}

struct ReductionState {
    tree: JoinTree,
    current: Vec<syn::Ident>,
    up: BTreeSet<usize>,
    down: BTreeSet<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Starting,
    Local,
    Reducing,
    Assembling,
    Finished,
}

pub(super) struct JoinState<'a> {
    source: &'a cq::Module,
    occurrences: Vec<&'a cq::Atom>,
    edges: Vec<Heading>,
    predicates: Vec<Predicate>,
    head: Heading,
    assembly: AssemblyState,
    reduction: Option<ReductionState>,
    phase: Phase,
    introduced: usize,
    definitions: Vec<ra::Definition>,
    output: Option<ra::Output>,
    max_arity: Option<usize>,
}

impl<'a> JoinState<'a> {
    pub fn new(source: &'a cq::Module, tree: Option<JoinTree>, max_arity: Option<usize>) -> Self {
        let occurrences: Vec<_> = source
            .program
            .query
            .body
            .iter()
            .filter_map(cq::BodyItem::atom)
            .collect();
        let edges = occurrences
            .iter()
            .map(|item| item.variables().map(symbol_name).collect())
            .collect();
        let predicates: Vec<_> = source
            .program
            .query
            .body
            .iter()
            .filter_map(|item| match item {
                cq::BodyItem::Filter(filter) => {
                    Some(cq::Predicate::from_expr(&filter.condition).expect("graph domain checked"))
                }
                _ => None,
            })
            .collect();
        Self {
            source,
            occurrences,
            edges,
            head: source
                .program
                .query
                .head
                .variables()
                .map(symbol_name)
                .collect(),
            assembly: AssemblyState {
                frontier: BTreeMap::new(),
                pending: (0..predicates.len()).collect(),
            },
            predicates,
            reduction: tree.map(|tree| ReductionState {
                tree,
                current: Vec::new(),
                up: BTreeSet::new(),
                down: BTreeSet::new(),
            }),
            phase: Phase::Starting,
            introduced: 0,
            definitions: Vec::new(),
            output: None,
            max_arity,
        }
    }

    /// An invalid command leaves all state and emitted definitions unchanged.
    pub fn apply(&mut self, rule: &Rule) -> syn::Result<()> {
        // Each rule checks all fallible premises before its single emission
        // and infallible state update. No whole-program rollback copy is needed.
        match *rule {
            Rule::Match { .. }
            | Rule::Guard { .. }
            | Rule::Bind { .. }
            | Rule::Constant { .. }
            | Rule::Finish => Err(error("clause rule requires a prefix state")),
            Rule::Introduce { occurrence } => self.introduce(occurrence),
            Rule::Select { group, predicate } => self.select(group, predicate),
            Rule::Begin => self.begin(),
            Rule::Up { child } => self.up(child),
            Rule::Down { child } => self.down(child),
            Rule::Freeze => self.freeze(),
            Rule::Combine { left, right } => self.combine(left, right),
            Rule::Trim { group } => self.trim(group),
            Rule::Output { group } => self.output(group),
        }
    }

    /// P-START: introduce the next occurrence with a complete positional rename.
    fn introduce(&mut self, occurrence: usize) -> syn::Result<()> {
        self.require_phase(Phase::Starting)?;
        if occurrence != self.introduced || occurrence >= self.edges.len() {
            return Err(error("introduce each original occurrence exactly once"));
        }
        let heading = self.edges[occurrence].clone();
        let atom = self.occurrences[occurrence];
        let input = self
            .source
            .program
            .inputs()
            .find(|r| symbol_name(&r.name) == symbol_name(&atom.relation))
            .unwrap();
        let mapping = input
            .columns
            .iter()
            .zip(atom.variables())
            .map(|(column, variable)| {
                let column = &column.name;
                quote!(#column -> #variable)
            });
        let relation = &atom.relation;
        let result = self.emit(
            syn::parse_quote!(rename #relation { #(#mapping),* }),
            &heading,
        )?;
        self.assembly.frontier.insert(
            occurrence,
            FrontierEntry {
                coverage: BTreeSet::from([occurrence]),
                heading,
                result,
            },
        );
        self.introduced += 1;
        if self.introduced == self.edges.len() {
            self.phase = Phase::Local;
        }
        Ok(())
    }

    /// P-SELECT: consume one available predicate and replace its frontier result.
    fn select(&mut self, group: usize, predicate: usize) -> syn::Result<()> {
        if !matches!(self.phase, Phase::Local | Phase::Assembling) {
            return Err(error("selection requires local or assembly phase"));
        }
        let g = self.group(group)?.clone();
        if !self.assembly.pending.contains(&predicate) {
            return Err(error("predicate must be pending"));
        }
        let condition = &self.predicates[predicate];
        if !condition.variables().is_subset(&g.heading) {
            return Err(error("selection variables are not available"));
        }
        let input = &g.result;
        let result = self.emit(syn::parse_quote!(select #input if (#condition)), &g.heading)?;
        self.assembly.frontier.get_mut(&group).unwrap().result = result;
        self.assembly.pending.remove(&predicate);
        Ok(())
    }

    /// P-BEGIN: local selections precede reduction or assembly.
    fn begin(&mut self) -> syn::Result<()> {
        self.require_phase(Phase::Local)?;
        if self.ready_selection().is_some() {
            return Err(error(
                "apply available local selections before reduction/assembly",
            ));
        }
        self.phase = if let Some(reduction) = &mut self.reduction {
            reduction.current = (0..self.edges.len())
                .map(|i| self.assembly.frontier[&i].result.clone())
                .collect();
            Phase::Reducing
        } else {
            Phase::Assembling
        };
        Ok(())
    }

    /// Y-UP: children are complete; reduce the parent using this child.
    fn up(&mut self, child: usize) -> syn::Result<()> {
        let parent = self.reduction_parent(child)?;
        let reduction = self.reduction.as_ref().unwrap();
        if reduction.up.contains(&child)
            || !reduction
                .tree
                .children(child)
                .all(|i| reduction.up.contains(&i))
            || !reduction.down.is_empty()
        {
            return Err(error(
                "upward rule requires completed children and an unused tree edge",
            ));
        }
        self.semijoin(parent, child)?;
        self.reduction.as_mut().unwrap().up.insert(child);
        Ok(())
    }

    /// Y-DOWN: the upward pass and the parent's downward step are complete.
    fn down(&mut self, child: usize) -> syn::Result<()> {
        let parent = self.reduction_parent(child)?;
        let reduction = self.reduction.as_ref().unwrap();
        if reduction.up.len() + 1 != self.edges.len()
            || reduction.down.contains(&child)
            || (reduction.tree.parent[parent].is_some() && !reduction.down.contains(&parent))
        {
            return Err(error(
                "downward rule requires completed upward pass and reduced parent",
            ));
        }
        self.semijoin(child, parent)?;
        self.reduction.as_mut().unwrap().down.insert(child);
        Ok(())
    }

    /// Y-FREEZE: both complete passes transfer immutable occurrence references.
    fn freeze(&mut self) -> syn::Result<()> {
        self.require_phase(Phase::Reducing)?;
        let r = self.reduction.as_ref().unwrap();
        if r.up.len() + 1 != self.edges.len() || r.down.len() + 1 != self.edges.len() {
            return Err(error("freeze requires both complete semijoin passes"));
        }
        let frozen = self.reduction.take().unwrap();
        for (i, result) in frozen.current.into_iter().enumerate() {
            self.assembly.frontier.get_mut(&i).unwrap().result = result;
        }
        self.phase = Phase::Assembling;
        Ok(())
    }

    /// P-COMBINE: join disjoint coverages after eager selection and trimming.
    fn combine(&mut self, left: usize, right: usize) -> syn::Result<()> {
        self.require_phase(Phase::Assembling)?;
        if self.ready_selection().is_some() || self.next_trim().is_some() {
            return Err(error(
                "combine requires eager selection and exact boundary pruning",
            ));
        }
        let l = self.group(left)?.clone();
        let r = self.group(right)?.clone();
        if !l.coverage.is_disjoint(&r.coverage) {
            return Err(error("combine requires disjoint occurrence coverage"));
        }
        let heading = l.heading.union(&r.heading).cloned().collect();
        let a = &l.result;
        let b = &r.result;
        let result = self.emit(syn::parse_quote!(natural_join #a with #b), &heading)?;
        self.assembly.frontier.remove(&right);
        self.assembly.frontier.insert(
            left,
            FrontierEntry {
                coverage: l.coverage.union(&r.coverage).copied().collect(),
                heading,
                result,
            },
        );
        Ok(())
    }

    /// P-TRIM: project to K(C,P) after all ready selections.
    fn trim(&mut self, group: usize) -> syn::Result<()> {
        self.require_phase(Phase::Assembling)?;
        if self.ready_selection().is_some() {
            return Err(error("select ready predicates before pruning"));
        }
        let g = self.group(group)?.clone();
        let heading = self.retained_heading(&g);
        if heading == g.heading {
            return Err(error("trim must remove an unused column"));
        }
        let attributes = heading.iter().map(|x| ident(x)).collect::<Vec<_>>();
        let input = &g.result;
        let result = self.emit(
            syn::parse_quote!(project #input keep { #(#attributes),* }),
            &heading,
        )?;
        self.assembly.frontier.insert(
            group,
            FrontierEntry {
                heading,
                result,
                ..g
            },
        );
        Ok(())
    }

    /// P-OUTPUT: complete coverage, no predicates, and exactly the head.
    fn output(&mut self, group: usize) -> syn::Result<()> {
        self.require_phase(Phase::Assembling)?;
        let g = self.group(group)?;
        if self.assembly.frontier.len() != 1
            || g.coverage.len() != self.edges.len()
            || !self.assembly.pending.is_empty()
            || g.heading != self.head
        {
            return Err(error(
                "output requires complete coverage, all predicates, and exactly the head",
            ));
        }
        let result = &g.result;
        let head = &self.source.program.query.head;
        self.output = Some(syn::parse2(quote!(output #result as #head;))?);
        self.phase = Phase::Finished;
        Ok(())
    }

    /// K(C,P): head, outside occurrences, and every still-pending predicate.
    fn retained_heading(&self, group: &FrontierEntry) -> Heading {
        let mut needed = self.head.clone();
        for (i, edge) in self.edges.iter().enumerate() {
            if !group.coverage.contains(&i) {
                needed.extend(edge.iter().cloned());
            }
        }
        for &p in &self.assembly.pending {
            needed.extend(self.predicates[p].variables());
        }
        group.heading.intersection(&needed).cloned().collect()
    }

    pub fn ready_selection(&self) -> Option<Rule> {
        for &predicate in &self.assembly.pending {
            for (&group, g) in &self.assembly.frontier {
                if self.predicates[predicate].variables().is_subset(&g.heading) {
                    return Some(Rule::Select { group, predicate });
                }
            }
        }
        None
    }

    pub fn next_trim(&self) -> Option<Rule> {
        self.assembly.frontier.iter().find_map(|(&group, g)| {
            (self.retained_heading(g) != g.heading).then_some(Rule::Trim { group })
        })
    }

    pub fn frontier(&self) -> &BTreeMap<usize, FrontierEntry> {
        &self.assembly.frontier
    }

    pub fn into_module(self) -> syn::Result<ra::Module> {
        self.require_phase(Phase::Finished)?;
        Ok(ra::Module {
            program: self.source.program.clone(),
            relational_token: Default::default(),
            relational_brace: Default::default(),
            plan: ra::Plan {
                definitions: self.definitions,
                output: self.output.unwrap(),
            },
        })
    }

    fn require_phase(&self, phase: Phase) -> syn::Result<()> {
        if self.phase == phase {
            Ok(())
        } else {
            Err(error("administrative rule used in the wrong phase"))
        }
    }

    fn group(&self, id: usize) -> syn::Result<&FrontierEntry> {
        self.assembly
            .frontier
            .get(&id)
            .ok_or_else(|| error("rule must consume a current frontier entry"))
    }

    fn reduction_parent(&self, child: usize) -> syn::Result<usize> {
        self.require_phase(Phase::Reducing)?;
        self.reduction
            .as_ref()
            .unwrap()
            .tree
            .parent
            .get(child)
            .copied()
            .flatten()
            .ok_or_else(|| error("reduction requires a non-root tree edge"))
    }

    /// Shared RA operation for Y-UP/Y-DOWN, after their distinct premises hold.
    fn semijoin(&mut self, target: usize, support: usize) -> syn::Result<()> {
        let reduction = self.reduction.as_ref().unwrap();
        let left = reduction.current[target].clone();
        let right = reduction.current[support].clone();
        let heading = self.edges[target].clone();
        let result = self.emit(syn::parse_quote!(semijoin #left with #right), &heading)?;
        self.reduction.as_mut().unwrap().current[target] = result;
        Ok(())
    }

    fn emit(&mut self, operator: ra::Operator, heading: &Heading) -> syn::Result<syn::Ident> {
        if self.max_arity.is_some_and(|limit| heading.len() > limit) {
            return Err(error(format!(
                "emitted relation arity {} exceeds configured limit",
                heading.len()
            )));
        }
        let result = format_ident!("r{}", self.definitions.len());
        self.definitions.push(ra::Definition {
            result: result.clone(),
            eq_token: Default::default(),
            operator,
            semi_token: Default::default(),
        });
        Ok(result)
    }
}

pub(super) fn ident(name: &str) -> syn::Ident {
    syn::parse_str(name)
        .unwrap_or_else(|_| syn::Ident::new_raw(name, proc_macro2::Span::call_site()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_rule_keeps_state_and_definitions_unchanged() {
        let source: cq::Module = syn::parse_quote! {
            struct Atomic; relation R(a:i32,b:i32); answer(a) :- R(a,b);
        };
        let mut state = JoinState::new(&source, None, Some(1));
        assert!(state.apply(&Rule::Introduce { occurrence: 0 }).is_err());
        assert!(state.definitions.is_empty() && state.assembly.frontier.is_empty());
        assert_eq!(state.introduced, 0);
        assert!(state.phase == Phase::Starting);
        state.max_arity = Some(2);
        state.apply(&Rule::Introduce { occurrence: 0 }).unwrap();
        let definitions = state.definitions.clone();
        assert!(state.apply(&Rule::Introduce { occurrence: 0 }).is_err());
        assert_eq!(state.definitions, definitions);
        assert_eq!(state.assembly.frontier.len(), 1);
        assert_eq!(state.introduced, 1);
        assert!(state.phase == Phase::Local);
    }
}
