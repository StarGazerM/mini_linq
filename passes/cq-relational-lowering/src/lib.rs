//! Relational lowering, VO to RA prime: a mechanical translation that applies
//! every recorded relational decision and preserves Rust payloads.
//!
//! Every Rust expression is opaque, pure and total by premise. Its only placement
//! requirements are its free variables. [`check`] compares fixed VO clause
//! occurrences with `rename R@clause`, positional bindings, equality closure,
//! explicit relational constraints, named reductions, Rust payloads and output.
//! Partial subatoms retain their projection and correlation obligations.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! Lowering projects arms to their common heading, emits explicit union/anti equations and names negative originals.
//! Recorded hoisted blocks use free/interface columns and a finite prefix domain for missing branch generators.
//! Recorded correlated arms retain their free prefix and relational anti dependencies.
//! Rust occurrence scopes record only free-variable dependencies, not the written prefix.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Union siblings are unordered; fallback retains its relational anti steps.
//! Only fallback construction proves union arms disjoint; literal values prove nothing.
//! CQ negations bind nothing and lower to anti joins over constant-selected,
//! existentially projected input relations, reusing the block anti machinery.
//!
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ra as ra;
use cq_lang_vo as valuation;
use cq_lang_vo::syntax as cq;
use cq_lang_vo::syntax::symbol_name;
use quote::format_ident;
use syn::Ident;

/// Lower a valuation plan into named RA, then check the RA contract.
/// Recorded restrictions select their occurrence; every other predicate is a
/// prefix step. A written reference is a VO without restrictions or reductions.
/// Formal rule RL-Guard: Append an opaque guard to the prefix, seeding Unit if absent.
pub fn compile(source: &valuation::Program) -> syn::Result<ra::Program> {
    compile_recorded(source, false).map(|(program, _)| program)
}

/// Actual emission witness. Paths use VO step/branch/step indices; `$output`
/// records return projection and an otherwise empty query's Unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Origins {
    pub definitions: Vec<(String, Vec<String>)>,
    pub elided: Vec<String>,
}

/// Run exactly the ordinary lowering while recording its emission decisions.
pub fn compile_with_origins(source: &valuation::Program) -> syn::Result<(ra::Program, Origins)> {
    let (program, origins) = compile_recorded(source, true)?;
    Ok((program, origins.expect("enabled origin recorder")))
}

/// Independently reproduce the mechanical emission witness, not the planner.
pub fn check_origins(source: &valuation::Program, target: &ra::Program, origins: &Origins) -> syn::Result<()> {
    let (expected, witness) = compile_with_origins(source)?;
    if expected != *target || witness != *origins {
        return Err(syn::Error::new_spanned(target, "RL-Origin: emission witness differs from actual lowering"));
    }
    Ok(())
}

#[derive(Default)]
struct Recorder {
    scope: String,
    active: BTreeSet<String>,
    definitions: BTreeMap<String, BTreeSet<String>>,
    elided: Vec<String>,
}

fn compile_recorded(source: &valuation::Program, record: bool) -> syn::Result<(ra::Program, Option<Origins>)> {
    let analysis = valuation::contract::check(source)?;
    let has_blocks = valuation::cq::blocks::walk(&source.cq.query.body)
        .iter()
        .any(|item| matches!(item, valuation::cq::BodyItem::Block(_)));

    let mut state = State {
        recorder: record.then(Recorder::default),
        inputs: &source.cq.inputs().cloned().collect::<Vec<_>>(),
        atoms: &analysis.atoms,
        analysis: &analysis,
        body: &source.cq.query.body,
        steps: &source.valuation.steps,
        context: BTreeSet::new(),
        path: None,
        fused: BTreeSet::new(),
        scoped: has_blocks,
        placed_rust: BTreeSet::new(),
        read_clauses: BTreeSet::new(),
        hypergraph: valuation::cq::hypergraph::Analysis::of(&source.cq),
        definitions: Vec::new(),
        reserved: source
            .cq
            .declarations
            .iter()
            .map(|declaration| symbol_name(&declaration.name))
            .collect(),
        next_result: 0,
        relations: BTreeMap::new(),
        prefix: None,
        heading: BTreeSet::new(),
        restrictions: Vec::new(),
    };
    state.scope_restrictions();
    state.run_steps(&source.valuation.steps);
    if let Some(recorder) = &mut state.recorder {
        recorder.active = BTreeSet::from(["$output".into()]);
    }
    let output = state.finish(&source.cq.query);
    let origins = state.recorder.map(|mut recorder| Origins {
        definitions: state.definitions.iter().map(|d| {
            let name = symbol_name(&d.result);
            let origins = recorder.definitions.remove(&name).expect("recorded emission");
            (name, origins.into_iter().collect())
        }).collect(),
        elided: recorder.elided,
    });
    let target = ra::Program {
        relational_token: Default::default(),
        relational_brace: Default::default(),
        plan: ra::Plan {
            declarations: source.cq.declarations.clone(),
            captures: source.cq.query.captures.clone(),
            definitions: state.definitions,
            output,
        },
    };
    ra::contract::check(&target)?;
    Ok((target, origins))
}

mod block_contract;
mod negation;
mod structural;

/// The RA definitions refine the supplied VO schedule, retaining its unchanged
/// Rust payloads and relational constraints.
/// Pure targets may use checked inverse dead expansions under canonicalization
/// to establish a primary witness, without recompiling or selecting a plan.
/// Formal rule CK-RelationalLowering: Check contracts, clause occurrences, recorded restrictions, named reductions, constraints and unchanged Rust payloads with their free variables.
pub fn check(source: &valuation::Program, target: &ra::Program) -> syn::Result<()> {
    let analysis = valuation::contract::check(source)?;
    check_reference(&source.cq, &source.valuation.steps, &analysis, target)
}

/// Check an occurrence-coverage proof reference, not an executable VO.
/// `analysis` is the WF binding analysis of `source` and `steps`, obtained by
/// `valuation::order::analyze_reference`; this path never chooses an order.
pub fn check_reference(
    source: &valuation::cq::Program,
    steps: &[valuation::Step],
    analysis: &valuation::Analysis,
    target: &ra::Program,
) -> syn::Result<()> {
    let target_schedule = ra::contract::Schedule::from_program(target)?;
    if target.plan.declarations != source.declarations {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            format!(
                "result `{}`: declarations differ from query `{}`",
                target.plan.output.input, source.name
            ),
        ));
    }
    if target.plan.output.head != source.query.head {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            format!(
                "result `{}`: output head differs from `{}`",
                target.plan.output.input, source.query.head.relation
            ),
        ));
    }
    if target.plan.captures != source.query.captures {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            format!(
                "result `{}`: captures differ from query `{}`",
                target.plan.output.input, source.name
            ),
        ));
    }
    structural::check(source, steps, analysis, &target_schedule, target).map_err(|message| {
        syn::Error::new_spanned(
            &target.plan.output,
            format!("result `{}`: {message}", target.plan.output.input),
        )
    })?;
    block_contract::check_placement(steps, target)?;
    check_restrictions(steps, target)?;
    Ok(())
}

/// Formal rule CK-RelationalLoweringRestrict: Every conjunct of a recorded restriction appears in the selection chain directly over its occurrence's rename.
fn check_restrictions(steps: &[valuation::Step], target: &ra::Program) -> syn::Result<()> {
    let definitions = &target.plan.definitions;
    for step in steps {
        match step {
            valuation::Step::Restrict(restrict, _) => {
                let fail = |message: &str| {
                    Err(syn::Error::new_spanned(
                        restrict,
                        format!("restriction of `{}`: {message}", restrict.clause),
                    ))
                };
                let Some(mut current) = definitions.iter().find_map(|d| match &d.operator {
                    ra::Operator::Rename(rename)
                        if rename.occurrence == restrict.clause
                            && symbol_name(&rename.relation) == symbol_name(&restrict.relation) =>
                    {
                        Some(&d.result)
                    }
                    _ => None,
                }) else {
                    return fail("the occurrence is never read");
                };
                let mut applied = ra::Theta::default();
                while let Some((result, select)) =
                    definitions.iter().find_map(|d| match &d.operator {
                        ra::Operator::Select(select) if select.input == *current => {
                            Some((&d.result, select))
                        }
                        _ => None,
                    })
                {
                    applied
                        .equalities
                        .extend(select.condition.equalities.iter().cloned());
                    applied
                        .constraints
                        .extend(select.condition.constraints.iter().cloned());
                    current = result;
                }
                let mut expected = ra::Theta::default();
                expected.push_constraint(restrict.predicate.clone());
                if !expected
                    .equalities
                    .iter()
                    .all(|e| applied.equalities.iter().any(|a| a == e))
                    || !expected
                        .constraints
                        .iter()
                        .all(|c| applied.constraints.iter().any(|a| a == c))
                {
                    return fail("the predicate does not select the occurrence");
                }
            }
            valuation::Step::Block(block, _) => {
                for arm in &block.branches {
                    check_restrictions(&arm.steps, target)?;
                }
            }
            _ => (),
        }
    }
    Ok(())
}

/// The definitions so far, each clause's current relation, the prefix
/// result and its heading.
struct State<'a> {
    recorder: Option<Recorder>,
    inputs: &'a [cq::RelationDecl],
    atoms: &'a BTreeMap<usize, cq::Atom>,
    analysis: &'a valuation::Analysis,
    body: &'a cq::CommaList<valuation::cq::BodyItem>,
    steps: &'a [valuation::Step],
    context: BTreeSet<String>,
    path: Option<(cq::Occurrence, usize)>,
    fused: BTreeSet<usize>,
    scoped: bool,
    placed_rust: BTreeSet<usize>,
    read_clauses: BTreeSet<usize>,
    hypergraph: valuation::cq::hypergraph::Analysis,
    definitions: Vec<ra::Definition>,
    reserved: BTreeSet<String>,
    next_result: usize,
    relations: BTreeMap<usize, Ident>,
    prefix: Option<Ident>,
    heading: BTreeSet<String>,
    /// The current scope's recorded restrictions `(clause, predicate)`, in VO order.
    restrictions: Vec<(usize, cq::Constraint)>,
}

impl<'a> State<'a> {
    // Restriction targets are local clause numbers: rebuild them on every scope
    // entry so sibling branches and the outer body cannot share them.
    fn scope_restrictions(&mut self) {
        self.restrictions = self
            .steps
            .iter()
            .filter_map(|step| match step {
                valuation::Step::Restrict(restrict, _) => {
                    Some((restrict.clause(), restrict.predicate.clone()))
                }
                _ => None,
            })
            .collect();
    }

    fn run_steps(&mut self, steps: &[valuation::Step]) {
        for (index, (step, introduces)) in steps.iter().zip(&self.analysis.introduces).enumerate() {
            if let Some(recorder) = &mut self.recorder {
                recorder.active = BTreeSet::from([format!("{}{index}", recorder.scope)]);
            }
            match step {
                valuation::Step::Block(block, _) => {
                    let clause = block.source.clause();
                    let right = self.relation(clause);
                    if self.fused.contains(&clause) {
                        self.prefix = Some(right);
                    } else {
                        let check = block
                            .source
                            .variables()
                            .all(|(_, variable)| self.heading.contains(&symbol_name(variable)));
                        self.subatom(&block.source, check);
                    }
                    self.read_clauses.insert(clause);
                }
                valuation::Step::Atom(subatom, _) => {
                    let check = subatom
                        .variables()
                        .all(|(_, variable)| self.heading.contains(&symbol_name(variable)));
                    self.subatom(subatom, check);
                    self.read_clauses.insert(subatom.clause());
                }
                valuation::Step::Reduce(reduce, _) => self.reduce(reduce),
                valuation::Step::Restrict(..) => {}
                valuation::Step::Negation(negation, _) => self.negation(negation),
                valuation::Step::Guard(filter, _) => {
                    let input = self.prefix_or_unit();
                    self.prefix = Some(self.guard(input, filter));
                }
                valuation::Step::Let(binding, _) => self.bind(binding),
                valuation::Step::For(binding, _) => self.for_bind(binding),
                valuation::Step::Equality(equality, _) => self.equality(equality),
                valuation::Step::Constraint(c, _) => self.constraint(c),
            }
            if let Some(recorder) = &mut self.recorder {
                if matches!(step, valuation::Step::Atom(..) | valuation::Step::Block(..)) {
                    if let Some(prefix) = &self.prefix {
                        recorder.definitions.get_mut(&symbol_name(prefix))
                            .expect("recorded prefix").extend(recorder.active.iter().cloned());
                    }
                }
                if !matches!(step, valuation::Step::Restrict(..))
                    && recorder.active.iter().all(|origin|
                        !recorder.definitions.values().any(|origins| origins.contains(origin))) {
                    recorder.elided.extend(recorder.active.iter().cloned());
                }
            }
            self.heading.extend(introduces.iter().cloned());
        }
    }

    /// Formal rule RL-Block: Elaborate the recorded Hoisted or Correlated alternative, never a compiler-mode choice.
    fn block_relation(&mut self, clause: usize) -> Ident {
        let valuation::cq::BodyItem::Block(written) = &self.body[clause] else {
            unreachable!("block clause")
        };
        let planned = self
            .steps
            .iter()
            .find_map(|step| match step {
                valuation::Step::Block(block, _) if block.source.clause() == clause => Some(block),
                _ => None,
            })
            .expect("VO places every block");
        let saved_origin = self.recorder.as_mut().map(|recorder| {
            let index = self.steps.iter().position(|step| matches!(step,
                valuation::Step::Block(block, _) if block.source.clause() == clause)).unwrap();
            let saved = (recorder.scope.clone(), recorder.active.clone());
            recorder.active = BTreeSet::from([format!("{}{index}", recorder.scope)]);
            saved
        });
        let written_prefix = valuation::cq::blocks::bound_before(self.body, clause, &self.context);
        let facts = written.analyze(&written_prefix);
        let hoisted = planned.placement == valuation::blocks::Placement::Hoisted;
        let correlated_prefix = (!hoisted).then(|| self.prefix_or_unit());
        let common: BTreeSet<_> = facts
            .free
            .keys()
            .chain(facts.interface.keys())
            .cloned()
            .collect();
        // If an arm has no generator for an F column, its finite domain is the
        // current prefix projected to F: WF-BlockPlacement and the reduction
        // premises place every request after VO binds F.
        let domain = (hoisted && written.needs_domain(&facts)).then(|| {
            let prefix = self.prefix_or_unit();
            self.project(prefix, facts.free.values().cloned().collect())
        });
        let parent_restrictions = std::mem::take(&mut self.restrictions);
        let parent_atoms = self.atoms;
        let parent_analysis = self.analysis;
        let parent_body = self.body;
        let parent_hypergraph = self.hypergraph.clone();
        let parent_steps = self.steps;
        let parent_context = self.context.clone();
        let parent_path = self.path.clone();
        let parent_prefix = self.prefix.clone();
        let parent_heading = self.heading.clone();
        let parent_relations = std::mem::take(&mut self.relations);
        let parent_fused = std::mem::take(&mut self.fused);
        let parent_placed_rust = std::mem::take(&mut self.placed_rust);
        let parent_reads = std::mem::take(&mut self.read_clauses);
        let mut originals: Vec<Ident> = Vec::new();
        let mut operands = Vec::new();
        for (j, (branch, source_branch)) in
            planned.branches.iter().zip(&written.branches).enumerate()
        {
            if let Some(recorder) = &mut self.recorder {
                let (scope, _) = saved_origin.as_ref().unwrap();
                let index = parent_steps.iter().position(|step| matches!(step,
                    valuation::Step::Block(block, _) if block.source.clause() == clause)).unwrap();
                recorder.scope = format!("{scope}{index}/{j}/");
                recorder.active = BTreeSet::from([format!("{scope}{index}")]);
            }
            self.atoms = &parent_analysis.branches[&clause][j].atoms;
            self.analysis = &parent_analysis.branches[&clause][j];
            self.body = &source_branch.body;
            self.hypergraph = valuation::cq::hypergraph::Analysis::of_body(
                self.body,
                &facts.free.keys().cloned().collect(),
            );
            self.steps = &branch.steps;
            self.path = Some((planned.source.clause.clone(), j));
            self.context = if hoisted {
                facts.free.keys().cloned().collect()
            } else {
                written_prefix.clone()
            };
            self.heading = if hoisted {
                if domain.is_some() {
                    facts.free.keys().cloned().collect()
                } else {
                    BTreeSet::new()
                }
            } else {
                parent_heading.clone()
            };
            self.prefix = correlated_prefix.clone().or_else(|| domain.clone());
            self.relations.clear();
            self.fused.clear();
            self.placed_rust.clear();
            self.read_clauses.clear();
            if !hoisted && written.kind == valuation::cq::blocks::Kind::Fallback {
                for previous in &originals {
                    let left = self.prefix_or_unit();
                    self.prefix = Some(self.anti(
                        left,
                        previous.clone(),
                        facts.free.values().cloned().collect(),
                    ));
                }
            }
            self.scope_restrictions();
            self.run_steps(&branch.steps);
            if let Some(recorder) = &mut self.recorder {
                let (scope, _) = saved_origin.as_ref().unwrap();
                let index = parent_steps.iter().position(|step| matches!(step,
                    valuation::Step::Block(block, _) if block.source.clause() == clause)).unwrap();
                recorder.active = BTreeSet::from([format!("{scope}{index}")]);
            }
            let keep: BTreeSet<_> = if hoisted {
                common.clone()
            } else {
                parent_heading.union(&common).cloned().collect()
            };
            let input = self.prefix_or_unit();
            let attributes = keep
                .iter()
                .map(|name| syn::Ident::new(name, proc_macro2::Span::call_site()))
                .collect();
            let original = self.project(input, attributes);
            originals.push(original.clone());
            let mut positive = original;
            if hoisted && written.kind == valuation::cq::blocks::Kind::Fallback {
                for previous in originals.iter().take(j) {
                    positive = self.anti(
                        positive,
                        previous.clone(),
                        facts.free.values().cloned().collect(),
                    );
                }
            }
            operands.push(positive);
        }
        self.restrictions = parent_restrictions;
        self.atoms = parent_atoms;
        self.analysis = parent_analysis;
        self.body = parent_body;
        self.hypergraph = parent_hypergraph;
        self.steps = parent_steps;
        self.context = parent_context;
        self.path = parent_path;
        self.prefix = parent_prefix;
        self.heading = parent_heading;
        self.relations = parent_relations;
        self.fused = parent_fused;
        self.placed_rust = parent_placed_rust;
        self.read_clauses = parent_reads;
        let result = self.emit(ra::Operator::Union(Box::new(ra::Union {
            union_token: Default::default(),
            operands: operands.into_iter().collect(),
            origin: Some(ra::BlockOrigin {
                block_token: Default::default(),
                at: Default::default(),
                occurrence: planned.source.clause.clone(),
                free_token: Default::default(),
                free_bracket: Default::default(),
                free: facts.free.values().cloned().collect(),
                interface_token: Default::default(),
                interface_bracket: Default::default(),
                interface: facts.interface.values().cloned().collect(),
                fallback: (written.kind == valuation::cq::blocks::Kind::Fallback)
                    .then(Default::default),
                over: correlated_prefix.map(|prefix| ra::Correlation {
                    over_token: Default::default(),
                    prefix,
                }),
                domain: domain.map(|input| ra::Domain {
                    domain_token: Default::default(),
                    input,
                }),
            }),
        })));
        if let (Some(recorder), Some((scope, active))) = (&mut self.recorder, saved_origin) {
            recorder.scope = scope;
            recorder.active = active;
        }
        if !hoisted {
            self.fused.insert(clause);
        }
        self.relations.insert(clause, result.clone());
        result
    }

    /// RL-Neg: constants and repeated columns select the negative relation;
    /// existential columns are projected away before the anti join.
    fn negation(&mut self, negation: &valuation::cq::Negation) {
        let clause = self
            .body
            .iter()
            .enumerate()
            .find_map(|(clause, item)| {
                (!self.read_clauses.contains(&clause)
                    && matches!(item, valuation::cq::BodyItem::Negation(n) if n == negation))
                .then_some(clause)
            })
            .expect("VO negation coverage");
        let bound = valuation::cq::blocks::bound_before(self.body, clause, &self.context);
        let declaration = self
            .inputs
            .iter()
            .find(|d| symbol_name(&d.name) == symbol_name(&negation.atom.relation))
            .expect("contracted negative input");
        let layout = negation::layout(declaration, negation, &bound);
        let filtered =
            !layout.condition.equalities.is_empty() || !layout.condition.constraints.is_empty();
        let needs_projection = layout.keys.len() != layout.mapping.len() && filtered;
        let mut right = self.emit(ra::Operator::Rename(Box::new(ra::Rename {
            rename_token: Default::default(),
            relation: negation.atom.relation.clone(),
            at: Default::default(),
            occurrence: self.path.as_ref().map_or_else(
                || cq::Occurrence::new(clause),
                |(path, branch)| path.child(*branch, clause),
            ),
            mapping_brace: Default::default(),
            mapping: layout
                .mapping
                .into_iter()
                .map(|(source, target)| ra::AttributeRename {
                    source,
                    minus_token: Default::default(),
                    gt_token: Default::default(),
                    target,
                })
                .collect(),
            constants: None,
        })));
        if filtered {
            right = self.select_theta(right, layout.condition);
        }
        // Anti observes only its keys: wildcards stay existential without
        // wrapping a direct host read in a materialized projection.
        if needs_projection {
            right = self.project(right, layout.keys.iter().cloned().collect());
        }
        let left = self.prefix_or_unit();
        self.prefix = Some(self.anti(left, right, layout.keys.into_iter().collect()));
        self.read_clauses.insert(clause);
    }
    fn anti(&mut self, left: Ident, right: Ident, columns: cq::CommaList<Ident>) -> Ident {
        self.emit(ra::Operator::Anti(Box::new(ra::Anti {
            anti_token: Default::default(),
            bracket: Default::default(),
            columns,
            left,
            comma: Default::default(),
            right,
        })))
    }

    /// The clause's relation: its rename, emitted at first mention, as
    /// reduced so far.
    /// Formal rule RL-Occurrence: Reuse a clause result or emit its complete positional rename at first use.
    /// Formal rule RL-Restrict: Select a new occurrence by its recorded literal-equality restrictions, then each same-column group of its other restrictions.
    fn relation(&mut self, clause: usize) -> Ident {
        if let Some(result) = self.relations.get(&clause) {
            return result.clone();
        }
        if matches!(&self.body[clause], valuation::cq::BodyItem::Block(_)) {
            return self.block_relation(clause);
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
            .zip(&atom.args)
            .filter_map(|(column, argument)| {
                cq::rust::variable(argument).map(|variable| ra::AttributeRename {
                    source: column.name.clone(),
                    minus_token: Default::default(),
                    gt_token: Default::default(),
                    target: variable.clone(),
                })
            })
            .collect();
        let constants = occurrence_constants(declaration, atom);
        let mut result = self.emit(ra::Operator::Rename(Box::new(ra::Rename {
            rename_token: Default::default(),
            relation: atom.relation.clone(),
            at: Default::default(),
            occurrence: match &self.path {
                Some((path, branch)) => path.child(*branch, clause),
                None => cq::Occurrence::new(clause),
            },
            mapping_brace: Default::default(),
            mapping,
            constants,
        })));
        // RL-Restrict: the recorded literal equalities are one keyed read of
        // this occurrence; other restrictions group same-column conjuncts
        // into one range selection each.
        let literals: Vec<_> = self
            .restrictions
            .iter()
            .filter(|(target, _)| *target == clause)
            .filter_map(|(_, predicate)| predicate.equality())
            .collect();
        if !literals.is_empty() {
            let saved = self.restriction_origins(clause, |predicate| predicate.equality().is_some());
            result = self.select(result, literals);
            self.restore_origins(saved);
        }
        let mut groups: Vec<(BTreeSet<String>, Vec<cq::Constraint>)> = Vec::new();
        for (target, predicate) in &self.restrictions {
            if *target != clause || predicate.equality().is_some() {
                continue;
            }
            let columns: BTreeSet<_> = predicate.variables().map(symbol_name).collect();
            if let Some((_, constraints)) = groups.iter_mut().find(|(key, _)| *key == columns) {
                constraints.push(predicate.clone());
            } else {
                groups.push((columns, vec![predicate.clone()]));
            }
        }
        for (_, constraints) in groups {
            let saved = self.restriction_origins(clause, |predicate| constraints.contains(predicate));
            result = self.select_constraints(result, constraints);
            self.restore_origins(saved);
        }
        self.relations.insert(clause, result.clone());
        result
    }

    fn restriction_origins(&mut self, clause: usize, includes: impl Fn(&cq::Constraint) -> bool) -> Option<BTreeSet<String>> {
        let recorder = self.recorder.as_mut()?;
        let active = self.steps.iter().enumerate().filter_map(|(index, step)| match step {
            valuation::Step::Restrict(restrict, _) if restrict.clause() == clause && includes(&restrict.predicate) =>
                Some(format!("{}{index}", recorder.scope)),
            _ => None,
        }).collect();
        Some(std::mem::replace(&mut recorder.active, active))
    }

    fn restore_origins(&mut self, saved: Option<BTreeSet<String>>) {
        if let (Some(recorder), Some(active)) = (&mut self.recorder, saved) {
            recorder.active = active;
        }
    }

    /// Formal rule RL-Check: A partial subatom with no new names semijoins an existing prefix.
    /// Formal rule RL-Project: Project a partial subatom unless it is an existing-prefix check.
    /// Formal rule RL-Root: Install the first full or projected subatom as the prefix.
    /// Formal rule RL-Join: Join a non-check subatom with the current prefix.
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

    /// Formal rule RL-Reduce: Match every listed support column to its target equality class and retain only the target heading.
    fn reduce(&mut self, reduce: &valuation::Reduce) {
        let clause = reduce.clause();
        let left = self.relation(clause);
        let result = match &reduce.reduction {
            valuation::Reduction::With(_, subatom) => {
                let right = self.relation(subatom.clause());
                let mut equalities = Vec::new();
                for variable in self.atoms[&clause].variables() {
                    for (_, support) in subatom.variables() {
                        if self
                            .hypergraph
                            .equivalent(&symbol_name(variable), &symbol_name(support))
                        {
                            let left = variable.clone();
                            let right = support.clone();
                            if symbol_name(&left) != symbol_name(&right) {
                                equalities.push(ra::Equality {
                                    left,
                                    eq_token: Default::default(),
                                    right: ra::EqualityValue::Variable(right),
                                });
                            }
                        }
                    }
                }
                if equalities.is_empty() {
                    self.semijoin(left, right)
                } else {
                    // Opaque Rust may protect both names in an equality class.
                    // A natural semijoin would then check only support existence.
                    // Express the equality-class semijoin as join/select/project
                    // over the support's recorded columns, without renaming Rust
                    // inputs or adding a Rust expression.
                    let support = if subatom.variables().count() == subatom.columns.len() {
                        right
                    } else {
                        let columns = subatom.variables().map(|(_, v)| v.clone()).collect();
                        self.project(right, columns)
                    };
                    let joined = self.emit(ra::Operator::NaturalJoin(Box::new(ra::NaturalJoin {
                        natural_join_token: Default::default(),
                        left,
                        with_token: Default::default(),
                        right: support,
                    })));
                    let matched = self.select(joined, equalities);
                    let attributes = self.atoms[&clause].variables().cloned().collect();
                    self.project(matched, attributes)
                }
            }
        };
        if self.read_clauses.contains(&clause) {
            if let Some(prefix) = self.prefix.take() {
                self.prefix = Some(self.semijoin(prefix, result.clone()));
            }
        }
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

    fn select(&mut self, input: Ident, equalities: Vec<ra::Equality>) -> Ident {
        self.select_theta(
            input,
            ra::Theta {
                equalities: equalities.into_iter().collect(),
                constraints: Default::default(),
            },
        )
    }

    fn select_constraints(&mut self, input: Ident, constraints: Vec<cq::Constraint>) -> Ident {
        let mut condition = ra::Theta::default();
        for constraint in constraints {
            condition.push_constraint(constraint);
        }
        self.select_theta(input, condition)
    }

    fn select_theta(&mut self, input: Ident, condition: ra::Theta) -> Ident {
        self.emit(ra::Operator::Select(Box::new(ra::Select {
            select_token: Default::default(),
            input,
            if_token: Default::default(),
            condition_paren: Default::default(),
            condition,
        })))
    }

    /// Formal rule RL-Constraint: Select the prefix by a recorded prefix constraint.
    fn constraint(&mut self, constraint: &cq::Constraint) {
        if let Some(equality) = constraint.equality() {
            self.equality(&equality);
            return;
        }
        let c = constraint.clone();
        let input = self.prefix_or_unit();
        self.prefix = Some(self.select_constraints(input, vec![c]));
    }

    fn equality(&mut self, equality: &ra::Equality) {
        let left = equality.left.clone();
        let right = match &equality.right {
            ra::EqualityValue::Variable(variable) => {
                let variable = variable.clone();
                if symbol_name(&variable) == symbol_name(&left) {
                    return;
                }
                ra::EqualityValue::Variable(variable)
            }
            other => other.clone(),
        };
        let input = self.prefix_or_unit();
        self.prefix = Some(self.select(
            input,
            vec![ra::Equality {
                left,
                eq_token: equality.eq_token,
                right,
            }],
        ));
    }

    fn environment(
        &mut self,
        filter: Option<&cq::Filter>,
        binding: Option<&cq::Let>,
        generator: Option<&cq::For>,
    ) -> Option<ra::Environment> {
        if !self.scoped {
            return None;
        }
        let clause = self
            .body
            .iter()
            .enumerate()
            .find(|(c, item)| {
                !self.placed_rust.contains(c)
                    && match item {
                        valuation::cq::BodyItem::Filter(value) => filter == Some(value),
                        valuation::cq::BodyItem::Let(value) => binding == Some(&**value),
                        valuation::cq::BodyItem::For(value) => generator == Some(&**value),
                        _ => false,
                    }
            })
            .map(|(c, _)| c)
            .expect("VO Rust step source slot");
        self.placed_rust.insert(clause);
        let scope = valuation::cq::blocks::bound_before(self.body, clause, &self.context);
        let names = self.body[clause]
            .rust_step()
            .expect("VO Rust source slot")
            .free_variables
            .dependencies(&scope);
        let mut ids = BTreeMap::new();
        for definition in &self.definitions {
            match &definition.operator {
                ra::Operator::Rename(rename) => {
                    for mapping in &rename.mapping {
                        ids.insert(symbol_name(&mapping.target), mapping.target.clone());
                    }
                }
                ra::Operator::Bind(binding) => {
                    for id in cq::rust::bindings(&binding.pattern) {
                        ids.insert(symbol_name(&id), id);
                    }
                }
                _ => (),
            }
        }
        Some(ra::Environment {
            path: Some(self.path.as_ref().map_or_else(
                || cq::Occurrence::new(clause),
                |(path, j)| path.child(*j, clause),
            )),
            scope_token: Default::default(),
            bracket: Default::default(),
            variables: names
                .iter()
                .map(|name| {
                    ids.get(name)
                        .cloned()
                        .unwrap_or_else(|| Ident::new(name, proc_macro2::Span::call_site()))
                })
                .collect(),
        })
    }

    fn guard(&mut self, input: Ident, filter: &cq::Filter) -> Ident {
        let environment = self.environment(Some(filter), None, None);
        self.emit(ra::Operator::Guard(Box::new(ra::Guard {
            guard_token: Default::default(),
            input,
            if_token: filter.if_token,
            capture: filter.capture,
            condition_paren: Default::default(),
            condition: filter.condition.clone(),
            environment,
        })))
    }

    /// Formal rule RL-Bind: Append an opaque pattern binding to the prefix.
    fn bind(&mut self, binding: &cq::Let) {
        let environment = self.environment(None, Some(binding), None);
        let input = self.prefix_or_unit();
        self.prefix = Some(self.emit(ra::Operator::Bind(Box::new(ra::Bind {
            bind_token: Default::default(),
            input,
            with_token: Default::default(),
            for_token: None,
            pattern: binding.pattern.clone(),
            annotation: binding.annotation.clone(),
            eq_token: binding.eq_token,
            capture: binding.capture,
            expression_paren: Default::default(),
            expression: binding.expression.clone(),
            environment,
        }))));
    }

    /// Formal rule RL-For: Append an opaque finite set-valued generator to the prefix.
    fn for_bind(&mut self, binding: &cq::For) {
        let environment = self.environment(None, None, Some(binding));
        let input = self.prefix_or_unit();
        self.prefix = Some(self.emit(ra::Operator::Bind(Box::new(ra::Bind {
            bind_token: Default::default(),
            input,
            with_token: Default::default(),
            for_token: Some(binding.for_token),
            pattern: binding.pattern.clone(),
            annotation: None,
            eq_token: Default::default(),
            capture: None,
            expression_paren: Default::default(),
            expression: binding.expression.clone(),
            environment,
        }))));
    }

    /// Retain the normalized head variables and name the output.
    /// Formal rule RL-Finish: Project the head variables after the complete schedule.
    fn finish(&mut self, query: &valuation::cq::Query) -> ra::Output {
        let head = &query.head;
        let retained: BTreeSet<String> = query.valuation_variables().map(symbol_name).collect();
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
                ra::Operator::Bind(e) => {
                    for name in cq::rust::bindings(&e.pattern) {
                        bindings.entry(symbol_name(&name)).or_insert(name);
                    }
                }
                _ => (),
            }
        }
        let attributes = retained.iter().map(|v| bindings[v].clone()).collect();
        let input = self.prefix_or_unit();
        let mut result = self.project(input, attributes);
        if let Some(group) = &query.group {
            result = self.emit(ra::Operator::Group(Box::new(group.operator(result))));
        }
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

    /// Formal rule RL-Emit: Append the operator under a fresh rN in emission order.
    fn emit(&mut self, operator: ra::Operator) -> Ident {
        let result = loop {
            let name = format!("r{}", self.next_result);
            self.next_result += 1;
            if self.reserved.insert(name.clone()) {
                break format_ident!("{name}");
            }
        };
        if let Some(recorder) = &mut self.recorder {
            recorder.definitions.insert(symbol_name(&result), recorder.active.clone());
        }
        self.definitions.push(ra::Definition {
            result: result.clone(),
            eq_token: Default::default(),
            operator,
            semi_token: Default::default(),
        });
        result
    }
}

fn occurrence_constants(
    declaration: &cq::RelationDecl,
    atom: &cq::Atom,
) -> Option<ra::OccurrenceConstants> {
    let equalities: cq::CommaList<_> = declaration
        .columns
        .iter()
        .zip(&atom.args)
        .filter(|(_, argument)| cq::equality::literal(argument))
        .map(|(column, argument)| ra::Equality {
            left: column.name.clone(),
            eq_token: Default::default(),
            right: ra::EqualityValue::Literal(argument.clone()),
        })
        .collect();
    (!equalities.is_empty()).then_some(ra::OccurrenceConstants {
        constants_token: Default::default(),
        brace: Default::default(),
        equalities,
    })
}
