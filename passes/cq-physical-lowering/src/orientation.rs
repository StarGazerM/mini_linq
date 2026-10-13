//! Static build-side selection before IX demand derivation.
//! The RA operand order records the choice: left streams, right is indexed.
use cq_lang_ix::ra::{self, Operator};
use cq_lang_ix::syntax::symbol_name;
use std::collections::BTreeMap;

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

/// Record hash-build orientation before index demands are fixed. A strictly
/// stronger filtered left branch builds; ties retain the original orientation.
/// Semijoins and antijoins are directional and are never commuted.
/// Formal rule L-Orient: Build the stronger statically filtered natural-join operand, preserving directional operators and Rust ancestry.
pub fn orient(source: &ra::Program) -> syn::Result<ra::Program> {
    ra::contract::check(source)?;
    let ranks = strengths(source);
    let mut target = source.clone();
    for definition in &mut target.plan.definitions {
        if let Operator::NaturalJoin(join) = &mut definition.operator {
            if ranks[&symbol_name(&join.left)] < ranks[&symbol_name(&join.right)] {
                std::mem::swap(&mut join.left, &mut join.right);
            }
        }
    }
    check_orientation(source, &target)?;
    Ok(target)
}

/// Independently check the recorded choice and the bottom+Set rewrite proof.
/// Formal rule CK-Orient: Require only rank-justified natural-join commutations and independently validate relational refinement.
pub fn check_orientation(source: &ra::Program, target: &ra::Program) -> syn::Result<()> {
    ra::check_rewrite(source, target)?;
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
    for (before, after) in source.plan.definitions.iter().zip(&target.plan.definitions) {
        let mut restored = after.clone();
        if let (Operator::NaturalJoin(original), Operator::NaturalJoin(chosen)) =
            (&before.operator, &mut restored.operator)
        {
            let build_left =
                ranks[&symbol_name(&original.left)] < ranks[&symbol_name(&original.right)];
            let (stream, build) = if build_left {
                (&original.right, &original.left)
            } else {
                (&original.left, &original.right)
            };
            if &chosen.left != stream || &chosen.right != build {
                return Err(syn::Error::new_spanned(
                    &after.result,
                    "natural join must build its stronger static filter side (written right on ties)",
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
