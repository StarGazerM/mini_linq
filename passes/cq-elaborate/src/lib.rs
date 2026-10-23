//! VO to RA prime, preserving the opaque Rust chain.
//!
//! The meaning is `T X = bottom + Set X`: one panic aborts evaluation.
//! Each Rust step is a partial function of its free variables; I/O, mutation
//! and evaluation counts are outside the meaning. Relational equalities are
//! Leibniz identities decided by admissible `==`; residual Rust guards remain opaque.
//! Relational constraints may
//! move earlier across a Rust node, shrinking its domain, while Rust nodes retain
//! written order. Identical chain payloads and source-before constraints retained
//! before each node are a sufficient refinement condition. [`check`] compares
//! fixed VO clause occurrences with `rename R@clause`, their positional
//! bindings, equality closure and constraints, named semijoin reductions,
//! Rust payloads and output. Partial subatoms retain their projection and
//! correlation obligations. A DAG traversal assigns facts to their first
//! observing Rust region; moving a fact earlier is allowed, delaying it is
//! rejected. This is an ordinary structural contract, with no containment
//! search or equivalence decision. Leibniz admissibility makes equalities
//! identities. Functional extensionality permits alias solving, deduplication
//! and projection before Rust while retaining its free variables.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! Elaboration projects arms to their common heading, emits explicit union/anti equations and names negative originals.
//! Rust-free blocks hoist over free/interface columns; a named finite prefix domain supplies missing branch generators.
//! Rust arms retain the prefix and anti Before dependencies.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
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

/// Elaborate a valuation plan into named RA, then check the RA contract.
/// Formal rule C-Guard: Append an opaque guard to the prefix, seeding Unit if absent.
pub fn compile(source: &valuation::Program) -> syn::Result<ra::Program> {
    compile_in(source, true)
}

/// Compile the exact written reference, with no relational hoisting or alias
/// solving. The valuation must contain one full atom per written generator and
/// every equality/bind/guard at its written position; reductions are forbidden.
/// This is the executable reference used for partial-expression refinement tests.
pub fn compile_written(source: &valuation::Program) -> syn::Result<ra::Program> {
    valuation::contract::check(source)?;
    if source.valuation.join_branches.is_some() {
        return Err(syn::Error::new_spanned(
            &source.valuation,
            "written reference cannot contain independent join branches",
        ));
    }
    use valuation::cq::BodyItem;
    fn exact(
        body: &cq::CommaList<BodyItem>,
        steps: &[valuation::Step],
        parent: Option<(&cq::Occurrence, usize)>,
    ) -> bool {
        body.len() == steps.len()
            && body
                .iter()
                .zip(steps)
                .enumerate()
                .all(|(clause, (item, step))| {
                    let path = parent.map_or_else(
                        || cq::Occurrence::new(clause),
                        |(path, j)| path.child(j, clause),
                    );
                    match (item, step) {
                        (BodyItem::Block(block), valuation::Step::Block(planned, _)) => {
                            block.kind == planned.kind
                                && planned.source.clause == path
                                && planned.source.variables().count()
                                    == planned.source.columns.len()
                                && block
                                    .branches
                                    .iter()
                                    .zip(&planned.branches)
                                    .enumerate()
                                    .all(|(j, (written, arm))| {
                                        exact(&written.body, &arm.steps, Some((&path, j)))
                                    })
                        }
                        (BodyItem::Positive { atom }, valuation::Step::Atom(subatom, _)) => {
                            subatom.clause == path
                                && subatom.relation == atom.relation
                                && subatom.variables().count() == atom.variables().count()
                                && subatom
                                    .variables()
                                    .map(|(_, v)| symbol_name(v))
                                    .eq(atom.variables().map(symbol_name))
                        }
                        (BodyItem::Equality(a), valuation::Step::Equality(b, _)) => a == b,
                        (BodyItem::Constraint(a), valuation::Step::Constraint(b, _)) => a == b,
                        (BodyItem::Negation(a), valuation::Step::Negation(b, _)) => a == b,
                        (BodyItem::Filter(a), valuation::Step::Guard(b, _)) => a == b,
                        (BodyItem::Let(a), valuation::Step::Let(b, _)) => a == b,
                        (BodyItem::For(a), valuation::Step::For(b, _)) => a == b,
                        _ => false,
                    }
                })
    }
    let exact = exact(&source.cq.query.body, &source.valuation.steps, None);
    if !exact {
        return Err(syn::Error::new_spanned(
            &source.valuation,
            "written reference requires exactly the full written NF steps, without splitting, reductions or reordering",
        ));
    }
    compile_in(source, false)
}

fn compile_in(source: &valuation::Program, relational_hoisting: bool) -> syn::Result<ra::Program> {
    let analysis = valuation::contract::check(source)?;
    let has_blocks = valuation::cq::blocks::walk(&source.cq.query.body)
        .iter()
        .any(|item| matches!(item, valuation::cq::BodyItem::Block(_)));

    let mut state = State {
        inputs: &source.cq.inputs().cloned().collect::<Vec<_>>(),
        atoms: &analysis.atoms,
        analysis: &analysis,
        body: &source.cq.query.body,
        steps: &source.valuation.steps,
        join_branches: source.valuation.join_branches.as_ref(),
        joined_branch_atoms: BTreeSet::new(),
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
        aliases: if relational_hoisting && !has_blocks {
            aliases(&source.cq)
        } else {
            BTreeMap::new()
        },
        relational_hoisting,
        constraints: Vec::new(),
        equalities: Vec::new(),
    };
    state.scope_predicates();
    state.run_steps(&source.valuation.steps);
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

mod block_contract;
mod join_branches;
mod negation;
mod reductions;
mod structural;

/// Simplify already elaborated reductions using exact named-result lineage.
///
/// In a semijoin or natural-join support (right) operand, `R semijoin T`
/// may be replaced by `R` when the other operand is contained in `T` on
/// every column shared by `R` and `T`. Containment follows only the same
/// named result through projections, selections, semijoin left inputs and
/// natural-join inputs; it never identifies independent projected witnesses
/// or infers facts from types, data, predicates or source occurrence numbers.
/// A shared restricted support is bypassed only when every bypassed result
/// becomes unreachable. Otherwise its consumers keep the same materialized
/// row witness, preserving independent projections' correlation obligations.
///
/// Opaque Rust operators and negative-operand ancestry are not rewritten.
/// Reductions containing Rust are not bypassed, and all remaining operator
/// payloads, result names and block dependencies are retained. Only definitions
/// made unreachable by these identities are pruned. The result is checked with
/// [`ra::check_rewrite`]; the separate VO-to-RA named-reduction obligation in
/// [`crate::check`] is deliberately unchanged.
pub fn simplify_reductions(source: &ra::Program) -> syn::Result<ra::Program> {
    reductions::simplify(source)
}

/// Eliminate expansions already witnessed by an earlier named-result lookup.
///
/// In positive relational scopes, an expansion whose introduced columns have
/// no observer is replaced by a semijoin. Under canon this preserves answers,
/// not intermediate bag multiplicities. Exact named-result lineage can then
/// remove a semijoin whose support is already witnessed by its left input.
/// This explicit pass is used by LE, not by the default or Hypergraph pipeline.
///
/// The identity does not cross Rust, negative, or block boundaries. Only
/// definitions made unreachable by checked identities are pruned. Each local
/// rewrite is checked with [`ra::check_rewrite`].
pub fn simplify_expansions(source: &ra::Program) -> syn::Result<ra::Program> {
    reductions::eliminate_expansions(source.clone())
}

/// The RA definitions refine the supplied VO schedule, retaining its Rust
/// chain and preceding relational constraints.
/// Pure targets may use checked inverse dead expansions under canonicalization
/// to establish a primary witness, without recompiling or selecting a plan.
/// Formal rule CK-Elaborate: Check contracts, clause occurrences, named reductions, constraints, Rust chain and Before.
pub fn check(source: &valuation::Program, target: &ra::Program) -> syn::Result<()> {
    let analysis = valuation::contract::check(source)?;
    let target_schedule = ra::contract::Schedule::from_program(target)?;
    if target.plan.declarations != source.cq.declarations {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            format!(
                "result `{}`: declarations differ from query `{}`",
                target.plan.output.input, source.cq.name
            ),
        ));
    }
    if target.plan.output.head != source.cq.query.head {
        return Err(syn::Error::new_spanned(
            &target.plan.output,
            format!(
                "result `{}`: output head differs from `{}`",
                target.plan.output.input, source.cq.query.head.relation
            ),
        ));
    }
    structural::check(source, &analysis, &target_schedule, target).map_err(|message| {
        syn::Error::new_spanned(
            &target.plan.output,
            format!("result `{}`: {message}", target.plan.output.input),
        )
    })?;
    join_branches::check(source, target)?;
    Ok(())
}

/// The definitions so far, each clause's current relation, the prefix
/// result and its heading.
struct State<'a> {
    inputs: &'a [cq::RelationDecl],
    atoms: &'a BTreeMap<usize, cq::Atom>,
    analysis: &'a valuation::Analysis,
    body: &'a cq::CommaList<valuation::cq::BodyItem>,
    steps: &'a [valuation::Step],
    join_branches: Option<&'a valuation::JoinBranches>,
    joined_branch_atoms: BTreeSet<usize>,
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
    aliases: BTreeMap<String, Ident>,
    relational_hoisting: bool,
    equalities: Vec<(ra::Equality, BTreeSet<usize>)>,
    constraints: Vec<(cq::Constraint, BTreeSet<usize>)>,
}

impl<'a> State<'a> {
    // Predicate targets are local clause numbers: rebuild them on every scope
    // entry so sibling branches and the outer body cannot share predicates.
    fn scope_predicates(&mut self) {
        let let_bound: BTreeSet<_> = self
            .body
            .iter()
            .filter_map(|item| match item {
                valuation::cq::BodyItem::Let(binding) => Some(cq::rust::bindings(&binding.pattern)),
                valuation::cq::BodyItem::For(binding) => Some(cq::rust::bindings(&binding.pattern)),
                _ => None,
            })
            .flatten()
            .map(|v| symbol_name(&v))
            .collect();
        // A finite branch domain is a separate proof of the written prefix.
        // Later predicates cannot restrict the atoms used to supply that domain.
        let mut domain_cuts = BTreeSet::new();
        let mut prefix = self.context.clone();
        for (clause, item) in self.body.iter().enumerate() {
            match item {
                valuation::cq::BodyItem::Positive { atom } => {
                    prefix.extend(atom.variables().map(symbol_name));
                }
                valuation::cq::BodyItem::Let(binding) => {
                    prefix.extend(cq::rust::bindings(&binding.pattern).iter().map(symbol_name));
                }
                valuation::cq::BodyItem::For(binding) => {
                    prefix.extend(cq::rust::bindings(&binding.pattern).iter().map(symbol_name));
                }
                valuation::cq::BodyItem::Block(block) => {
                    let facts = block.analyze(&prefix);
                    if self.relational_hoisting && !facts.rust && needs_domain(block, &facts) {
                        domain_cuts.insert(clause);
                    }
                    prefix.extend(facts.interface.into_keys());
                }
                _ => (),
            }
        }
        let targets = |variables: BTreeSet<String>, before: usize| -> BTreeSet<usize> {
            if variables.is_empty() || !variables.is_disjoint(&let_bound) {
                return BTreeSet::new();
            }
            self.atoms
                .iter()
                .filter(|(clause, atom)| {
                    **clause < before
                        && domain_cuts.range(**clause + 1..before).next().is_none()
                        && matches!(
                            self.body[**clause],
                            valuation::cq::BodyItem::Positive { .. }
                        )
                        && variables.is_subset(&atom.variables().map(symbol_name).collect())
                })
                .map(|(clause, _)| *clause)
                .collect()
        };
        self.constraints = self
            .body
            .iter()
            .enumerate()
            .flat_map(|(clause, item)| match item {
                valuation::cq::BodyItem::Constraint(c) => std::iter::once(c.clone())
                    .chain(c.implied_domains())
                    .map(|predicate| {
                        let binders =
                            targets(predicate.variables().map(symbol_name).collect(), clause);
                        (predicate, binders)
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect();
        self.equalities = self
            .body
            .iter()
            .enumerate()
            .filter_map(|(clause, item)| match item {
                valuation::cq::BodyItem::Equality(e) => Some((
                    e.clone(),
                    targets(e.variables().map(symbol_name).collect(), clause),
                )),
                _ => None,
            })
            .collect();
    }

    fn run_steps(&mut self, steps: &[valuation::Step]) {
        // Grouped atoms introduce their entire branch interface at first read.
        // Their local predicates already live on the branch's named relations.
        for (step, introduces) in steps.iter().zip(&self.analysis.introduces) {
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
                    if self.joined_branch_atoms.contains(&subatom.clause()) {
                        continue;
                    }
                    if let Some(branch) = self.join_branches.and_then(|groups| {
                        groups
                            .branches
                            .iter()
                            .find(|b| b.clauses().contains(&subatom.clause()))
                    }) {
                        self.join_branch(branch);
                        continue;
                    }
                    let check = subatom.variables().all(|(_, variable)| {
                        self.heading
                            .contains(&symbol_name(&self.canonical(variable)))
                    });
                    self.subatom(subatom, check);
                    self.read_clauses.insert(subatom.clause());
                }
                valuation::Step::Reduce(reduce, _) => self.reduce(reduce),
                valuation::Step::Negation(negation, _) => self.negation(negation),
                valuation::Step::Guard(filter, _) => {
                    let input = self.prefix_or_unit();
                    self.prefix = Some(self.guard(input, filter));
                }
                valuation::Step::Let(binding, _) => self.bind(binding),
                valuation::Step::For(binding, _) => self.for_bind(binding),
                valuation::Step::Equality(equality, _) => {
                    let local = self.branch_local(equality.variables().map(symbol_name).collect());
                    if !local {
                        self.equality(equality);
                    }
                }
                valuation::Step::Constraint(c, _) => {
                    let local = self.branch_local(c.variables().map(symbol_name).collect());
                    if !local {
                        self.constraint(c);
                    }
                }
            }
            let introduced: Vec<_> = introduces
                .iter()
                .map(|name| {
                    self.aliases
                        .get(name)
                        .map(symbol_name)
                        .unwrap_or_else(|| name.clone())
                })
                .collect();
            self.heading.extend(introduced);
        }
    }

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
        let written_prefix = valuation::cq::blocks::bound_before(self.body, clause, &self.context);
        let facts = written.analyze(&written_prefix);
        let hoisted = self.relational_hoisting && !facts.rust;
        let correlated_prefix = (!hoisted).then(|| self.prefix_or_unit());
        let common: BTreeSet<_> = facts
            .free
            .keys()
            .chain(facts.interface.keys())
            .cloned()
            .collect();
        // If an arm has no generator for an F column, its finite domain is
        // supplied by the written prefix's projected context relation.
        let needs_domain = hoisted && needs_domain(written, &facts);
        let domain = needs_domain.then(|| {
            let saved_prefix = self.prefix.clone();
            let saved_heading = self.heading.clone();
            if !facts.free.keys().all(|name| self.heading.contains(name)) {
                // A reducer may request D before the valuation prefix. Build
                // its finite domain from the original named prefix relations;
                // their renames are reused by subsequent reductions/reads.
                self.prefix = None;
                self.heading = self.context.clone();
                for (c, item) in self.body.iter().take(clause).enumerate() {
                    match item {
                        valuation::cq::BodyItem::Positive { atom } => {
                            let right = self.relation(c);
                            self.prefix = Some(match self.prefix.take() {
                                None => right,
                                Some(left) => self.emit(ra::Operator::NaturalJoin(Box::new(
                                    ra::NaturalJoin {
                                        natural_join_token: Default::default(),
                                        left,
                                        with_token: Default::default(),
                                        right,
                                    },
                                ))),
                            });
                            self.heading.extend(atom.variables().map(symbol_name));
                        }
                        valuation::cq::BodyItem::Equality(eq) => self.equality(eq),
                        valuation::cq::BodyItem::Constraint(c) => self.constraint(c),
                        _ => (),
                    }
                }
            }
            let prefix = self.prefix_or_unit();
            let domain = self.project(prefix, facts.free.values().cloned().collect());
            self.prefix = saved_prefix;
            self.heading = saved_heading;
            domain
        });
        let parent_constraints = std::mem::take(&mut self.constraints);
        let parent_equalities = std::mem::take(&mut self.equalities);
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
            self.scope_predicates();
            self.run_steps(&branch.steps);
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
        self.constraints = parent_constraints;
        self.equalities = parent_equalities;
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
        if !hoisted {
            self.fused.insert(clause);
        }
        self.relations.insert(clause, result.clone());
        result
    }

    /// E-Neg: constants and repeated columns select the negative relation;
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
        let layout = negation::layout(declaration, negation, &bound, |v| self.canonical(v));
        let needs_projection =
            layout.keys.len() != layout.mapping.len() && !layout.condition.constraints.is_empty();
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
        if !layout.condition.constraints.is_empty() {
            right =
                self.select_constraints(right, layout.condition.constraints.into_iter().collect());
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
    /// Formal rule E-Relation: Reuse a clause result or emit its complete positional rename at first use.
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
                    target: self.canonical(variable),
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
        // A literal equality is a keyed relational read of this occurrence.
        // It may move earlier across Rust because it only restricts its domain.
        let literals: Vec<_> = self
            .equalities
            .iter()
            .filter(|(e, targets)| {
                self.relational_hoisting
                    && targets.contains(&clause)
                    && matches!(e.right, ra::EqualityValue::Literal(_))
            })
            .map(|(e, _)| ra::Equality {
                left: self.canonical(&e.left),
                ..e.clone()
            })
            .collect();
        if !literals.is_empty() {
            result = self.select(result, literals);
        }
        // Ordinary relational pushdown: every used column belongs to this
        // occurrence. Group same-column conjuncts into one range selection.
        if self.relational_hoisting {
            let mut groups: Vec<(BTreeSet<String>, Vec<cq::Constraint>)> = Vec::new();
            for (c, targets) in &self.constraints {
                let columns: BTreeSet<_> = c.variables().map(symbol_name).collect();
                if targets.contains(&clause) {
                    let mut predicate = c.clone();
                    predicate.map_terms(&mut |t| {
                        if let cq::constraint::Term::Variable(v) = t {
                            *v = self.canonical(v);
                        }
                    });
                    if let Some((_, constraints)) =
                        groups.iter_mut().find(|(key, _)| *key == columns)
                    {
                        constraints.push(predicate);
                    } else {
                        groups.push((columns, vec![predicate]));
                    }
                }
            }
            for (_, constraints) in groups {
                result = self.select_constraints(result, constraints);
            }
        }
        self.relations.insert(clause, result.clone());
        result
    }

    /// Formal rule E-Check: A partial subatom with no new names semijoins an existing prefix.
    /// Formal rule E-Project: Project a partial subatom unless it is an existing-prefix check.
    /// Formal rule B-Root: Install the first full or projected subatom as the prefix.
    /// Formal rule B-Join: Join a non-check subatom with the current prefix.
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
                .map(|(position, _)| self.canonical(variables[position]))
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

    /// Formal rule E-ReduceWith: Match every listed support column to its target equality class and retain only the target heading.
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
                            let left = self.canonical(variable);
                            let right = self.canonical(support);
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
                    // Express the equality-class semijoin as join/select/project,
                    // without renaming Rust inputs or adding any Rust chain node.
                    let joined = self.emit(ra::Operator::NaturalJoin(Box::new(ra::NaturalJoin {
                        natural_join_token: Default::default(),
                        left,
                        with_token: Default::default(),
                        right,
                    })));
                    let matched = self.select(joined, equalities);
                    let attributes = self.atoms[&clause]
                        .variables()
                        .map(|v| self.canonical(v))
                        .collect();
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

    fn canonical(&self, variable: &Ident) -> Ident {
        self.aliases
            .get(&symbol_name(variable))
            .unwrap_or(variable)
            .clone()
    }

    fn select(&mut self, input: Ident, equalities: Vec<ra::Equality>) -> Ident {
        self.emit(ra::Operator::Select(Box::new(ra::Select {
            select_token: Default::default(),
            input,
            if_token: Default::default(),
            condition_paren: Default::default(),
            condition: ra::Theta {
                equalities: equalities.into_iter().collect(),
                constraints: Default::default(),
            },
        })))
    }

    fn select_constraints(&mut self, input: Ident, constraints: Vec<cq::Constraint>) -> Ident {
        self.emit(ra::Operator::Select(Box::new(ra::Select {
            select_token: Default::default(),
            input,
            if_token: Default::default(),
            condition_paren: Default::default(),
            condition: ra::Theta {
                equalities: Default::default(),
                constraints: constraints.into_iter().collect(),
            },
        })))
    }

    /// Formal rule E-Constraint: Push a central constraint onto its complete binder or select the prefix.
    fn constraint(&mut self, constraint: &cq::Constraint) {
        if self.relational_hoisting
            && self
                .constraints
                .iter()
                .filter(|(c, _)| c == constraint)
                .all(|(_, targets)| !targets.is_empty())
        {
            return;
        }
        let mut c = constraint.clone();
        c.map_terms(&mut |t| {
            if let cq::constraint::Term::Variable(v) = t {
                *v = self.canonical(v);
            }
        });
        let input = self.prefix_or_unit();
        self.prefix = Some(self.select_constraints(input, vec![c]));
    }

    fn equality(&mut self, equality: &ra::Equality) {
        if self.relational_hoisting
            && matches!(equality.right, ra::EqualityValue::Literal(_))
            && self
                .equalities
                .iter()
                .filter(|(e, _)| e == equality)
                .all(|(_, targets)| !targets.is_empty())
        {
            return;
        }
        let left = self.canonical(&equality.left);
        let right = match &equality.right {
            ra::EqualityValue::Variable(variable) => {
                let variable = self.canonical(variable);
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
        let names = valuation::cq::blocks::bound_before(self.body, clause, &self.context);
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
            condition_paren: Default::default(),
            condition: filter.condition.clone(),
            environment,
        })))
    }

    /// Formal rule C-Bind: Append an opaque pattern binding to the prefix.
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
            expression_paren: Default::default(),
            expression: binding.expression.clone(),
            environment,
        }))));
    }

    /// Formal rule C-For: Append an opaque finite set-valued generator to the prefix.
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
            expression_paren: Default::default(),
            expression: binding.expression.clone(),
            environment,
        }))));
    }

    /// Retain the normalized head variables and name the output.
    /// Formal rule C-Finish: Project the head variables after the complete chain.
    fn finish(&mut self, head: &cq::Atom) -> ra::Output {
        let retained: BTreeSet<String> = head.variables().map(symbol_name).collect();
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

    /// Formal rule E-Emit: Append the operator under a fresh rN in emission order.
    fn emit(&mut self, operator: ra::Operator) -> Ident {
        let result = loop {
            let name = format!("r{}", self.next_result);
            self.next_result += 1;
            if self.reserved.insert(name.clone()) {
                break format_ident!("{name}");
            }
        };
        self.definitions.push(ra::Definition {
            result: result.clone(),
            eq_token: Default::default(),
            operator,
            semi_token: Default::default(),
        });
        result
    }
}

fn needs_domain(
    block: &valuation::cq::blocks::Block,
    facts: &valuation::cq::blocks::Analysis,
) -> bool {
    block.branches.iter().any(|branch| {
        let generated: BTreeSet<_> = valuation::cq::blocks::walk(&branch.body)
            .into_iter()
            .filter_map(valuation::cq::BodyItem::atom)
            .flat_map(cq::Atom::variables)
            .map(symbol_name)
            .collect();
        facts.free.keys().any(|name| !generated.contains(name))
            || valuation::cq::blocks::walk(&branch.body)
                .into_iter()
                .any(|item| {
                    matches!(item, valuation::cq::BodyItem::Negation(n)
                    if n.atom.variables().any(|v| facts.free.contains_key(&symbol_name(v))))
                })
    })
}

/// Solve generator equalities through shared join headings when doing so keeps
/// every opaque expression's names and every atom's distinct rename targets.
fn aliases(source: &valuation::cq::Program) -> BTreeMap<String, Ident> {
    use valuation::cq::BodyItem;
    let atoms: Vec<_> = source
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            BodyItem::Positive { atom } => Some(atom),
            _ => None,
        })
        .collect();
    let let_bound: BTreeSet<_> = source
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            BodyItem::Let(b) => Some(cq::rust::bindings(&b.pattern)),
            BodyItem::For(b) => Some(cq::rust::bindings(&b.pattern)),
            _ => None,
        })
        .flatten()
        .map(|v| symbol_name(&v))
        .collect();
    let generators: BTreeSet<_> = atoms
        .iter()
        .flat_map(|atom| atom.variables().map(symbol_name))
        .filter(|v| !let_bound.contains(v))
        .collect();
    let mut protected: BTreeSet<_> = source.query.head.variables().map(symbol_name).collect();
    for item in &source.query.body {
        let expression = match item {
            BodyItem::Negation(negation) => {
                // Negative operand identities retain their exact source key
                // names; solve positive aliases without rewriting that input.
                protected.extend(negation.atom.variables().map(symbol_name));
                continue;
            }
            BodyItem::Filter(filter) => &filter.condition,
            BodyItem::Let(binding) => {
                protected.extend(cq::rust::bindings(&binding.pattern).iter().map(symbol_name));
                &binding.expression
            }
            BodyItem::For(binding) => {
                protected.extend(cq::rust::bindings(&binding.pattern).iter().map(symbol_name));
                &binding.expression
            }
            _ => continue,
        };
        protected.extend(cq::rust::free_variables(expression).dependencies(&generators));
    }
    let mut aliases = BTreeMap::<String, Ident>::new();
    let canonical = |id: &Ident, aliases: &BTreeMap<String, Ident>| {
        aliases.get(&symbol_name(id)).unwrap_or(id).clone()
    };
    for item in &source.query.body {
        let BodyItem::Equality(equality) = item else {
            continue;
        };
        let ra::EqualityValue::Variable(right) = &equality.right else {
            continue;
        };
        if !generators.contains(&symbol_name(&equality.left))
            || !generators.contains(&symbol_name(right))
        {
            continue;
        }
        let left = canonical(&equality.left, &aliases);
        let right = canonical(right, &aliases);
        if left == right {
            continue;
        }
        let (keep, replace) = if protected.contains(&symbol_name(&right)) {
            (right, left)
        } else {
            (left, right)
        };
        if protected.contains(&symbol_name(&replace)) {
            continue;
        }
        let mut proposed = aliases.clone();
        for value in proposed.values_mut() {
            if symbol_name(value) == symbol_name(&replace) {
                *value = keep.clone();
            }
        }
        proposed.insert(symbol_name(&replace), keep);
        if atoms.iter().any(|atom| {
            let names: BTreeSet<_> = atom
                .variables()
                .map(|v| symbol_name(&canonical(v, &proposed)))
                .collect();
            names.len() != atom.variables().count()
        }) {
            continue;
        }
        aliases = proposed;
    }
    aliases
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
