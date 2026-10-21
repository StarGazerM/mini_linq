//! Column coverage and layouts for explicitly recorded filtered indexes.
use super::*;
use syntax::constraint::{Comparison, Constraint, Term};

/// Columns whose values are read downstream, including relational keys and
/// predicates and the free variables of Rust payloads. Opaque Rust keeps its
/// complete environment. A bind is evaluated even when its outputs are unused.
pub fn required_columns(
    program: &physical::Program,
    facts: &physical::Analysis,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut needed: BTreeMap<_, BTreeSet<String>> = facts
        .headings
        .keys()
        .map(|name| (name.clone(), BTreeSet::new()))
        .collect();
    let uses = |expression: &syn::Expr, result: &str| {
        syntax::rust::free_variables(expression)
            .dependencies(&facts.headings[result].iter().cloned().collect())
    };
    let returned = symbol_name(&program.program.returned);
    for expression in &program.program.head.args {
        needed
            .get_mut(&returned)
            .unwrap()
            .extend(uses(expression, &returned));
    }
    for definition in program.program.definitions.iter().rev() {
        let columns = needed[&symbol_name(&definition.name)].clone();
        let mut require = |input: &Ident, columns: BTreeSet<String>| {
            let name = symbol_name(input);
            needed.get_mut(&name).unwrap().extend(
                columns
                    .into_iter()
                    .filter(|c| facts.headings[&name].contains(c)),
            );
        };
        match &definition.operator {
            Operator::Union(union) => {
                for operand in &union.operands {
                    require(operand, columns.clone());
                }
            }
            Operator::Unit(_) | Operator::Rename(_) => {}
            Operator::Probe(_, keyed) | Operator::Merge(_, keyed) => {
                let mut columns = columns;
                columns.extend(keyed.columns.iter().map(symbol_name));
                require(&keyed.left, columns.clone());
                require(&keyed.right, columns);
            }
            Operator::Semi(_, keyed) | Operator::Anti(_, keyed) => {
                let keys: BTreeSet<_> = keyed.columns.iter().map(symbol_name).collect();
                require(&keyed.left, columns.union(&keys).cloned().collect());
                require(&keyed.right, keys);
            }
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                require(
                    &s.input,
                    columns.union(&s.condition.columns()).cloned().collect(),
                );
            }
            Operator::Project(p) => require(&p.input, columns),
            Operator::Guard(g) => {
                require(
                    &g.input,
                    columns
                        .union(&uses(&g.condition, &symbol_name(&g.input)))
                        .cloned()
                        .collect(),
                );
            }
            Operator::Bind(b) => {
                require(
                    &b.input,
                    columns
                        .union(&uses(&b.expression, &symbol_name(&b.input)))
                        .cloned()
                        .collect(),
                );
            }
        }
    }
    needed
}

/// Singleton equality bindings proved by a selection's conjuncts. Multi-point
/// membership and disjunction do not prove a single value for a column.
pub fn selection_constants(select: &syntax::algebra::Select) -> BTreeMap<String, Term> {
    let mut constants = BTreeMap::new();
    for equality in &select.condition.equalities {
        if let syntax::EqualityValue::Literal(value) = &equality.right {
            constants.insert(
                symbol_name(&equality.left),
                Term::Literal(Box::new(value.clone())),
            );
        }
    }
    for constraint in &select.condition.constraints {
        let binding = match constraint {
            Constraint::Compare {
                left: Term::Variable(column),
                comparison: Comparison::Eq(_),
                right,
            } if right.constant() => Some((column, right.clone())),
            Constraint::Compare {
                left,
                comparison: Comparison::Eq(_),
                right: Term::Variable(column),
            } if left.constant() => Some((column, left.clone())),
            Constraint::In {
                term: Term::Variable(column),
                literals,
            } if literals.len() == 1 => Some((
                column,
                Term::Literal(Box::new(literals.first().unwrap().clone())),
            )),
            _ => None,
        };
        if let Some((column, value)) = binding {
            constants.insert(symbol_name(column), value);
        }
    }
    constants
}

/// Formal rule STR-Existential: A projected full-key index can replace payload
/// enumeration with membership when every observed column is a key or constant.
pub(super) fn check(program: &Program, analysis: &Analysis) -> syn::Result<()> {
    let required = required_columns(&program.physical, &analysis.physical);
    for (name, filtered) in &analysis.filtered {
        let definition = program
            .physical
            .program
            .definitions
            .iter()
            .find(|d| symbol_name(&d.name) == *name)
            .unwrap();
        let select = match &definition.operator {
            Operator::Point(_, s) | Operator::Range(_, s) => Some(s),
            Operator::Union(_) => None,
            _ => {
                return error(
                    &definition.name,
                    "filtered storage requires a point or range selection, or union",
                );
            }
        };
        let multiple = matches!(&definition.operator,Operator::Union(union) if union.origin.as_ref().is_none_or(|origin|origin.over.is_none()));
        let built = analysis
            .built
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if analysis.physical.modes[name] != ModeKind::Let
            || (analysis.physical.scanned.contains(name) || analysis.retained.contains_key(name))
                && !multiple
            || built.len() != 1 && !multiple
            || built
                .iter()
                .any(|(levels, _)| levels.len() != 1 && !multiple)
        {
            return error(
                &definition.name,
                "filtered storage requires a shared index or non-correlated union indices, without row scans or retained rows",
            );
        }
        for entry in &program.structures.entries {
            if symbol_name(&entry.name) == *name {
                if let Structure::Built { keyed, .. } = &entry.structure {
                    if keyed.filtered.as_ref() != Some(filtered) {
                        return error(
                            entry,
                            "every direct union index must record the same filtered projection",
                        );
                    }
                }
            }
        }
        for (reader, columns, supply) in analysis
            .physical
            .keyed_reads
            .get(name)
            .into_iter()
            .flatten()
        {
            let reader = program
                .physical
                .program
                .definitions
                .iter()
                .find(|d| symbol_name(&d.name) == *reader)
                .unwrap();
            if *supply != SupplyKind::Stream
                || !built
                    .iter()
                    .any(|(levels, _)| prefix_depth(levels, columns).is_some())
                || !matches!(&reader.operator,Operator::Probe(_,key) if matches!(key.choice,Some(physical::Choice::First(_))))
                    && !matches!(&reader.operator, Operator::Semi(..) | Operator::Anti(..))
            {
                return error(
                    &reader.name,
                    "filtered index readers must use one of their recorded probe or membership keys",
                );
            }
        }
        let expected = select.map(|s| selection_constants(s)).unwrap_or_default();
        let mut constants = BTreeMap::new();
        for binding in &filtered.constants {
            let column = symbol_name(&binding.column);
            if constants
                .insert(column.clone(), binding.value.clone())
                .is_some()
            {
                return error(binding, "duplicate constant binding");
            }
            if expected.get(&column) != Some(&binding.value) {
                return error(binding, "constant binding is not proved by the selection");
            }
        }
        if constants != expected {
            return error(
                filtered,
                "filtered storage must record every constant binding of the selection",
            );
        }
        let mut coverage: BTreeSet<_> = constants.keys().cloned().collect();
        let mut seen = BTreeSet::new();
        for column in &filtered.payload.columns {
            let column_name = symbol_name(column);
            if !analysis.physical.headings[name].contains(&column_name)
                || constants.contains_key(&column_name)
                || !seen.insert(column_name.clone())
            {
                return error(
                    column,
                    "filtered payload must contain distinct non-constant columns of the selected result",
                );
            }
            coverage.insert(column_name);
        }
        if let Some(column) = required[name].difference(&coverage).next() {
            return error(
                filtered,
                format!(
                    "filtered payload and constant bindings do not cover downstream column `{column}`"
                ),
            );
        }
        // No downstream value distinguishes witnesses sharing this key.
        for (_, key) in analysis
            .membership
            .iter()
            .filter(|(result, _)| result == name)
        {
            if filtered
                .payload
                .columns
                .iter()
                .any(|column| !key.contains(&symbol_name(column)))
                || analysis
                    .physical
                    .keyed_reads
                    .get(name)
                    .into_iter()
                    .flatten()
                    .any(|(_, columns, supply)| columns != key || *supply != SupplyKind::Stream)
                || analysis
                    .physical
                    .merge_reads
                    .get(name)
                    .is_some_and(|reads| !reads.is_empty())
            {
                return error(
                    filtered,
                    "membership projection must observe only its complete key and proven constants",
                );
            }
        }
    }
    Ok(())
}

pub(super) fn layouts(program: &Program, analysis: &Analysis) -> BTreeMap<String, Vec<String>> {
    let required = required_columns(&program.physical, &analysis.physical);
    let mut layouts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for definition in &program.physical.program.definitions {
        let name = symbol_name(&definition.name);
        let input_layout = |input: &Ident| layouts[&symbol_name(input)].clone();
        let mut columns = match &definition.operator {
            Operator::Probe(_, k) | Operator::Merge(_, k) => {
                let mut columns = input_layout(&k.left);
                let right = symbol_name(&k.right);
                if let Some(filtered) = analysis.filtered.get(&right) {
                    columns.extend(filtered.payload.columns.iter().map(symbol_name));
                    columns.extend(
                        filtered
                            .constants
                            .iter()
                            .map(|b| symbol_name(&b.column))
                            .filter(|c| required[&right].contains(c)),
                    );
                } else {
                    columns.extend(input_layout(&k.right));
                }
                columns
            }
            Operator::Semi(_, k) | Operator::Anti(_, k) => input_layout(&k.left),
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                input_layout(&s.input)
            }
            Operator::Guard(g) => input_layout(&g.input),
            Operator::Project(p) => input_layout(&p.input),
            Operator::Bind(b) => {
                let mut columns = input_layout(&b.input);
                columns.extend(syntax::rust::bindings(&b.pattern).iter().map(symbol_name));
                columns
            }
            _ => analysis.physical.headings[&name].clone(),
        };
        columns.retain(|c| analysis.physical.headings[&name].contains(c));
        columns.sort();
        columns.dedup();
        layouts.insert(name, columns);
    }
    layouts
}
