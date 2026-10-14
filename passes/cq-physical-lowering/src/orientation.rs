//! Static build-side selection before IX demand derivation.
//! The RA operand order records the choice: left streams, right is indexed.
use cq_lang_ix::ra::{self, Operator};
use cq_lang_ix::syntax::symbol_name;
use std::collections::{BTreeMap, BTreeSet};

/// Predicate strength propagated through the relational DAG. This is not a
/// cardinality estimate: no host size, key constraint, or data value is read.
fn strengths(program: &ra::Program) -> BTreeMap<String, u8> {
    let mut ranks: BTreeMap<String, u8> = BTreeMap::new();
    for definition in &program.plan.definitions {
        let rank = match &definition.operator {
            // Unit is a scope seed, not evidence of a filtered driving branch.
            Operator::Unit(_) => 3,
            Operator::Rename(_) => 3,
            Operator::Select(s) => {
                ranks[&symbol_name(&s.input)].min(if s.condition.point_columns().is_some() {
                    0
                } else if s.condition.range_bounds().is_some() {
                    1
                } else {
                    2
                })
            }
            Operator::NaturalJoin(j) => {
                ranks[&symbol_name(&j.left)].min(ranks[&symbol_name(&j.right)])
            }
            Operator::Semijoin(j) => {
                ranks[&symbol_name(&j.left)].min(ranks[&symbol_name(&j.right)])
            }
            Operator::Anti(a) => ranks[&symbol_name(&a.left)],
            Operator::Project(p) => ranks[&symbol_name(&p.input)],
            Operator::Guard(g) => ranks[&symbol_name(&g.input)],
            Operator::Bind(b) => ranks[&symbol_name(&b.input)],
            Operator::Union(u) => u
                .operands
                .iter()
                .map(|r| ranks[&symbol_name(r)])
                .max()
                .unwrap_or(2)
                .min(2),
        };
        ranks.insert(symbol_name(&definition.result), rank);
    }
    ranks
}

/// Identify compound dimension branches structurally. The widest shared-key
/// occurrence is the static fact spine; no relation name or key constraint is
/// consulted. A materialized multi-atom branch builds against that spine even
/// when the fact itself has a local range predicate.
fn branch_sides(program: &ra::Program) -> BTreeMap<String, (bool, usize, usize)> {
    let mut incidence: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut atoms = Vec::new();
    for definition in &program.plan.definitions {
        if let Operator::Rename(r) = &definition.operator {
            let columns: BTreeSet<_> = r.mapping.iter().map(|m| symbol_name(&m.target)).collect();
            for column in &columns {
                incidence
                    .entry(column.clone())
                    .or_default()
                    .insert(r.occurrence.clause());
            }
            atoms.push((r.occurrence.clause(), columns));
        }
    }
    let fact = atoms
        .iter()
        .max_by_key(|(c, columns)| {
            (
                columns.iter().filter(|v| incidence[*v].len() > 1).count(),
                std::cmp::Reverse(*c),
            )
        })
        .filter(|(_, columns)| columns.iter().filter(|v| incidence[*v].len() > 1).count() >= 3)
        .map(|(c, _)| *c);
    let fact_columns = atoms
        .iter()
        .find(|(c, _)| Some(*c) == fact)
        .map(|(_, columns)| columns);
    let mut lineage: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for definition in &program.plan.definitions {
        let members = match &definition.operator {
            Operator::Rename(r) => BTreeSet::from([r.occurrence.clause()]),
            Operator::NaturalJoin(j) => lineage[&symbol_name(&j.left)]
                .union(&lineage[&symbol_name(&j.right)])
                .copied()
                .collect(),
            Operator::Semijoin(j) => lineage[&symbol_name(&j.left)].clone(),
            Operator::Select(s) => lineage[&symbol_name(&s.input)].clone(),
            Operator::Project(p) => lineage[&symbol_name(&p.input)].clone(),
            _ => BTreeSet::new(),
        };
        lineage.insert(symbol_name(&definition.result), members);
    }
    lineage
        .into_iter()
        .map(|(name, members)| {
            let keys: BTreeSet<_> = atoms
                .iter()
                .filter(|(c, _)| members.contains(c))
                .flat_map(|(_, columns)| columns.iter())
                .filter(|v| fact_columns.is_some_and(|fact| fact.contains(*v)))
                .collect();
            (
                name,
                (
                    fact.is_some_and(|f| members.contains(&f)),
                    members.len(),
                    keys.len(),
                ),
            )
        })
        .collect()
}

/// Declared relation names reachable from each result.
fn relations(program: &ra::Program) -> BTreeMap<String, BTreeSet<String>> {
    let mut lineage: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for definition in &program.plan.definitions {
        let mut members = BTreeSet::new();
        match &definition.operator {
            Operator::Rename(r) => {
                members.insert(symbol_name(&r.relation));
            }
            Operator::NaturalJoin(j) => {
                members.extend(lineage[&symbol_name(&j.left)].iter().cloned());
                members.extend(lineage[&symbol_name(&j.right)].iter().cloned());
            }
            Operator::Semijoin(j) => members.extend(lineage[&symbol_name(&j.left)].iter().cloned()),
            Operator::Anti(a) => members.extend(lineage[&symbol_name(&a.left)].iter().cloned()),
            Operator::Select(s) => members.extend(lineage[&symbol_name(&s.input)].iter().cloned()),
            Operator::Project(p) => members.extend(lineage[&symbol_name(&p.input)].iter().cloned()),
            Operator::Guard(g) => members.extend(lineage[&symbol_name(&g.input)].iter().cloned()),
            Operator::Bind(b) => members.extend(lineage[&symbol_name(&b.input)].iter().cloned()),
            Operator::Union(u) => {
                for operand in &u.operands {
                    members.extend(lineage[&symbol_name(operand)].iter().cloned());
                }
            }
            Operator::Unit(_) => {}
        }
        lineage.insert(symbol_name(&definition.result), members);
    }
    lineage
}

/// Caller advice: relations that should be streamed rather than built. When
/// exactly one join side reaches an advised relation, that side streams;
/// otherwise the static rule decides. Advice never changes meaning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Advice {
    pub stream: BTreeSet<String>,
}

fn advised(
    left: &str,
    right: &str,
    advice: &Advice,
    lineage: &BTreeMap<String, BTreeSet<String>>,
) -> Option<bool> {
    let reaches = |side: &str| lineage[side].iter().any(|r| advice.stream.contains(r));
    match (reaches(left), reaches(right)) {
        (true, false) => Some(false),
        (false, true) => Some(true),
        _ => None,
    }
}

fn build_left(
    left: &str,
    right: &str,
    ranks: &BTreeMap<String, u8>,
    sides: &BTreeMap<String, (bool, usize, usize)>,
) -> bool {
    let (left_fact, left_atoms, left_keys) = sides[left];
    let (right_fact, right_atoms, right_keys) = sides[right];
    if left_fact && !right_fact && right_atoms > 1 {
        return false;
    }
    if right_fact && !left_fact && left_atoms > 1 {
        return true;
    }
    // Inside the dimension forest, a one-key child builds against a
    // composite-key parent (e.g. Part or Supplier/Nation against PartSupp).
    if !left_fact && !right_fact && left_keys != right_keys {
        return left_keys < right_keys;
    }
    ranks[left] < ranks[right]
}

/// Record hash-build orientation before index demands are fixed. Compound
/// dimension branches build against the fact spine, narrower fact-key children
/// build within dimensions, and filter strength breaks remaining choices.
/// Formal rule L-Orient: Build compound dimensions against the fact spine, narrower fact-key children within dimensions, otherwise the stronger static filter side; preserve directional operators and Rust ancestry.
pub fn orient(source: &ra::Program) -> syn::Result<ra::Program> {
    orient_with(source, &Advice::default())
}

/// [`orient`] with caller advice; the advised choice is recorded in RA operand
/// order and checked by [`check_orientation_with`] under the same advice.
pub fn orient_with(source: &ra::Program, advice: &Advice) -> syn::Result<ra::Program> {
    ra::contract::check(source)?;
    let lineage = relations(source);
    for relation in &advice.stream {
        if !lineage.values().any(|members| members.contains(relation)) {
            return Err(syn::Error::new_spanned(
                &source.plan.output,
                format!("stream advice names `{relation}`, which the plan does not read"),
            ));
        }
    }
    let ranks = strengths(source);
    let sides = branch_sides(source);
    let mut target = source.clone();
    for definition in &mut target.plan.definitions {
        if let Operator::NaturalJoin(join) = &mut definition.operator {
            let (left, right) = (symbol_name(&join.left), symbol_name(&join.right));
            if advised(&left, &right, advice, &lineage)
                .unwrap_or_else(|| build_left(&left, &right, &ranks, &sides))
            {
                std::mem::swap(&mut join.left, &mut join.right);
            }
        }
    }
    check_orientation_with(source, &target, advice)?;
    Ok(target)
}

/// Independently check the recorded choice and the bottom+Set rewrite proof.
/// Formal rule CK-Orient: Require branch-or-rank-justified natural-join commutations and independently validate relational refinement.
pub fn check_orientation(source: &ra::Program, target: &ra::Program) -> syn::Result<()> {
    check_orientation_with(source, target, &Advice::default())
}

/// [`check_orientation`] under caller advice.
pub fn check_orientation_with(
    source: &ra::Program,
    target: &ra::Program,
    advice: &Advice,
) -> syn::Result<()> {
    ra::check_rewrite(source, target)?;
    let lineage = relations(source);
    if source.plan.declarations != target.plan.declarations
        || source.plan.output != target.plan.output
        || source.plan.definitions.len() != target.plan.definitions.len()
    {
        return Err(syn::Error::new_spanned(
            target,
            "orientation must preserve the RA interface and definitions",
        ));
    }
    let ranks = strengths(source);
    let sides = branch_sides(source);
    for (before, after) in source.plan.definitions.iter().zip(&target.plan.definitions) {
        let mut restored = after.clone();
        if let (Operator::NaturalJoin(original), Operator::NaturalJoin(chosen)) =
            (&before.operator, &mut restored.operator)
        {
            let (left, right) = (symbol_name(&original.left), symbol_name(&original.right));
            let build_left = advised(&left, &right, advice, &lineage)
                .unwrap_or_else(|| build_left(&left, &right, &ranks, &sides));
            let (stream, build) = if build_left {
                (&original.right, &original.left)
            } else {
                (&original.left, &original.right)
            };
            if &chosen.left != stream || &chosen.right != build {
                return Err(syn::Error::new_spanned(
                    &after.result,
                    "natural join must stream its advised side, else build its independent dimension branch or stronger static filter side (written right on ties)",
                ));
            }
            chosen.left = original.left.clone();
            chosen.right = original.right.clone();
        }
        if &restored != before {
            return Err(syn::Error::new_spanned(
                &after.result,
                "orientation may change only natural-join operand order",
            ));
        }
    }
    Ok(())
}
