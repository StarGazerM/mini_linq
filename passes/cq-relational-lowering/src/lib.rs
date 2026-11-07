//! Relational lowering, VO to RA prime: a mechanical translation that applies
//! every recorded relational decision and preserves Rust payloads.
//!
//! Every Rust expression is opaque, pure and total by premise. Its only placement
//! requirements are its free variables. [`lower`] returns, beside the RA, the
//! seam side table [`Steps`] naming the VO step of every rename and block
//! union. [`check`] relates VO steps and RA result names through that table:
//! rename facts, recorded restrictions, named reductions and block placement,
//! and compares the structural reference of the VO with the RA program.
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

/// Seam side table of one lowering, not part of either program: RA result
/// name -> VO step position. A rename maps to its occurrence's first read
/// step (a negation's rename to its negation step); a block union maps to its
/// block step. A position is the top-level step index followed, inside each
/// enclosing block step, by the branch index and the step index in that branch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Steps(pub BTreeMap<String, Vec<usize>>);

/// Immutable VO input paired with analysis produced only by its checker.
pub struct Checked<'a> {
    source: &'a valuation::Program,
    analysis: valuation::Analysis,
}

impl<'a> Checked<'a> {
    pub fn new(source: &'a valuation::Program) -> syn::Result<Self> {
        Ok(Self { source, analysis: valuation::contract::check(source)? })
    }
}

/// Check the VO contract, then lower it.
pub fn compile(source: &valuation::Program) -> syn::Result<(ra::Program, Steps)> {
    Ok(lower(&Checked::new(source)?))
}

/// Lower a checked valuation plan into named RA and the seam
/// table. Recorded restrictions select their occurrence; every other
/// predicate is a prefix step. A written reference is a VO without
/// restrictions or reductions.
/// Formal rule RL-Guard: Append an opaque guard to the prefix, seeding Unit if absent.
pub fn lower(checked: &Checked<'_>) -> (ra::Program, Steps) {
    let source = checked.source;
    let analysis = &checked.analysis;
    let has_blocks = valuation::cq::blocks::walk(&source.cq.query.body)
        .iter()
        .any(|item| matches!(item, valuation::cq::BodyItem::Block(_)));

    let mut state = State {
        inputs: &source.cq.inputs().cloned().collect::<Vec<_>>(),
        atoms: &analysis.atoms,
        analysis,
        body: &source.cq.query.body,
        steps: &source.valuation.steps,
        context: BTreeSet::new(),
        scope: Vec::new(),
        step: Vec::new(),
        table: BTreeMap::new(),
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
    let output = state.finish(&source.cq.query);
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
    (target, Steps(state.table))
}

mod block_contract;
mod negation;
mod seam;
mod structural;

/// The RA definitions refine the supplied VO schedule through the seam table:
/// every rename and block union names its VO step, and the structural
/// reference of the VO equals the RA program under the RA rewrite check.
/// Formal rule CK-RelationalLowering: Check WF-VO, the seam table, rename facts, recorded restrictions, named reductions, block placement and the structural reference.
pub fn check(source: &valuation::Program, target: &ra::Program, steps: &Steps) -> syn::Result<()> {
    let analysis = valuation::contract::check(source)?;
    check_reference(&source.cq, &source.valuation.steps, &analysis, target)?;
    seam::check(source, target, steps)
}

/// Check the structural reference of `steps`, which need not be an
/// executable VO (an occurrence-coverage proof reference), against `target`.
/// `analysis` is the WF binding analysis of `source` and `steps`, obtained by
/// `valuation::order::analyze_reference`; this path never chooses an order.
/// Formal rule CK-RLStructure: The structural reference RL(s) of the VO steps and the target pass the RA rewrite check.
pub fn check_reference(
    source: &valuation::cq::Program,
    steps: &[valuation::Step],
    analysis: &valuation::Analysis,
    target: &ra::Program,
) -> syn::Result<()> {
    ra::contract::check(target)?;
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
    let reference = structural::reference(source, steps, analysis);
    ra::check_rewrite(&reference, target)
}

/// The definitions so far, each clause's current relation, the prefix
/// result and its heading.
struct State<'a> {
    inputs: &'a [cq::RelationDecl],
    atoms: &'a BTreeMap<usize, cq::Atom>,
    analysis: &'a valuation::Analysis,
    body: &'a cq::CommaList<valuation::cq::BodyItem>,
    steps: &'a [valuation::Step],
    context: BTreeSet<String>,
    /// Position of the current scope: empty at top level, else the enclosing
    /// block step's position followed by the branch index.
    scope: Vec<usize>,
    /// Position of the step being lowered.
    step: Vec<usize>,
    /// The seam side table being recorded.
    table: BTreeMap<String, Vec<usize>>,
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
            self.step = self.scope.iter().copied().chain([index]).collect();
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
            self.heading.extend(introduces.iter().cloned());
        }
    }

    /// Formal rule RL-Block: Elaborate the recorded Hoisted or Correlated alternative, never a compiler-mode choice.
    fn block_relation(&mut self, clause: usize) -> Ident {
        let valuation::cq::BodyItem::Block(written) = &self.body[clause] else {
            unreachable!("block clause")
        };
        let index = self
            .steps
            .iter()
            .position(|step| {
                matches!(step, valuation::Step::Block(block, _) if block.source.clause() == clause)
            })
            .expect("VO places every block");
        let valuation::Step::Block(planned, _) = &self.steps[index] else {
            unreachable!("block step")
        };
        let position: Vec<usize> = self.scope.iter().copied().chain([index]).collect();
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
        let parent_scope = self.scope.clone();
        let parent_step = self.step.clone();
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
            self.scope = position.iter().copied().chain([j]).collect();
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
        self.scope = parent_scope;
        self.step = parent_step;
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
        self.table.insert(symbol_name(&result), position);
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
        self.table.insert(symbol_name(&right), self.step.clone());
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
            mapping_brace: Default::default(),
            mapping,
            constants,
        })));
        // The rename names its occurrence's first read step (WF-VO: every
        // positive occurrence of the scope has one).
        let first_read = self
            .steps
            .iter()
            .position(|step| matches!(step, valuation::Step::Atom(read, _) if read.clause() == clause))
            .expect("VO reads every positive occurrence");
        self.table.insert(
            symbol_name(&result),
            self.scope.iter().copied().chain([first_read]).collect(),
        );
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
            result = self.select(result, literals);
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
            result = self.select_constraints(result, constraints);
        }
        self.relations.insert(clause, result.clone());
        result
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
