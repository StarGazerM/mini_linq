//! The administrative kernel. Policies choose rules; only this module emits RA.
use super::{
    analysis::{Heading, JoinTree},
    error,
};
use cq_ir::{cq, cq::Predicate, relational_plan as ra, symbol_name};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    Introduce { occurrence: usize },
    Select { group: usize, predicate: usize },
    Begin,
    Up { child: usize },
    Down { child: usize },
    Freeze,
    Combine { left: usize, right: usize },
    Trim { group: usize },
    Output { group: usize },
}

#[derive(Clone)]
pub(super) struct Group {
    pub coverage: BTreeSet<usize>,
    pub heading: Heading,
    pub result: syn::Ident,
}

struct AssemblyState {
    frontier: BTreeMap<usize, Group>,
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

pub(super) struct Kernel<'a> {
    source: &'a cq::Module,
    occurrences: Vec<&'a cq::Atom>,
    edges: Vec<Heading>,
    predicates: Vec<&'a Predicate>,
    head: Heading,
    assembly: AssemblyState,
    reduction: Option<ReductionState>,
    phase: Phase,
    introduced: usize,
    definitions: Vec<ra::Definition>,
    output: Option<ra::Output>,
    max_arity: Option<usize>,
}

impl<'a> Kernel<'a> {
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
            .map(|item| item.variables.iter().map(symbol_name).collect())
            .collect();
        let predicates: Vec<_> = source
            .program
            .query
            .body
            .iter()
            .filter_map(|item| match item {
                cq::BodyItem::Filter(filter) => Some(&filter.condition),
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
                .variables
                .iter()
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
            Rule::Introduce { occurrence } => self.introduce(occurrence),
            Rule::Select { group, predicate } => self.select(group, predicate),
            Rule::Begin => {
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
            Rule::Up { child } => self.reduce(child, false),
            Rule::Down { child } => self.reduce(child, true),
            Rule::Freeze => {
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
            Rule::Combine { left, right } => self.combine(left, right),
            Rule::Trim { group } => self.trim(group),
            Rule::Output { group } => {
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
                self.output = Some(syn::parse2(quote!(output #result as #head.))?);
                self.phase = Phase::Finished;
                Ok(())
            }
        }
    }

    fn require_phase(&self, phase: Phase) -> syn::Result<()> {
        if self.phase == phase {
            Ok(())
        } else {
            Err(error("administrative rule used in the wrong phase"))
        }
    }

    fn group(&self, id: usize) -> syn::Result<&Group> {
        self.assembly
            .frontier
            .get(&id)
            .ok_or_else(|| error("rule must consume a current frontier entry"))
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
            .inputs
            .iter()
            .find(|r| symbol_name(&r.name) == symbol_name(&atom.relation))
            .unwrap();
        let mapping = input
            .columns
            .iter()
            .zip(&atom.variables)
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
            Group {
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

    fn select(&mut self, group: usize, predicate: usize) -> syn::Result<()> {
        if !matches!(self.phase, Phase::Local | Phase::Assembling) {
            return Err(error("selection requires local or assembly phase"));
        }
        let g = self.group(group)?.clone();
        if !self.assembly.pending.contains(&predicate) {
            return Err(error("predicate must be pending"));
        }
        let condition = self.predicates[predicate];
        if !condition.variables().is_subset(&g.heading) {
            return Err(error("selection variables are not available"));
        }
        let input = &g.result;
        let result = self.emit(syn::parse_quote!(select #input if (#condition)), &g.heading)?;
        self.assembly.frontier.get_mut(&group).unwrap().result = result;
        self.assembly.pending.remove(&predicate);
        Ok(())
    }

    fn reduce(&mut self, child: usize, down: bool) -> syn::Result<()> {
        self.require_phase(Phase::Reducing)?;
        let r = self.reduction.as_ref().unwrap();
        let parent = r
            .tree
            .parent
            .get(child)
            .copied()
            .flatten()
            .ok_or_else(|| error("reduction requires a non-root tree edge"))?;
        if down {
            if r.up.len() + 1 != self.edges.len()
                || r.down.contains(&child)
                || (r.tree.parent[parent].is_some() && !r.down.contains(&parent))
            {
                return Err(error(
                    "downward rule requires completed upward pass and reduced parent",
                ));
            }
        } else if r.up.contains(&child)
            || !r.tree.children(child).all(|i| r.up.contains(&i))
            || !r.down.is_empty()
        {
            return Err(error(
                "upward rule requires completed children and an unused tree edge",
            ));
        }
        let (target, support) = if down {
            (child, parent)
        } else {
            (parent, child)
        };
        let left = r.current[target].clone();
        let right = r.current[support].clone();
        let heading = self.edges[target].clone();
        let result = self.emit(syn::parse_quote!(semijoin #left with #right), &heading)?;
        let r = self.reduction.as_mut().unwrap();
        r.current[target] = result;
        if down {
            r.down.insert(child);
        } else {
            r.up.insert(child);
        }
        Ok(())
    }

    /// K(C,P): head, outside occurrences, and every still-pending predicate.
    fn keep(&self, group: &Group) -> Heading {
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

    pub fn next_trim(&self) -> Option<Rule> {
        self.assembly
            .frontier
            .iter()
            .find_map(|(&group, g)| (self.keep(g) != g.heading).then_some(Rule::Trim { group }))
    }

    fn trim(&mut self, group: usize) -> syn::Result<()> {
        self.require_phase(Phase::Assembling)?;
        if self.ready_selection().is_some() {
            return Err(error("select ready predicates before pruning"));
        }
        let g = self.group(group)?.clone();
        let heading = self.keep(&g);
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
            Group {
                heading,
                result,
                ..g
            },
        );
        Ok(())
    }

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
            Group {
                coverage: l.coverage.union(&r.coverage).copied().collect(),
                heading,
                result,
            },
        );
        Ok(())
    }

    pub fn frontier(&self) -> &BTreeMap<usize, Group> {
        &self.assembly.frontier
    }

    pub fn finish(self) -> syn::Result<ra::Module> {
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
            struct Atomic; relation R(a:i32,b:i32); answer(a) :- R(a,b).
        };
        let mut state = Kernel::new(&source, None, Some(1));
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
