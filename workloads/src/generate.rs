//! Deterministic, shape-aware workload generation.
//!
//! `Scenario::Coverage` plants complete proofs and access-specific edge cases:
//! active raw duplicates, two proofs with one projected head, reachable missing
//! partial keys, multi-candidate buckets, full-membership omissions, and one
//! near miss per composite-key component. `EmptyInput` and `NoMatch` are
//! separate semantic distributions rather than misleading special rows hidden
//! in that positive dataset. `Scale` controls only volume.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::fmt;

use cq_lang_cq as cq;
use cq_syntax::symbol_name;

use crate::dataset::{Dataset, DatasetError, RelationData};
use crate::{GenerationConfig, QueryCase, Scale, Scenario, Suite};

const NOISE_DOMAIN_WIDTH: usize = 1 << 20;

/// Generate one catalog case from its exact CQ source.
pub fn generate(case: &QueryCase, config: GenerationConfig) -> Result<Dataset, GenerateError> {
    let module = case.module().map_err(GenerateError::InvalidQuery)?;
    generate_module(&module, case.suite, case.name, config)
}

/// Check whether a catalog case admits a scenario without materializing rows.
pub fn check_scenario(case: &QueryCase, scenario: Scenario) -> Result<(), GenerateError> {
    let module = case.module().map_err(GenerateError::InvalidQuery)?;
    check_module_scenario(&module, scenario)
}

/// Check whether a parsed CQ admits a scenario without materializing rows.
pub fn check_module_scenario(module: &cq::Module, scenario: Scenario) -> Result<(), GenerateError> {
    cq::contract::check(module).map_err(GenerateError::InvalidQuery)?;
    if !module.program.query.is_basic() {
        return Err(GenerateError::UnsupportedBodyItem);
    }
    let atoms = positive_atoms(&module.program.query)?;
    let input_positions = module
        .program
        .inputs()
        .enumerate()
        .map(|(position, input)| (symbol_name(&input.name), position))
        .collect::<BTreeMap<_, _>>();
    let memberships = access_patterns(&atoms)
        .into_iter()
        .filter(|access| access.key_columns.len() == atoms[access.occurrence].args.len())
        .map(|access| access.occurrence)
        .collect::<Vec<_>>();
    check_scenario_analysis(&atoms, &input_positions, &memberships, scenario)
}

/// Generate data for a parsed CQ. This is also useful for focused tests and tools.
pub fn generate_module(
    module: &cq::Module,
    suite: Suite,
    case_name: &str,
    config: GenerationConfig,
) -> Result<Dataset, GenerateError> {
    cq::contract::check(module).map_err(GenerateError::InvalidQuery)?;
    if !module.program.query.is_basic() {
        return Err(GenerateError::UnsupportedBodyItem);
    }
    let atoms = positive_atoms(&module.program.query)?;
    let components = connected_components(&atoms);
    let limits = Limits::for_shape(config.scale, components);

    let input_positions = module
        .program
        .inputs()
        .enumerate()
        .map(|(position, input)| (symbol_name(&input.name), position))
        .collect::<BTreeMap<_, _>>();
    let variables = variable_order(&atoms);
    let accesses = access_patterns(&atoms);
    let membership_occurrences = accesses
        .iter()
        .filter(|access| access.key_columns.len() == atoms[access.occurrence].args.len())
        .map(|access| access.occurrence)
        .collect::<Vec<_>>();
    check_scenario_analysis(
        &atoms,
        &input_positions,
        &membership_occurrences,
        config.scenario,
    )?;
    let mut rows = module
        .program
        .inputs()
        .map(|_| BTreeSet::<Vec<i32>>::new())
        .collect::<Vec<_>>();

    let mut rng = SplitMix64::new(config.seed);
    let existential_variables = existential_variables(module, &variables);
    let edge_marker_count = edge_marker_count(&accesses, &atoms, &existential_variables);
    let layout = PlantingLayout::new(
        &mut rng,
        limits,
        variables.len(),
        membership_occurrences.len(),
        edge_marker_count,
    )?;
    let mut active_duplicates = vec![None; module.program.inputs().count()];

    if config.scenario != Scenario::NoMatch {
        plant_coverage(
            module,
            &atoms,
            &accesses,
            &variables,
            &existential_variables,
            &membership_occurrences,
            &input_positions,
            limits,
            layout,
            &mut rows,
            &mut active_duplicates,
        )?;
    }

    fill_noise(module, limits, &mut rng, &mut rows)?;

    if config.scenario == Scenario::EmptyInput {
        let empty_input = deepest_first_used_input(&atoms, &input_positions)?;
        rows[empty_input].clear();
        active_duplicates[empty_input] = None;
    }

    let relations = materialize_relations(module, rows, active_duplicates, &mut rng)?;
    Dataset::new(module, suite, case_name, config, relations).map_err(GenerateError::Dataset)
}

fn check_scenario_analysis(
    atoms: &[&cq::Atom],
    input_positions: &BTreeMap<String, usize>,
    membership_occurrences: &[usize],
    scenario: Scenario,
) -> Result<(), GenerateError> {
    match scenario {
        Scenario::Coverage | Scenario::EmptyInput => {
            check_distinguishable_memberships(atoms, membership_occurrences)
        }
        Scenario::NoMatch if disjoint_noise_forces_no_match(atoms, input_positions)? => Ok(()),
        Scenario::NoMatch => Err(GenerateError::ScenarioNotApplicable {
            scenario,
            reason: "nonempty disjoint-column inputs cannot make this query empty",
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn plant_coverage(
    module: &cq::Module,
    atoms: &[&cq::Atom],
    accesses: &[AccessPattern],
    variables: &[String],
    existential_variables: &[String],
    membership_occurrences: &[usize],
    input_positions: &BTreeMap<String, usize>,
    limits: Limits,
    layout: PlantingLayout,
    rows: &mut [BTreeSet<Vec<i32>>],
    active_duplicates: &mut [Option<Vec<i32>>],
) -> Result<(), GenerateError> {
    // The first complete proof supplies a participating duplicate for every
    // physical input used by the query.
    let primary = make_assignment(variables, layout.complete_base, layout.complete_stride, 0)?;
    let projected = project_assignment(
        atoms,
        input_positions,
        module.program.inputs().count(),
        &primary,
        None,
    )?;
    for (relation, additions) in projected.iter().enumerate() {
        active_duplicates[relation] = additions.iter().next().cloned();
    }
    insert_projected_rows(rows, projected, limits.distinct_rows)?;

    // A full-key lookup is otherwise observationally redundant on data made
    // only from complete witnesses. Give every such occurrence its own
    // assignment, insert every other occurrence, and deliberately omit the
    // target row. Variable-separated positive domains make role-playing
    // self-joins safe: another occurrence can insert the omitted row only when
    // it is the exact same symbolic atom, which is rejected above.
    for (anti_witness, &omitted_occurrence) in membership_occurrences.iter().enumerate() {
        let assignment = make_assignment(
            variables,
            layout.anti_base,
            layout.anti_stride,
            anti_witness,
        )?;
        let projected = project_assignment(
            atoms,
            input_positions,
            module.program.inputs().count(),
            &assignment,
            Some(omitted_occurrence),
        )?;
        let omitted_atom = atoms[omitted_occurrence];
        let omitted_relation = relation_position(omitted_atom, input_positions)?;
        let omitted_row = project_atom(omitted_atom, &assignment)?;
        if projected[omitted_relation].contains(&omitted_row)
            || rows[omitted_relation].contains(&omitted_row)
        {
            return Err(GenerateError::IndistinguishableMembership {
                occurrence: omitted_occurrence + 1,
                relation: symbol_name(&omitted_atom.relation),
            });
        }
        insert_projected_rows(rows, projected, limits.distinct_rows)?;
    }

    let mut edge_marker = 0;
    let mut fresh_value = 0;

    // Reach every feasible partial lookup with a key for which its target
    // relation has no candidate bucket. Some self-joins structurally guarantee
    // a hit because the prefix itself already inserted that key; those cases
    // are covered by the fanout marker below instead.
    for access in accesses.iter().filter(|access| access.is_partial(atoms)) {
        let assignment =
            make_assignment(variables, layout.edge_base, layout.edge_stride, edge_marker)?;
        edge_marker += 1;
        let prefix = project_prefix(
            atoms,
            input_positions,
            module.program.inputs().count(),
            &assignment,
            access.occurrence,
        )?;
        if !prefix_guarantees_key(access, atoms, input_positions, &assignment)? {
            insert_projected_rows(rows, prefix, limits.distinct_rows)?;
        }
    }

    // Every component of a composite key gets its own isolated near miss. All
    // other atoms are present, while the target row differs in exactly that
    // component, so a lowering that drops the component admits a false proof.
    for access in accesses
        .iter()
        .filter(|access| access.key_columns.len() >= 2)
    {
        for &key_column in &access.key_columns {
            let assignment =
                make_assignment(variables, layout.edge_base, layout.edge_stride, edge_marker)?;
            edge_marker += 1;
            let mut projected = project_assignment(
                atoms,
                input_positions,
                module.program.inputs().count(),
                &assignment,
                Some(access.occurrence),
            )?;
            let target = atoms[access.occurrence];
            let relation = relation_position(target, input_positions)?;
            let correct = project_atom(target, &assignment)?;
            if projected[relation].contains(&correct) {
                return Err(GenerateError::IndistinguishableAccess {
                    occurrence: access.occurrence + 1,
                    relation: symbol_name(&target.relation),
                });
            }
            let mut decoy = correct;
            decoy[key_column] = layout.fresh_value(fresh_value)?;
            fresh_value += 1;
            projected[relation].insert(decoy);
            insert_projected_rows(rows, projected, limits.distinct_rows)?;
        }
    }

    // A proper-key index must enumerate more than one complement. The two
    // assignments agree on the entire prefix and differ only in variables that
    // are not yet bound, so both candidates survive to complete proofs.
    for access in accesses.iter().filter(|access| access.is_partial(atoms)) {
        let first = make_assignment(variables, layout.edge_base, layout.edge_stride, edge_marker)?;
        edge_marker += 1;
        let mut second = first.clone();
        for variable in variables {
            if !access.bound_variables.contains(variable) {
                second.insert(variable.clone(), layout.fresh_value(fresh_value)?);
                fresh_value += 1;
            }
        }
        let first_rows = project_assignment(
            atoms,
            input_positions,
            module.program.inputs().count(),
            &first,
            None,
        )?;
        let second_rows = project_assignment(
            atoms,
            input_positions,
            module.program.inputs().count(),
            &second,
            None,
        )?;
        let combined = merge_projected(first_rows, second_rows);
        insert_projected_rows(rows, combined, limits.distinct_rows)?;
    }

    // If the head projects variables away, make two distinct complete proofs
    // agree on the head. This makes final set semantics observable.
    if !existential_variables.is_empty() {
        let first = make_assignment(variables, layout.edge_base, layout.edge_stride, edge_marker)?;
        edge_marker += 1;
        let mut second = first.clone();
        for variable in existential_variables {
            second.insert(variable.clone(), layout.fresh_value(fresh_value)?);
            fresh_value += 1;
        }
        let first_rows = project_assignment(
            atoms,
            input_positions,
            module.program.inputs().count(),
            &first,
            None,
        )?;
        let second_rows = project_assignment(
            atoms,
            input_positions,
            module.program.inputs().count(),
            &second,
            None,
        )?;
        insert_projected_rows(
            rows,
            merge_projected(first_rows, second_rows),
            limits.distinct_rows,
        )?;
    }

    debug_assert_eq!(edge_marker, layout.edge_marker_count);

    // Fill the remaining positive-row budget with complete witnesses. A query
    // with several occurrences of one physical relation (notably D72's three
    // DateDim roles) may need one fewer tiny-scale witness so all anti-witnesses
    // fit. Noise below still brings every relation to the exact scale target.
    for witness in 1..limits.planted_witnesses {
        let assignment = make_assignment(
            variables,
            layout.complete_base,
            layout.complete_stride,
            witness,
        )?;
        let projected = project_assignment(
            atoms,
            input_positions,
            module.program.inputs().count(),
            &assignment,
            None,
        )?;
        if !projected_rows_fit(rows, &projected, limits.distinct_rows) {
            break;
        }
        extend_projected_rows(rows, projected);
    }

    Ok(())
}

fn fill_noise(
    module: &cq::Module,
    limits: Limits,
    rng: &mut SplitMix64,
    rows: &mut [BTreeSet<Vec<i32>>],
) -> Result<(), GenerateError> {
    let total_columns = module
        .program
        .inputs()
        .map(|input| input.columns.len())
        .sum::<usize>();
    ensure_noise_space(total_columns)?;
    let mut next_namespace = 0;
    for (relation_index, input) in module.program.inputs().enumerate() {
        let arity = input.columns.len();
        let parameters = (0..arity)
            .map(|_| {
                let namespace = next_namespace;
                next_namespace += 1;
                NoiseColumn::new(namespace, rng)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let noise_needed = limits
            .distinct_rows
            .saturating_sub(rows[relation_index].len());
        for row_number in 0..noise_needed {
            rows[relation_index].insert(
                parameters
                    .iter()
                    .map(|column| column.value(row_number))
                    .collect(),
            );
        }
    }
    Ok(())
}

fn materialize_relations(
    module: &cq::Module,
    rows: Vec<BTreeSet<Vec<i32>>>,
    active_duplicates: Vec<Option<Vec<i32>>>,
    rng: &mut SplitMix64,
) -> Result<Vec<RelationData>, GenerateError> {
    let mut relations = Vec::with_capacity(module.program.inputs().count());
    for ((input, distinct_rows), active_duplicate) in
        module.program.inputs().zip(rows).zip(active_duplicates)
    {
        let mut relation_rows = distinct_rows.into_iter().collect::<Vec<_>>();
        let duplicate_count = relation_rows.len() / 20;
        let mut duplicates = active_duplicate.into_iter().collect::<Vec<_>>();
        let remaining = duplicate_count.saturating_sub(duplicates.len());
        let additional = relation_rows
            .iter()
            .filter(|row| !duplicates.contains(row))
            .take(remaining)
            .cloned()
            .collect::<Vec<_>>();
        duplicates.extend(additional);
        relation_rows.extend(duplicates);
        shuffle(&mut relation_rows, rng);
        relations.push(RelationData::new(
            symbol_name(&input.name),
            input.columns.len(),
            relation_rows,
        )?);
    }
    Ok(relations)
}

#[derive(Clone, Copy, Debug)]
struct PlantingLayout {
    complete_base: i64,
    complete_stride: i64,
    anti_base: i64,
    anti_stride: i64,
    edge_base: i64,
    edge_stride: i64,
    fresh_base: i64,
    edge_marker_count: usize,
}

impl PlantingLayout {
    fn new(
        rng: &mut SplitMix64,
        limits: Limits,
        variable_count: usize,
        membership_count: usize,
        edge_marker_count: usize,
    ) -> Result<Self, GenerateError> {
        let complete_base = 10_000_000_i64 + (rng.next_u64() % 10_000_000) as i64;
        let complete_stride = limits.planted_witnesses as i64 + 1;
        let anti_stride = membership_count as i64 + 1;
        let anti_base = complete_base
            .checked_add(
                (variable_count as i64)
                    .checked_mul(complete_stride)
                    .ok_or(GenerateError::ValueSpaceExhausted)?,
            )
            .and_then(|value| value.checked_add(1))
            .ok_or(GenerateError::ValueSpaceExhausted)?;
        let edge_stride = edge_marker_count as i64 + 1;
        let edge_base = anti_base
            .checked_add(
                (variable_count as i64)
                    .checked_mul(anti_stride)
                    .ok_or(GenerateError::ValueSpaceExhausted)?,
            )
            .and_then(|value| value.checked_add(1))
            .ok_or(GenerateError::ValueSpaceExhausted)?;
        let fresh_base = edge_base
            .checked_add(
                (variable_count as i64)
                    .checked_mul(edge_stride)
                    .ok_or(GenerateError::ValueSpaceExhausted)?,
            )
            .and_then(|value| value.checked_add(1))
            .ok_or(GenerateError::ValueSpaceExhausted)?;
        Ok(Self {
            complete_base,
            complete_stride,
            anti_base,
            anti_stride,
            edge_base,
            edge_stride,
            fresh_base,
            edge_marker_count,
        })
    }

    fn fresh_value(self, offset: usize) -> Result<i32, GenerateError> {
        self.fresh_base
            .checked_add(offset as i64)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(GenerateError::ValueSpaceExhausted)
    }
}

#[derive(Clone, Debug)]
struct AccessPattern {
    occurrence: usize,
    key_columns: Vec<usize>,
    bound_variables: BTreeSet<String>,
}

impl AccessPattern {
    fn is_partial(&self, atoms: &[&cq::Atom]) -> bool {
        !self.key_columns.is_empty() && self.key_columns.len() < atoms[self.occurrence].args.len()
    }
}

fn access_patterns(atoms: &[&cq::Atom]) -> Vec<AccessPattern> {
    let mut bound_variables = BTreeSet::new();
    atoms
        .iter()
        .enumerate()
        .map(|(occurrence, atom)| {
            let key_columns = atom
                .variables()
                .enumerate()
                .filter_map(|(column, variable)| {
                    bound_variables
                        .contains(&symbol_name(variable))
                        .then_some(column)
                })
                .collect();
            let access = AccessPattern {
                occurrence,
                key_columns,
                bound_variables: bound_variables.clone(),
            };
            bound_variables.extend(atom.variables().map(symbol_name));
            access
        })
        .collect()
}

fn existential_variables(module: &cq::Module, variables: &[String]) -> Vec<String> {
    let head = module
        .program
        .query
        .head
        .variables()
        .map(symbol_name)
        .collect::<BTreeSet<_>>();
    variables
        .iter()
        .filter(|variable| !head.contains(*variable))
        .cloned()
        .collect()
}

fn edge_marker_count(
    accesses: &[AccessPattern],
    atoms: &[&cq::Atom],
    existential_variables: &[String],
) -> usize {
    accesses
        .iter()
        .filter(|access| access.is_partial(atoms))
        .count()
        * 2
        + accesses
            .iter()
            .filter(|access| access.key_columns.len() >= 2)
            .map(|access| access.key_columns.len())
            .sum::<usize>()
        + usize::from(!existential_variables.is_empty())
}

#[cfg(test)]
fn full_bound_occurrences(atoms: &[&cq::Atom]) -> Vec<usize> {
    let mut bound = BTreeSet::new();
    let mut occurrences = Vec::new();
    for (occurrence, atom) in atoms.iter().enumerate() {
        if atom
            .variables()
            .all(|variable| bound.contains(&symbol_name(variable)))
        {
            occurrences.push(occurrence);
        }
        bound.extend(atom.variables().map(symbol_name));
    }
    occurrences
}

fn check_distinguishable_memberships(
    atoms: &[&cq::Atom],
    memberships: &[usize],
) -> Result<(), GenerateError> {
    for &membership in memberships {
        let target = atoms[membership];
        let relation = symbol_name(&target.relation);
        let variables = target.variables().map(symbol_name).collect::<Vec<_>>();
        if atoms.iter().enumerate().any(|(occurrence, atom)| {
            occurrence != membership
                && symbol_name(&atom.relation) == relation
                && atom
                    .variables()
                    .map(symbol_name)
                    .eq(variables.iter().cloned())
        }) {
            return Err(GenerateError::IndistinguishableMembership {
                occurrence: membership + 1,
                relation,
            });
        }
    }
    Ok(())
}

fn make_assignment(
    variables: &[String],
    base: i64,
    stride: i64,
    assignment: usize,
) -> Result<BTreeMap<String, i32>, GenerateError> {
    variables
        .iter()
        .enumerate()
        .map(|(variable, name)| {
            let value = (variable as i64)
                .checked_mul(stride)
                .and_then(|offset| offset.checked_add(assignment as i64))
                .and_then(|offset| base.checked_add(offset))
                .ok_or(GenerateError::ValueSpaceExhausted)?;
            let value = i32::try_from(value).map_err(|_| GenerateError::ValueSpaceExhausted)?;
            Ok((name.clone(), value))
        })
        .collect()
}

fn project_assignment(
    atoms: &[&cq::Atom],
    input_positions: &BTreeMap<String, usize>,
    relation_count: usize,
    assignment: &BTreeMap<String, i32>,
    omitted_occurrence: Option<usize>,
) -> Result<Vec<BTreeSet<Vec<i32>>>, GenerateError> {
    let mut projected = (0..relation_count)
        .map(|_| BTreeSet::new())
        .collect::<Vec<_>>();
    for (occurrence, atom) in atoms.iter().enumerate() {
        if Some(occurrence) == omitted_occurrence {
            continue;
        }
        let relation = relation_position(atom, input_positions)?;
        projected[relation].insert(project_atom(atom, assignment)?);
    }
    Ok(projected)
}

fn project_prefix(
    atoms: &[&cq::Atom],
    input_positions: &BTreeMap<String, usize>,
    relation_count: usize,
    assignment: &BTreeMap<String, i32>,
    end: usize,
) -> Result<Vec<BTreeSet<Vec<i32>>>, GenerateError> {
    let mut projected = (0..relation_count)
        .map(|_| BTreeSet::new())
        .collect::<Vec<_>>();
    for atom in &atoms[..end] {
        let relation = relation_position(atom, input_positions)?;
        projected[relation].insert(project_atom(atom, assignment)?);
    }
    Ok(projected)
}

fn merge_projected(
    mut left: Vec<BTreeSet<Vec<i32>>>,
    right: Vec<BTreeSet<Vec<i32>>>,
) -> Vec<BTreeSet<Vec<i32>>> {
    for (left, right) in left.iter_mut().zip(right) {
        left.extend(right);
    }
    left
}

fn project_columns(row: &[i32], columns: &[usize]) -> Vec<i32> {
    columns.iter().map(|&column| row[column]).collect()
}

fn prefix_guarantees_key(
    access: &AccessPattern,
    atoms: &[&cq::Atom],
    input_positions: &BTreeMap<String, usize>,
    assignment: &BTreeMap<String, i32>,
) -> Result<bool, GenerateError> {
    let prefix = project_prefix(
        atoms,
        input_positions,
        input_positions.len(),
        assignment,
        access.occurrence,
    )?;
    let target = atoms[access.occurrence];
    let relation = relation_position(target, input_positions)?;
    let key = access
        .key_columns
        .iter()
        .map(|&column| {
            assignment[&symbol_name(
                cq::rust::variable(&target.args[column]).expect("variable-only workload"),
            )]
        })
        .collect::<Vec<_>>();
    Ok(prefix[relation]
        .iter()
        .any(|row| project_columns(row, &access.key_columns) == key))
}

fn deepest_first_used_input(
    atoms: &[&cq::Atom],
    input_positions: &BTreeMap<String, usize>,
) -> Result<usize, GenerateError> {
    let mut deepest = None;
    for (occurrence, atom) in atoms.iter().enumerate() {
        let current = (occurrence, relation_position(atom, input_positions)?);
        if deepest.is_none_or(|previous: (usize, usize)| previous.0 < occurrence) {
            deepest = Some(current);
        }
    }
    deepest
        .map(|(_, relation)| relation)
        .ok_or(GenerateError::ValueSpaceExhausted)
}

/// Disjoint `(physical relation, column)` domains make the query empty when at
/// least one repeated variable crosses two distinct domains. If no variable
/// does so, one nonempty row per input already satisfies the query and a
/// nonempty no-match scenario is mathematically unavailable to this schema.
fn disjoint_noise_forces_no_match(
    atoms: &[&cq::Atom],
    input_positions: &BTreeMap<String, usize>,
) -> Result<bool, GenerateError> {
    let mut domains = BTreeMap::<String, (usize, usize)>::new();
    for atom in atoms {
        let relation = relation_position(atom, input_positions)?;
        for (column, variable) in atom.variables().enumerate() {
            let variable = symbol_name(variable);
            if domains
                .insert(variable.clone(), (relation, column))
                .is_some_and(|previous| previous != (relation, column))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn relation_position(
    atom: &cq::Atom,
    input_positions: &BTreeMap<String, usize>,
) -> Result<usize, GenerateError> {
    let relation_name = symbol_name(&atom.relation);
    input_positions
        .get(&relation_name)
        .copied()
        .ok_or(GenerateError::UnknownRelation(relation_name))
}

fn project_atom(
    atom: &cq::Atom,
    assignment: &BTreeMap<String, i32>,
) -> Result<Vec<i32>, GenerateError> {
    atom.variables()
        .map(|variable| {
            assignment
                .get(&symbol_name(variable))
                .copied()
                .ok_or(GenerateError::ValueSpaceExhausted)
        })
        .collect()
}

fn projected_rows_fit(
    rows: &[BTreeSet<Vec<i32>>],
    projected: &[BTreeSet<Vec<i32>>],
    distinct_rows: usize,
) -> bool {
    rows.iter().zip(projected).all(|(existing, additions)| {
        existing.len() + additions.difference(existing).count() <= distinct_rows
    })
}

fn extend_projected_rows(rows: &mut [BTreeSet<Vec<i32>>], projected: Vec<BTreeSet<Vec<i32>>>) {
    for (existing, additions) in rows.iter_mut().zip(projected) {
        existing.extend(additions);
    }
}

fn insert_projected_rows(
    rows: &mut [BTreeSet<Vec<i32>>],
    projected: Vec<BTreeSet<Vec<i32>>>,
    distinct_rows: usize,
) -> Result<(), GenerateError> {
    if !projected_rows_fit(rows, &projected, distinct_rows) {
        return Err(GenerateError::NearMissesExceedScale { distinct_rows });
    }
    extend_projected_rows(rows, projected);
    Ok(())
}

fn positive_atoms(query: &cq::Query) -> Result<Vec<&cq::Atom>, GenerateError> {
    query
        .body
        .iter()
        .map(|item| {
            #[allow(unreachable_patterns)]
            match item {
                cq::BodyItem::Positive { atom } => Ok(atom),
                _ => Err(GenerateError::UnsupportedBodyItem),
            }
        })
        .collect()
}

fn variable_order(atoms: &[&cq::Atom]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for atom in atoms {
        for variable in atom.variables() {
            let name = symbol_name(variable);
            if seen.insert(name.clone()) {
                order.push(name);
            }
        }
    }
    order
}

fn connected_components(atoms: &[&cq::Atom]) -> usize {
    let mut parents = (0..atoms.len()).collect::<Vec<_>>();
    let variable_sets = atoms
        .iter()
        .map(|atom| atom.variables().map(symbol_name).collect::<BTreeSet<_>>())
        .collect::<Vec<_>>();
    for left in 0..atoms.len() {
        for right in left + 1..atoms.len() {
            if !variable_sets[left].is_disjoint(&variable_sets[right]) {
                union(&mut parents, left, right);
            }
        }
    }
    (0..atoms.len())
        .map(|atom| find(&mut parents, atom))
        .collect::<BTreeSet<_>>()
        .len()
}

fn find(parents: &mut [usize], item: usize) -> usize {
    if parents[item] != item {
        parents[item] = find(parents, parents[item]);
    }
    parents[item]
}

fn union(parents: &mut [usize], left: usize, right: usize) {
    let left = find(parents, left);
    let right = find(parents, right);
    if left != right {
        parents[right] = left;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Limits {
    distinct_rows: usize,
    planted_witnesses: usize,
}

impl Limits {
    fn for_shape(scale: Scale, components: usize) -> Self {
        let (ordinary_rows, ordinary_witnesses, cartesian_cap) = match scale {
            Scale::Tiny => (96, 12, 4_096),
            Scale::Medium => (25_000, 512, 100_000),
            Scale::Large => (1_000_000, 4_096, 1_000_000),
        };
        let distinct_rows = if components > 1 {
            ordinary_rows.min(integer_nth_root(cartesian_cap, components).max(2))
        } else {
            ordinary_rows
        };
        Self {
            distinct_rows,
            planted_witnesses: ordinary_witnesses.min(distinct_rows),
        }
    }
}

fn integer_nth_root(value: usize, exponent: usize) -> usize {
    debug_assert!(exponent > 0);
    let mut low = 1;
    let mut high = value.max(1);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if power_at_most(middle, exponent, value) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    low
}

fn power_at_most(base: usize, exponent: usize, limit: usize) -> bool {
    let mut product = 1_usize;
    for _ in 0..exponent {
        let Some(next) = product.checked_mul(base) else {
            return false;
        };
        if next > limit {
            return false;
        }
        product = next;
    }
    true
}

fn ensure_noise_space(columns: usize) -> Result<(), GenerateError> {
    let upper = i32::MIN as i64 + (columns as i64) * NOISE_DOMAIN_WIDTH as i64;
    if upper >= 0 {
        return Err(GenerateError::TooManyColumns(columns));
    }
    Ok(())
}

struct NoiseColumn {
    base: i64,
    multiplier: u64,
    offset: u64,
}

impl NoiseColumn {
    fn new(namespace: usize, rng: &mut SplitMix64) -> Result<Self, GenerateError> {
        let base = i32::MIN as i64 + (namespace as i64) * NOISE_DOMAIN_WIDTH as i64;
        let end = base + NOISE_DOMAIN_WIDTH as i64 - 1;
        if end >= 0 {
            return Err(GenerateError::TooManyColumns(namespace + 1));
        }
        Ok(Self {
            base,
            multiplier: (rng.next_u64() | 1) & (NOISE_DOMAIN_WIDTH as u64 - 1),
            offset: rng.next_u64() & (NOISE_DOMAIN_WIDTH as u64 - 1),
        })
    }

    fn value(&self, row: usize) -> i32 {
        let permuted = ((row as u64)
            .wrapping_mul(self.multiplier)
            .wrapping_add(self.offset))
            & (NOISE_DOMAIN_WIDTH as u64 - 1);
        i32::try_from(self.base + permuted as i64)
            .expect("a checked noise namespace always remains in i32")
    }
}

fn shuffle<T>(values: &mut [T], rng: &mut SplitMix64) {
    for upper in (1..values.len()).rev() {
        let other = (rng.next_u64() % (upper as u64 + 1)) as usize;
        values.swap(upper, other);
    }
}

/// The fixed SplitMix64 algorithm used by every generated scale.
#[derive(Clone, Copy, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

#[derive(Debug)]
pub enum GenerateError {
    InvalidQuery(syn::Error),
    UnsupportedBodyItem,
    UnknownRelation(String),
    IndistinguishableMembership {
        occurrence: usize,
        relation: String,
    },
    IndistinguishableAccess {
        occurrence: usize,
        relation: String,
    },
    ScenarioNotApplicable {
        scenario: Scenario,
        reason: &'static str,
    },
    NearMissesExceedScale {
        distinct_rows: usize,
    },
    TooManyColumns(usize),
    ValueSpaceExhausted,
    Dataset(DatasetError),
}

impl fmt::Display for GenerateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuery(error) => write!(formatter, "invalid workload query: {error}"),
            Self::UnsupportedBodyItem => formatter
                .write_str("the baseline generator accepts only positive conjunctive queries"),
            Self::UnknownRelation(relation) => {
                write!(formatter, "body uses undeclared relation `{relation}`")
            }
            Self::IndistinguishableMembership {
                occurrence,
                relation,
            } => write!(
                formatter,
                "body occurrence {occurrence} of `{relation}` is an exact duplicate, so no dataset can make deleting that membership observable"
            ),
            Self::IndistinguishableAccess {
                occurrence,
                relation,
            } => write!(
                formatter,
                "body occurrence {occurrence} of `{relation}` is duplicated by another occurrence, so an isolated access decoy cannot be planted"
            ),
            Self::ScenarioNotApplicable { scenario, reason } => {
                write!(
                    formatter,
                    "scenario `{}` is not applicable: {reason}",
                    scenario.as_str()
                )
            }
            Self::NearMissesExceedScale { distinct_rows } => write!(
                formatter,
                "the required access-coverage rows exceed the scale target of {distinct_rows} distinct rows per relation"
            ),
            Self::TooManyColumns(columns) => write!(
                formatter,
                "{columns} declared columns exceed the generator's disjoint i32 noise domains"
            ),
            Self::ValueSpaceExhausted => {
                formatter.write_str("generated planted values exceed the i32 row ABI")
            }
            Self::Dataset(error) => error.fmt(formatter),
        }
    }
}

impl StdError for GenerateError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::InvalidQuery(error) => Some(error),
            Self::Dataset(error) => Some(error),
            Self::UnsupportedBodyItem
            | Self::UnknownRelation(_)
            | Self::IndistinguishableMembership { .. }
            | Self::IndistinguishableAccess { .. }
            | Self::ScenarioNotApplicable { .. }
            | Self::NearMissesExceedScale { .. }
            | Self::TooManyColumns(_)
            | Self::ValueSpaceExhausted => None,
        }
    }
}

impl From<DatasetError> for GenerateError {
    fn from(error: DatasetError) -> Self {
        Self::Dataset(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CLASSIC, RETAIL};

    type Assignment = BTreeMap<String, i32>;

    fn config(scenario: Scenario, scale: Scale, seed: u64) -> GenerationConfig {
        GenerationConfig {
            scenario,
            scale,
            seed,
        }
    }

    #[derive(Debug)]
    struct Evaluation {
        assignments: BTreeSet<Assignment>,
        output: BTreeSet<Vec<i32>>,
    }

    fn evaluate(
        module: &cq::Module,
        dataset: &Dataset,
        omitted_occurrence: Option<usize>,
    ) -> Evaluation {
        evaluate_with_fault(module, dataset, omitted_occurrence, None)
    }

    fn evaluate_with_fault(
        module: &cq::Module,
        dataset: &Dataset,
        omitted_occurrence: Option<usize>,
        ignored_equality: Option<(usize, usize)>,
    ) -> Evaluation {
        let atoms = positive_atoms(&module.program.query).unwrap();
        let relations = dataset
            .relations
            .iter()
            .map(|relation| {
                (
                    relation.name.clone(),
                    relation.rows.iter().cloned().collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut assignments = BTreeSet::new();
        evaluate_occurrence(
            &atoms,
            &relations,
            omitted_occurrence,
            ignored_equality,
            0,
            &Assignment::new(),
            &mut assignments,
        );
        let output = assignments
            .iter()
            .map(|assignment| {
                module
                    .program
                    .query
                    .head
                    .variables()
                    .map(|variable| assignment[&symbol_name(variable)])
                    .collect::<Vec<_>>()
            })
            .collect();
        Evaluation {
            assignments,
            output,
        }
    }

    fn evaluate_occurrence(
        atoms: &[&cq::Atom],
        relations: &BTreeMap<String, BTreeSet<Vec<i32>>>,
        omitted_occurrence: Option<usize>,
        ignored_equality: Option<(usize, usize)>,
        occurrence: usize,
        assignment: &Assignment,
        results: &mut BTreeSet<Assignment>,
    ) {
        if occurrence == atoms.len() {
            results.insert(assignment.clone());
            return;
        }
        if Some(occurrence) == omitted_occurrence {
            evaluate_occurrence(
                atoms,
                relations,
                omitted_occurrence,
                ignored_equality,
                occurrence + 1,
                assignment,
                results,
            );
            return;
        }

        let atom = atoms[occurrence];
        for row in &relations[&symbol_name(&atom.relation)] {
            let mut extended = assignment.clone();
            let compatible =
                atom.variables()
                    .zip(row)
                    .enumerate()
                    .all(|(column, (variable, value))| {
                        let variable = symbol_name(variable);
                        match extended.get(&variable) {
                            Some(bound) => {
                                ignored_equality == Some((occurrence, column)) || bound == value
                            }
                            None => {
                                extended.insert(variable, *value);
                                true
                            }
                        }
                    });
            if compatible {
                evaluate_occurrence(
                    atoms,
                    relations,
                    omitted_occurrence,
                    ignored_equality,
                    occurrence + 1,
                    &extended,
                    results,
                );
            }
        }
    }

    fn candidate_counts(module: &cq::Module, dataset: &Dataset) -> Vec<BTreeSet<usize>> {
        let atoms = positive_atoms(&module.program.query).unwrap();
        let relations = dataset
            .relations
            .iter()
            .map(|relation| {
                (
                    relation.name.clone(),
                    relation.rows.iter().cloned().collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut counts = vec![BTreeSet::new(); atoms.len()];
        trace_occurrence(&atoms, &relations, 0, &Assignment::new(), &mut counts);
        counts
    }

    fn trace_occurrence(
        atoms: &[&cq::Atom],
        relations: &BTreeMap<String, BTreeSet<Vec<i32>>>,
        occurrence: usize,
        assignment: &Assignment,
        counts: &mut [BTreeSet<usize>],
    ) {
        if occurrence == atoms.len() {
            return;
        }
        let atom = atoms[occurrence];
        let candidates = relations[&symbol_name(&atom.relation)]
            .iter()
            .filter(|row| {
                atom.variables().zip(*row).all(|(variable, value)| {
                    assignment
                        .get(&symbol_name(variable))
                        .is_none_or(|bound| bound == value)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        counts[occurrence].insert(candidates.len());
        for row in candidates {
            let mut extended = assignment.clone();
            for (variable, value) in atom.variables().zip(row) {
                extended.entry(symbol_name(variable)).or_insert(value);
            }
            trace_occurrence(atoms, relations, occurrence + 1, &extended, counts);
        }
    }

    fn successful_rows_by_key(
        module: &cq::Module,
        evaluation: &Evaluation,
        access: &AccessPattern,
    ) -> BTreeMap<Vec<i32>, BTreeSet<Vec<i32>>> {
        let atoms = positive_atoms(&module.program.query).unwrap();
        let atom = atoms[access.occurrence];
        let mut by_key = BTreeMap::<Vec<i32>, BTreeSet<Vec<i32>>>::new();
        for assignment in &evaluation.assignments {
            let row = project_atom(atom, assignment).unwrap();
            by_key
                .entry(project_columns(&row, &access.key_columns))
                .or_default()
                .insert(row);
        }
        by_key
    }

    fn assert_memberships_are_observable(label: &str, module: &cq::Module, dataset: &Dataset) {
        let atoms = positive_atoms(&module.program.query).unwrap();
        let memberships = full_bound_occurrences(&atoms);
        let variables = variable_order(&atoms);
        let accesses = access_patterns(&atoms);
        let existentials = existential_variables(module, &variables);
        let limits = Limits::for_shape(dataset.manifest.scale, connected_components(&atoms));
        let mut rng = SplitMix64::new(dataset.manifest.seed);
        let layout = PlantingLayout::new(
            &mut rng,
            limits,
            variables.len(),
            memberships.len(),
            edge_marker_count(&accesses, &atoms, &existentials),
        )
        .unwrap();
        let full = evaluate(module, dataset, None);

        for (anti_witness, &membership) in memberships.iter().enumerate() {
            let without = evaluate(module, dataset, Some(membership));
            let anti_assignment = make_assignment(
                &variables,
                layout.anti_base,
                layout.anti_stride,
                anti_witness,
            )
            .unwrap();
            let anti_output = module
                .program
                .query
                .head
                .variables()
                .map(|variable| anti_assignment[&symbol_name(variable)])
                .collect::<Vec<_>>();

            assert!(
                without.assignments.contains(&anti_assignment),
                "{label} occurrence {} did not admit its planted leave-one-out assignment",
                membership + 1
            );
            assert!(
                !full.assignments.contains(&anti_assignment),
                "{label} occurrence {} unexpectedly admitted its leave-one-out assignment",
                membership + 1
            );
            assert!(
                without.output.contains(&anti_output) && !full.output.contains(&anti_output),
                "{label} occurrence {} does not make its planted head tuple observable",
                membership + 1
            );

            for (occurrence, atom) in atoms.iter().enumerate() {
                let row = project_atom(atom, &anti_assignment).unwrap();
                let present = dataset
                    .relation(&symbol_name(&atom.relation))
                    .unwrap()
                    .rows
                    .contains(&row);
                assert_eq!(
                    present,
                    occurrence != membership,
                    "{label} anti-witness for occurrence {} has the wrong presence for occurrence {}",
                    membership + 1,
                    occurrence + 1,
                );
            }
        }
    }

    #[test]
    fn splitmix64_sequence_is_fixed() {
        let mut random = SplitMix64::new(0);
        assert_eq!(random.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(random.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(random.next_u64(), 0x06c4_5d18_8009_454f);
    }

    #[test]
    fn every_classic_tiny_dataset_is_reproducible_and_schema_valid() {
        for case in CLASSIC {
            let first = generate(case, config(Scenario::Coverage, Scale::Tiny, 7)).unwrap();
            let second = generate(case, config(Scenario::Coverage, Scale::Tiny, 7)).unwrap();
            let module = case.module().unwrap();

            assert_eq!(first, second, "{} was not reproducible", case.name);
            first.validate(&module).unwrap();
            assert!(
                first
                    .manifest
                    .relations
                    .iter()
                    .all(|relation| relation.distinct_rows
                        == Limits::for_shape(
                            Scale::Tiny,
                            connected_components(&positive_atoms(&module.program.query).unwrap())
                        )
                        .distinct_rows),
                "{} did not preserve the exact distinct-row scale target",
                case.name,
            );
        }
    }

    #[test]
    fn every_catalog_membership_has_a_result_changing_anti_witness() {
        for case in CLASSIC.iter().chain(RETAIL) {
            let module = case.module().unwrap();
            let dataset = generate(case, config(Scenario::Coverage, Scale::Tiny, 7)).unwrap();
            let target = Limits::for_shape(
                Scale::Tiny,
                connected_components(&positive_atoms(&module.program.query).unwrap()),
            )
            .distinct_rows;
            assert!(
                dataset
                    .manifest
                    .relations
                    .iter()
                    .all(|relation| relation.distinct_rows == target),
                "{} did not preserve the exact distinct-row target",
                case.name,
            );
            assert_memberships_are_observable(case.name, &module, &dataset);
        }
    }

    #[test]
    fn coverage_scenario_makes_every_advertised_edge_observable() {
        for case in CLASSIC.iter().chain(RETAIL) {
            let module = case.module().unwrap();
            let atoms = positive_atoms(&module.program.query).unwrap();
            let accesses = access_patterns(&atoms);
            let variables = variable_order(&atoms);
            let dataset = generate(case, config(Scenario::Coverage, Scale::Tiny, 31))
                .unwrap_or_else(|error| {
                    panic!("{} coverage generation failed: {error}", case.name)
                });
            let evaluation = evaluate(&module, &dataset, None);
            assert!(
                !evaluation.output.is_empty(),
                "{} coverage scenario has no successful result",
                case.name
            );

            // Every declared catalog input participates in a successful proof,
            // and at least one such row is duplicated in the raw CSV stream.
            for relation in &dataset.relations {
                let multiplicities = relation.rows.iter().fold(
                    BTreeMap::<Vec<i32>, usize>::new(),
                    |mut counts, row| {
                        *counts.entry(row.clone()).or_default() += 1;
                        counts
                    },
                );
                let active_duplicate = atoms
                    .iter()
                    .filter(|atom| symbol_name(&atom.relation) == relation.name)
                    .any(|atom| {
                        evaluation.assignments.iter().any(|assignment| {
                            let row = project_atom(atom, assignment).unwrap();
                            multiplicities.get(&row).is_some_and(|count| *count >= 2)
                        })
                    });
                assert!(
                    active_duplicate,
                    "{} relation {} has no duplicated row in a successful proof",
                    case.name, relation.name
                );
            }

            let existentials = existential_variables(&module, &variables);
            if !existentials.is_empty() {
                let projected_multiplicity = evaluation.assignments.iter().fold(
                    BTreeMap::<Vec<i32>, usize>::new(),
                    |mut counts, assignment| {
                        let head = module
                            .program
                            .query
                            .head
                            .variables()
                            .map(|variable| assignment[&symbol_name(variable)])
                            .collect::<Vec<_>>();
                        *counts.entry(head).or_default() += 1;
                        counts
                    },
                );
                assert!(
                    projected_multiplicity.values().any(|count| *count >= 2),
                    "{} has existential variables but no two proofs project to one head",
                    case.name
                );
            }

            let counts = candidate_counts(&module, &dataset);
            let synthetic = variables
                .iter()
                .enumerate()
                .map(|(position, variable)| (variable.clone(), position as i32 + 1))
                .collect::<Assignment>();
            for access in accesses.iter().filter(|access| access.is_partial(&atoms)) {
                let guaranteed = prefix_guarantees_key(
                    access,
                    &atoms,
                    &module
                        .program
                        .inputs()
                        .enumerate()
                        .map(|(position, input)| (symbol_name(&input.name), position))
                        .collect(),
                    &synthetic,
                )
                .unwrap();
                if !guaranteed {
                    assert!(
                        counts[access.occurrence].contains(&0),
                        "{} occurrence {} never reaches a missing partial key",
                        case.name,
                        access.occurrence + 1
                    );
                }
                assert!(
                    counts[access.occurrence]
                        .iter()
                        .any(|candidate_count| *candidate_count >= 2),
                    "{} occurrence {} never reaches a multi-candidate bucket",
                    case.name,
                    access.occurrence + 1
                );
                assert!(
                    successful_rows_by_key(&module, &evaluation, access)
                        .values()
                        .any(|rows| rows.len() >= 2),
                    "{} occurrence {} has no key with two surviving complements",
                    case.name,
                    access.occurrence + 1
                );
            }

            for access in accesses
                .iter()
                .filter(|access| access.key_columns.len() >= 2)
            {
                for &column in &access.key_columns {
                    let faulty = evaluate_with_fault(
                        &module,
                        &dataset,
                        None,
                        Some((access.occurrence, column)),
                    );
                    assert!(
                        faulty
                            .output
                            .difference(&evaluation.output)
                            .next()
                            .is_some(),
                        "{} occurrence {} key column {} has no result-observable decoy",
                        case.name,
                        access.occurrence + 1,
                        column
                    );
                }
            }

            let mut reversed = dataset.clone();
            for relation in &mut reversed.relations {
                relation.rows.reverse();
            }
            assert_eq!(
                evaluate(&module, &reversed, None).output,
                evaluation.output,
                "{} depends on raw input order",
                case.name
            );
        }
    }

    #[test]
    fn empty_and_nonempty_no_match_are_separate_scenarios() {
        let mut no_match_not_applicable = BTreeSet::new();
        for case in CLASSIC.iter().chain(RETAIL) {
            let module = case.module().unwrap();
            let empty = generate(case, config(Scenario::EmptyInput, Scale::Tiny, 41)).unwrap();
            assert_eq!(empty.manifest.scenario, Scenario::EmptyInput);
            assert_eq!(
                empty
                    .relations
                    .iter()
                    .filter(|relation| relation.rows.is_empty())
                    .count(),
                1,
                "{} empty-input scenario must empty exactly one relation",
                case.name
            );
            assert!(
                evaluate(&module, &empty, None).output.is_empty(),
                "{} empty-input scenario unexpectedly has results",
                case.name
            );

            match generate(case, config(Scenario::NoMatch, Scale::Tiny, 43)) {
                Ok(no_match) => {
                    assert_eq!(no_match.manifest.scenario, Scenario::NoMatch);
                    assert!(
                        no_match
                            .relations
                            .iter()
                            .all(|relation| !relation.rows.is_empty()),
                        "{} no-match scenario contains an empty input",
                        case.name
                    );
                    assert!(
                        evaluate(&module, &no_match, None).output.is_empty(),
                        "{} no-match scenario unexpectedly has results",
                        case.name
                    );
                }
                Err(GenerateError::ScenarioNotApplicable {
                    scenario: Scenario::NoMatch,
                    ..
                }) => {
                    no_match_not_applicable.insert(case.name);
                }
                Err(error) => panic!("{} no-match generation failed: {error}", case.name),
            }
        }
        assert_eq!(
            no_match_not_applicable,
            BTreeSet::from(["cartesian-product"])
        );
    }

    #[test]
    fn role_playing_self_join_does_not_refill_the_omitted_tuple() {
        let module = syn::parse_str::<cq::Module>(
            r#"
struct ReverseEdgeProgram;
relation Edge(c0: i32, c1: i32);
reverse_edge(x, y) :- Edge(x, y), Edge(y, x);
"#,
        )
        .unwrap();
        let dataset = generate_module(
            &module,
            Suite::Classic,
            "reverse-edge",
            config(Scenario::Coverage, Scale::Tiny, 19),
        )
        .unwrap();

        assert_eq!(
            full_bound_occurrences(&positive_atoms(&module.program.query).unwrap()),
            vec![1]
        );
        assert_memberships_are_observable("reverse-edge", &module, &dataset);
    }

    #[test]
    fn exact_duplicate_membership_is_reported_as_indistinguishable() {
        let module = syn::parse_str::<cq::Module>(
            r#"
struct DuplicateMembershipProgram;
relation R(c0: i32);
duplicate(x) :- R(x), R(x);
"#,
        )
        .unwrap();
        let error = generate_module(
            &module,
            Suite::Classic,
            "duplicate-membership",
            config(Scenario::Coverage, Scale::Tiny, 23),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            GenerateError::IndistinguishableMembership {
                occurrence: 2,
                relation,
            } if relation == "R"
        ));
    }

    #[test]
    fn seed_changes_generated_rows_without_changing_the_schema() {
        let case = CLASSIC.iter().find(|case| case.name == "triangle").unwrap();
        let first = generate(case, config(Scenario::Coverage, Scale::Tiny, 1)).unwrap();
        let second = generate(case, config(Scenario::Coverage, Scale::Tiny, 2)).unwrap();

        assert_ne!(first.relations, second.relations);
        assert_eq!(
            first
                .relations
                .iter()
                .map(|relation| (&relation.name, relation.arity))
                .collect::<Vec<_>>(),
            second
                .relations
                .iter()
                .map(|relation| (&relation.name, relation.arity))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn disconnected_medium_inputs_are_capped_before_the_cartesian_product() {
        let limits = Limits::for_shape(Scale::Medium, 2);
        assert_eq!(limits.distinct_rows, 316);
        assert!(limits.distinct_rows.pow(2) <= 100_000);
        assert!((limits.distinct_rows + 1).pow(2) > 100_000);

        let case = CLASSIC
            .iter()
            .find(|case| case.name == "cartesian-product")
            .unwrap();
        let dataset = generate(case, config(Scenario::Coverage, Scale::Medium, 3)).unwrap();
        assert!(
            dataset
                .manifest
                .relations
                .iter()
                .all(|relation| relation.distinct_rows == 316)
        );
    }

    #[test]
    fn scale_targets_match_the_documented_ranges() {
        assert_eq!(Limits::for_shape(Scale::Tiny, 1).distinct_rows, 96);
        assert_eq!(Limits::for_shape(Scale::Medium, 1).distinct_rows, 25_000);
        assert_eq!(Limits::for_shape(Scale::Large, 1).distinct_rows, 1_000_000);
    }
}
