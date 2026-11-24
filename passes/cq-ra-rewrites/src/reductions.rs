use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ra::{self as ra, Operator, symbol_name};
use syn::Ident;

// Formal rule RA-AbsorbPolicy: Absorb only when every use is a direct absorbing join, replacing all uses together to a fixed point.
pub(super) fn simplify(source: &ra::Program) -> syn::Result<ra::Program> {
    let mut target = source.clone();
    loop {
        let analysis = ra::contract::check(&target)?;
        let definitions = &target.plan.definitions;
        let indices: BTreeMap<_, _> = definitions
            .iter()
            .enumerate()
            .map(|(i, d)| (symbol_name(&d.result), i))
            .collect();
        let dependencies: Vec<Vec<usize>> = definitions
            .iter()
            .map(|d| {
                let mut inputs = Vec::new();
                operands(&d.operator, |input| inputs.push(indices[&symbol_name(input)]));
                inputs
            })
            .collect();
        let mut negative = vec![false; definitions.len()];
        let mut opaque = vec![false; definitions.len()];
        for (i, d) in definitions.iter().enumerate() {
            if matches!(d.operator, Operator::Anti(_)) {
                negative[dependencies[i][1]] = true;
            }
            opaque[i] = matches!(
                d.operator,
                Operator::Bind(_) | Operator::Guard(_) | Operator::Group(_)
            ) || dependencies[i].iter().any(|&input| opaque[input]);
        }
        let output = indices[&symbol_name(&target.plan.output.input)];
        for i in (0..definitions.len()).rev() {
            if negative[i] {
                for &input in &dependencies[i] {
                    negative[input] = true;
                }
            }
        }
        let lineage = Lineage {
            definitions,
            dependencies: &dependencies,
            headings: definitions
                .iter()
                .map(|d| &analysis[&symbol_name(&d.result)].heading)
                .collect(),
        };
        let reduction = (0..definitions.len()).rev().find(|&d| {
            !negative[d] && !opaque[d] && lineage.all_consumers_absorb(d, output)
        });
        let Some(d) = reduction else { break };
        let old = definitions[d].result.clone();
        let base = definitions[dependencies[d][0]].result.clone();
        let mut live = vec![false; definitions.len()];
        for (i, inputs) in dependencies.iter().enumerate() {
            if inputs.contains(&d) {
                replace_operand(&mut target.plan.definitions[i].operator, &old, &base);
            }
        }
        live[indices[&symbol_name(&target.plan.output.input)]] = true;
        for (i, definition) in target.plan.definitions.iter().enumerate().rev() {
            if live[i] {
                operands(&definition.operator, |input| {
                    live[indices[&symbol_name(input)]] = true;
                });
            }
        }
        let mut i = 0;
        target.plan.definitions.retain(|_| {
            let keep = live[i];
            i += 1;
            keep
        });
    }
    ra::check_rewrite(source, &target)?;
    Ok(target)
}

struct Lineage<'a> {
    definitions: &'a [ra::Definition],
    dependencies: &'a [Vec<usize>],
    headings: Vec<&'a BTreeSet<String>>,
}

impl Lineage<'_> {
    fn all_consumers_absorb(&self, d: usize, output: usize) -> bool {
        if d == output || !matches!(self.definitions[d].operator, Operator::Semijoin(_)) {
            return false;
        }
        let [base, support] = self.dependencies[d][..] else {
            unreachable!("semijoin has two operands")
        };
        let keys: Vec<_> = self.headings[base]
            .intersection(self.headings[support])
            .collect();
        let mut used = false;
        for (consumer, inputs) in self.dependencies.iter().enumerate() {
            for (side, &input) in inputs.iter().enumerate() {
                if input != d {
                    continue;
                }
                if !matches!(
                    self.definitions[consumer].operator,
                    Operator::NaturalJoin(_) | Operator::Semijoin(_)
                ) || !self.witness(inputs[1 - side], support, &keys) {
                    return false;
                }
                used = true;
            }
        }
        used
    }

    fn witness(&self, other: usize, support: usize, keys: &[&String]) -> bool {
        let mut pending = vec![other];
        let mut seen = BTreeSet::new();
        while let Some(index) = pending.pop() {
            if !seen.insert(index) || keys.iter().any(|key| !self.headings[index].contains(*key)) {
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
