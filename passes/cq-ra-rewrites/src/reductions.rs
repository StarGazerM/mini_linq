use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ra::{self as ra, Operator, symbol_name};
use syn::Ident;

pub(super) fn simplify(source: &ra::Program) -> syn::Result<ra::Program> {
    let analysis = ra::contract::check(source)?;
    let definitions = &source.plan.definitions;
    let indices: BTreeMap<_, _> = definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| (symbol_name(&definition.result), index))
        .collect();
    let dependencies: Vec<Vec<usize>> = definitions
        .iter()
        .map(|definition| {
            let mut inputs = Vec::new();
            operands(&definition.operator, |input| {
                inputs.push(indices[&symbol_name(input)]);
            });
            inputs
        })
        .collect();
    let mut negative = vec![false; definitions.len()];
    let mut opaque = vec![false; definitions.len()];
    for (index, definition) in definitions.iter().enumerate() {
        if let Operator::Anti(anti) = &definition.operator {
            negative[indices[&symbol_name(&anti.right)]] = true;
        }
        opaque[index] = matches!(
            definition.operator,
            Operator::Bind(_) | Operator::Guard(_) | Operator::Group(_)
        ) || dependencies[index].iter().any(|input| opaque[*input]);
    }
    for index in (0..definitions.len()).rev() {
        if negative[index] {
            for input in &dependencies[index] {
                negative[*input] = true;
            }
        }
    }
    let headings: Vec<_> = definitions
        .iter()
        .map(|definition| &analysis[&symbol_name(&definition.result)].heading)
        .collect();
    let proof = Lineage {
        definitions,
        dependencies: &dependencies,
        headings,
        negative,
        opaque,
    };
    let mut target = source.clone();
    for (index, definition) in target.plan.definitions.iter_mut().enumerate() {
        if proof.negative[index] {
            continue;
        }
        let (left, right) = match &mut definition.operator {
            Operator::Semijoin(join) => (&mut join.left, &mut join.right),
            Operator::NaturalJoin(join) => (&mut join.left, &mut join.right),
            _ => continue,
        };
        // Prune backward support versions first: their removal can restore
        // piped scans and filters. Then remove a duplicate left reduction only
        // when the surviving RHS proves it redundant.
        let reduced_right = proof.absorb(dependencies[index][1], dependencies[index][0]);
        let reduced_left = proof.absorb(dependencies[index][0], reduced_right);
        *right = definitions[reduced_right].result.clone();
        *left = definitions[reduced_left].result.clone();
    }
    // Partial absorption of a shared restricted result gives its projected
    // consumers different row witnesses. Keep that shared materialization
    // unless every bypassed result becomes dead. Restoring one operand can
    // revive another bypassed result, so propagate restoration to a fixed point.
    let mut reachable = vec![false; definitions.len()];
    loop {
        reachable.fill(false);
        reachable[indices[&symbol_name(&target.plan.output.input)]] = true;
        for (index, definition) in target.plan.definitions.iter().enumerate().rev() {
            if reachable[index] {
                operands(&definition.operator, |input| {
                    reachable[indices[&symbol_name(input)]] = true;
                });
            }
        }
        let mut restored = false;
        for (index, definition) in target.plan.definitions.iter_mut().enumerate() {
            let (left, right) = match &mut definition.operator {
                Operator::Semijoin(join) => (&mut join.left, &mut join.right),
                Operator::NaturalJoin(join) => (&mut join.left, &mut join.right),
                _ => continue,
            };
            for (side, operand) in [left, right].into_iter().enumerate() {
                let original = dependencies[index][side];
                let replacement = indices[&symbol_name(operand)];
                let mut bypassed = original;
                while bypassed != replacement {
                    if reachable[bypassed] {
                        *operand = definitions[original].result.clone();
                        restored = true;
                        break;
                    }
                    bypassed = dependencies[bypassed][0];
                }
            }
        }
        if !restored {
            break;
        }
    }
    let mut index = 0;
    target.plan.definitions.retain(|_| {
        let keep = reachable[index];
        index += 1;
        keep
    });
    ra::check_rewrite(source, &target)?;
    Ok(target)
}

struct Lineage<'a> {
    definitions: &'a [ra::Definition],
    dependencies: &'a [Vec<usize>],
    headings: Vec<&'a BTreeSet<String>>,
    negative: Vec<bool>,
    opaque: Vec<bool>,
}

impl Lineage<'_> {
    fn absorb(&self, mut reduced: usize, other: usize) -> usize {
        while matches!(self.definitions[reduced].operator, Operator::Semijoin(_))
            && !self.negative[reduced]
            && !self.opaque[reduced]
        {
            let [base, support] = self.dependencies[reduced][..] else {
                unreachable!("semijoin has two operands")
            };
            let shared: Vec<_> = self.headings[base]
                .intersection(self.headings[support])
                .collect();
            if !self.contained(other, support, &shared) {
                break;
            }
            reduced = base;
        }
        reduced
    }

    fn contained(&self, other: usize, support: usize, shared: &[&String]) -> bool {
        let mut pending = vec![other];
        let mut seen = BTreeSet::new();
        while let Some(index) = pending.pop() {
            if !seen.insert(index)
                || shared
                    .iter()
                    .any(|name| !self.headings[index].contains(*name))
            {
                continue;
            }
            if index == support {
                return true;
            }
            match self.definitions[index].operator {
                Operator::Semijoin(_) | Operator::Project(_) | Operator::Select(_) => {
                    pending.push(self.dependencies[index][0]);
                }
                Operator::NaturalJoin(_) => pending.extend(&self.dependencies[index]),
                _ => {}
            }
        }
        false
    }
}

// RA denotes sets. A join whose added columns have no observer can therefore
// become a semijoin: canon(project_H(L join R)) = canon(L semi R), H ⊆ cols(L).
// Do not apply this identity across Rust, negative, or block boundaries.
pub(super) fn eliminate_expansions(mut source: ra::Program) -> syn::Result<ra::Program> {
    ra::contract::check(&source)?;
    if source.plan.definitions.iter().any(|definition| {
        matches!(
            definition.operator,
            Operator::Bind(_)
                | Operator::Guard(_)
                | Operator::Anti(_)
                | Operator::Union(_)
                | Operator::Group(_)
        )
    }) {
        return Ok(source);
    }
    loop {
        let analysis = ra::contract::check(&source)?;
        let definitions = &source.plan.definitions;
        let indices: BTreeMap<_, _> = definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| (symbol_name(&definition.result), index))
            .collect();
        let headings: Vec<_> = definitions
            .iter()
            .map(|definition| &analysis[&symbol_name(&definition.result)].heading)
            .collect();
        let mut demand = vec![BTreeSet::new(); definitions.len()];
        let output = indices[&symbol_name(&source.plan.output.input)];
        demand[output].extend(headings[output].iter().cloned());
        let dependencies: Vec<Vec<usize>> = definitions
            .iter()
            .map(|definition| {
                let mut inputs = Vec::new();
                operands(&definition.operator, |input| {
                    inputs.push(indices[&symbol_name(input)])
                });
                inputs
            })
            .collect();
        for (index, definition) in definitions.iter().enumerate().rev() {
            let mut needed = demand[index].clone();
            match &definition.operator {
                Operator::NaturalJoin(_) | Operator::Semijoin(_) => {
                    let left = dependencies[index][0];
                    let right = dependencies[index][1];
                    let shared: BTreeSet<_> = headings[left]
                        .intersection(headings[right])
                        .cloned()
                        .collect();
                    needed.extend(shared.iter().cloned());
                    demand[left].extend(needed.intersection(headings[left]).cloned());
                    if matches!(definition.operator, Operator::NaturalJoin(_)) {
                        demand[right].extend(needed.intersection(headings[right]).cloned());
                    } else {
                        demand[right].extend(shared);
                    }
                }
                Operator::Project(project) => {
                    demand[dependencies[index][0]]
                        .extend(project.attributes.iter().map(symbol_name));
                }
                Operator::Select(select) => {
                    for equality in &select.condition.equalities {
                        needed.extend(equality.variables().map(symbol_name));
                    }
                    for constraint in &select.condition.constraints {
                        needed.extend(constraint.variables().map(symbol_name));
                    }
                    demand[dependencies[index][0]].extend(needed);
                }
                Operator::Unit(_) | Operator::Rename(_) => {}
                _ => unreachable!("opaque and block operators excluded"),
            }
        }
        let mut replacement = None;
        for (index, definition) in definitions.iter().enumerate().rev() {
            if !matches!(
                definition.operator,
                Operator::NaturalJoin(_) | Operator::Semijoin(_)
            ) {
                continue;
            }
            let left = dependencies[index][0];
            let right = dependencies[index][1];
            let shared: Vec<_> = headings[left].intersection(headings[right]).collect();
            let mut pending = vec![left];
            let mut seen = BTreeSet::new();
            let mut implied = false;
            while let Some(current) = pending.pop() {
                if !seen.insert(current)
                    || shared
                        .iter()
                        .any(|column| !headings[current].contains(*column))
                {
                    continue;
                }
                if current == right {
                    implied = true;
                    break;
                }
                match &definitions[current].operator {
                    Operator::NaturalJoin(_) | Operator::Semijoin(_) => {
                        pending.extend(&dependencies[current]);
                    }
                    Operator::Project(_) | Operator::Select(_) => {
                        pending.push(dependencies[current][0]);
                    }
                    _ => {}
                }
            }
            if !implied {
                continue;
            }
            match &definition.operator {
                Operator::NaturalJoin(join)
                    if headings[dependencies[index][1]]
                        .difference(headings[dependencies[index][0]])
                        .all(|column| !demand[index].contains(column)) =>
                {
                    let mut target = source.clone();
                    target.plan.definitions[index].operator =
                        Operator::Semijoin(Box::new(ra::Semijoin {
                            semijoin_token: Default::default(),
                            left: join.left.clone(),
                            with_token: Default::default(),
                            right: join.right.clone(),
                        }));
                    replacement = Some(target);
                    break;
                }
                Operator::Semijoin(join) => {
                    let mut target = source.clone();
                    for definition in &mut target.plan.definitions {
                        replace_operand(
                            &mut definition.operator,
                            &definitions[index].result,
                            &join.left,
                        );
                    }
                    if target.plan.output.input == definitions[index].result {
                        target.plan.output.input = join.left.clone();
                    }
                    let mut reachable = vec![false; definitions.len()];
                    reachable[indices[&symbol_name(&target.plan.output.input)]] = true;
                    for (index, definition) in target.plan.definitions.iter().enumerate().rev() {
                        if reachable[index] {
                            operands(&definition.operator, |input| {
                                reachable[indices[&symbol_name(input)]] = true;
                            });
                        }
                    }
                    let mut index = 0;
                    target.plan.definitions.retain(|_| {
                        let keep = reachable[index];
                        index += 1;
                        keep
                    });
                    replacement = Some(target);
                    break;
                }
                _ => {}
            }
        }
        let Some(target) = replacement else {
            return Ok(source);
        };
        ra::check_rewrite(&source, &target)?;
        source = target;
    }
}

fn replace_operand(operator: &mut Operator, old: &Ident, new: &Ident) {
    let replace = |input: &mut Ident| {
        if input == old {
            *input = new.clone();
        }
    };
    match operator {
        Operator::NaturalJoin(join) => {
            replace(&mut join.left);
            replace(&mut join.right);
        }
        Operator::Semijoin(join) => {
            replace(&mut join.left);
            replace(&mut join.right);
        }
        Operator::Project(project) => replace(&mut project.input),
        Operator::Group(group) => replace(&mut group.input),
        Operator::Select(select) => replace(&mut select.input),
        Operator::Unit(_) | Operator::Rename(_) => {}
        _ => unreachable!("opaque and block operators excluded"),
    }
}

// Metadata references participate in liveness just like ordinary operands.
// In particular, a block's finite domain and opaque prefix are not disposable.
fn operands(operator: &Operator, mut visit: impl FnMut(&Ident)) {
    match operator {
        Operator::Unit(_) | Operator::Rename(_) => {}
        Operator::NaturalJoin(join) => {
            visit(&join.left);
            visit(&join.right);
        }
        Operator::Semijoin(join) => {
            visit(&join.left);
            visit(&join.right);
        }
        Operator::Anti(anti) => {
            visit(&anti.left);
            visit(&anti.right);
        }
        Operator::Project(project) => visit(&project.input),
        Operator::Group(group) => visit(&group.input),
        Operator::Select(select) => visit(&select.input),
        Operator::Bind(bind) => visit(&bind.input),
        Operator::Guard(guard) => visit(&guard.input),
        Operator::Union(union) => {
            for input in &union.operands {
                visit(input);
            }
            if let Some(origin) = &union.origin {
                if let Some(over) = &origin.over {
                    visit(&over.prefix);
                }
                if let Some(domain) = &origin.domain {
                    visit(&domain.input);
                }
            }
        }
    }
}
