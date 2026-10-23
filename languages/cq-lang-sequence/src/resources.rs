//! Structural allocation demands for checked dispatches. Storage sizes belong to
//! the machine; this traversal records only payload carriers and reduction scopes.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_structures::physical::{Definition, Operator, symbol_name};
use cq_syntax::{algebra::Environment, rust};

use crate::{
    Analysis, Capability, Dispatch, DispatchShape, Machine, Monoid, Obligation, Program,
    ResourceAllocation, ResourceColumn, Term,
};

type Columns = BTreeSet<String>;

#[derive(Clone)]
enum Operation {
    Map(Columns),
    Lookup {
        keys: Columns,
        payload: Columns,
        membership: bool,
    },
    Flatten,
    Tile {
        items: usize,
    },
    Store,
    Fold(Monoid),
    Canon,
}

struct Chain {
    result: String,
    operations: Vec<Operation>,
}

struct Builder<'a> {
    program: &'a Program,
    analysis: &'a Analysis,
    definitions: BTreeMap<String, &'a Term>,
    physical: BTreeMap<String, &'a Definition>,
    publications: BTreeSet<String>,
    carried: BTreeMap<String, Columns>,
    scanned: BTreeSet<String>,
    device_results: BTreeSet<String>,
    coded: BTreeSet<String>,
    width: Option<usize>,
}

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}

fn publication<'a>(
    dispatch: &'a Dispatch,
    definitions: &BTreeMap<String, &Term>,
) -> Option<&'a String> {
    dispatch.inner_stores.iter().rev().find(|name| {
        definitions
            .get(*name)
            .is_some_and(|term| matches!(term, Term::Store { .. }))
    })
}

pub(crate) fn dispatch_shape(
    program: &Program,
    analysis: &Analysis,
    dispatch: &Dispatch,
    machine: &Machine,
) -> syn::Result<DispatchShape> {
    let mut node = &machine.root;
    let mut path = vec![symbol_name(&node.name)];
    let mut collective = node.collaborative.then(|| (path.clone(), node.width));
    for name in dispatch.producer_level.iter().skip(1) {
        node = node
            .children
            .iter()
            .find(|node| symbol_name(&node.name) == *name)
            .ok_or_else(|| error("BE-Resource: producer level is absent from the machine"))?;
        path.push(name.clone());
        if node.collaborative {
            collective = Some((path.clone(), node.width));
        }
    }
    let (level, width) = collective.unwrap_or((path, node.width));
    let definitions: BTreeMap<_, _> = program
        .sequence
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.name), &definition.term))
        .collect();
    let name = publication(dispatch, &definitions)
        .ok_or_else(|| error("BE-Resource: dispatch has no explicit publication store"))?;
    let Term::Store { inner, .. } = definitions[name] else {
        unreachable!("publication selects an explicit store")
    };
    let mut publications: BTreeSet<_> = analysis
        .dispatches
        .iter()
        .filter_map(|dispatch| publication(dispatch, &definitions).cloned())
        .collect();
    publications.extend(
        analysis
            .stores
            .keys()
            .filter(|name| {
                analysis
                    .levels
                    .get(*name)
                    .is_some_and(|level| level.len() == 1)
            })
            .cloned(),
    );
    let builder = Builder {
        program,
        analysis,
        definitions,
        physical: program
            .structures
            .physical
            .program
            .definitions
            .iter()
            .map(|definition| (symbol_name(&definition.name), definition))
            .collect(),
        publications,
        carried: carried_columns(program, analysis),
        scanned: analysis
            .dispatches
            .iter()
            .map(|dispatch| {
                analysis
                    .results
                    .get(&dispatch.outer_store)
                    .unwrap_or(&dispatch.outer_store)
                    .clone()
            })
            .collect(),
        device_results: analysis
            .dispatches
            .iter()
            .flat_map(|dispatch| {
                let source = analysis.results.get(&dispatch.outer_store);
                dispatch
                    .inner_stores
                    .iter()
                    .filter_map(|name| analysis.results.get(name))
                    .filter(move |result| Some(*result) != source)
                    .cloned()
            })
            .collect(),
        coded: analysis
            .obligations
            .iter()
            .filter_map(|obligation| {
                if let Obligation::Codec { columns, .. } = obligation {
                    Some(columns.iter().cloned())
                } else {
                    None
                }
            })
            .flatten()
            .collect(),
        width,
    };
    let mut chains = builder.chains(inner)?;
    builder.rehead(name, &mut chains);
    let final_name = analysis
        .output
        .as_ref()
        .map(|(name, _)| name.clone())
        .or_else(|| {
            program
                .sequence
                .definitions
                .last()
                .map(|definition| symbol_name(&definition.name))
        });
    let final_output = final_name.as_ref() == Some(name);
    let canonical = chains.iter().all(|chain| {
        chain
            .operations
            .iter()
            .any(|operation| matches!(operation, Operation::Canon))
    });
    let host_union_operand = program.sequence.definitions.iter().any(|definition| {
        analysis
            .levels
            .get(&symbol_name(&definition.name))
            .is_some_and(|level| level.len() == 1)
            && builder.host_union(&definition.term).is_some_and(|arms| {
                arms.iter().any(
                    |arm| matches!(arm, Term::Ref(reference) if symbol_name(reference) == *name),
                )
            })
    });
    let virtual_range = !final_output
        && !canonical
        && !host_union_operand
        && chains.len() == 1
        && !analysis.structures.built.contains_key(&chains[0].result)
        && !builder.host_consumed_results().contains(&chains[0].result)
        && chains[0]
            .operations
            .iter()
            .rposition(|operation| {
                matches!(
                    operation,
                    Operation::Lookup {
                        membership: false,
                        ..
                    }
                )
            })
            .is_some_and(|at| {
                terminal_range(&chains[0].operations[at + 1..])
                    && chains[0].operations[at + 1..]
                        .iter()
                        .any(|operation| matches!(operation, Operation::Flatten))
            });
    let outputs = if final_output {
        let returned = symbol_name(&program.structures.physical.program.returned);
        program
            .structures
            .physical
            .program
            .head
            .args
            .iter()
            .flat_map(|expression| {
                rust_columns(expression, &analysis.structures.layouts[&returned], None)
            })
            .collect()
    } else if canonical && builder.output_fold(inner) {
        // A scalar publication reuses the single declared output attribute.
        program
            .structures
            .physical
            .program
            .head
            .args
            .first()
            .and_then(rust::variable)
            .map(symbol_name)
            .into_iter()
            .collect()
    } else {
        chains
            .iter()
            .flat_map(|chain| {
                analysis
                    .structures
                    .physical
                    .headings
                    .get(&chain.result)
                    .into_iter()
                    .flatten()
                    .cloned()
            })
            .collect()
    };
    let items = chains
        .iter()
        .map(|chain| {
            chain
                .operations
                .iter()
                .take_while(|operation| {
                    !matches!(
                        operation,
                        Operation::Lookup {
                            membership: false,
                            ..
                        } | Operation::Fold(_)
                    )
                })
                .filter_map(|operation| {
                    if let Operation::Tile { items } = operation {
                        Some(*items)
                    } else {
                        None
                    }
                })
                .last()
                .unwrap_or(1)
        })
        .max()
        .unwrap_or(1);
    let demands = chains
        .iter()
        .map(|chain| {
            let mut demands = Vec::new();
            builder.partition(
                &chain.operations,
                true,
                &outputs,
                virtual_range,
                &mut demands,
            )?;
            Ok(demands)
        })
        .collect::<syn::Result<_>>()?;
    Ok(DispatchShape {
        publication: name.clone(),
        level,
        width,
        tile: dispatch.parts,
        items,
        chains: demands,
    })
}

impl<'p> Builder<'p> {
    fn rehead(&self, name: &str, chains: &mut [Chain]) {
        if let Some(result) = self.analysis.results.get(name) {
            for chain in chains {
                chain.result.clone_from(result);
            }
        }
    }

    fn chains(&self, term: &Term) -> syn::Result<Vec<Chain>> {
        match term {
            Term::Rows(name) => Ok(vec![Chain {
                result: symbol_name(name),
                operations: Vec::new(),
            }]),
            Term::Ref(name) => {
                let name = symbol_name(name);
                if self.publications.contains(&name) {
                    return Ok(vec![Chain {
                        result: self.reference_result(&name)?,
                        operations: Vec::new(),
                    }]);
                }
                let term = self
                    .definitions
                    .get(&name)
                    .ok_or_else(|| error("BE-Resource: reference has no checked definition"))?;
                let mut chains = self.chains(term)?;
                self.rehead(&name, &mut chains);
                Ok(chains)
            }
            Term::Union(arms) => {
                let mut chains = Vec::new();
                for arm in arms {
                    chains.extend(self.chains(arm)?);
                }
                Ok(chains)
            }
            Term::Choose(arms) => {
                let mut chains = Vec::new();
                for (_, arm) in arms {
                    chains.extend(self.chains(arm)?);
                }
                Ok(chains)
            }
            Term::Compact(name, inner) if matches!(inner.as_ref(), Term::Lookup { .. }) => {
                let Term::Lookup {
                    provider,
                    keys,
                    inner,
                } = inner.as_ref()
                else {
                    unreachable!()
                };
                let mut chains = self.lookup(provider, keys, inner, true)?;
                for chain in &mut chains {
                    chain.result = symbol_name(name);
                }
                Ok(chains)
            }
            Term::Lookup {
                provider,
                keys,
                inner,
            } => {
                let mut columns: Vec<_> = keys.iter().map(symbol_name).collect();
                columns.sort();
                let capability = self
                    .analysis
                    .capabilities
                    .get(&(symbol_name(provider), columns))
                    .ok_or_else(|| error("BE-Resource: lookup has no checked capability"))?;
                self.lookup(provider, keys, inner, *capability == Capability::Membership)
            }
            Term::Map(name, inner) | Term::Compact(name, inner) | Term::Group(name, inner) => {
                let mut chains = self.chains(inner)?;
                let result = symbol_name(name);
                for chain in &mut chains {
                    let dependencies = self.map_columns(&result, &chain.result)?;
                    chain.operations.push(Operation::Map(dependencies));
                    chain.result.clone_from(&result);
                }
                Ok(chains)
            }
            Term::Flatten(inner)
            | Term::Tile { inner, .. }
            | Term::Store { inner, .. }
            | Term::Fold(_, inner)
            | Term::Canon(inner) => {
                let mut chains = self.chains(inner)?;
                let operation = match term {
                    Term::Flatten(_) => Operation::Flatten,
                    Term::Tile { n, items, .. } => Operation::Tile {
                        items: items.unwrap_or_else(|| n.div_ceil(self.width.unwrap_or(*n).max(1))),
                    },
                    Term::Store { .. } => Operation::Store,
                    Term::Fold(monoid, _) => Operation::Fold(monoid.clone()),
                    Term::Canon(_) => Operation::Canon,
                    _ => unreachable!(),
                };
                for chain in &mut chains {
                    chain.operations.push(operation.clone());
                }
                Ok(chains)
            }
        }
    }

    fn lookup(
        &self,
        provider: &syn::Ident,
        keys: &[syn::Ident],
        inner: &Term,
        membership: bool,
    ) -> syn::Result<Vec<Chain>> {
        let mut chains = self.chains(inner)?;
        let provider = symbol_name(provider);
        let keys: Columns = keys.iter().map(symbol_name).collect();
        let payload = if membership {
            Columns::new()
        } else {
            self.payload(&provider, &keys)?
        };
        for chain in &mut chains {
            if !membership {
                chain.result = self.lookup_result(&provider, &keys, &chain.result)?;
            }
            chain.operations.push(Operation::Lookup {
                keys: keys.clone(),
                payload: payload.clone(),
                membership,
            });
        }
        Ok(chains)
    }

    fn lookup_result(&self, provider: &str, keys: &Columns, input: &str) -> syn::Result<String> {
        self.program
            .structures
            .physical
            .program
            .definitions
            .iter()
            .find_map(|definition| match &definition.operator {
                Operator::Probe(_, keyed) | Operator::Merge(_, keyed)
                    if symbol_name(&keyed.right) == provider
                        && symbol_name(&keyed.left) == input
                        && keyed.columns.iter().map(symbol_name).collect::<Columns>() == *keys =>
                {
                    Some(symbol_name(&definition.name))
                }
                _ => None,
            })
            .ok_or_else(|| error("BE-Resource: lookup has no checked Physical row provenance"))
    }

    fn reference_result(&self, name: &str) -> syn::Result<String> {
        if let Some(result) = self.analysis.results.get(name) {
            return Ok(result.clone());
        }
        // Scalar stores do not have an Analysis.results entry. Recover only
        // their row-slot provenance, without importing their allocation stream.
        let term = self
            .definitions
            .get(name)
            .ok_or_else(|| error("BE-Resource: publication has no checked definition"))?;
        self.row_result(term)
    }

    fn row_result(&self, term: &Term) -> syn::Result<String> {
        match term {
            Term::Rows(name)
            | Term::Map(name, _)
            | Term::Compact(name, _)
            | Term::Group(name, _) => Ok(symbol_name(name)),
            Term::Ref(name) => self.reference_result(&symbol_name(name)),
            Term::Flatten(inner)
            | Term::Tile { inner, .. }
            | Term::Store { inner, .. }
            | Term::Fold(_, inner)
            | Term::Canon(inner) => self.row_result(inner),
            Term::Choose(arms) => self.row_result(
                &arms
                    .first()
                    .ok_or_else(|| error("BE-Resource: empty checked choice"))?
                    .1,
            ),
            Term::Union(arms) => self.row_result(
                arms.first()
                    .ok_or_else(|| error("BE-Resource: empty checked union"))?,
            ),
            Term::Lookup {
                provider,
                keys,
                inner,
            } => {
                let provider = symbol_name(provider);
                let keys: Columns = keys.iter().map(symbol_name).collect();
                let input = self.row_result(inner)?;
                let capability = self
                    .analysis
                    .capabilities
                    .get(&(provider.clone(), keys.iter().cloned().collect()))
                    .ok_or_else(|| error("BE-Resource: lookup has no checked capability"))?;
                if *capability == Capability::Membership {
                    Ok(input)
                } else {
                    self.lookup_result(&provider, &keys, &input)
                }
            }
        }
    }

    fn payload(&self, provider: &str, keys: &Columns) -> syn::Result<Columns> {
        let structures = &self.analysis.structures;
        let result = self
            .analysis
            .results
            .get(provider)
            .map(String::as_str)
            .unwrap_or(provider);
        let Some(layouts) = structures.built.get(result) else {
            // A native checked host reader exposes its complete declared row;
            // it does not imply an unrecorded keyed build.
            return structures
                .layouts
                .get(result)
                .map(|layout| layout.iter().cloned().collect())
                .ok_or_else(|| error("BE-Resource: payload provider has no checked row layout"));
        };
        let (levels, _) = layouts
            .iter()
            .filter(|(levels, _)| {
                let mut prefix = Columns::new();
                levels.iter().any(|level| {
                    prefix.extend(level.iter().cloned());
                    &prefix == keys
                }) || (keys.is_empty() && levels.iter().all(Vec::is_empty))
            })
            .min_by_key(|(levels, _)| (levels.concat().len() != keys.len(), levels.len()))
            .ok_or_else(|| error("BE-Resource: lookup keys have no checked provider prefix"))?;
        if self.scanned.contains(result) && !self.device_results.contains(result) {
            return Ok(structures.layouts[result].iter().cloned().collect());
        }
        let order = levels.concat();
        let mut payload = if structures
            .membership
            .contains(&(result.to_owned(), order.clone()))
        {
            Columns::new()
        } else if let Some(filtered) = structures.filtered.get(result) {
            filtered.payload.columns.iter().map(symbol_name).collect()
        } else {
            self.carried[result].clone()
        };
        // Equality reconstructs the leading directory key from the parent;
        // deeper directory keys are part of the enumerated provider payload.
        if let Some(first) = order.first() {
            payload.remove(first);
        }
        payload.extend(order.into_iter().skip(1));
        Ok(payload)
    }

    fn map_columns(&self, result: &str, input: &str) -> syn::Result<Columns> {
        let mut columns = Columns::new();
        let mut current = result.to_owned();
        while current != input {
            let definition = self
                .physical
                .get(&current)
                .ok_or_else(|| error("BE-Resource: map has no checked Physical definition"))?;
            columns.extend(operator_columns(&definition.operator, self.analysis));
            let Some(parent) = definition.operator.input() else {
                break;
            };
            current = symbol_name(parent);
        }
        Ok(columns)
    }

    fn live(&self, operations: &[Operation], sink: &Columns) -> Columns {
        let mut live = sink.clone();
        for operation in operations.iter().rev() {
            match operation {
                Operation::Map(columns) => live.extend(columns.iter().cloned()),
                Operation::Lookup {
                    keys,
                    payload,
                    membership,
                    ..
                } => {
                    if !membership {
                        live.retain(|column| !payload.contains(column));
                    }
                    live.extend(keys.iter().cloned());
                }
                Operation::Fold(monoid) => live = fold_columns(monoid),
                _ => {}
            }
        }
        live
    }

    fn column(&self, name: &str) -> syn::Result<ResourceColumn> {
        let mut types = Vec::new();
        for ty in self
            .analysis
            .structures
            .physical
            .rows
            .values()
            .filter_map(|row| row.types.get(name))
        {
            if !types.contains(ty) {
                types.push(ty.clone());
            }
        }
        if types.is_empty() {
            return Err(error(format!(
                "BE-Resource: parent column {name} has no checked type"
            )));
        }
        Ok(ResourceColumn {
            name: name.to_owned(),
            types,
            coded: self.coded.contains(name),
        })
    }

    fn partition(
        &self,
        operations: &[Operation],
        source: bool,
        sink: &Columns,
        virtual_range: bool,
        demands: &mut Vec<ResourceAllocation>,
    ) -> syn::Result<()> {
        let Some(at) = operations
            .iter()
            .position(|operation| matches!(operation, Operation::Fold(_)))
        else {
            return self.rows(operations, sink, virtual_range, demands);
        };
        let target = active_tile(&operations[..at]);
        let next_lookup = operations
            .iter()
            .position(|operation| {
                matches!(
                    operation,
                    Operation::Lookup {
                        membership: false,
                        ..
                    }
                )
            })
            .unwrap_or(at)
            .min(at);
        let current = active_tile(&operations[..next_lookup]);
        if !(source && target.is_none()) && (target != current || target.is_none()) {
            return self.rows(operations, sink, virtual_range, demands);
        }
        let Operation::Fold(monoid) = &operations[at] else {
            unreachable!()
        };
        // The reduction owns its scratch before its input body is traversed.
        // Payload carries inside that body see only the reduction's input uses.
        demands.push(ResourceAllocation::Fold);
        self.rows(
            &operations[..at],
            &fold_columns(monoid),
            virtual_range,
            demands,
        )?;
        self.rows(&operations[at + 1..], sink, virtual_range, demands)
    }

    fn rows(
        &self,
        operations: &[Operation],
        sink: &Columns,
        virtual_range: bool,
        demands: &mut Vec<ResourceAllocation>,
    ) -> syn::Result<()> {
        for (at, operation) in operations.iter().enumerate() {
            let rest = &operations[at + 1..];
            match operation {
                Operation::Lookup {
                    payload,
                    membership: false,
                    ..
                } => {
                    if virtual_range && terminal_range(rest) {
                        return Ok(());
                    }
                    let live = self.live(rest, sink);
                    let parents = live
                        .difference(payload)
                        .map(|name| self.column(name))
                        .collect::<syn::Result<_>>()?;
                    demands.push(ResourceAllocation::Payload { parents });
                    return self.partition(rest, false, sink, virtual_range, demands);
                }
                Operation::Fold(_) => {
                    // A scalar result starts the next reduction scope. Targets
                    // may separately restrict unsupported scalar-fold templates.
                    return self.partition(&operations[at..], true, sink, virtual_range, demands);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn host_union(&self, term: &'p Term) -> Option<&'p [Term]> {
        match term {
            Term::Union(arms) => Some(arms),
            Term::Store { inner, .. } | Term::Canon(inner) => self.host_union(inner),
            Term::Ref(name) => self.host_union(self.definitions.get(&symbol_name(name))?),
            _ => None,
        }
    }

    fn host_consumed_results(&self) -> Columns {
        let mut consumed = Columns::new();
        for definition in &self.program.sequence.definitions {
            let name = symbol_name(&definition.name);
            if matches!(definition.term, Term::Store { .. })
                && self
                    .analysis
                    .levels
                    .get(&name)
                    .is_some_and(|level| level.len() == 1)
                && !self
                    .analysis
                    .dispatches
                    .iter()
                    .any(|dispatch| dispatch.inner_stores.contains(&name))
                && let Some(result) = self.analysis.results.get(&name)
            {
                self.dependencies(result, &mut consumed);
            }
        }
        consumed
    }

    fn dependencies(&self, name: &str, consumed: &mut Columns) {
        if !consumed.insert(name.to_owned()) {
            return;
        }
        if let Some(definition) = self.physical.get(name) {
            for operand in definition.operator.operands() {
                self.dependencies(&symbol_name(operand), consumed);
            }
        }
    }

    fn output_fold(&self, term: &Term) -> bool {
        match term {
            Term::Fold(_, _) => true,
            Term::Canon(inner) | Term::Flatten(inner) | Term::Store { inner, .. } => {
                self.output_fold(inner)
            }
            Term::Ref(name) => self
                .definitions
                .get(&symbol_name(name))
                .is_some_and(|term| self.output_fold(term)),
            _ => false,
        }
    }
}

fn terminal_range(operations: &[Operation]) -> bool {
    operations
        .iter()
        .all(|operation| matches!(operation, Operation::Flatten | Operation::Store))
}

fn active_tile(operations: &[Operation]) -> Option<usize> {
    let mut partitions = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        match operation {
            Operation::Tile { .. } => partitions.push(Some(index)),
            Operation::Lookup {
                membership: false, ..
            } => partitions.push(None),
            Operation::Flatten => {
                partitions.pop();
            }
            _ => {}
        }
    }
    partitions.last().copied().flatten()
}

fn fold_columns(monoid: &Monoid) -> Columns {
    match monoid {
        Monoid::Sum(column) => [symbol_name(column)].into_iter().collect(),
        Monoid::Count | Monoid::Or => Columns::new(),
    }
}

fn rust_columns(
    expression: &syn::Expr,
    heading: &[String],
    environment: Option<&Environment>,
) -> Columns {
    let names = environment
        .map(|environment| environment.variables.iter().map(symbol_name).collect())
        .unwrap_or_else(|| heading.iter().cloned().collect());
    rust::free_variables(expression).dependencies(&names)
}

fn operator_columns(operator: &Operator, analysis: &Analysis) -> Columns {
    let layouts = &analysis.structures.layouts;
    match operator {
        Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
            select.condition.columns()
        }
        Operator::Guard(guard) => rust_columns(
            &guard.condition,
            &layouts[&symbol_name(&guard.input)],
            guard.environment.as_ref(),
        ),
        Operator::Bind(bind) => {
            let mut columns = rust_columns(
                &bind.expression,
                &layouts[&symbol_name(&bind.input)],
                bind.environment.as_ref(),
            );
            columns.extend(
                bind.binding_types()
                    .iter()
                    .map(|(name, _)| symbol_name(name)),
            );
            columns
        }
        Operator::Group(group) => layouts[&symbol_name(&group.input)]
            .iter()
            .cloned()
            .collect(),
        _ => Columns::new(),
    }
}

/// Provider payload demand follows the checked Physical DAG, not the Sequence
/// schedule. Explicit storage keys, filtered payloads and canonical valuations
/// survive downstream projection, just as they do at tuple adapter boundaries.
fn carried_columns(program: &Program, analysis: &Analysis) -> BTreeMap<String, Columns> {
    let structures = &analysis.structures;
    let physical = &program.structures.physical.program;
    let mut used: BTreeMap<_, Columns> = structures
        .layouts
        .keys()
        .map(|name| (name.clone(), Columns::new()))
        .collect();
    let returned = symbol_name(&physical.returned);
    for expression in &physical.head.args {
        used.get_mut(&returned)
            .expect("checked returned layout")
            .extend(rust_columns(
                expression,
                &structures.layouts[&returned],
                None,
            ));
    }
    for definition in &physical.definitions {
        let name = symbol_name(&definition.name);
        let columns = used.get_mut(&name).expect("checked Physical layout");
        if definition.canon.is_some() {
            columns.extend(structures.layouts[&name].iter().cloned());
        }
        if let Some(builds) = structures.built.get(&name) {
            for (levels, _) in builds {
                columns.extend(levels.iter().flatten().cloned());
            }
        }
        if let Some(filtered) = structures.filtered.get(&name) {
            columns.extend(filtered.payload.columns.iter().map(symbol_name));
        }
        if structures.host_read(&name)
            && (structures.physical.key_reads.contains_key(&name)
                || structures.physical.merge_reads.contains_key(&name))
        {
            columns.extend(structures.layouts[&name].iter().cloned());
        }
    }
    for (name, keys) in structures.keysets.keys() {
        used.get_mut(name)
            .expect("checked keyset layout")
            .extend(keys.iter().cloned());
    }
    for definition in physical.definitions.iter().rev() {
        let output = used[&symbol_name(&definition.name)].clone();
        let mut demand = |input: &syn::Ident, extra: &Columns, carry: bool| {
            let input = symbol_name(input);
            let heading = &structures.layouts[&input];
            used.get_mut(&input).expect("checked input layout").extend(
                output
                    .iter()
                    .filter(|_| carry)
                    .chain(extra)
                    .filter(|column| heading.contains(column))
                    .cloned(),
            );
        };
        match &definition.operator {
            Operator::Probe(_, keyed)
            | Operator::Merge(_, keyed)
            | Operator::Semi(_, keyed)
            | Operator::Anti(_, keyed) => {
                let keys = keyed.columns.iter().map(symbol_name).collect();
                demand(&keyed.left, &keys, true);
                demand(
                    &keyed.right,
                    &keys,
                    matches!(
                        definition.operator,
                        Operator::Probe(_, _) | Operator::Merge(_, _)
                    ),
                );
            }
            Operator::Union(union) => {
                for operand in &union.operands {
                    demand(operand, &Columns::new(), true);
                }
            }
            operator => {
                if let Some(input) = operator.input() {
                    demand(input, &operator_columns(operator, analysis), true);
                }
            }
        }
    }
    used
}
