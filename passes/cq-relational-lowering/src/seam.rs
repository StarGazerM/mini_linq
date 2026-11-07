//! The VO/RA seam rules over (VO step position, RA result name), stated
//! through the side table [`Steps`] returned by [`lower`](super::lower).
//! Neither program records the other's identities.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ra as ra;
use cq_lang_vo::{self as vo, cq, symbol_name};
use quote::ToTokens;

use super::Steps;

/// What a VO step position names on the RA side.
enum Named<'a> {
    /// The first read of a positive occurrence (clause of its scope's body).
    Read(&'a cq::Atom),
    Negation(&'a cq::Negation),
    Block(&'a vo::blocks::Block, &'a cq::blocks::Block),
}

/// One VO scope: the top-level schedule or one block arm.
struct Scope<'a> {
    position: Vec<usize>,
    steps: &'a [vo::Step],
    body: &'a cq::CommaList<cq::BodyItem>,
    hypergraph: cq::hypergraph::Analysis,
}

impl Scope<'_> {
    fn at(&self, index: usize) -> Vec<usize> {
        self.position.iter().copied().chain([index]).collect()
    }

    /// The step position whose RA result stands for `clause` in this scope.
    fn named(&self, clause: usize) -> Option<Vec<usize>> {
        self.steps
            .iter()
            .position(|step| match step {
                vo::Step::Atom(read, _) => read.clause() == clause,
                vo::Step::Block(block, _) => block.source.clause() == clause,
                _ => false,
            })
            .map(|index| self.at(index))
    }
}

fn scopes<'a>(
    position: Vec<usize>,
    steps: &'a [vo::Step],
    body: &'a cq::CommaList<cq::BodyItem>,
    context: BTreeSet<String>,
    hypergraph: cq::hypergraph::Analysis,
    result: &mut Vec<Scope<'a>>,
) {
    // Later reads of a block reuse the union its first block step names.
    let mut blocks = BTreeSet::new();
    for (index, step) in steps.iter().enumerate() {
        let vo::Step::Block(block, _) = step else {
            continue;
        };
        let clause = block.source.clause();
        if !blocks.insert(clause) {
            continue;
        }
        let Some(cq::BodyItem::Block(written)) = body.iter().nth(clause) else {
            continue;
        };
        let prefix = cq::blocks::bound_before(body, clause, &context);
        let facts = written.analyze(&prefix);
        let free: BTreeSet<_> = facts.free.keys().cloned().collect();
        let hoisted = block.placement == vo::blocks::Placement::Hoisted;
        for (j, (arm, branch)) in block.branches.iter().zip(&written.branches).enumerate() {
            let arm_position = position.iter().copied().chain([index, j]).collect();
            scopes(
                arm_position,
                &arm.steps,
                &branch.body,
                if hoisted { free.clone() } else { prefix.clone() },
                cq::hypergraph::Analysis::of_body(&branch.body, &free),
                result,
            );
        }
    }
    result.push(Scope {
        position,
        steps,
        body,
        hypergraph,
    });
}

pub(super) fn check(
    source: &vo::Program,
    target: &ra::Program,
    table: &Steps,
) -> syn::Result<()> {
    let fail = |message: String| {
        Err(syn::Error::new_spanned(
            &target.plan.output,
            format!("result `{}`: {message}", target.plan.output.input),
        ))
    };
    let mut all = Vec::new();
    scopes(
        Vec::new(),
        &source.valuation.steps,
        &source.cq.query.body,
        BTreeSet::new(),
        cq::hypergraph::Analysis::of(&source.cq),
        &mut all,
    );
    let mut expected = BTreeMap::new();
    for scope in &all {
        let mut read = BTreeSet::new();
        let mut blocks = BTreeSet::new();
        for (index, step) in scope.steps.iter().enumerate() {
            let named = match step {
                vo::Step::Atom(subatom, _) if read.insert(subatom.clause()) => {
                    match scope.body.iter().nth(subatom.clause()) {
                        Some(cq::BodyItem::Positive { atom }) => Named::Read(atom),
                        _ => continue,
                    }
                }
                vo::Step::Negation(negation, _) => Named::Negation(negation),
                vo::Step::Block(block, _) if blocks.insert(block.source.clause()) => match scope.body.iter().nth(block.source.clause()) {
                    Some(cq::BodyItem::Block(written)) => Named::Block(block, written),
                    _ => continue,
                },
                _ => continue,
            };
            expected.insert(scope.at(index), named);
        }
    }
    let definitions: BTreeMap<_, _> = target
        .plan
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.result), &definition.operator))
        .collect();
    // Formal rule RL-Seam: The side table is a bijection between the renames and block unions of the RA and the first reads, negations and blocks of the VO, kind for kind.
    let mut names = BTreeMap::new();
    for (name, position) in &table.0 {
        if names.insert(position, name).is_some() {
            return fail(format!("VO step {position:?} names two RA results"));
        }
        let (Some(operator), Some(named)) = (definitions.get(name), expected.get(position))
        else {
            return fail(format!("`{name}` -> {position:?} names no RA result or no VO step"));
        };
        let kind = matches!(
            (named, operator),
            (Named::Read(_) | Named::Negation(_), ra::Operator::Rename(_))
        ) || matches!(
            (named, operator),
            (Named::Block(..), ra::Operator::Union(union)) if union.origin.is_some()
        );
        if !kind {
            return fail(format!("`{name}` is not the RA result kind of VO step {position:?}"));
        }
    }
    if let Some(position) = expected.keys().find(|position| !names.contains_key(position)) {
        return fail(format!("VO step {position:?} has no RA result in the side table"));
    }
    if let Some(name) = definitions.iter().find_map(|(name, operator)| {
        let named = match operator {
            ra::Operator::Rename(_) => true,
            ra::Operator::Union(union) => union.origin.is_some(),
            _ => false,
        };
        (named && !table.0.contains_key(name)).then_some(name)
    }) {
        return fail(format!("`{name}` has no VO step in the side table"));
    }
    let declarations: BTreeMap<_, _> = source
        .cq
        .inputs()
        .map(|declaration| (symbol_name(&declaration.name), declaration))
        .collect();
    for (position, named) in &expected {
        let name = names[position];
        match (named, definitions[name.as_str()]) {
            // Formal rule RL-SeamRename: A first read's rename has its occurrence's relation, variable columns and literal columns; a negation's rename has its relation.
            (Named::Read(atom), ra::Operator::Rename(rename)) => {
                let declaration = declarations[&symbol_name(&atom.relation)];
                let mapping: BTreeSet<_> = declaration
                    .columns
                    .iter()
                    .zip(&atom.args)
                    .filter_map(|(column, argument)| {
                        cq::rust::variable(argument)
                            .map(|variable| (symbol_name(&column.name), symbol_name(variable)))
                    })
                    .collect();
                let constants = |constants: Option<&ra::OccurrenceConstants>| {
                    let mut equalities: Vec<_> = constants
                        .into_iter()
                        .flat_map(|constants| &constants.equalities)
                        .map(|equality| equality.to_token_stream().to_string())
                        .collect();
                    equalities.sort();
                    equalities
                };
                let actual: BTreeSet<_> = rename
                    .mapping
                    .iter()
                    .map(|m| (symbol_name(&m.source), symbol_name(&m.target)))
                    .collect();
                if symbol_name(&rename.relation) != symbol_name(&atom.relation)
                    || actual != mapping
                    || rename.mapping.len() != mapping.len()
                    || constants(rename.constants.as_ref())
                        != constants(super::occurrence_constants(declaration, atom).as_ref())
                {
                    return fail(format!(
                        "`{name}` does not rename the occurrence read at VO step {position:?}"
                    ));
                }
            }
            (Named::Negation(negation), ra::Operator::Rename(rename)) => {
                if symbol_name(&rename.relation) != symbol_name(&negation.atom.relation) {
                    return fail(format!(
                        "`{name}` does not rename the relation negated at VO step {position:?}"
                    ));
                }
            }
            // Formal rule CK-BlockPlacement: Every VO block step names a union with its branch count, fallback kind and exactly its recorded correlation alternative.
            (Named::Block(block, written), ra::Operator::Union(union)) => {
                let origin = union.origin.as_ref().expect("RL-Seam kind");
                let correlated = block.placement == vo::blocks::Placement::Correlated;
                if origin.over.is_some() != correlated
                    || union.operands.len() != block.branches.len()
                    || origin.fallback.is_some()
                        != (written.kind == cq::blocks::Kind::Fallback)
                {
                    return Err(syn::Error::new_spanned(
                        block,
                        format!("RA block `{name}` differs from the recorded VO placement"),
                    ));
                }
            }
            _ => unreachable!("RL-Seam kind"),
        }
    }
    let operands = |name: &str| -> Vec<String> {
        let names: Vec<&syn::Ident> = match definitions.get(name) {
            Some(ra::Operator::Union(u)) => u.operands.iter().collect(),
            Some(ra::Operator::Anti(a)) => vec![&a.left, &a.right],
            Some(ra::Operator::NaturalJoin(j)) => vec![&j.left, &j.right],
            Some(ra::Operator::Semijoin(j)) => vec![&j.left, &j.right],
            Some(ra::Operator::Group(g)) => vec![&g.input],
            Some(ra::Operator::Project(p)) => vec![&p.input],
            Some(ra::Operator::Select(s)) => vec![&s.input],
            Some(ra::Operator::Bind(b)) => vec![&b.input],
            Some(ra::Operator::Guard(g)) => vec![&g.input],
            _ => Vec::new(),
        };
        names.into_iter().map(symbol_name).collect()
    };
    let reaches = |from: &syn::Ident, to: &str| {
        let mut seen = BTreeSet::new();
        let mut pending = vec![symbol_name(from)];
        while let Some(name) = pending.pop() {
            if name == to {
                return true;
            }
            if seen.insert(name.clone()) {
                pending.extend(operands(&name));
            }
        }
        false
    };
    for scope in &all {
        for step in scope.steps {
            match step {
                vo::Step::Restrict(restrict, _) => {
                    // Formal rule CK-RelationalLoweringRestrict: Every conjunct of a recorded restriction selects the selection chain directly over the rename its occurrence's first read names.
                    let Some(mut current) = scope
                        .named(restrict.clause())
                        .and_then(|position| names.get(&position))
                        .map(|name| name.to_string())
                    else {
                        return Err(syn::Error::new_spanned(
                            restrict,
                            format!("restriction of `{}`: the occurrence is never read", restrict.clause),
                        ));
                    };
                    let mut applied = ra::Theta::default();
                    while let Some((result, select)) =
                        target.plan.definitions.iter().find_map(|d| match &d.operator {
                            ra::Operator::Select(select) if symbol_name(&select.input) == current => {
                                Some((symbol_name(&d.result), select))
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
                    if !expected.equalities.iter().all(|e| applied.equalities.iter().any(|a| a == e))
                        || !expected
                            .constraints
                            .iter()
                            .all(|c| applied.constraints.iter().any(|a| a == c))
                    {
                        return Err(syn::Error::new_spanned(
                            restrict,
                            format!(
                                "restriction of `{}`: the predicate does not select the occurrence",
                                restrict.clause
                            ),
                        ));
                    }
                }
                vo::Step::Reduce(reduce, _) => {
                    // Formal rule RL-SeamReduce: A recorded reduction is a semijoin, or a join selected by every distinct-name support match and projected, whose operands reach the RA results its target and support steps name.
                    let vo::Reduction::With(_, support) = &reduce.reduction;
                    let (Some(left), Some(right)) = (
                        scope.named(reduce.clause()).and_then(|p| names.get(&p)),
                        scope.named(support.clause()).and_then(|p| names.get(&p)),
                    ) else {
                        return Err(syn::Error::new_spanned(
                            reduce,
                            "reduction names an occurrence that is never read",
                        ));
                    };
                    let target_variables: Vec<_> = match scope.body.iter().nth(reduce.clause()) {
                        Some(cq::BodyItem::Positive { atom }) => {
                            atom.variables().map(symbol_name).collect()
                        }
                        _ => Vec::new(),
                    };
                    let matches: BTreeSet<_> = target_variables
                        .iter()
                        .flat_map(|variable| {
                            support
                                .variables()
                                .map(|(_, other)| symbol_name(other))
                                .filter(|other| {
                                    other != variable
                                        && scope.hypergraph.equivalent(variable, other)
                                })
                                .map(|other| {
                                    let mut pair = [variable.clone(), other];
                                    pair.sort();
                                    pair
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect();
                    let found = target.plan.definitions.iter().any(|definition| {
                        match &definition.operator {
                            ra::Operator::Semijoin(join) => {
                                matches.is_empty()
                                    && reaches(&join.left, left)
                                    && reaches(&join.right, right)
                            }
                            ra::Operator::Project(project) => {
                                let Some(ra::Operator::Select(select)) =
                                    definitions.get(symbol_name(&project.input).as_str())
                                else {
                                    return false;
                                };
                                let Some(ra::Operator::NaturalJoin(join)) =
                                    definitions.get(symbol_name(&select.input).as_str())
                                else {
                                    return false;
                                };
                                let selected: BTreeSet<_> = select
                                    .condition
                                    .equalities
                                    .iter()
                                    .filter_map(|equality| match &equality.right {
                                        ra::EqualityValue::Variable(other) => {
                                            let mut pair =
                                                [symbol_name(&equality.left), symbol_name(other)];
                                            pair.sort();
                                            Some(pair)
                                        }
                                        _ => None,
                                    })
                                    .collect();
                                !matches.is_empty()
                                    && matches.is_subset(&selected)
                                    && reaches(&join.left, left)
                                    && reaches(&join.right, right)
                            }
                            _ => false,
                        }
                    });
                    if !found {
                        return Err(syn::Error::new_spanned(
                            reduce,
                            format!(
                                "missing semijoin reduction `{left}` with `{right}` or its support column matches"
                            ),
                        ));
                    }
                }
                _ => (),
            }
        }
    }
    Ok(())
}
